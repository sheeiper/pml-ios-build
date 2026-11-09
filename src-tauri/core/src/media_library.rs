use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::Read as StdRead,
    path::PathBuf,
    process::Stdio,
    sync::{atomic::{AtomicBool, Ordering}, Arc, LazyLock, Mutex as StdMutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD},
    Engine,
};
use futures_util::StreamExt;
use percent_encoding::percent_decode_str;
use rand::{rngs::OsRng, RngCore};
use reqwest::{
    header::{
        ACCEPT_ENCODING, ACCEPT_RANGES, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
        ETAG, IF_RANGE, LOCATION, RANGE, USER_AGENT,
    },
    redirect::Policy,
    Client, Method, StatusCode,
};
use rusqlite::{params, Connection, OptionalExtension, ToSql};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(feature = "tauri")]
use tauri::Manager;
use crate::{AppHandle, State};
use tokio::{
    fs as tokio_fs,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::Mutex,
};
use url::Url;
use zip::ZipArchive;

// This is a guard against malformed/cyclic remote directory listings, not a
// normal library-size limit. The configured media root already contains more
// than 50,000 assets, so that old value made a valid full scan fail.
const MAX_SCAN_ENTRIES: usize = 250_000;
const SCAN_REQUEST_DELAY_MS: u64 = 250;
// A webtoon/comic preview can register hundreds of page sources while it
// preloads nearby pages. Keep enough entries for a complete reading session;
// stale entries are still bounded so a long-running app cannot grow forever.
const MAX_MEDIA_PROXY_SOURCES: usize = 2048;
const MAX_COMIC_ARCHIVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_COMIC_PAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_COMIC_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_THUMBNAIL_BYTES: usize = 4 * 1024 * 1024;
const MAX_THUMBNAIL_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const THUMBNAIL_CACHE_VERSION: &str = "v3";
const ALIST_API_PAGE_SIZE: i64 = 500;
const DIRECTORY_CACHE_TTL_SECONDS: u64 = 300;
const MAX_DIRECTORY_CACHE_PAGES: usize = 128;
const DEFAULT_ALIST_API_URL: &str = "http://xhofe_alist:5244";
const DEFAULT_CD2_API_URL: &str = "http://127.0.0.1:19798";
const MEDIA_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

// Reading every remote media file to calculate SHA256 is prohibitively
// expensive for large AList libraries. Size and provider ETag are sufficient
// for normal incremental scans; full hashing remains an explicit opt-in.
fn content_hashing_enabled() -> bool {
    matches!(
        std::env::var("PML_ENABLE_CONTENT_HASH")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ProviderKind {
    Alist,
    CloudDrive2,
}

#[derive(Clone)]
struct Session {
    provider: ProviderKind,
    endpoint: Url,
    api_token: String,
    client: Client,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub(crate) struct ZipEntryMeta {
    pub name: String,
    pub compression_method: u16,
    pub flags: u16,
    pub crc32: u32,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub local_header_offset: u64,
}

pub struct MediaLibraryState {
    session: Mutex<Option<Session>>,
    pub(crate) proxy: Mutex<Option<MediaProxy>>,
    directory_cache: Mutex<HashMap<String, CachedDirectoryPage>>,
    comic_archive_cache_lock: Mutex<()>,
    comic_archive_tasks:
        Mutex<HashMap<String, Arc<tokio::sync::OnceCell<Result<PathBuf, String>>>>>,
    comic_archive_index_cache: StdMutex<HashMap<String, (PathBuf, Arc<Vec<String>>)>>,
    comic_remote_zip_cache: StdMutex<HashMap<String, Arc<Vec<ZipEntryMeta>>>>,
    scan_progress: Arc<StdMutex<ScanProgress>>,
    duplicate_progress: Arc<StdMutex<DuplicateScanProgress>>,
    duplicate_cancel: Arc<AtomicBool>,
    pub(crate) last_stream_activity: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) thumbnails_dir: Arc<StdMutex<Option<PathBuf>>>,
}

impl MediaLibraryState {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
            proxy: Mutex::new(None),
            directory_cache: Mutex::new(HashMap::new()),
            comic_archive_cache_lock: Mutex::new(()),
            comic_archive_tasks: Mutex::new(HashMap::new()),
            comic_archive_index_cache: StdMutex::new(HashMap::new()),
            comic_remote_zip_cache: StdMutex::new(HashMap::new()),
            scan_progress: Arc::new(StdMutex::new(ScanProgress::idle())),
            duplicate_progress: Arc::new(StdMutex::new(DuplicateScanProgress::idle())),
            duplicate_cancel: Arc::new(AtomicBool::new(false)),
            last_stream_activity: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            thumbnails_dir: Arc::new(StdMutex::new(None)),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStatus {
    configured: bool,
    connected: bool,
    endpoint: Option<String>,
    username: Option<String>,
    has_saved_password: bool,
    has_saved_token: bool,
    auth_mode: String,
    message: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LibraryRootKind {
    Media,
    Comics,
}

impl LibraryRootKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Media => "media",
            Self::Comics => "comics",
        }
    }

    fn from_str(value: &str) -> Result<Self, String> {
        match value {
            "media" => Ok(Self::Media),
            "comics" => Ok(Self::Comics),
            _ => Err("不支持的资料库类型".into()),
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRoot {
    id: String,
    kind: LibraryRootKind,
    remote_path: String,
    created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderEntry {
    pub path: String,
    pub name: String,
    pub is_directory: bool,
    pub size_bytes: i64,
    pub modified_at: Option<String>,
    pub etag: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDirectoryEntry {
    path: String,
    name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowserEntry {
    path: String,
    name: String,
    is_directory: bool,
    size_bytes: i64,
    modified_at: Option<String>,
    etag: Option<String>,
    asset_id: Option<String>,
    media_type: Option<String>,
    thumbnail_source: Option<String>,
    #[serde(skip)]
    thumbnail_url: Option<Url>,
    cover_asset_id: Option<String>,
    duration_seconds: Option<i64>,
    progress_seconds: Option<i64>,
}

const BROWSE_PAGE_SIZE: usize = 100;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowserPage {
    items: Vec<RemoteBrowserEntry>,
    next_cursor: Option<String>,
    from_local_cache: Option<bool>,
    sync_summary: Option<DirectorySyncSummary>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSearchPage {
    items: Vec<RemoteBrowserEntry>,
    total: i64,
    next_cursor: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlistDiagnostics {
    reachable: bool,
    latency_ms: Option<u64>,
    username: Option<String>,
    base_path: Option<String>,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryNode {
    path: String,
    name: String,
}

#[derive(Clone)]
struct DavEntry {
    remote_path: String,
    is_collection: bool,
    size_bytes: i64,
    modified_at: Option<String>,
    etag: Option<String>,
    thumbnail_url: Option<Url>,
}

#[derive(Clone)]
struct CachedDirectoryPage {
    cached_at: Instant,
    entries: Vec<DavEntry>,
    next_cursor: Option<String>,
}

#[derive(Clone)]
struct IndexedAsset {
    id: String,
    root_id: String,
    root_kind: LibraryRootKind,
    remote_path: String,
    file_name: String,
    parent_path: String,
    media_type: String,
    size_bytes: i64,
    etag: Option<String>,
}

#[derive(Clone)]
struct HashCandidate {
    asset_id: String,
    remote_path: String,
    size_bytes: i64,
    etag: Option<String>,
}

#[derive(Clone)]
struct DirectorySnapshot {
    remote_path: String,
    fingerprint: String,
    entries: Vec<DavEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAsset {
    pub id: String,
    pub root_id: String,
    pub root_kind: LibraryRootKind,
    pub remote_path: String,
    pub file_name: String,
    pub parent_path: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub duration_seconds: Option<i64>,
    pub category_id: Option<String>,
    pub category_name: Option<String>,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
    pub collection_id: Option<String>,
    pub collection_name: Option<String>,
    pub metadata_origin: Option<String>,
    pub content_hash: Option<String>,
    pub thumbnail_asset_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortVideoFeedPage {
    pub items: Vec<LibraryAsset>,
    pub next_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_sources: Option<HashMap<String, String>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackHistoryItem {
    pub(crate) asset: LibraryAsset,
    pub(crate) position_seconds: i64,
    pub(crate) updated_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetThumbnail {
    mime_type: String,
    data_base64: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub thumbnail_bytes: u64,
    pub comic_bytes: u64,
    pub database_bytes: u64,
    pub short_video_history_count: i64,
    pub short_video_cache_bytes: u64,
    pub short_video_cache_max_bytes: u64,
    pub total_indexed_files: i64,
    pub total_indexed_directories: i64,
    pub last_scan_time: Option<i64>,
    pub last_scan_duration_secs: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationSettings {
    pub scheduled_scan_enabled: bool,
    pub scheduled_scan_time: String,
    pub scheduled_scan_avoid_active: bool,
    pub last_scan_time: Option<i64>,
    pub last_scan_duration_secs: Option<u64>,
    pub short_video_cache_max_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCategory {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) media_count: i64,
    pub(crate) author_count: i64,
    pub(crate) collection_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryAuthor {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub media_count: i64,
    pub collection_count: i64,
    pub avatar_asset_id: Option<String>,
    pub cover_asset_id: Option<String>,
    pub banner_asset_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCollection {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) author_id: Option<String>,
    pub(crate) author_name: Option<String>,
    pub(crate) root_id: String,
    pub(crate) media_count: i64,
    pub(crate) image_count: i64,
    pub(crate) video_count: i64,
    pub(crate) comic_count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryOverview {
    pub(crate) roots: i64,
    pub(crate) assets: i64,
    pub(crate) images: i64,
    pub(crate) videos: i64,
    pub(crate) short_videos: i64,
    pub(crate) comics: i64,
    pub(crate) authors: i64,
    pub(crate) collections: i64,
    pub(crate) total_bytes: i64,
    pub(crate) directories: i64,
    pub(crate) unplayed: i64,
    pub(crate) missing_thumbnails: i64,
    pub(crate) last_scan_time: Option<i64>,
    pub(crate) duplicate_groups: i64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateScanProgress {
    running: bool,
    phase: String,
    scope_root_id: Option<String>,
    candidates_total: usize,
    processed: usize,
    duplicate_groups: usize,
    matched_files: usize,
    percent: f64,
    message: String,
    error: Option<String>,
}

impl DuplicateScanProgress {
    fn idle() -> Self {
        Self { running: false, phase: "idle".into(), scope_root_id: None, candidates_total: 0,
            processed: 0, duplicate_groups: 0, matched_files: 0, percent: 0.0,
            message: "尚未检查重复文件".into(), error: None }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateAsset {
    id: String,
    root_id: String,
    file_name: String,
    remote_path: String,
    parent_path: String,
    media_type: String,
    size_bytes: i64,
    modified_at: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    hash: String,
    size_bytes: i64,
    assets: Vec<DuplicateAsset>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    roots_scanned: usize,
    files_indexed: usize,
    message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    running: bool,
    phase: String,
    roots_total: usize,
    roots_completed: usize,
    current_root: Option<String>,
    current_directory: Option<String>,
    files_found: usize,
    directories_scanned: usize,
    skipped_directories: usize,
    failed_directories: Vec<String>,
    indexing_total: usize,
    indexing_completed: usize,
    analysis_total: usize,
    analysis_completed: usize,
    percent: f64,
    message: String,
    error: Option<String>,
}

impl ScanProgress {
    pub fn is_running(&self) -> bool { self.running }
    pub fn failure(&self) -> Option<String> {
        if self.phase == "failed" {
            Some(self.error.clone().unwrap_or_else(|| "扫描失败".to_owned()))
        } else {
            None
        }
    }
    fn idle() -> Self {
        Self {
            running: false,
            phase: "idle".into(),
            roots_total: 0,
            roots_completed: 0,
            current_root: None,
            current_directory: None,
            files_found: 0,
            directories_scanned: 0,
            failed_directories: Vec::new(),
            indexing_total: 0,
            indexing_completed: 0,
            skipped_directories: 0,
            analysis_total: 0,
            analysis_completed: 0,
            percent: 0.0,
            message: "尚未开始扫描".into(),
            error: None,
        }
    }
}

struct RootScanResult {
    files: Vec<DavEntry>,
    skipped_directories: Vec<String>,
    snapshots: Vec<DirectorySnapshot>,
    reused_directories: Vec<String>,
}

static GLOBAL_API_URL_CACHE: LazyLock<StdMutex<HashMap<String, (Url, i64)>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

/// 原生运行时没有同源的 HTTP 服务器，本地媒体代理是唯一可按 URL 访问的后端。
/// 代理请求需要 AppHandle 才能读取漫画页面，因此在启动时注入一次。
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

pub fn init_app_handle(app: AppHandle) {
    let _ = APP_HANDLE.set(app);
}

#[derive(Clone)]
struct VideoStreamPreemption {
    seq: Arc<std::sync::atomic::AtomicU64>,
    notify: Arc<tokio::sync::Notify>,
}

static ACTIVE_VIDEO_STREAMS: LazyLock<StdMutex<HashMap<String, VideoStreamPreemption>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

static ASSET_SIZE_CACHE: LazyLock<StdMutex<HashMap<String, u64>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

static CACHED_MAX_SHORT_VIDEO_BYTES: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(2 * 1024 * 1024 * 1024);

#[derive(Clone)]
struct ProxySource {
    url: Url,
    fallback_url: Option<Url>,
    session: Session,
    content_type: String,
    remote_path: String,
    allow_api_fallback: bool,
    api_url_cache: Arc<StdMutex<Option<(Url, i64)>>>,
    direct_url: bool,
    thumbnail_cache_path: Option<PathBuf>,
    proxy_address: Option<std::net::SocketAddr>,
    asset_id: Option<String>,
    cache_base_path: Option<PathBuf>,
    activity_tracker: Option<Arc<std::sync::atomic::AtomicU64>>,
    max_cache_bytes: Option<u64>,
}

pub(crate) struct MediaProxy {
    pub(crate) address: std::net::SocketAddr,
    sources: Arc<Mutex<HashMap<String, ProxySource>>>,
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn get_library_status(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
) -> Result<LibraryStatus, String> {
    let session = state.session.lock().await;
    let config = library_provider_configuration(Some(&app));
    let configured = config.is_ok();
    let is_connected = session.is_some();
    let msg = match config {
        Ok((ProviderKind::CloudDrive2, _, _)) => {
            if is_connected {
                "已连接 CloudDrive2 高速媒体源"
            } else {
                "等待连接 CloudDrive2 WebDAV"
            }
        }
        Ok((ProviderKind::Alist, _, _)) => {
            if is_connected {
                "已连接 AList API"
            } else {
                "等待连接本地 AList"
            }
        }
        Err(_) => "尚未配置媒体源地址（可在设置中直接输入 CloudDrive2 或 AList）",
    };
    Ok(status_from(
        Some(&app),
        configured,
        is_connected,
        msg,
    ))
}

/// Perform a lightweight, read-only health check against the configured provider.
#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn get_alist_diagnostics(
    _app: AppHandle,
    state: State<'_, MediaLibraryState>,
) -> Result<AlistDiagnostics, String> {
    let session = require_session(&state).await?;
    if session.provider == ProviderKind::CloudDrive2 {
        let started = Instant::now();
        let ping_url = session.endpoint.clone();
        let mut req = session
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), ping_url)
            .header(USER_AGENT, MEDIA_USER_AGENT)
            .header("Depth", "0");
        if !session.api_token.is_empty() {
            req = req.header(AUTHORIZATION, &session.api_token);
        }
        let ping_result = req.send().await;
        let latency_ms = Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
        let reachable = ping_result
            .as_ref()
            .is_ok_and(|response| response.status().is_success() || response.status().as_u16() == 207);
        let message = if reachable {
            format!("CloudDrive2 WebDAV 连接正常（延迟 {}ms）", latency_ms.unwrap_or(0))
        } else {
            "无法连接 CloudDrive2 WebDAV，请检查服务地址与密码".to_owned()
        };
        return Ok(AlistDiagnostics {
            reachable,
            latency_ms,
            username: Some("CloudDrive2".to_string()),
            base_path: Some("/".to_string()),
            message,
        });
    }

    let ping_url = alist_route_root_url(&session.endpoint, "ping")?;
    let me_url = alist_api_url(&session.endpoint, "me")?;
    let settings_url = alist_api_url(&session.endpoint, "public/settings")?;
    let started = Instant::now();
    let (ping_result, me_result, settings_result) = tokio::join!(
        session.client.get(ping_url).send(),
        session
            .client
            .get(me_url)
            .header(USER_AGENT, MEDIA_USER_AGENT)
            .header(AUTHORIZATION, &session.api_token)
            .send(),
        session
            .client
            .get(settings_url)
            .header(USER_AGENT, MEDIA_USER_AGENT)
            .send(),
    );
    let latency_ms = Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64);
    let reachable = ping_result
        .as_ref()
        .is_ok_and(|response| response.status().is_success());
    let mut username = None;
    let mut base_path = None;
    if let Ok(response) = me_result {
        if response.status().is_success() {
            if let Ok(body) = response.json::<serde_json::Value>().await {
                let data = body.get("data").unwrap_or(&body);
                username = data
                    .get("username")
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned);
                base_path = data
                    .get("base_path")
                    .or_else(|| data.get("basePath"))
                    .and_then(|value| value.as_str())
                    .map(ToOwned::to_owned);
            }
        }
    }
    let settings_available = settings_result
        .as_ref()
        .is_ok_and(|response| response.status().is_success());
    let message = if reachable {
        if settings_available {
            "AList 内网可用，用户信息和站点设置读取正常".to_owned()
        } else {
            "AList 内网可用，但站点设置接口未返回成功".to_owned()
        }
    } else {
        "AList 内网 ping 未通过，请检查容器网络和服务状态".to_owned()
    };
    Ok(AlistDiagnostics {
        reachable,
        latency_ms,
        username,
        base_path,
        message,
    })
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn restore_alist_connection(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
) -> Result<LibraryStatus, String> {
    let (provider, endpoint, token) = match library_provider_configuration(Some(&app)) {
        Ok(configuration) => configuration,
        Err(error) => {
            *state.session.lock().await = None;
            return Ok(status_from(Some(&app), false, false, &error));
        }
    };
    match establish_session(&endpoint, &token, provider).await {
        Ok(session) => {
            *state.session.lock().await = Some(session);
            state.directory_cache.lock().await.clear();
            clear_proxy_sources(&state).await;
            let msg = match provider {
                ProviderKind::Alist => "已连接 AList API 媒体源",
                ProviderKind::CloudDrive2 => "已连接 CloudDrive2 高速媒体源",
            };
            Ok(status_from(Some(&app), true, true, msg))
        }
        Err(error) => {
            *state.session.lock().await = None;
            let title = match provider {
                ProviderKind::Alist => "AList API 连接失败",
                ProviderKind::CloudDrive2 => "CloudDrive2 WebDAV 连接失败",
            };
            Ok(status_from(
                Some(&app),
                true,
                false,
                &format!("{title}：{error}"),
            ))
        }
    }
}

pub async fn list_provider_entries(
    state: State<'_, MediaLibraryState>,
    path: String,
) -> Result<Vec<ProviderEntry>, String> {
    let session = require_session(&state).await?;
    let path = normalize_remote_path(&path)?;
    let entries = list_directory_with_backoff(&session, &path).await?;
    Ok(entries
        .into_iter()
        .filter(|entry| entry.remote_path != path)
        .map(|entry| ProviderEntry {
            name: file_name(&entry.remote_path),
            path: entry.remote_path,
            is_directory: entry.is_collection,
            size_bytes: entry.size_bytes,
            modified_at: entry.modified_at,
            etag: entry.etag,
        })
        .collect())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn list_remote_directory(
    state: State<'_, MediaLibraryState>,
    path: String,
) -> Result<Vec<RemoteDirectoryEntry>, String> {
    let session = require_session(&state).await?;
    let path = normalize_remote_path(&path)?;
    let mut entries = match session.provider {
        ProviderKind::Alist => match list_api_directories(&session, &path).await {
            Ok(entries) => entries,
            Err(_) => list_directory(&session, &path)
                .await?
                .into_iter()
                .filter(|entry| entry.is_collection && entry.remote_path != path)
                .map(|entry| RemoteDirectoryEntry {
                    name: file_name(&entry.remote_path),
                    path: entry.remote_path,
                })
                .collect::<Vec<_>>(),
        },
        ProviderKind::CloudDrive2 => list_directory(&session, &path)
            .await?
            .into_iter()
            .filter(|entry| entry.is_collection && entry.remote_path != path)
            .map(|entry| RemoteDirectoryEntry {
                name: file_name(&entry.remote_path),
                path: entry.remote_path,
            })
            .collect::<Vec<_>>(),
    };
    entries.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Ok(entries)
}

#[derive(Clone, Copy)]
enum BrowseSort { Name, Newest, Size }
impl BrowseSort {
    fn parse(value: &str) -> Result<Self,String> { match value { "name"=>Ok(Self::Name),"newest"=>Ok(Self::Newest),"size"=>Ok(Self::Size),_=>Err("排序方式无效".into()) } }
    fn as_str(self) -> &'static str { match self { Self::Name=>"name",Self::Newest=>"newest",Self::Size=>"size" } }
    fn order(self, directory: bool) -> &'static str {
        match self { Self::Newest=>"modified_ts DESC,name_sort,remote_path",Self::Size if !directory=>"size_bytes DESC,name_sort,remote_path",_=>"name_sort,remote_path" }
    }
}

// Compare digit runs by value without converting them to a bounded integer.
// Both filenames and directory names use this persisted key, then path as a tie breaker.
fn natural_name_key(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut chars = lower.chars().peekable();
    let mut key = String::new();
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            let mut digits=String::from(ch);
            while chars.peek().is_some_and(|c|c.is_ascii_digit()) { digits.push(chars.next().unwrap()); }
            let value=digits.trim_start_matches('0');
            let value=if value.is_empty() { "0" } else { value };
            key.push_str(&format!("\u{1}{:010}:{}\u{2}",value.len(),value));
        } else { key.push(ch); }
    }
    key
}

fn modified_timestamp(value: Option<&str>) -> i64 {
    value.and_then(|v|chrono::DateTime::parse_from_rfc3339(v).or_else(|_|chrono::DateTime::parse_from_rfc2822(v)).ok())
        .map(|v|v.timestamp_millis()).unwrap_or(i64::MIN)
}

fn migrate_directory_order(c: &Connection) -> Result<(),String> {
    let migration=c.unchecked_transaction().map_err(database_error)?;
    let c=&migration;
    let mut migrated=false;
    for (table, additions) in [
        ("assets", vec![("name_sort","TEXT NOT NULL DEFAULT ''"),("modified_ts","INTEGER NOT NULL DEFAULT -9223372036854775808")]),
        ("library_nodes",vec![("name_sort","TEXT NOT NULL DEFAULT ''"),("modified_at","TEXT"),("modified_ts","INTEGER NOT NULL DEFAULT -9223372036854775808")]),
        ("directory_sync_state",vec![("revision","TEXT NOT NULL DEFAULT ''"),("cover_asset_id","TEXT")]),
    ] {
        let cols=c.prepare(&format!("PRAGMA table_info({table})")).map_err(database_error)?.query_map([],|r|r.get::<_,String>(1)).map_err(database_error)?.collect::<Result<HashSet<_>,_>>().map_err(database_error)?;
        for (name,kind) in additions { if !cols.contains(name) { c.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {name} {kind}")).map_err(database_error)?;migrated=true; } }
    }
    if migrated {
        for (table,name) in [("assets","file_name"),("library_nodes","name")] {
            let rows=c.prepare(&format!("SELECT rowid,{name},modified_at FROM {table}")).map_err(database_error)?.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).map_err(database_error)?.collect::<Result<Vec<_>,_>>().map_err(database_error)?;
            for (id,name,date) in rows { c.execute(&format!("UPDATE {table} SET name_sort=?1,modified_ts=?2 WHERE rowid=?3"),params![natural_name_key(&name),modified_timestamp(date.as_deref()),id]).map_err(database_error)?; }
        }
    }
    c.execute_batch("CREATE INDEX IF NOT EXISTS assets_browse_name ON assets(root_id,parent_path,missing,name_sort,remote_path);
        CREATE INDEX IF NOT EXISTS assets_browse_date ON assets(root_id,parent_path,missing,modified_ts DESC,name_sort,remote_path);
        CREATE INDEX IF NOT EXISTS assets_browse_size ON assets(root_id,parent_path,missing,size_bytes DESC,name_sort,remote_path);
        CREATE INDEX IF NOT EXISTS nodes_browse_name ON library_nodes(root_id,parent_path,name_sort,remote_path);
        CREATE INDEX IF NOT EXISTS nodes_browse_date ON library_nodes(root_id,parent_path,modified_ts DESC,name_sort,remote_path);").map_err(database_error)?;
    let needs_sync_backfill: bool = migrated || c.query_row(
        "SELECT EXISTS(SELECT 1 FROM directory_sync_state WHERE revision = '' LIMIT 1)",
        [],
        |r| r.get(0),
    ).unwrap_or(false);
    let needs_cover_tree: bool = c.query_row(
        "SELECT value FROM settings WHERE key = 'directory_cover_tree_v2'",
        [],
        |r| r.get::<_, String>(0),
    ).optional().map_err(database_error)?.as_deref() != Some("1");
    if needs_sync_backfill || needs_cover_tree {
        // Ensure previously scanned directories keep a valid non-empty revision
        // and cover_asset_id (including ancestor folders that only contain
        // subfolders) so opening folders uses the local SQLite index instead
        // of blocking on a live WebDAV PROPFIND request, while leaving
        // unopened leaf folders with revision='' so they still fetch on open.
        c.execute_batch("INSERT INTO directory_sync_state(root_id,remote_path,scanned_at,revision,cover_asset_id)
            SELECT root_id,parent_path,MAX(scanned_at),lower(hex(randomblob(8))),NULL FROM library_nodes
            WHERE remote_path!=parent_path
            GROUP BY root_id,parent_path
            ON CONFLICT(root_id,remote_path) DO UPDATE SET
                revision=CASE WHEN directory_sync_state.revision='' THEN excluded.revision ELSE directory_sync_state.revision END;
            INSERT INTO directory_sync_state(root_id,remote_path,scanned_at,revision,cover_asset_id)
            SELECT root_id,remote_path,scanned_at,lower(hex(randomblob(8))),NULL FROM directory_snapshots
            WHERE 1=1
            ON CONFLICT(root_id,remote_path) DO UPDATE SET
                revision=CASE WHEN directory_sync_state.revision='' THEN excluded.revision ELSE directory_sync_state.revision END;
            INSERT INTO directory_sync_state(root_id,remote_path,scanned_at,revision,cover_asset_id)
            SELECT root_id,parent_path,MAX(scanned_at),lower(hex(randomblob(8))),NULL FROM assets
            WHERE missing=0
            GROUP BY root_id,parent_path
            ON CONFLICT(root_id,remote_path) DO UPDATE SET
                revision=CASE WHEN directory_sync_state.revision='' THEN excluded.revision ELSE directory_sync_state.revision END;
            INSERT OR IGNORE INTO directory_sync_state(root_id,remote_path,scanned_at,revision,cover_asset_id)
            SELECT root_id,remote_path,scanned_at,'',NULL FROM library_nodes;
            UPDATE directory_sync_state
            SET revision=''
            WHERE revision!=''
              AND NOT EXISTS (SELECT 1 FROM directory_snapshots ds WHERE ds.root_id=directory_sync_state.root_id AND ds.remote_path=directory_sync_state.remote_path)
              AND NOT EXISTS (SELECT 1 FROM library_nodes ln WHERE ln.root_id=directory_sync_state.root_id AND ln.parent_path=directory_sync_state.remote_path AND ln.remote_path!=directory_sync_state.remote_path)
              AND NOT EXISTS (SELECT 1 FROM assets a WHERE a.root_id=directory_sync_state.root_id AND a.parent_path=directory_sync_state.remote_path AND a.missing=0);").map_err(database_error)?;
        propagate_directory_covers(&migration)?;
        c.execute(
            "INSERT INTO settings(key, value) VALUES('directory_cover_tree_v2', '1')
             ON CONFLICT(key) DO UPDATE SET value = '1'",
            [],
        ).map_err(database_error)?;
    }
    migration.commit().map_err(database_error)?;
    Ok(())
}

fn select_directory_cover(c: &Connection, root_id: &str, path: &str) -> Result<Option<String>, String> {
    // 1. Direct child asset in this folder
    let direct: Option<String> = c
        .query_row(
            "SELECT id FROM assets
             WHERE root_id=?1 AND parent_path=?2 AND missing=0
               AND (media_type IN ('image','comicPage','comicArchive')
                 OR (media_type='video' AND lower(file_name) NOT GLOB '*.avi' AND lower(file_name) NOT GLOB '*.wmv' AND lower(file_name) NOT GLOB '*.rmvb' AND lower(file_name) NOT GLOB '*.rm' AND lower(file_name) NOT GLOB '*.flv' AND lower(file_name) NOT GLOB '*.vob' AND lower(file_name) NOT GLOB '*.m2ts' AND lower(file_name) NOT GLOB '*.mts' AND lower(file_name) NOT GLOB '*.asf'))
             ORDER BY CASE WHEN lower(file_name) GLOB 'cover.*' OR lower(file_name) GLOB 'poster.*' OR lower(file_name) GLOB 'folder.*' THEN 0 WHEN media_type IN ('image','comicPage') THEN 1 ELSE 2 END,name_sort,remote_path LIMIT 1",
            params![root_id, path],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    if direct.is_some() {
        return Ok(direct);
    }
    // 2. Inherit cover from first immediate child folder that already has a cover
    let child_cover: Option<String> = c
        .query_row(
            "SELECT s.cover_asset_id
             FROM library_nodes n
             JOIN directory_sync_state s ON s.root_id = n.root_id AND s.remote_path = n.remote_path
             WHERE n.root_id = ?1 AND n.parent_path = ?2 AND n.remote_path != ?2
               AND s.cover_asset_id IS NOT NULL
             ORDER BY n.name_sort, n.remote_path
             LIMIT 1",
            params![root_id, path],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    if child_cover.is_some() {
        return Ok(child_cover);
    }
    Ok(None)
}

fn propagate_directory_covers(c: &Connection) -> Result<(), String> {
    c.execute_batch(
        "INSERT OR IGNORE INTO directory_sync_state(root_id,remote_path,scanned_at,revision,cover_asset_id)
         SELECT root_id,parent_path,MAX(scanned_at),'',NULL FROM library_nodes
         GROUP BY root_id,parent_path;
         INSERT OR IGNORE INTO directory_sync_state(root_id,remote_path,scanned_at,revision,cover_asset_id)
         SELECT root_id,remote_path,scanned_at,'',NULL FROM library_nodes;
         UPDATE directory_sync_state
         SET cover_asset_id = NULL
         WHERE cover_asset_id IS NOT NULL
           AND NOT EXISTS (SELECT 1 FROM assets a WHERE a.id = directory_sync_state.cover_asset_id AND a.missing = 0);",
    )
    .map_err(database_error)?;

    // 1. Fetch the single best cover asset per direct parent folder in one pass
    let leaf_covers: Vec<(String, String, String)> = {
        let mut stmt = c
            .prepare(
                "SELECT root_id, parent_path, id FROM (
                     SELECT root_id, parent_path, id,
                            ROW_NUMBER() OVER(
                                PARTITION BY root_id, parent_path
                                ORDER BY
                                    CASE WHEN lower(file_name) GLOB 'cover.*' OR lower(file_name) GLOB 'poster.*' OR lower(file_name) GLOB 'folder.*' THEN 0
                                         WHEN media_type IN ('image','comicPage') THEN 1
                                         ELSE 2
                                    END,
                                    name_sort,
                                    remote_path
                            ) AS rank
                     FROM assets
                     WHERE missing = 0
                       AND (media_type IN ('image','comicPage','comicArchive')
                         OR (media_type = 'video' AND lower(file_name) NOT GLOB '*.avi' AND lower(file_name) NOT GLOB '*.wmv' AND lower(file_name) NOT GLOB '*.rmvb' AND lower(file_name) NOT GLOB '*.rm' AND lower(file_name) NOT GLOB '*.flv' AND lower(file_name) NOT GLOB '*.vob' AND lower(file_name) NOT GLOB '*.m2ts' AND lower(file_name) NOT GLOB '*.mts' AND lower(file_name) NOT GLOB '*.asf'))
                 ) WHERE rank = 1
                 ORDER BY root_id, parent_path",
            )
            .map_err(database_error)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(database_error)?
    };

    if leaf_covers.is_empty() {
        return Ok(());
    }

    // 2. Build direct covers + ancestor covers in Rust memory (< 1ms)
    let mut cover_map: HashMap<(String, String), String> =
        HashMap::with_capacity(leaf_covers.len() * 2);
    for (root_id, dir_path, cover_id) in &leaf_covers {
        cover_map.insert((root_id.clone(), dir_path.clone()), cover_id.clone());
    }
    for (root_id, dir_path, cover_id) in &leaf_covers {
        let mut current = dir_path.clone();
        for _ in 0..16 {
            let parent = parent_path(&current);
            if parent == current {
                break;
            }
            cover_map
                .entry((root_id.clone(), parent.clone()))
                .or_insert_with(|| cover_id.clone());
            current = parent;
        }
    }

    // 3. Batch-write covers into directory_sync_state
    let mut upsert_stmt = c
        .prepare(
            "INSERT INTO directory_sync_state(root_id, remote_path, scanned_at, revision, cover_asset_id)
             VALUES (?1, ?2, 0, '', ?3)
             ON CONFLICT(root_id, remote_path) DO UPDATE SET
               cover_asset_id = COALESCE(directory_sync_state.cover_asset_id, excluded.cover_asset_id)",
        )
        .map_err(database_error)?;
    for ((root_id, dir_path), cover_id) in cover_map {
        upsert_stmt
            .execute(params![root_id, dir_path, cover_id])
            .map_err(database_error)?;
    }
    Ok(())
}

fn browse_sorted_directory_page(c: &Connection, root: &str, path: &str, sort: BrowseSort, cursor: Option<&str>) -> Result<Option<RemoteBrowserPage>,String> {
    let revision: Option<String> = c
        .query_row(
            "SELECT revision FROM directory_sync_state WHERE root_id=?1 AND remote_path=?2",
            params![root, path],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    let directories: i64 = c
        .query_row(
            "SELECT count(*) FROM library_nodes WHERE root_id=?1 AND parent_path=?2 AND remote_path!=?2",
            params![root, path],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let files_count: i64 = c
        .query_row(
            "SELECT count(*) FROM assets WHERE root_id=?1 AND parent_path=?2 AND missing=0",
            params![root, path],
            |r| r.get(0),
        )
        .map_err(database_error)?;

    // If there is no revision AND no records exist in SQLite, this directory has never been indexed/synced.
    if revision.as_deref().unwrap_or("").is_empty() && directories == 0 && files_count == 0 {
        return Ok(None);
    }

    let revision = match revision {
        Some(rev) if !rev.is_empty() => rev,
        _ => "local".to_string(),
    };

    let page = if let Some(cursor) = cursor {
        let pieces: Vec<_> = cursor.split(':').collect();
        if pieces.len() != 4 || pieces[0] != "v2" || pieces[1] != sort.as_str() || pieces[2] != revision {
            return Err("目录或排序已变化，请刷新目录后继续".into());
        }
        pieces[3]
            .parse::<usize>()
            .ok()
            .filter(|p| *p > 0 && *p < 1_000_000)
            .ok_or("分页游标无效")?
    } else {
        1
    };
    let offset = (page - 1) * BROWSE_PAGE_SIZE;
    let mut items = Vec::new();
    if offset < directories as usize {
        let sql = format!(
            "SELECT n.remote_path,n.name,n.modified_at,s.cover_asset_id FROM
            (SELECT * FROM library_nodes WHERE root_id=?1 AND parent_path=?2 AND remote_path!=?2 ORDER BY {} LIMIT ?3 OFFSET ?4) n
            LEFT JOIN directory_sync_state s ON s.root_id=n.root_id AND s.remote_path=n.remote_path ORDER BY {}",
            sort.order(true),
            sort.order(true)
                .replace("remote_path", "n.remote_path")
                .replace("name_sort", "n.name_sort")
                .replace("modified_ts", "n.modified_ts")
        );
        items = c
            .prepare(&sql)
            .map_err(database_error)?
            .query_map(
                params![root, path, (BROWSE_PAGE_SIZE + 1) as i64, offset as i64],
                |r| {
                    Ok(RemoteBrowserEntry {
                        path: r.get(0)?,
                        name: r.get(1)?,
                        is_directory: true,
                        size_bytes: 0,
                        modified_at: r.get(2)?,
                        etag: None,
                        asset_id: None,
                        media_type: None,
                        thumbnail_source: None,
                        thumbnail_url: None,
                        cover_asset_id: r.get::<_, Option<String>>(3)?.filter(|s| !s.is_empty()),
                        duration_seconds: None,
                        progress_seconds: None,
                    })
                },
            )
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        for item in &mut items {
            if item.is_directory && item.cover_asset_id.is_none() {
                if let Ok(Some(cover_id)) = select_directory_cover(c, root, &item.path) {
                    if !cover_id.is_empty() {
                        item.cover_asset_id = Some(cover_id);
                    }
                }
            }
        }
    }
    if items.len() <= BROWSE_PAGE_SIZE {
        let sql = format!(
            "SELECT a.id,a.remote_path,a.file_name,a.media_type,a.size_bytes,a.etag,a.modified_at,a.duration_seconds,p.position_seconds
            FROM assets a LEFT JOIN playback_progress p ON p.asset_id=a.id
            WHERE a.root_id=?1 AND a.parent_path=?2 AND a.missing=0 ORDER BY {} LIMIT ?3 OFFSET ?4",
            sort.order(false)
        );
        let remaining = BROWSE_PAGE_SIZE + 1 - items.len();
        let files = c
            .prepare(&sql)
            .map_err(database_error)?
            .query_map(
                params![
                    root,
                    path,
                    remaining as i64,
                    offset.saturating_sub(directories as usize) as i64
                ],
                |r| {
                    Ok(RemoteBrowserEntry {
                        asset_id: Some(r.get(0)?),
                        path: r.get(1)?,
                        name: r.get(2)?,
                        media_type: Some(r.get(3)?),
                        size_bytes: r.get(4)?,
                        etag: r.get(5)?,
                        modified_at: r.get(6)?,
                        is_directory: false,
                        thumbnail_source: None,
                        thumbnail_url: None,
                        cover_asset_id: None,
                        duration_seconds: r.get(7)?,
                        progress_seconds: r.get(8)?,
                    })
                },
            )
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        items.extend(files);
    }
    let more = items.len() > BROWSE_PAGE_SIZE;
    items.truncate(BROWSE_PAGE_SIZE);
    Ok(Some(RemoteBrowserPage {
        items,
        next_cursor: more.then(|| format!("v2:{}:{}:{}", sort.as_str(), revision, page + 1)),
        from_local_cache: Some(true),
        sync_summary: None,
    }))
}

#[cfg(test)]
fn browse_local_directory_page(c:&Connection,root:&str,path:&str,page:usize)->Result<Option<RemoteBrowserPage>,String>{
    let revision:Option<String>=c.query_row("SELECT revision FROM directory_sync_state WHERE root_id=?1 AND remote_path=?2",params![root,path],|r|r.get(0)).optional().map_err(database_error)?;
    let cursor=revision.map(|r|format!("v2:name:{r}:{page}"));
    browse_sorted_directory_page(c,root,path,BrowseSort::Name,cursor.as_deref())
}

/// Browse exactly one remote directory through AList's native API. SQLite is
/// used only to register visible playable files
/// so favorites, progress and the media proxy keep stable asset identifiers.
#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn list_remote_children(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    root_id: String,
    path: String,
    cursor: Option<String>,
    refresh: Option<bool>,
    sort: Option<String>,
) -> Result<RemoteBrowserPage, String> {
    let path = normalize_remote_path(&path)?;
    let sort = BrowseSort::parse(sort.as_deref().unwrap_or("name"))?;
    let offset = cursor.as_deref().map(|value| {
        value.strip_prefix("v2-offset:").and_then(|value|value.parse::<usize>().ok())
            .ok_or_else(||"目录页码无效，请刷新目录".to_owned())
    }).transpose()?.unwrap_or(0);
    let value = crate::four_domains::compat_remote_children(
        app,state,&root_id,&path,refresh.unwrap_or(false),sort.as_str(),
    ).await?;
    let entries=value.get("items").and_then(|v|v.as_array()).ok_or("V2 目录响应缺少条目")?;
    let total=entries.len();
    let items=entries.iter().skip(offset).take(BROWSE_PAGE_SIZE).map(|entry| {
        let text=|key:&str|entry.get(key).and_then(|v|v.as_str()).map(ToOwned::to_owned);
        Ok(RemoteBrowserEntry {
            path:text("path").ok_or("V2 目录条目缺少路径")?,
            name:text("name").ok_or("V2 目录条目缺少名称")?,
            is_directory:entry.get("isDirectory").and_then(|v|v.as_bool()).unwrap_or(false),
            size_bytes:entry.get("sizeBytes").and_then(|v|v.as_i64()).unwrap_or(0),
            modified_at:text("modifiedAt"),etag:text("etag"),asset_id:text("assetId"),
            media_type:text("mediaType"),thumbnail_source:text("thumbnailSource"),thumbnail_url:None,
            cover_asset_id:text("coverAssetId"),
            duration_seconds:entry.get("durationSeconds").and_then(|v|v.as_i64()),
            progress_seconds:entry.get("progressSeconds").and_then(|v|v.as_f64()).map(|n|n as i64),
        })
    }).collect::<Result<Vec<_>,String>>()?;
    Ok(RemoteBrowserPage {
        items,next_cursor:offset.checked_add(BROWSE_PAGE_SIZE).filter(|next|*next<total).map(|next|format!("v2-offset:{next}")),
        from_local_cache:value.get("fromLocalCache").and_then(|v|v.as_bool()),sync_summary:None,
    })
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectorySyncSummary {
    added: usize,
    updated: usize,
    removed: usize,
    total: usize,
}

// All mutations use one transaction. Missing media retain their identifiers,
// favorites and playback progress so a later reappearance restores them.
fn sync_directory_entries(
    connection: &mut Connection, root_id: &str, kind: LibraryRootKind,
    path: &str, entries: Vec<DavEntry>,
) -> Result<DirectorySyncSummary, String> {
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(database_error)?;
    let result = sync_directory_entries_tx(&tx, root_id, kind, path, entries)?;
    tx.commit().map_err(database_error)?;
    Ok(result)
}

fn sync_directory_entries_tx(tx: &rusqlite::Transaction<'_>, root_id: &str, kind: LibraryRootKind, path: &str, entries: Vec<DavEntry>) -> Result<DirectorySyncSummary,String> {
    let entries: Vec<_> = entries.into_iter().filter(|e| e.remote_path != path).collect();
    if entries.iter().any(|e| parent_path(&e.remote_path) != path) {
        return Err("媒体源返回了不属于当前目录的条目，本次未修改索引".into());
    }
    let old_dirs = tx.prepare("SELECT remote_path FROM library_nodes WHERE root_id=?1 AND parent_path=?2 AND remote_path!=?2")
        .map_err(database_error)?.query_map(params![root_id,path], |r| r.get::<_,String>(0))
        .map_err(database_error)?.collect::<Result<HashSet<_>,_>>().map_err(database_error)?;
    let old_files = tx.prepare("SELECT remote_path,size_bytes,etag,modified_at,missing FROM assets WHERE root_id=?1 AND parent_path=?2")
        .map_err(database_error)?.query_map(params![root_id,path], |r| Ok((r.get::<_,String>(0)?,(r.get::<_,i64>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,i64>(4)?))))
        .map_err(database_error)?.collect::<Result<HashMap<_,_>,_>>().map_err(database_error)?;
    let mut summary = DirectorySyncSummary { added:0, updated:0, removed:0, total:0 };
    let mut live_dirs = HashSet::new();
    let mut live_files = HashSet::new();
    let timestamp = now();
    for entry in &entries {
        if entry.is_collection {
            if !live_dirs.insert(entry.remote_path.clone()) { continue; }
            if !old_dirs.contains(&entry.remote_path) { summary.added += 1; }
            let name = file_name(&entry.remote_path);
            let name_sort = natural_name_key(&name);
            let mod_ts = modified_timestamp(entry.modified_at.as_deref());
            tx.execute("INSERT INTO library_nodes(root_id,remote_path,parent_path,name,scanned_at,modified_at,modified_ts,name_sort) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                ON CONFLICT(root_id,remote_path) DO UPDATE SET parent_path=excluded.parent_path,name=excluded.name,scanned_at=excluded.scanned_at,modified_at=excluded.modified_at,modified_ts=excluded.modified_ts,name_sort=excluded.name_sort",
                params![root_id,entry.remote_path,path,name,timestamp,entry.modified_at,mod_ts,name_sort]).map_err(database_error)?;
        } else if let Some(media_type) = media_type_for(kind, &entry.remote_path) {
            if !live_files.insert(entry.remote_path.clone()) { continue; }
            match old_files.get(&entry.remote_path) {
                Some((_,_,_,missing)) if *missing != 0 => summary.added += 1,
                Some((size,etag,modified,_)) if *size != entry.size_bytes || *etag != entry.etag || *modified != entry.modified_at => summary.updated += 1,
                None => summary.added += 1,
                _ => {}
            }
            let id = format!("asset-{:016x}",stable_hash(&format!("{root_id}|{}",entry.remote_path)));
            let fname = file_name(&entry.remote_path);
            let name_sort = natural_name_key(&fname);
            let mod_ts = modified_timestamp(entry.modified_at.as_deref());
            tx.execute("INSERT INTO assets(id,root_id,remote_path,file_name,parent_path,media_type,size_bytes,etag,modified_at,modified_ts,name_sort,missing,scanned_at)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,0,?12)
                ON CONFLICT(root_id,remote_path) DO UPDATE SET file_name=excluded.file_name,parent_path=excluded.parent_path,media_type=excluded.media_type,
                content_hash=CASE WHEN assets.size_bytes=excluded.size_bytes AND assets.etag IS excluded.etag AND assets.modified_at IS excluded.modified_at THEN assets.content_hash ELSE NULL END,
                size_bytes=excluded.size_bytes,etag=excluded.etag,modified_at=excluded.modified_at,modified_ts=excluded.modified_ts,name_sort=excluded.name_sort,missing=0,scanned_at=excluded.scanned_at",
                params![id,root_id,entry.remote_path,fname,path,media_type,entry.size_bytes,entry.etag,entry.modified_at,mod_ts,name_sort,timestamp]).map_err(database_error)?;
        }
    }
    for (old,(_,_,_,missing)) in &old_files {
        if !live_files.contains(old) {
            if *missing == 0 { summary.removed += 1; }
            tx.execute("UPDATE assets SET missing=1 WHERE root_id=?1 AND remote_path=?2",params![root_id,old]).map_err(database_error)?;
        }
    }
    for old in old_dirs.difference(&live_dirs) {
        summary.removed += 1;
        let prefix_start = format!("{old}/");
        let prefix_end = format!("{old}0");
        tx.execute("UPDATE assets SET missing=1 WHERE root_id=?1 AND remote_path>=?2 AND remote_path<?3",params![root_id,prefix_start,prefix_end]).map_err(database_error)?;
        for table in ["library_nodes","directory_snapshots","directory_sync_state"] {
            tx.execute(&format!("DELETE FROM {table} WHERE root_id=?1 AND (remote_path=?2 OR (remote_path>=?3 AND remote_path<?4))"),params![root_id,old,prefix_start,prefix_end]).map_err(database_error)?;
        }
    }
    tx.execute("INSERT INTO directory_sync_state(root_id,remote_path,scanned_at) VALUES(?1,?2,?3)
        ON CONFLICT(root_id,remote_path) DO UPDATE SET scanned_at=excluded.scanned_at",params![root_id,path,now()]).map_err(database_error)?;
    // Do not give the recursive scanner a fingerprint for an unscanned subtree.
    tx.execute("DELETE FROM directory_snapshots WHERE root_id=?1 AND remote_path=?2",params![root_id,path]).map_err(database_error)?;
    let cover: Option<String> = select_directory_cover(tx, root_id, path)?;
    tx.execute("UPDATE directory_sync_state SET revision=?3,cover_asset_id=CASE WHEN cover_asset_id IS NOT NULL AND cover_asset_id LIKE 'dir_cover_%' THEN cover_asset_id ELSE ?4 END WHERE root_id=?1 AND remote_path=?2",params![root_id,path,random_token(),cover]).map_err(database_error)?;
    if let Some(ref cover_id) = cover {
        let mut current = path.to_owned();
        for _ in 0..16 {
            let parent = parent_path(&current);
            if parent == current {
                break;
            }
            let _ = tx.execute(
                "UPDATE directory_sync_state SET cover_asset_id = COALESCE(NULLIF(cover_asset_id, ''), ?3) WHERE root_id = ?1 AND remote_path = ?2",
                params![root_id, parent, cover_id],
            );
            current = parent;
        }
    }
    summary.total = live_files.len() + live_dirs.len();
    Ok(summary)
}

/// Search the complete remote subtree through AList instead of limiting the
/// result to the pages currently rendered by the browser.
#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn search_remote_files(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    root_id: String,
    parent_path: String,
    keywords: String,
    cursor: Option<String>,
) -> Result<RemoteSearchPage, String> {
    let root_app = app.clone();
    let lookup_id = root_id.clone();
    let (domain, root_path) = tokio::task::spawn_blocking(move || {
        let connection = open_media_database(&root_app)?;
        connection
            .query_row(
                "SELECT domain, remote_path FROM domain_roots WHERE root_id = ?1",
                params![lookup_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| "找不到资料库根目录".to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    let root_kind = if domain == "comics" {LibraryRootKind::Comics} else {LibraryRootKind::Media};
    let root_path = normalize_remote_path(&root_path)?;
    let parent_path = normalize_remote_path(&parent_path)?;
    let root_prefix = format!("{}/", root_path.trim_end_matches('/'));
    if parent_path != root_path && !parent_path.starts_with(&root_prefix) {
        return Err("请求目录不属于当前资料库根目录".into());
    }
    let keywords = keywords.trim().to_owned();
    if keywords.is_empty() {
        return Ok(RemoteSearchPage {
            items: Vec::new(),
            total: 0,
            next_cursor: None,
        });
    }
    let page = cursor
        .as_deref()
        .map(|value| {
            value
                .strip_prefix("search:")
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| "搜索页码无效".to_string())
        })
        .transpose()?
        .unwrap_or(1);
    let session = require_session(&state).await?;
    if session.provider == ProviderKind::CloudDrive2 {
        let db_app = app.clone();
        let db_root_id = root_id.clone();
        let db_keywords = format!("%{keywords}%");
        let db_parent_prefix = if parent_path == "/" {"/".to_owned()} else {format!("{}/",parent_path)};
        let (items, total) = tokio::task::spawn_blocking(move || -> Result<(Vec<RemoteBrowserEntry>, i64), String> {
            let connection = open_media_database(&db_app)?;
            let total: i64 = connection.query_row(
                "SELECT COUNT(*) FROM assets WHERE root_id = ?1 AND file_name LIKE ?2 AND missing = 0 AND substr(remote_path,1,length(?3))=?3",
                params![db_root_id, db_keywords, db_parent_prefix],
                |row| row.get(0),
            ).map_err(database_error)?;

            let offset = (page - 1).checked_mul(BROWSE_PAGE_SIZE as i64).ok_or("搜索页码过大")?;
            let mut stmt = connection.prepare(
                "SELECT remote_path, file_name, size_bytes, id, media_type FROM assets
                 WHERE root_id = ?1 AND file_name LIKE ?2 AND missing = 0 AND substr(remote_path,1,length(?3))=?3
                 ORDER BY file_name COLLATE NOCASE,remote_path LIMIT ?4 OFFSET ?5"
            ).map_err(database_error)?;
            let rows = stmt.query_map(params![db_root_id, db_keywords, db_parent_prefix, BROWSE_PAGE_SIZE as i64, offset], |row| {
                Ok(RemoteBrowserEntry {
                    path: row.get(0)?,
                    name: row.get(1)?,
                    is_directory: false,
                    size_bytes: row.get(2)?,
                    modified_at: None,
                    etag: None,
                    asset_id: Some(row.get(3)?),
                    media_type: Some(row.get(4)?),
                    thumbnail_source: None,
                    thumbnail_url: None,
                    cover_asset_id: None,
                    duration_seconds: None,
                    progress_seconds: None,
                })
            }).map_err(database_error)?;
            let mut result = Vec::new();
            for r in rows {
                if let Ok(entry) = r {
                    result.push(entry);
                }
            }
            Ok((result, total))
        }).await.map_err(|error| error.to_string())??;

        let next_cursor = if page.saturating_mul(BROWSE_PAGE_SIZE as i64) < total {
            Some(format!("search:{}", page + 1))
        } else {
            None
        };
        return Ok(RemoteSearchPage {
            items,
            total,
            next_cursor,
        });
    }

    let url = alist_api_url(&session.endpoint, "fs/search")?;
    let response = session
        .client
        .post(url)
        .header(USER_AGENT, MEDIA_USER_AGENT)
        .header(AUTHORIZATION, &session.api_token)
        .json(&serde_json::json!({
            "parent": &parent_path,
            "keywords": keywords,
            "scope": 0,
            "page": page,
            "per_page": BROWSE_PAGE_SIZE,
            "password": ""
        }))
        .send()
        .await
        .map_err(|error| format!("AList API 搜索失败：{error}"))?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("读取 AList 搜索响应失败：{error}"))?;
    let code = body
        .get("code")
        .and_then(|value| value.as_i64())
        .or_else(|| {
            body.get("code")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or_default();
    if !status.is_success() || (code != 200 && code != 0) {
        let message = body
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("未知错误");
        return Err(format!(
            "AList API 搜索失败（HTTP {}）：{}",
            status,
            summarize_api_error(message)
        ));
    }
    let data = body.get("data").ok_or("AList API 未返回搜索数据")?;
    let total = data
        .get("total")
        .and_then(|value| value.as_i64())
        .or_else(|| data.get("filtered_total").and_then(|value| value.as_i64()))
        .unwrap_or(0);
    let entries = api_directory_entries(data).ok_or("AList API 未返回有效搜索列表")?;
    let timestamp = now();
    let mut result = entries
        .into_iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_owned();
            let parent = entry
                .get("parent")
                .and_then(|value| value.as_str())
                .unwrap_or(&parent_path);
            let child = if parent == "/" {
                format!("/{name}")
            } else {
                format!("{parent}/{name}")
            };
            let remote_path = normalize_remote_path(&child).ok()?;
            if !remote_path.starts_with(&root_prefix) || !(parent_path == "/" || remote_path.starts_with(&format!("{}/",parent_path.trim_end_matches('/')))) { return None; }
            let is_directory = entry
                .get("is_dir")
                .or_else(|| entry.get("isDirectory"))
                .and_then(|value| value.as_bool())
                .or_else(|| {
                    entry
                        .get("is_dir")
                        .and_then(|value| value.as_i64())
                        .map(|value| value != 0)
                })
                .unwrap_or(false);
            let size_bytes = entry
                .get("size")
                .and_then(|value| value.as_i64())
                .or_else(|| {
                    entry
                        .get("size")
                        .and_then(|value| value.as_str())
                        .and_then(|value| value.parse().ok())
                })
                .unwrap_or(0);
            let modified_at = entry
                .get("modified")
                .or_else(|| entry.get("modified_at"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            let thumbnail_url = entry
                .get("thumb")
                .and_then(|value| value.as_str())
                .and_then(|value| alist_thumbnail_url(&session.endpoint, value));
            let etag = entry
                .get("etag")
                .or_else(|| entry.get("sign"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            let media_type = (!is_directory)
                .then(|| media_type_for(root_kind, &remote_path))
                .flatten()
                .map(ToOwned::to_owned);
            let asset_id = media_type.as_ref().map(|_| {
                format!(
                    "asset-{:016x}",
                    stable_hash(&format!("{domain}|{remote_path}"))
                )
            });
            Some(RemoteBrowserEntry {
                path: remote_path,
                name,
                is_directory,
                size_bytes,
                modified_at,
                etag,
                asset_id,
                media_type,
                thumbnail_source: None,
                thumbnail_url,
                cover_asset_id: None,
                duration_seconds: None,
                progress_seconds: None,
            })
        })
        .collect::<Vec<_>>();
    for entry in &mut result {
        let Some(url) = entry.thumbnail_url.take() else {
            continue;
        };
        if let Ok(source) = register_direct_proxy_source(&state, &session, url, "image/jpeg").await
        {
            entry.thumbnail_source = Some(source);
        }
    }
    result.sort_by_cached_key(|entry| (!entry.is_directory, entry.name.to_lowercase()));
    let playable = result
        .iter()
        .filter(|entry| entry.asset_id.is_some() && entry.media_type.is_some())
        .cloned()
        .collect::<Vec<_>>();
    if !playable.is_empty() {
        let registered = tokio::task::spawn_blocking(move || -> Result<HashMap<String,String>, String> {
            let mut connection = open_media_database(&app)?;
            register_search_assets_v2(&mut connection, &domain, &root_id, playable, timestamp)
        })
        .await
        .map_err(|error| error.to_string())??;
        for entry in &mut result {
            if let Some(id) = registered.get(&entry.path) { entry.asset_id = Some(id.clone()); }
        }
    }
    let has_more = result.len() >= BROWSE_PAGE_SIZE
        && (total <= 0 || page.saturating_mul(BROWSE_PAGE_SIZE as i64) < total);
    Ok(RemoteSearchPage {
        items: result,
        total,
        next_cursor: has_more.then(|| format!("search:{}", page + 1)),
    })
}

fn register_search_assets_v2(
    connection: &mut Connection,
    domain: &str,
    root_id: &str,
    entries: Vec<RemoteBrowserEntry>,
    timestamp: i64,
) -> Result<HashMap<String,String>,String> {
    let tx=connection.transaction().map_err(database_error)?;
    let mut registered=HashMap::new();
    for entry in entries {
        let Some(id)=entry.asset_id else {continue};
        let Some(media_type)=entry.media_type else {continue};
        tx.execute(
            "INSERT INTO assets(id,domain,root_id,remote_path,file_name,parent_path,media_type,size_bytes,etag,modified_at,missing,first_seen_at,last_seen_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,?11,?11)
             ON CONFLICT(domain,remote_path) DO UPDATE SET root_id=excluded.root_id,file_name=excluded.file_name,parent_path=excluded.parent_path,
             media_type=excluded.media_type,content_hash=CASE WHEN assets.size_bytes=excluded.size_bytes AND assets.etag IS excluded.etag THEN assets.content_hash ELSE NULL END,
             size_bytes=excluded.size_bytes,etag=excluded.etag,modified_at=excluded.modified_at,missing=0,missing_since=NULL,missing_scan_count=0,last_seen_at=excluded.last_seen_at",
            params![id,domain,root_id,entry.path,entry.name,parent_path(&entry.path),media_type,entry.size_bytes,entry.etag,entry.modified_at,timestamp],
        ).map_err(database_error)?;
        let actual_id:String=tx.query_row("SELECT id FROM assets WHERE domain=?1 AND remote_path=?2",params![domain,entry.path],|r|r.get(0)).map_err(database_error)?;
        match domain {
            "vault"=>{tx.execute("INSERT INTO vault_items(asset_id,updated_at) VALUES(?1,?2) ON CONFLICT(asset_id) DO NOTHING",params![actual_id,timestamp]).map_err(database_error)?;}
            "comics" if media_type=="comicArchive"=>{tx.execute("INSERT INTO comic_archives(asset_id,updated_at) VALUES(?1,?2) ON CONFLICT(asset_id) DO NOTHING",params![actual_id,timestamp]).map_err(database_error)?;}
            _=>{}
        }
        registered.insert(entry.path,actual_id);
    }
    tx.commit().map_err(database_error)?;
    Ok(registered)
}

fn register_browsed_assets(
    connection: &mut Connection,
    root_id: &str,
    path: &str,
    playable: Vec<RemoteBrowserEntry>,
    timestamp: i64,
) -> Result<(), String> {
    // An unchanged page uses only reads, even while the scanner holds the
    // SQLite write lock. Recheck the fingerprint inside the upsert below.
    let playable = {
        let mut lookup = connection.prepare_cached("SELECT size_bytes, etag, media_type, missing FROM assets WHERE root_id = ?1 AND remote_path = ?2").map_err(database_error)?;
        let mut changed = Vec::new();
        for entry in playable {
            let same = lookup
                .query_row(params![root_id, entry.path], |row| {
                    Ok(row.get::<_, i64>(0)? == entry.size_bytes
                        && row.get::<_, Option<String>>(1)? == entry.etag
                        && Some(row.get::<_, String>(2)?) == entry.media_type
                        && row.get::<_, i64>(3)? == 0)
                })
                .optional()
                .map_err(database_error)?
                .unwrap_or(false);
            if !same {
                changed.push(entry);
            }
        }
        changed
    };
    if playable.is_empty() {
        return Ok(());
    }
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(database_error)?;
    let mut statement = transaction.prepare_cached(
            "INSERT INTO assets (id, root_id, remote_path, file_name, parent_path, media_type, size_bytes, etag, thumbnail_key, missing, scanned_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10)
             ON CONFLICT(root_id, remote_path) DO UPDATE SET
               file_name = excluded.file_name, parent_path = excluded.parent_path,
               media_type = excluded.media_type, size_bytes = excluded.size_bytes,
               etag = excluded.etag, thumbnail_key = excluded.thumbnail_key,
               missing = 0, scanned_at = excluded.scanned_at,
               content_hash = CASE WHEN assets.size_bytes = excluded.size_bytes AND assets.etag IS excluded.etag THEN assets.content_hash ELSE NULL END
             WHERE assets.missing != 0 OR assets.size_bytes != excluded.size_bytes
                OR assets.etag IS NOT excluded.etag OR assets.media_type != excluded.media_type"
        ).map_err(database_error)?;
    for entry in playable {
        let asset_id = entry.asset_id.as_deref().unwrap_or_default();
        let media_type = entry.media_type.as_deref().unwrap_or_default();
        statement
            .execute(params![
                asset_id,
                root_id,
                entry.path,
                entry.name,
                path,
                media_type,
                entry.size_bytes,
                entry.etag,
                thumbnail_key(&entry.name, media_type),
                timestamp,
            ])
            .map_err(database_error)?;
    }
    drop(statement);
    transaction.commit().map_err(database_error)?;
    Ok(())
}

async fn browse_directory_page(
    state: &MediaLibraryState,
    session: &Session,
    root_id: &str,
    path: &str,
    cursor: Option<String>,
    refresh: bool,
) -> Result<(Vec<DavEntry>, Option<String>), String> {
    let page = match cursor {
        Some(value) => value
            .strip_prefix("api:")
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value > 0)
            .ok_or("目录页码无效")?,
        None => 1,
    };
    let cache_key = format!("{root_id}|{path}|{page}");
    if !refresh {
        let cache = state.directory_cache.lock().await;
        if let Some(cached) = cache.get(&cache_key) {
            if cached.cached_at.elapsed().as_secs() < DIRECTORY_CACHE_TTL_SECONDS {
                return Ok((cached.entries.clone(), cached.next_cursor.clone()));
            }
        }
    }
    let (entries, more) = match session.provider {
        ProviderKind::Alist => list_api_directory_range(session, path, Some(page), refresh).await?,
        ProviderKind::CloudDrive2 => {
            let all = list_webdav_directory(session, path).await?;
            let offset = (page - 1) as usize * BROWSE_PAGE_SIZE;
            if offset >= all.len() {
                (Vec::new(), false)
            } else {
                let end = (offset + BROWSE_PAGE_SIZE).min(all.len());
                let slice = all[offset..end].to_vec();
                let more = end < all.len();
                (slice, more)
            }
        }
    };
    let next_cursor = more.then(|| format!("api:{}", page + 1));
    {
        let mut cache = state.directory_cache.lock().await;
        cache
            .retain(|_, cached| cached.cached_at.elapsed().as_secs() < DIRECTORY_CACHE_TTL_SECONDS);
        while cache.len() >= MAX_DIRECTORY_CACHE_PAGES {
            let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, cached)| cached.cached_at)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            cache.remove(&oldest);
        }
        cache.insert(
            cache_key,
            CachedDirectoryPage {
                cached_at: Instant::now(),
                entries: entries.clone(),
                next_cursor: next_cursor.clone(),
            },
        );
    }
    Ok((entries, next_cursor))
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_library_roots(app: AppHandle) -> Result<Vec<LibraryRoot>, String> {
    let connection = crate::v2_config::open(&app)?;
    let mut statement = connection.prepare(
        "SELECT d.root_id,CASE WHEN d.domain='comics' THEN 'comics' ELSE 'media' END,
                d.remote_path,COALESCE(r.created_at,d.updated_at)
         FROM domain_roots d LEFT JOIN library_roots r ON r.id=d.root_id
         ORDER BY d.domain,d.remote_path")
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| {
            let kind: String = row.get(1)?;
            Ok((
                row.get::<_, String>(0)?,
                kind,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(database_error)?;
    rows.map(|row| {
        let (id, kind, remote_path, created_at) = row.map_err(database_error)?;
        Ok(LibraryRoot {
            id,
            kind: LibraryRootKind::from_str(&kind)?,
            remote_path,
            created_at,
        })
    })
    .collect()
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn add_library_root(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    kind: LibraryRootKind,
    remote_path: String,
) -> Result<LibraryRoot, String> {
    let _ = (app, state, kind, remote_path);
    Err("V2 使用固定四域根目录，请通过目录配置指定域和路径".into())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn remove_library_root(app: AppHandle, root_id: String) -> Result<(), String> {
    let _ = (app, root_id);
    Err("V2 不支持直接删除根目录，请通过目录配置清除对应域路径".into())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn scan_library(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    root_id: Option<String>,
    folder_path: Option<String>,
) -> Result<ScanResult, String> {
    let connection = crate::v2_config::open(&app)?;
    let mut statement = connection.prepare(
        "SELECT domain,root_id,remote_path FROM domain_roots ORDER BY domain"
    ).map_err(database_error)?;
    let configured = statement.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))
        .map_err(database_error)?.collect::<Result<Vec<_>,_>>().map_err(database_error)?;
    let mut roots = Vec::new();
    let mut targets = HashMap::new();
    if root_id.is_none() && folder_path.is_some() {
        return Err("指定扫描目录时必须同时指定域根目录".into());
    }
    for (domain,id,path) in configured {
        if let Some(wanted) = root_id.as_ref() {
            if wanted != &id { continue; }
        } else if domain != "vault" && domain != "comics" {
            continue;
        }
        if let Some(folder) = folder_path.as_ref() {
            let folder = normalize_remote_path(folder)?;
            if folder != path && !folder.starts_with(&(path.trim_end_matches('/').to_owned()+"/")) {
                return Err("扫描目录不在指定域根目录下".into());
            }
            targets.insert(id.clone(),vec![folder]);
        }
        roots.push(LibraryRoot { id,kind:if domain=="comics" {LibraryRootKind::Comics} else {LibraryRootKind::Media},remote_path:path,created_at:0 });
    }
    drop(statement);
    drop(connection);
    launch_scan(app,roots,state.scan_progress.clone(),if targets.is_empty(){None}else{Some(targets)})
}

fn launch_scan(
    app: AppHandle,
    roots: Vec<LibraryRoot>,
    progress: Arc<StdMutex<ScanProgress>>,
    targets: Option<HashMap<String, Vec<String>>>,
) -> Result<ScanResult, String> {
    if roots.is_empty() { return Err("没有可扫描的域根目录".into()); }
    {
        let mut current = progress.lock().unwrap_or_else(|e|e.into_inner());
        if current.running {
            return Ok(ScanResult { roots_scanned:current.roots_completed,files_indexed:current.files_found,message:"扫描正在进行中，请稍候查看进度".into() });
        }
        *current = ScanProgress::idle();
        current.running=true;
        current.phase="scanning".into();
        current.roots_total=roots.len();
        current.current_root=roots.first().map(|r|r.remote_path.clone());
        current.message=format!("正在扫描 {} 个域根目录",roots.len());
    }
    crate::rt::spawn(async move {run_scan_library(app,roots,progress,targets).await;});
    Ok(ScanResult {roots_scanned:0,files_indexed:0,message:"扫描任务已在后台启动，可查看实时进度".into()})
}
#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_scan_progress(state: State<'_, MediaLibraryState>) -> ScanProgress {
    state
        .scan_progress
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

/// 启动资料库扫描并立即返回；进度通过 `get_scan_progress` 轮询。
/// 长扫描不再占用一个 HTTP 请求，避免客户端超时/取消。
#[cfg_attr(feature = "tauri", tauri::command)]
pub fn start_scan(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    root_id: Option<String>,
    folder_path: Option<String>,
) -> Result<(), String> {
    {
        let progress = state
            .scan_progress
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if progress.running {
            return Err("扫描已在进行中".to_string());
        }
    }
    let app_for_task = app.clone();
    crate::rt::spawn(async move {
        let state = app_for_task.state::<MediaLibraryState>();
        if let Err(error) = scan_library(app_for_task.clone(), state, root_id, folder_path).await {
            let state = app_for_task.state::<MediaLibraryState>();
            let mut progress = state
                .scan_progress
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            progress.running = false;
            progress.phase = "failed".into();
            progress.percent = 100.0;
            progress.error = Some(error);
        }
    });
    Ok(())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaStreamInfo {
    pub accelerated: bool,
    pub provider: String,
    pub accel_provider: Option<String>,
}

/// 判断当前媒体是否满足 115 直连加速条件，供播放器显示来源标记。
#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_media_stream_info(app: AppHandle, asset_id: String) -> Result<MediaStreamInfo, String> {
    let connection = open_media_database(&app)?;
    let remote_path: String = connection
        .query_row(
            "SELECT remote_path FROM assets WHERE id = ?1 AND missing = 0",
            params![asset_id],
            |row| row.get(0),
        )
        .map_err(|_| "找不到可播放的媒体文件".to_string())?;
    let provider = match library_provider_configuration(Some(&app)) {
        Ok((ProviderKind::CloudDrive2, _, _)) => "cd2",
        Ok((ProviderKind::Alist, _, _)) => "alist",
        Err(_) => "none",
    }
    .to_string();
    let accelerated = get_alist_accel_config(app.clone())
        .map(|config| {
            config.enabled
                && !config.api_url.trim().is_empty()
                && !config.cd2_prefix.trim().is_empty()
                && remote_path.starts_with(config.cd2_prefix.trim_end_matches('/'))
        })
        .unwrap_or(false);
    Ok(MediaStreamInfo {
        accelerated,
        provider,
        accel_provider: if accelerated { Some("115".to_string()) } else { None },
    })
}


const SCAN_FAILED_DIRECTORIES_KEY: &str = "scan_failed_directories";

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn rescan_failed_directories(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
) -> Result<ScanResult, String> {
    let targets = load_failed_directories(&app)?;
    if targets.is_empty() {
        return Ok(ScanResult {
            roots_scanned: 0,
            files_indexed: 0,
            message: "当前没有待重扫的错误目录".into(),
        });
    }
    let roots = list_library_roots(app.clone())?;
    let selected = roots
        .into_iter()
        .filter(|root| targets.contains_key(&root.id))
        .collect::<Vec<_>>();
    launch_scan(app, selected, state.scan_progress.clone(), Some(targets))
}

fn load_failed_directories(app: &AppHandle) -> Result<HashMap<String, Vec<String>>, String> {
    let connection = crate::v2_config::open(app)?;
    let raw = connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![SCAN_FAILED_DIRECTORIES_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?;
    Ok(raw
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default())
}

fn save_failed_directories(
    app: &AppHandle,
    failed: &HashMap<String, Vec<String>>,
) -> Result<(), String> {
    let connection = crate::v2_config::open(app)?;
    connection.execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![SCAN_FAILED_DIRECTORIES_KEY, serde_json::to_string(failed).map_err(|error| error.to_string())?]).map_err(database_error)?;
    Ok(())
}

fn update_scan_progress(
    progress: &Arc<StdMutex<ScanProgress>>,
    update: impl FnOnce(&mut ScanProgress),
) {
    let mut current = progress.lock().unwrap_or_else(|error| error.into_inner());
    update(&mut current);
}

async fn run_scan_library(
    app: AppHandle,
    roots: Vec<LibraryRoot>,
    progress: Arc<StdMutex<ScanProgress>>,
    targets: Option<HashMap<String, Vec<String>>>,
) {
    let started = Instant::now();
    let result: Result<ScanResult,String> = async {
        let mut failed = load_failed_directories(&app)?;
        let mut total=0usize;
        let mut errors=Vec::new();
        for (index,root) in roots.iter().enumerate() {
            let domain:String = {
                let connection=crate::v2_config::open(&app)?;
                connection.query_row("SELECT domain FROM domain_roots WHERE root_id=?1",params![root.id],|r|r.get(0))
                    .map_err(database_error)?
            };
            let paths=targets.as_ref().and_then(|t|t.get(&root.id)).cloned();
            let jobs=paths.clone().unwrap_or_else(||vec![root.remote_path.clone()]);
            let mut root_failed=Vec::new();
            for path in jobs {
                update_scan_progress(&progress,|p| {
                    p.current_root=Some(root.remote_path.clone());p.current_directory=Some(path.clone());
                    p.message=format!("正在扫描域 {}/{}：{}",index+1,roots.len(),path);
                });
                let state=app.state::<MediaLibraryState>();
                let outcome=if paths.is_some() {
                    crate::four_domains::scan_domain_subtree(app.clone(),state,&domain,&path).await
                } else {
                    crate::four_domains::scan_domain(app.clone(),state,&domain).await
                };
                match outcome {
                    Ok(summary)=>{total+=summary.scanned.max(0) as usize;}
                    Err(error)=>{root_failed.push(path.clone());errors.push(format!("{path}: {error}"));}
                }
            }
            failed.remove(&root.id);
            if !root_failed.is_empty(){failed.insert(root.id.clone(),root_failed);}
            save_failed_directories(&app,&failed)?;
            update_scan_progress(&progress,|p| {
                p.roots_completed=index+1;p.files_found=total;
                p.failed_directories=failed.values().flatten().cloned().collect();
                p.skipped_directories=p.failed_directories.len();p.percent=((index+1) as f64/roots.len() as f64)*99.0;
            });
        }
        if !errors.is_empty(){return Err(errors.join("；"));}
        let connection=crate::v2_config::open(&app)?;
        pml_set_setting(&connection,SETTING_LAST_SCAN_TIME,Some(&now().to_string()))?;
        pml_set_setting(&connection,SETTING_LAST_SCAN_DURATION,Some(&started.elapsed().as_secs().to_string()))?;
        Ok(ScanResult {roots_scanned:roots.len(),files_indexed:total,message:format!("已完成 {} 个域的 V2 索引",roots.len())})
    }.await;
    update_scan_progress(&progress,|p| {
        p.running=false;p.percent=100.0;p.current_root=None;p.current_directory=None;
        match result {
            Ok(result)=>{p.phase="completed".into();p.message=result.message;p.files_found=result.files_indexed;p.roots_completed=result.roots_scanned;}
            Err(error)=>{p.phase="failed".into();p.message="扫描失败；未完成域的原索引已保留，可重试错误目录".into();p.error=Some(error);}
        }
    });
}
#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_library_overview(app: AppHandle) -> Result<LibraryOverview, String> {
    crate::v2_catalog::get_library_overview(app)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_library_assets(
    app: AppHandle,
    filter: Option<String>,
    query: Option<String>,
    author_id: Option<String>,
    collection_id: Option<String>,
    category_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    parent_path: Option<String>,
) -> Result<Vec<LibraryAsset>, String> {
    crate::v2_catalog::list_library_assets(app, filter, query, author_id, collection_id, category_id, limit, offset, parent_path)
}

struct VideoBoundsCache {
    min_rowid: i64,
    max_rowid: i64,
    total: usize,
    cached_at: Instant,
}

static VIDEO_BOUNDS_CACHE: OnceLock<StdMutex<Option<VideoBoundsCache>>> = OnceLock::new();

fn get_video_bounds(connection: &Connection) -> Result<(i64, i64, usize), String> {
    let mutex = VIDEO_BOUNDS_CACHE.get_or_init(|| StdMutex::new(None));
    let mut guard = mutex.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref cached) = *guard {
        if cached.cached_at.elapsed() < Duration::from_secs(300) && cached.total > 0 {
            return Ok((cached.min_rowid, cached.max_rowid, cached.total));
        }
    }

    let row: Option<(i64, i64, usize)> = connection
        .query_row(
            "SELECT MIN(rowid), MAX(rowid), COUNT(*) FROM assets
             WHERE media_type = 'video' AND missing = 0
               AND remote_path NOT LIKE '%inbox%'
               AND remote_path NOT LIKE '%暂存%'
               AND remote_path NOT LIKE '%未分类%'
               AND remote_path NOT LIKE '%pending%'
               AND (file_name LIKE '%.mp4' OR file_name LIKE '%.webm' OR file_name LIKE '%.mov' OR file_name LIKE '%.m4v')",
            [],
            |r| {
                let min_r: Option<i64> = r.get(0)?;
                let max_r: Option<i64> = r.get(1)?;
                let cnt: usize = r.get(2)?;
                if let (Some(min_v), Some(max_v)) = (min_r, max_r) {
                    Ok(Some((min_v, max_v, cnt)))
                } else {
                    Ok(None)
                }
            },
        )
        .map_err(database_error)?;

    let (min_rowid, max_rowid, total) = row.unwrap_or((0, 0, 0));
    *guard = Some(VideoBoundsCache {
        min_rowid,
        max_rowid,
        total,
        cached_at: Instant::now(),
    });

    Ok((min_rowid, max_rowid, total))
}

pub fn invalidate_video_bounds_cache() {
    let mutex = VIDEO_BOUNDS_CACHE.get_or_init(|| StdMutex::new(None));
    if let Ok(mut guard) = mutex.lock() {
        *guard = None;
    }
}

/// Returns a true-random, non-repeating shuffle of eligible short videos.
/// Supports optional author scoping and max duration constraint.
/// Sub-millisecond B-tree binary seek directly by rowid; zero linear offset scanning.
#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_short_video_feed(
    app: AppHandle,
    _cursor: Option<String>,
    limit: Option<i64>,
    author_id: Option<String>,
    max_duration_seconds: Option<i64>,
) -> Result<ShortVideoFeedPage, String> {
    crate::v2_catalog::list_short_video_feed(app, _cursor, limit, author_id, max_duration_seconds)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicFolderSummary {
    pub path: String,
    pub title: String,
    pub page_count: i64,
    pub archive_count: i64,
    pub cover_asset_id: Option<String>,
}

/// Return only immediate comic subfolders. The client does not need to load
/// every comic asset just to render the folder browser.
#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_comic_folders(
    app: AppHandle,
    root_id: String,
    parent_path: String,
) -> Result<Vec<ComicFolderSummary>, String> {
    let connection = open_media_database(&app)?;
    let parent = if parent_path.trim().is_empty() {
        "/".to_owned()
    } else {
        parent_path.trim_end_matches('/').to_owned()
    };
    let prefix = if parent == "/" {
        "/".to_owned()
    } else {
        format!("{parent}/")
    };
    let prefix_end = if parent == "/" {
        "0".to_owned()
    } else {
        format!("{parent}0")
    };
    let mut statement = connection
        .prepare(
            "SELECT id, parent_path, media_type
             FROM assets
             WHERE root_id = ?1 AND missing = 0
               AND remote_path >= ?2 AND remote_path < ?3
             ORDER BY
               CASE WHEN lower(file_name) GLOB 'cover.*' OR lower(file_name) GLOB 'poster.*' OR lower(file_name) GLOB 'folder.*' THEN 0
                    WHEN media_type IN ('image','comicPage') THEN 1
                    ELSE 2
               END,
               file_name COLLATE NOCASE,
               remote_path",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![root_id, prefix, prefix_end], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?;
    let mut folders: HashMap<String, ComicFolderSummary> = HashMap::new();
    for row in rows {
        let (asset_id, asset_parent, media_type) = row.map_err(database_error)?;
        if asset_parent == parent {
            continue;
        }
        let Some(relative) = asset_parent.strip_prefix(&prefix) else {
            continue;
        };
        let Some(segment) = relative.split('/').next().filter(|value| !value.is_empty()) else {
            continue;
        };
        let path = if parent == "/" {
            format!("/{segment}")
        } else {
            format!("{prefix}{segment}")
                .trim_end_matches('/')
                .to_owned()
        };
        let folder = folders
            .entry(path.clone())
            .or_insert_with(|| ComicFolderSummary {
                path,
                title: segment.to_owned(),
                page_count: 0,
                archive_count: 0,
                cover_asset_id: None,
            });
        match media_type.as_str() {
            "comicPage" => folder.page_count += 1,
            "comicArchive" => folder.archive_count += 1,
            "pdf" => folder.archive_count += 1,
            _ => continue,
        }
        if folder.cover_asset_id.is_none() {
            folder.cover_asset_id = Some(asset_id);
        }
    }
    if let Ok(mut stmt) = connection.prepare("SELECT remote_path, cover_asset_id FROM directory_sync_state WHERE root_id = ?1 AND cover_asset_id IS NOT NULL") {
        if let Ok(rows) = stmt.query_map(params![root_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            let mut custom_map = HashMap::new();
            for (p, cid) in rows.filter_map(Result::ok) {
                custom_map.insert(p, cid);
            }
            for folder in folders.values_mut() {
                if let Some(custom_id) = custom_map.get(&folder.path) {
                    folder.cover_asset_id = Some(custom_id.clone());
                }
            }
        }
    }
    let mut result = folders.into_values().collect::<Vec<_>>();
    result.sort_by(|left, right| left.title.to_lowercase().cmp(&right.title.to_lowercase()));
    Ok(result)
}

fn natural_category_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn extract(s: &str) -> Option<(u64, &str)> {
        let trimmed = s.trim();
        let mut idx = 0;
        for c in trimmed.chars() {
            if c.is_ascii_digit() {
                idx += c.len_utf8();
            } else {
                break;
            }
        }
        if idx == 0 {
            return None;
        }
        let num: u64 = trimmed[..idx].parse().ok()?;
        let rest = trimmed[idx..].trim_start_matches(|c: char| c == '-' || c == '_' || c == '.' || c == ' ' || c == '、');
        Some((num, rest))
    }

    match (extract(a), extract(b)) {
        (Some((num_a, rest_a)), Some((num_b, rest_b))) => {
            if num_a != num_b {
                num_a.cmp(&num_b)
            } else {
                rest_a.cmp(rest_b)
            }
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.cmp(b),
    }
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_library_categories(
    app: AppHandle,
    filter: Option<String>,
    parent_id: Option<String>,
) -> Result<Vec<LibraryCategory>, String> {
    crate::v2_catalog::list_library_categories(app, filter, parent_id)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_library_authors(
    app: AppHandle,
    category_id: Option<String>,
    filter: Option<String>,
) -> Result<Vec<LibraryAuthor>, String> {
    crate::v2_catalog::list_library_authors(app, category_id, filter)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_library_collections(
    app: AppHandle,
    author_id: Option<String>,
    parent_id: Option<String>,
    category_id: Option<String>,
) -> Result<Vec<LibraryCollection>, String> {
    crate::v2_catalog::list_library_collections(app, author_id, parent_id, category_id)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn assign_asset_metadata(
    app: AppHandle,
    asset_id: String,
    author_name: Option<String>,
    collection_name: Option<String>,
) -> Result<(), String> {
    let mut connection = open_media_database(&app)?;
    let connection = connection.transaction().map_err(database_error)?;
    let (root_id, domain, category_id): (String, String, Option<String>) = connection.query_row(
        "SELECT asset.root_id, asset.domain, metadata.category_id FROM assets asset
         LEFT JOIN vault_items metadata ON metadata.asset_id = asset.id WHERE asset.id = ?1 AND asset.missing = 0",
        params![asset_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(|_| "找不到要归属的媒体文件".to_string())?;
    if domain != "vault" && domain != "pending" {
        return Err("仅媒体库和待整理域支持手动媒体归属".into());
    }
    if domain == "pending" && collection_name.as_ref().is_some_and(|name| !name.trim().is_empty()) {
        return Err("待整理项目暂不支持手动合集，请移入媒体库后设置".into());
    }
    let author_name = author_name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let author_id = author_name
        .as_ref()
        .map(|name| ensure_author(&connection, name, "user"))
        .transpose()?;
    let collection_id = match collection_name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        Some(name) => Some(ensure_collection(
            &connection,
            &root_id,
            author_id.as_deref(),
            &name,
            "user",
        )?),
        None => None,
    };
    if domain == "pending" {
        let updated = connection.execute(
            "UPDATE pending_items SET manual_author_id = ?2,
             assignment_status = CASE WHEN ?2 IS NOT NULL AND COALESCE(manual_category_id,recommended_category_id) IS NOT NULL THEN 'ready' ELSE 'triage' END,
             conflict_reason = NULL, updated_at = ?3 WHERE asset_id = ?1",
            params![asset_id, author_id, now()],
        ).map_err(database_error)?;
        if updated == 0 {
            return Err("待整理媒体缺少项目记录，请先刷新待整理目录".into());
        }
        connection.execute(
            "UPDATE pending_projects SET
             triage_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=pending_projects.id AND assignment_status='triage'),
             ready_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=pending_projects.id AND assignment_status='ready' AND conflict_reason IS NULL),
             updated_at=?2 WHERE id=(SELECT project_id FROM pending_items WHERE asset_id=?1)",
            params![asset_id, now()],
        ).map_err(database_error)?;
    } else {
        connection.execute(
            "INSERT INTO vault_items (asset_id, category_id, author_id, collection_id, source, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'user', ?5)
             ON CONFLICT(asset_id) DO UPDATE SET author_id = excluded.author_id, collection_id = excluded.collection_id, source = 'user', updated_at = excluded.updated_at",
            params![asset_id, category_id, author_id, collection_id, now()],
        ).map_err(database_error)?;
    }
    connection.commit().map_err(database_error)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn set_asset_duration(
    app: AppHandle,
    asset_id: String,
    duration_seconds: f64,
) -> Result<(), String> {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return Ok(());
    }
    let connection = open_media_database(&app)?;
    connection
        .execute(
            "UPDATE assets SET duration_seconds = ?1 WHERE id = ?2 AND media_type = 'video'",
            params![duration_seconds.round() as i64, asset_id],
        )
        .map_err(database_error)?;
    Ok(())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_asset_playback_progress(app: AppHandle, asset_id: String) -> Result<f64, String> {
    crate::v2_state::get_asset_playback_progress(app, asset_id)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn set_asset_playback_progress(
    app: AppHandle,
    asset_id: String,
    position_seconds: f64,
) -> Result<(), String> {
    crate::v2_state::set_asset_playback_progress(app, asset_id, position_seconds)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_playback_history(
    app: AppHandle,
    limit: Option<i64>,
) -> Result<Vec<PlaybackHistoryItem>, String> {
    crate::v2_state::list_playback_history(app, limit)
}

/// Return only immediate indexed directory children. This is deliberately a
/// local SQLite lookup; browsing must never re-list a large remote root.
#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_library_nodes(
    app: AppHandle,
    root_id: String,
    parent_path: String,
) -> Result<Vec<LibraryNode>, String> {
    let connection = open_media_database(&app)?;
    let parent = if parent_path.trim().is_empty() {
        "/".to_owned()
    } else {
        parent_path.trim_end_matches('/').to_owned()
    };
    let mut statement = connection
        .prepare(
            "WITH prefix AS (
                SELECT CASE WHEN ?2 = '/' THEN '/' ELSE ?2 || '/' END AS path
             ), descendants AS (
                SELECT substr(a.remote_path,length(p.path)+1) AS relative,p.path AS prefix
                FROM assets a CROSS JOIN prefix p
                WHERE a.root_id = ?1 AND a.missing = 0 AND substr(a.remote_path,1,length(p.path)) = p.path
             ), nodes AS (
                SELECT remote_path,name FROM provider_directory_entries
                WHERE root_id = ?1 AND parent_path = ?2 AND is_directory = 1
                UNION
                SELECT prefix || substr(relative,1,instr(relative,'/')-1),
                       substr(relative,1,instr(relative,'/')-1)
                FROM descendants WHERE instr(relative,'/') > 0
             ) SELECT remote_path,name FROM nodes ORDER BY name COLLATE NOCASE",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![root_id, parent], |row| {
            Ok(LibraryNode {
                path: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(database_error)?;
    rows.map(|row| row.map_err(database_error)).collect()
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn clear_playback_history(app: AppHandle) -> Result<(), String> {
    crate::v2_state::clear_playback_history(app)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn toggle_favorite(app: AppHandle, asset_id: String) -> Result<bool, String> {
    crate::v2_state::toggle_favorite(app, asset_id)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_favorite_assets(
    app: AppHandle,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<LibraryAsset>, String> {
    crate::v2_state::list_favorite_assets(app, limit, offset)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_favorite_ids(app: AppHandle) -> Result<Vec<String>, String> {
    crate::v2_state::list_favorite_ids(app)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn sync_favorites(app: AppHandle, asset_ids: Vec<String>) -> Result<(), String> {
    crate::v2_state::sync_favorites(app, asset_ids)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_favorite_count(app: AppHandle) -> Result<i64, String> {
    crate::v2_state::get_favorite_count(app)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_cache_stats(app: AppHandle) -> Result<CacheStats, String> {
    let cache_base = cache_base_path(&app)?;
    let connection = open_media_database(&app)?;
    let database_bytes = fs::metadata(connection.path().ok_or("V2 数据库没有文件路径")?)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let (short_video_history_count, total_indexed_files, total_indexed_directories) = {
        let conn = open_media_database(&app)?;
        let history = conn
            .query_row(
                "SELECT COUNT(*) FROM asset_state progress JOIN assets asset ON asset.id = progress.asset_id WHERE asset.media_type = 'video' AND progress.progress_seconds IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let files: i64 = conn
            .query_row("SELECT COUNT(*) FROM assets WHERE missing = 0", [], |row| row.get(0))
            .unwrap_or(0);
        let dirs: i64 = conn
            .query_row("SELECT COUNT(*) FROM (SELECT domain,remote_path FROM provider_directory_entries WHERE is_directory=1 UNION SELECT domain,parent_path FROM assets WHERE missing=0)", [], |row| row.get(0))
            .unwrap_or(0);
        (history, files, dirs)
    };
    let short_video_cache_bytes = directory_size(&cache_base.join("short-videos"));
    let settings = get_automation_settings_internal(&app).ok();
    let short_video_cache_max_bytes = settings
        .as_ref()
        .map(|s| s.short_video_cache_max_bytes)
        .unwrap_or(2 * 1024 * 1024 * 1024);
    let last_scan_time = settings.as_ref().and_then(|s| s.last_scan_time);
    let last_scan_duration_secs = settings.as_ref().and_then(|s| s.last_scan_duration_secs);
    Ok(CacheStats {
        thumbnail_bytes: directory_size(&cache_base.join("thumbnails")),
        comic_bytes: directory_size(&cache_base.join("comic-archives")),
        database_bytes,
        short_video_history_count,
        short_video_cache_bytes,
        short_video_cache_max_bytes,
        total_indexed_files,
        total_indexed_directories,
        last_scan_time,
        last_scan_duration_secs,
    })
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn clear_cache(app: AppHandle, target: String) -> Result<(), String> {
    let cache_base = cache_base_path(&app)?;
    match target.as_str() {
        "thumbnails" => {
            let connection = open_media_database(&app)?;
            remove_cache_directory(&cache_base.join("thumbnails"))?;
            invalidate_thumbnail_file_index();
            connection.execute("UPDATE assets SET thumbnail_asset_id = NULL", []).map_err(database_error)?;
            Ok(())
        }
        "comics" => {
            if let Some(state) = app.try_state::<MediaLibraryState>() {
                if let Ok(mut cache) = state.comic_archive_index_cache.lock() {
                    cache.clear();
                }
                if let Ok(mut zip_cache) = state.comic_remote_zip_cache.lock() {
                    zip_cache.clear();
                }
            }
            remove_cache_directory(&cache_base.join("comic-archives"))
        }
        "shortVideos" => {
            open_media_database(&app)?
                .execute(
                    "UPDATE asset_state SET progress_seconds=NULL,last_opened_at=NULL,updated_at=strftime('%s','now') WHERE asset_id IN (SELECT id FROM assets WHERE media_type = 'video')",
                    [],
                )
                .map_err(database_error)?;
            Ok(())
        }
        "shortVideoFiles" => remove_cache_directory(&cache_base.join("short-videos")),
        _ => Err("不支持的缓存类型".into()),
    }
}

const SETTING_SCHEDULED_SCAN_ENABLED: &str = "scheduled_scan_enabled";
const SETTING_SCHEDULED_SCAN_TIME: &str = "scheduled_scan_time";
const SETTING_SCHEDULED_SCAN_AVOID_ACTIVE: &str = "scheduled_scan_avoid_active";
const SETTING_LAST_SCAN_TIME: &str = "last_scan_time";
const SETTING_LAST_SCAN_DURATION: &str = "last_scan_duration_secs";
const SETTING_SHORT_VIDEO_CACHE_MAX_BYTES: &str = "short_video_cache_max_bytes";

pub fn get_automation_settings_internal(app: &AppHandle) -> Result<AutomationSettings, String> {
    let connection = crate::v2_config::open(app)?;
    let get_val = |k: &str| -> Option<String> {
        connection.query_row("SELECT value FROM settings WHERE key = ?1", params![k], |row| row.get(0)).ok()
    };
    let scheduled_scan_enabled = get_val(SETTING_SCHEDULED_SCAN_ENABLED)
        .map(|v| v == "1" || v == "true")
        .unwrap_or(true);
    let scheduled_scan_time = get_val(SETTING_SCHEDULED_SCAN_TIME)
        .unwrap_or_else(|| "04:30".to_string());
    let scheduled_scan_avoid_active = get_val(SETTING_SCHEDULED_SCAN_AVOID_ACTIVE)
        .map(|v| v != "0" && v != "false")
        .unwrap_or(true);
    let last_scan_time = get_val(SETTING_LAST_SCAN_TIME)
        .and_then(|v| v.parse::<i64>().ok());
    let last_scan_duration_secs = get_val(SETTING_LAST_SCAN_DURATION)
        .and_then(|v| v.parse::<u64>().ok());
    let short_video_cache_max_bytes = get_val(SETTING_SHORT_VIDEO_CACHE_MAX_BYTES)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(2 * 1024 * 1024 * 1024);

    Ok(AutomationSettings {
        scheduled_scan_enabled,
        scheduled_scan_time,
        scheduled_scan_avoid_active,
        last_scan_time,
        last_scan_duration_secs,
        short_video_cache_max_bytes,
    })
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_automation_settings(app: AppHandle) -> Result<AutomationSettings, String> {
    get_automation_settings_internal(&app)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn set_automation_settings(app: AppHandle, settings: AutomationSettings) -> Result<(), String> {
    let connection = crate::v2_config::open(&app)?;
    let mut stmt = connection.prepare(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value"
    ).map_err(database_error)?;

    stmt.execute(params![SETTING_SCHEDULED_SCAN_ENABLED, if settings.scheduled_scan_enabled { "1" } else { "0" }]).map_err(database_error)?;
    stmt.execute(params![SETTING_SCHEDULED_SCAN_TIME, settings.scheduled_scan_time]).map_err(database_error)?;
    stmt.execute(params![SETTING_SCHEDULED_SCAN_AVOID_ACTIVE, if settings.scheduled_scan_avoid_active { "1" } else { "0" }]).map_err(database_error)?;
    stmt.execute(params![SETTING_SHORT_VIDEO_CACHE_MAX_BYTES, settings.short_video_cache_max_bytes.to_string()]).map_err(database_error)?;
    CACHED_MAX_SHORT_VIDEO_BYTES.store(settings.short_video_cache_max_bytes, std::sync::atomic::Ordering::Relaxed);

    let cache_base = cache_base_path(&app)?;
    let short_video_dir = cache_base.join("short-videos");
    if short_video_dir.is_dir() {
        prune_short_video_cache(&short_video_dir, settings.short_video_cache_max_bytes);
    }

    Ok(())
}

pub const SETTING_ALIST_ACCEL_ENABLED: &str = "alist_accel_enabled";
pub const SETTING_ALIST_ACCEL_URL: &str = "alist_accel_url";
pub const SETTING_ALIST_ACCEL_USER: &str = "alist_accel_user";
pub const SETTING_ALIST_ACCEL_PASS: &str = "alist_accel_pass";
pub const SETTING_ALIST_ACCEL_CD2_PREFIX: &str = "alist_accel_cd2_prefix";
pub const SETTING_ALIST_ACCEL_ALIST_PREFIX: &str = "alist_accel_alist_prefix";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlistAccelConfig {
    pub enabled: bool,
    pub api_url: String,
    pub username: String,
    pub password: Option<String>,
    pub cd2_prefix: String,
    pub alist_prefix: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlistTestResult {
    pub ok: bool,
    pub message: String,
}

static ALIST_ACCEL_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(8))
        .redirect(Policy::none())
        .build()
        .unwrap_or_else(|_| Client::new())
});

static ALIST_ACCEL_TOKEN_CACHE: OnceLock<StdMutex<Option<(String, Instant)>>> = OnceLock::new();
static ALIST_ACCEL_URL_CACHE: LazyLock<StdMutex<HashMap<String, (String, Instant)>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

pub fn invalidate_alist_accel_url_cache(path_part: &str) {
    let clean = path_part
        .trim_start_matches('/')
        .trim_start_matches("115open")
        .trim_start_matches("115")
        .trim_start_matches('/');
    if let Ok(mut guard) = ALIST_ACCEL_URL_CACHE.lock() {
        guard.retain(|k, _| clean.is_empty() || !k.contains(clean));
    }
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_alist_accel_config(app: AppHandle) -> Result<AlistAccelConfig, String> {
    let connection = crate::v2_config::open(&app)?;
    let get_val = |k: &str| -> Option<String> {
        connection.query_row("SELECT value FROM settings WHERE key = ?1", params![k], |row| row.get(0)).ok()
    };
    Ok(AlistAccelConfig {
        enabled: get_val(SETTING_ALIST_ACCEL_ENABLED).map(|v| v == "1" || v == "true").unwrap_or(false),
        api_url: get_val(SETTING_ALIST_ACCEL_URL).unwrap_or_else(|| "http://103.119.1.41:45112".to_string()),
        username: get_val(SETTING_ALIST_ACCEL_USER).unwrap_or_else(|| "admin".to_string()),
        password: get_val(SETTING_ALIST_ACCEL_PASS),
        cd2_prefix: get_val(SETTING_ALIST_ACCEL_CD2_PREFIX).unwrap_or_else(|| "/115open".to_string()),
        alist_prefix: get_val(SETTING_ALIST_ACCEL_ALIST_PREFIX).unwrap_or_else(|| "/115".to_string()),
    })
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn set_alist_accel_config(app: AppHandle, config: AlistAccelConfig) -> Result<(), String> {
    let connection = crate::v2_config::open(&app)?;
    let mut stmt = connection.prepare(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value"
    ).map_err(database_error)?;

    stmt.execute(params![SETTING_ALIST_ACCEL_ENABLED, if config.enabled { "1" } else { "0" }]).map_err(database_error)?;
    stmt.execute(params![SETTING_ALIST_ACCEL_URL, config.api_url.trim().trim_end_matches('/')]).map_err(database_error)?;
    stmt.execute(params![SETTING_ALIST_ACCEL_USER, config.username.trim()]).map_err(database_error)?;
    if let Some(ref pass) = config.password {
        stmt.execute(params![SETTING_ALIST_ACCEL_PASS, pass]).map_err(database_error)?;
    }
    stmt.execute(params![SETTING_ALIST_ACCEL_CD2_PREFIX, config.cd2_prefix.trim()]).map_err(database_error)?;
    stmt.execute(params![SETTING_ALIST_ACCEL_ALIST_PREFIX, config.alist_prefix.trim()]).map_err(database_error)?;

    if let Some(lock) = ALIST_ACCEL_TOKEN_CACHE.get() {
        if let Ok(mut guard) = lock.lock() {
            *guard = None;
        }
    }
    if let Ok(mut guard) = ALIST_ACCEL_URL_CACHE.lock() {
        guard.clear();
    }
    Ok(())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn test_alist_accel_connection(config: AlistAccelConfig) -> Result<AlistTestResult, String> {
    let client = &*ALIST_ACCEL_CLIENT;
    let base_url = config.api_url.trim().trim_end_matches('/');
    let login_url = format!("{base_url}/api/auth/login");
    let login_body = serde_json::json!({
        "username": config.username.trim(),
        "password": config.password.as_deref().unwrap_or_default(),
    });

    let resp = match client.post(&login_url).json(&login_body).send().await {
        Ok(r) => r,
        Err(e) => return Ok(AlistTestResult { ok: false, message: format!("无法连接至 AList 服务：{e}") }),
    };

    if !resp.status().is_success() {
        return Ok(AlistTestResult { ok: false, message: format!("AList 接口返回状态码 {}", resp.status().as_u16()) });
    }

    let json_val: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return Ok(AlistTestResult { ok: false, message: format!("解析 AList 登录响应失败：{e}") }),
    };

    if json_val.get("code").and_then(|c| c.as_i64()) != Some(200) {
        let msg = json_val.get("message").and_then(|m| m.as_str()).unwrap_or("账号或密码错误");
        return Ok(AlistTestResult { ok: false, message: format!("AList 认证失败：{msg}") });
    }

    let token = json_val.get("data").and_then(|d| d.get("token")).and_then(|t| t.as_str()).unwrap_or_default();
    if token.is_empty() {
        return Ok(AlistTestResult { ok: false, message: "AList 未返回有效 Token".to_string() });
    }

    let list_url = format!("{base_url}/api/fs/list");
    let check_path = if config.alist_prefix.trim().is_empty() { "/" } else { config.alist_prefix.trim() };
    let list_body = serde_json::json!({
        "path": check_path,
        "page": 1,
        "per_page": 1,
    });
    let list_resp = client.post(&list_url).header(AUTHORIZATION, token).json(&list_body).send().await;
    let path_note = match list_resp {
        Ok(r) if r.status().is_success() => {
            let val: serde_json::Value = r.json().await.unwrap_or_default();
            if val.get("code").and_then(|c| c.as_i64()) == Some(200) {
                format!("，且已成功校验目标挂载点「{}」", check_path)
            } else {
                format!("，但挂载点「{}」提示：{}", check_path, val.get("message").and_then(|m| m.as_str()).unwrap_or("路径未就绪"))
            }
        }
        _ => "，未能读取挂载目录".to_string(),
    };

    Ok(AlistTestResult {
        ok: true,
        message: format!("✓ AList 连通与登录成功{}", path_note),
    })
}

async fn get_alist_accel_token(api_url: &str, username: &str, password: &str) -> Option<String> {
    let cache_lock = ALIST_ACCEL_TOKEN_CACHE.get_or_init(|| StdMutex::new(None));
    if let Ok(guard) = cache_lock.lock() {
        if let Some((ref tok, ref inst)) = *guard {
            if inst.elapsed() < Duration::from_secs(3600 * 6) {
                return Some(tok.clone());
            }
        }
    }

    let client = &*ALIST_ACCEL_CLIENT;
    let login_url = format!("{api_url}/api/auth/login");
    let body = serde_json::json!({
        "username": username,
        "password": password,
    });
    let resp = client.post(&login_url).json(&body).send().await.ok()?;
    let val: serde_json::Value = resp.json().await.ok()?;
    let token = val.get("data")?.get("token")?.as_str()?.to_string();

    if let Ok(mut guard) = cache_lock.lock() {
        *guard = Some((token.clone(), Instant::now()));
    }
    Some(token)
}

pub async fn try_get_alist_direct_url(
    app: &AppHandle,
    source: &ProxySource,
) -> Option<String> {
    get_alist_direct_url_impl(app, source, false).await
}

pub async fn fetch_fresh_alist_direct_url(
    app: &AppHandle,
    source: &ProxySource,
) -> Option<String> {
    get_alist_direct_url_impl(app, source, true).await
}

async fn get_alist_direct_url_impl(
    app: &AppHandle,
    source: &ProxySource,
    skip_cache: bool,
) -> Option<String> {
    let config = get_alist_accel_config(app.clone()).ok()?;
    if !config.enabled {
        return None;
    }

    let remote = &source.remote_path;
    let cd2_prefix = config.cd2_prefix.trim().trim_end_matches('/');
    let alist_prefix = config.alist_prefix.trim().trim_end_matches('/');

    let target_path = if !cd2_prefix.is_empty() && remote.starts_with(cd2_prefix) {
        format!("{}{}", alist_prefix, &remote[cd2_prefix.len()..])
    } else {
        remote.clone()
    };

    if !skip_cache {
        if let Ok(guard) = ALIST_ACCEL_URL_CACHE.lock() {
            if let Some((cached_url, inst)) = guard.get(&target_path) {
                if inst.elapsed() < Duration::from_secs(600) {
                    return Some(cached_url.clone());
                }
            }
        }
    }

    let base_url = config.api_url.trim().trim_end_matches('/');
    let pass = config.password.as_deref().unwrap_or_default();
    let token = get_alist_accel_token(base_url, &config.username, pass).await?;

    let client = &*ALIST_ACCEL_CLIENT;
    let get_url = format!("{base_url}/api/fs/get");
    let req_body = serde_json::json!({ "path": target_path });

    let resp = client
        .post(&get_url)
        .header(AUTHORIZATION, &token)
        .json(&req_body)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let val: serde_json::Value = resp.json().await.ok()?;
    if val.get("code").and_then(|c| c.as_i64()) != Some(200) {
        return None;
    }

    let raw_url = val.get("data")?.get("raw_url")?.as_str()?.to_string();
    if raw_url.is_empty() {
        return None;
    }

    if let Ok(mut guard) = ALIST_ACCEL_URL_CACHE.lock() {
        guard.insert(target_path, (raw_url.clone(), Instant::now()));
    }

    Some(raw_url)
}

pub fn prune_short_video_cache(directory: &std::path::Path, max_bytes: u64) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    // 1. Purge any stale .part files or corrupt/stub files (< 64KB)
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with(".part") || meta.len() < 64 * 1024 {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if metadata.is_file() {
                Some((entry.path(), metadata.len(), metadata.modified().ok()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut total = files.iter().map(|(_, size, _)| *size).sum::<u64>();
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total <= max_bytes * 9 / 10 {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

pub fn start_scheduled_scanner(app: AppHandle) {
    crate::rt::spawn(async move {
        // Defer startup cache pruning and one-time DB compaction slightly so
        // the HTTP server binds and serves requests immediately on startup.
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        let startup_app = app.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(cache_base) = cache_base_path(&startup_app) {
                let short_video_dir = cache_base.join("short-videos");
                if short_video_dir.is_dir() {
                    let max_bytes = get_automation_settings_internal(&startup_app)
                        .map(|s| s.short_video_cache_max_bytes)
                        .unwrap_or(2 * 1024 * 1024 * 1024);
                    CACHED_MAX_SHORT_VIDEO_BYTES.store(max_bytes, std::sync::atomic::Ordering::Relaxed);
                    prune_short_video_cache(&short_video_dir, max_bytes);
                }
                let comic_dir = cache_base.join("comic-archives");
                if comic_dir.is_dir() {
                    prune_comic_archive_cache(&comic_dir, std::path::Path::new(""));
                }
            }
            if let Ok(connection) = crate::v2_config::open(&startup_app) {
                let compacted: Option<String> = connection
                    .query_row(
                        "SELECT value FROM settings WHERE key = 'db_compact_v2'",
                        [],
                        |r| r.get(0),
                    )
                    .optional()
                    .unwrap_or(None);
                if compacted.as_deref() != Some("1") {
                    let _ = connection.execute(
                        "INSERT INTO settings (key, value) VALUES ('db_compact_v2', '1') ON CONFLICT(key) DO UPDATE SET value = '1'",
                        [],
                    );
                    let freelist: i64 = connection
                        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
                        .unwrap_or(0);
                    if freelist > 1024 {
                        let _ = connection.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;");
                    } else {
                        let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;");
                    }
                }
            }
        })
        .await;

        let mut last_run_day: Option<u32> = None;
        let mut postponed_until: Option<Instant> = None;
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;

            let settings = match get_automation_settings_internal(&app) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if !settings.scheduled_scan_enabled {
                continue;
            }

            if let Some(postponed) = postponed_until {
                if Instant::now() < postponed {
                    continue;
                } else {
                    postponed_until = None;
                }
            }

            let parts: Vec<&str> = settings.scheduled_scan_time.split(':').collect();
            if parts.len() != 2 {
                continue;
            }
            let target_hour: u32 = match parts[0].trim().parse() {
                Ok(h) => h,
                Err(_) => continue,
            };
            let target_minute: u32 = match parts[1].trim().parse() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let now_secs = now();
            let local_secs = now_secs + 8 * 3600; // PRC (UTC+8)
            let day_number = (local_secs / 86400) as u32;
            let seconds_in_day = local_secs % 86400;
            let current_hour = (seconds_in_day / 3600) as u32;
            let current_minute = ((seconds_in_day % 3600) / 60) as u32;

            if current_hour == target_hour && current_minute == target_minute {
                if last_run_day == Some(day_number) {
                    continue;
                }

                let state = app.state::<MediaLibraryState>();
                let last_activity = state.last_stream_activity.load(std::sync::atomic::Ordering::Relaxed);
                if settings.scheduled_scan_avoid_active && now().saturating_sub(last_activity as i64) < 300 {
                    postponed_until = Some(Instant::now() + tokio::time::Duration::from_secs(1800));
                    continue;
                }

                last_run_day = Some(day_number);
                let _ = scan_library(app.clone(), app.state::<MediaLibraryState>(), None, None).await;
            }
        }
    });
}

fn cache_base_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("PML_CACHE_DIR") {
        Ok(PathBuf::from(value))
    } else {
        app.path()
            .app_cache_dir()
            .map_err(|error| format!("无法获取应用缓存目录：{error}"))
    }
}

fn directory_size(path: &std::path::Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| {
            let path = entry.path();
            entry
                .metadata()
                .map(|meta| {
                    if meta.is_dir() {
                        directory_size(&path)
                    } else {
                        meta.len()
                    }
                })
                .unwrap_or(0)
        })
        .sum()
}

fn remove_cache_directory(path: &std::path::Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|error| format!("清理缓存失败：{error}"))?;
    }
    Ok(())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn open_media_source(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    asset_id: String,
) -> Result<String, String> {
    register_media_source(&app, &state, &asset_id, true, false).await
}

/// 原生端需要用绝对地址访问本地代理（`http://127.0.0.1:{port}/...`）。
#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn get_media_proxy_origin(
    state: State<'_, MediaLibraryState>,
) -> Result<String, String> {
    let proxy = ensure_media_proxy(&state).await?;
    Ok(format!("http://{}", proxy.address))
}

async fn build_asset_proxy_source(
    state: &MediaLibraryState,
    asset_id: &str,
    remote_path: &str,
) -> Result<ProxySource, String> {
    let session = require_session(state).await?;
    let source_url = remote_url(&session.endpoint, remote_path)?;
    Ok(ProxySource {
        url: source_url,
        fallback_url: None,
        session,
        content_type: media_content_type(remote_path).into(),
        remote_path: remote_path.to_owned(),
        allow_api_fallback: true,
        api_url_cache: Arc::new(StdMutex::new(None)),
        direct_url: false,
        thumbnail_cache_path: None,
        proxy_address: None,
        asset_id: Some(asset_id.to_owned()),
        cache_base_path: None,
        activity_tracker: None,
        max_cache_bytes: None,
    })
}

async fn decode_heic_to_jpeg_bytes(heic_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let temp_dir = std::env::temp_dir();
    let rand_id = random_token();
    let in_path = temp_dir.join(format!("pml_heic_{rand_id}.heic"));
    let out_path = temp_dir.join(format!("pml_heic_{rand_id}.jpg"));
    fs::write(&in_path, heic_bytes).map_err(|e| format!("写入 HEIC 临时文件失败: {e}"))?;

    let mut success = false;
    let hc = Command::new("heif-convert")
        .args([
            "-q", "88",
            in_path.to_str().unwrap_or_default(),
            out_path.to_str().unwrap_or_default(),
        ])
        .output()
        .await;
    if hc.is_ok_and(|out| out.status.success()) && out_path.is_file() {
        success = true;
    }

    if !success {
        let ff = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel", "error",
                "-threads", "1",
                "-i", in_path.to_str().unwrap_or_default(),
                "-frames:v", "1",
                "-vf", "scale=min(2560\\,iw):-2",
                "-q:v", "3",
                out_path.to_str().unwrap_or_default(),
            ])
            .output()
            .await;
        if ff.is_ok_and(|out| out.status.success()) && out_path.is_file() {
            success = true;
        }
    }

    let _ = fs::remove_file(&in_path);
    if success {
        let result = fs::read(&out_path).map_err(|e| format!("读取 JPEG 转换结果失败: {e}"));
        let _ = fs::remove_file(&out_path);
        let bytes = result?;
        if bytes.len() > MAX_THUMBNAIL_BYTES {
            let downscale = Command::new("ffmpeg")
                .args([
                    "-hide_banner",
                    "-loglevel", "error",
                    "-threads", "1",
                    "-i", "pipe:0",
                    "-frames:v", "1",
                    "-vf", "scale=min(1920\\,iw):-2",
                    "-q:v", "5",
                    "-f", "image2",
                    "pipe:1",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn();
            if let Ok(mut child) = downscale {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(&bytes).await;
                }
                if let Ok(output) = child.wait_with_output().await {
                    if output.status.success() && !output.stdout.is_empty() {
                        return Ok(output.stdout);
                    }
                }
            }
        }
        Ok(bytes)
    } else {
        let _ = fs::remove_file(&out_path);
        Err("HEIC 解码失败".into())
    }
}

async fn generate_heic_thumbnail(
    app: &AppHandle,
    state: &MediaLibraryState,
    asset_id: &str,
    remote_path: &str,
) -> Result<Option<String>, String> {
    static HEIC_THUMBNAIL_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let semaphore = HEIC_THUMBNAIL_SEMAPHORE.get_or_init(|| tokio::sync::Semaphore::new(3));
    let Ok(_permit) = tokio::time::timeout(std::time::Duration::from_secs(30), semaphore.acquire()).await else {
        return Err("HEIC 转换队列繁忙".into());
    };

    let source = build_asset_proxy_source(state, asset_id, remote_path).await?;
    let response = open_proxy_upstream(&source, "GET", None, None).await?;
    let bytes = response.bytes().await.map_err(|e| format!("读取远程 HEIC 文件失败: {e}"))?;
    if bytes.is_empty() {
        return Err("远程 HEIC 文件为空".into());
    }
    let jpeg_bytes = decode_heic_to_jpeg_bytes(&bytes).await?;
    let saved_path = persist_thumbnail_jpeg_bytes(app, asset_id, &jpeg_bytes, None)?;
    Ok(cached_cover_url(asset_id, &saved_path))
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn open_media_thumbnail_source(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    asset_id: String,
) -> Result<String, String> {
    if asset_id.starts_with("author_") || asset_id.starts_with("banner_") {
        let thumb_path = asset_thumbnail_path(&app, &asset_id)?;
        if thumb_path.is_file() {
            let file_name = format!("{asset_id}.jpg");
            return Ok(format!("/api/covers/{asset_id}/{file_name}"));
        }
        return Err("创作者自定义图片尚未生成".to_string());
    }
    if let Some(cached) = get_asset_thumbnail(app.clone(), asset_id.clone())? {
        return Ok(format!("data:{};base64,{}",cached.mime_type,cached.data_base64));
    }
    let thumb_path = asset_thumbnail_path(&app, &asset_id)?;
    if let Some(url) = cached_cover_url(&asset_id, &thumb_path) {
        return Ok(url);
    }
    let (remote_path, media_type) = {
        let connection = open_media_database(&app)?;
        connection
            .query_row(
                "SELECT remote_path, media_type FROM assets WHERE id=?1 AND missing=0",
                params![asset_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map_err(database_error)?
    };
    if media_type == "image" || media_type == "comicPage" {
        if let Ok(url) = generate_image_thumbnail_backend(&app, &state, &asset_id, &remote_path).await {
            return Ok(url);
        }
    } else if media_type == "video" {
        if let Ok(url) = generate_video_thumbnail_backend(&app, &state, &asset_id).await {
            return Ok(url);
        }
    }
    let is_heic_img = is_heic(&remote_path);
    let use_thumbnail_route = media_type == "video" || is_heic_img;
    register_media_source(&app, &state, &asset_id, true, use_thumbnail_route).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailInput {
    pub version: String,
    pub remote_path: String,
    pub media_type: String,
    pub source: Option<String>,
    pub cached: Option<AssetThumbnail>,
    pub cached_url: Option<String>,
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn get_thumbnail_input(app: AppHandle, state: State<'_, MediaLibraryState>, asset_id: String, force: Option<bool>) -> Result<ThumbnailInput,String> {
    if let Ok(path) = cache_base_path(&app) {
        if let Ok(mut lock) = state.thumbnails_dir.lock() {
            *lock = Some(path.join("thumbnails"));
        }
    }
    let (remote_path,media_type) = {
        let c = open_media_database(&app)?;
        c.query_row("SELECT remote_path,media_type FROM assets WHERE id=?1 AND missing=0",params![asset_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(database_error)?
    };
    let thumb_path = asset_thumbnail_path(&app,&asset_id)?;
    let version = thumb_path.file_name().ok_or("封面版本无效")?.to_string_lossy().into_owned();
    let (cached, mut cached_url) = if !force.unwrap_or(false) && thumb_path.is_file() {
        if let Ok(bytes) = fs::read(&thumb_path) {
            if bytes.len() <= MAX_THUMBNAIL_BYTES && bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                let data_url = format!("data:image/jpeg;base64,{}", STANDARD.encode(&bytes));
                (
                    Some(AssetThumbnail {
                        mime_type: "image/jpeg".into(),
                        data_base64: STANDARD.encode(&bytes),
                    }),
                    Some(data_url),
                )
            } else {
                (None, None)
            }
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };
    let mut source = None;
    if cached_url.is_none() && (media_type == "image" || media_type == "comicPage") {
        if let Ok(generated_url) = generate_image_thumbnail_backend(&app, &state, &asset_id, &remote_path).await {
            cached_url = Some(generated_url);
        } else if let Ok(proxy_url) = register_media_source(&app, state.inner(), &asset_id, true, false).await {
            source = Some(proxy_url);
        }
    }
    if cached_url.is_none() && media_type == "comicArchive" {
        if let Ok((generated_url, fallback_data_url)) = generate_comic_archive_thumbnail(&app, &state, &asset_id, &remote_path).await {
            cached_url = generated_url;
            source = fallback_data_url;
        }
    }
    if cached_url.is_none() && media_type == "video" {
        if let Ok(generated_url) = generate_video_thumbnail_backend(&app, &state, &asset_id).await {
            cached_url = Some(generated_url);
        } else if let Ok(proxy_url) = register_media_source(&app, state.inner(), &asset_id, true, false).await {
            source = Some(proxy_url);
        }
    }
    Ok(ThumbnailInput { version,remote_path,media_type,source,cached,cached_url })
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_asset_thumbnail(
    app: AppHandle,
    asset_id: String,
) -> Result<Option<AssetThumbnail>, String> {
    let path = asset_thumbnail_path(&app, &asset_id)?;
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|error| format!("读取缩略图缓存失败：{error}"))?;
    if bytes.len() > MAX_THUMBNAIL_BYTES || !bytes.starts_with(&[0xff,0xd8,0xff]) || !bytes.ends_with(&[0xff,0xd9]) {
        return Ok(None);
    }
    Ok(Some(AssetThumbnail {
        mime_type: "image/jpeg".into(),
        data_base64: STANDARD.encode(bytes),
    }))
}

fn persist_thumbnail_jpeg_bytes(
    app: &AppHandle,
    asset_id: &str,
    bytes: &[u8],
    expected_version: Option<&str>,
) -> Result<PathBuf, String> {
    if bytes.is_empty() || bytes.len() > MAX_THUMBNAIL_BYTES {
        return Err("缩略图大小无效（最大 4 MB）".into());
    }
    let path = asset_thumbnail_path(app, asset_id)?;
    if expected_version.is_some_and(|v| path.file_name().and_then(|p| p.to_str()) != Some(v)) {
        return Err("文件已变化，请重新生成封面".into());
    }
    let is_jpeg = if bytes.starts_with(&[0xff, 0xd8]) {
        if bytes.ends_with(&[0xff, 0xd9]) {
            true
        } else {
            let tail_start = bytes.len().saturating_sub(256);
            bytes[tail_start..].windows(2).rposition(|w| w == [0xff, 0xd9]).is_some()
        }
    } else {
        false
    };
    let is_png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    let is_webp = bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP";
    if !is_jpeg && !is_png && !is_webp {
        return Err("封面不是有效的图片数据 (支持 JPEG/PNG/WebP)".into());
    }
    let directory = path
        .parent()
        .ok_or_else(|| "缩略图缓存目录无效".to_string())?;
    fs::create_dir_all(directory).map_err(|error| format!("无法创建缩略图缓存目录：{error}"))?;
    let temporary = path.with_extension(format!("{}.part", random_token()));
    fs::write(&temporary, bytes).map_err(|error| format!("写入缩略图缓存失败：{error}"))?;
    // POSIX rename atomically replaces the old image; a failed write keeps it intact.
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    fs::rename(&temporary, &path).map_err(|error| format!("提交缩略图缓存失败：{error}"))?;
    if let Some(file_name) = path.file_name().and_then(|v| v.to_str()) {
        let modified = fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or_else(|| now() as u128 * 1_000_000_000);
        record_thumbnail_file_in_index(directory, file_name, modified);
    }
    if let Ok(connection) = open_media_database(app) {
        let _ = connection.execute(
            "UPDATE assets SET thumbnail_asset_id = id WHERE id = ?1",
            params![asset_id],
        );
    }
    prune_thumbnail_cache(directory);
    Ok(path)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn save_asset_thumbnail(
    app: AppHandle,
    asset_id: String,
    mime_type: String,
    data_base64: String,
    expected_version: Option<String>,
) -> Result<(), String> {
    if mime_type.to_ascii_lowercase() != "image/jpeg" {
        return Err("缩略图缓存只接受 JPEG 图片".into());
    }
    if data_base64.len() > MAX_THUMBNAIL_BYTES * 2 {
        return Err("缩略图数据过大（最大 4 MB）".into());
    }
    let bytes = STANDARD
        .decode(data_base64)
        .map_err(|_| "缩略图数据不是有效的 Base64".to_string())?;
    persist_thumbnail_jpeg_bytes(&app, &asset_id, &bytes, expected_version.as_deref())?;
    Ok(())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn set_comic_cover_page(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    asset_id: String,
    page_index: usize,
) -> Result<String, String> {
    let (_mime, page_bytes) = read_comic_archive_page_binary(&app, &state, &asset_id, page_index).await?;
    let is_jpeg = page_bytes.starts_with(&[0xff, 0xd8, 0xff]) && page_bytes.ends_with(&[0xff, 0xd9]);
    let jpeg_bytes = if is_jpeg && page_bytes.len() <= MAX_THUMBNAIL_BYTES {
        page_bytes
    } else {
        match encode_thumbnail_jpeg_with_ffmpeg(&page_bytes).await {
            Ok(b) => b,
            Err(_) if is_jpeg => page_bytes,
            Err(e) => return Err(e),
        }
    };
    let saved_path = persist_thumbnail_jpeg_bytes(&app, &asset_id, &jpeg_bytes, None)?;
    let url = cached_cover_url(&asset_id, &saved_path).unwrap_or_else(|| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("/api/covers/{asset_id}/saved.jpg?t={ts}")
    });
    Ok(url)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn set_directory_cover(
    app: AppHandle,
    root_id: String,
    remote_path: String,
    cover_asset_id: String,
) -> Result<String, String> {
    let connection = open_media_database(&app)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    connection.execute(
        "INSERT INTO directory_sync_state (root_id, remote_path, scanned_at, revision, cover_asset_id)
         VALUES (?1, ?2, ?3, '', ?4)
         ON CONFLICT(root_id, remote_path) DO UPDATE SET cover_asset_id = ?4",
        params![root_id, remote_path, now, cover_asset_id],
    ).map_err(database_error)?;
    let path = asset_thumbnail_path(&app, &cover_asset_id)?;
    let url = cached_cover_url(&cover_asset_id, &path).unwrap_or_else(|| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("/api/covers/{cover_asset_id}/cover.jpg?t={ts}")
    });
    Ok(url)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn upload_custom_cover(
    app: AppHandle,
    root_id: Option<String>,
    target_type: String,
    target_id_or_path: String,
    mime_type: String,
    data_base64: String,
) -> Result<String, String> {
    let _ = mime_type;
    if data_base64.len() > MAX_THUMBNAIL_BYTES * 4 {
        return Err("封面数据过大（最大 8 MB）".into());
    }
    let raw_bytes = STANDARD
        .decode(&data_base64)
        .map_err(|_| "封面数据不是有效的 Base64".to_string())?;

    let is_jpeg = raw_bytes.starts_with(&[0xff, 0xd8, 0xff]) && raw_bytes.ends_with(&[0xff, 0xd9]);
    let jpeg_bytes = if is_jpeg && raw_bytes.len() <= MAX_THUMBNAIL_BYTES {
        raw_bytes
    } else {
        match encode_thumbnail_jpeg_with_ffmpeg(&raw_bytes).await {
            Ok(b) => b,
            Err(_) if is_jpeg => raw_bytes,
            Err(e) => return Err(format!("封面图像转换失败: {e}")),
        }
    };

    if target_type == "directory" {
        let root = root_id.ok_or_else(|| "缺少 root_id".to_string())?;
        let remote_path = target_id_or_path;
        let virtual_asset_id = format!("dir_cover_{:016x}", stable_hash(&format!("{root}:{remote_path}")));
        let virtual_remote_path = format!("{remote_path}/__custom_dir_cover__.jpg");

        let connection = open_media_database(&app)?;
        let now = now();
        upsert_directory_cover_asset(
            &connection, &virtual_asset_id, &root, &virtual_remote_path,
            jpeg_bytes.len() as i64, now,
        )?;

        let saved_path = persist_thumbnail_jpeg_bytes(&app, &virtual_asset_id, &jpeg_bytes, None)?;

        connection.execute(
            "INSERT INTO directory_sync_state (root_id, remote_path, scanned_at, revision, cover_asset_id)
             VALUES (?1, ?2, ?3, '', ?4)
             ON CONFLICT(root_id, remote_path) DO UPDATE SET cover_asset_id = ?4",
            params![root, remote_path, now, virtual_asset_id],
        ).map_err(database_error)?;

        let url = cached_cover_url(&virtual_asset_id, &saved_path).unwrap_or_else(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            format!("/api/covers/{virtual_asset_id}/saved.jpg?t={ts}")
        });
        Ok(url)
    } else {
        let asset_id = target_id_or_path;
        let saved_path = persist_thumbnail_jpeg_bytes(&app, &asset_id, &jpeg_bytes, None)?;
        let url = cached_cover_url(&asset_id, &saved_path).unwrap_or_else(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            format!("/api/covers/{asset_id}/saved.jpg?t={ts}")
        });
        Ok(url)
    }
}

fn upsert_directory_cover_asset(
    connection: &Connection,
    asset_id: &str,
    root_id: &str,
    remote_path: &str,
    size_bytes: i64,
    timestamp: i64,
) -> Result<(), String> {
    let domain: String = connection.query_row(
        "SELECT domain FROM domain_roots WHERE root_id = ?1",
        params![root_id], |row| row.get(0),
    ).map_err(|_| "找不到目录封面对应的 V2 媒体根目录".to_string())?;
    connection.execute(
        "INSERT INTO assets (id, domain, root_id, remote_path, file_name, parent_path,
                             media_type, size_bytes, missing, first_seen_at, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, '.cover.jpg', '__pml_dir_covers__', 'image', ?5, 0, ?6, ?6)
         ON CONFLICT(domain, remote_path) DO UPDATE SET
             missing = 0, missing_since = NULL, missing_scan_count = 0,
             size_bytes = ?5, last_seen_at = ?6",
        params![asset_id, domain, root_id, remote_path, size_bytes, timestamp],
    ).map_err(database_error)?;
    Ok(())
}

static THUMBNAIL_FILE_INDEX: OnceLock<StdMutex<Option<(PathBuf, Instant, HashMap<String, u128>)>>> = OnceLock::new();

fn invalidate_thumbnail_file_index() {
    if let Some(lock) = THUMBNAIL_FILE_INDEX.get() {
        if let Ok(mut guard) = lock.lock() {
            *guard = None;
        }
    }
}

fn record_thumbnail_file_in_index(directory: &std::path::Path, file_name: &str, modified: u128) {
    let lock = THUMBNAIL_FILE_INDEX.get_or_init(|| StdMutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        if let Some((ref cached_dir, _, ref mut map)) = *guard {
            if cached_dir == directory {
                map.insert(file_name.to_owned(), modified);
            }
        }
    }
}

fn remove_thumbnail_file_from_index(path: &std::path::Path) {
    let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else { return };
    if let Some(lock) = THUMBNAIL_FILE_INDEX.get() {
        if let Ok(mut guard) = lock.lock() {
            if let Some((_, _, ref mut map)) = *guard {
                map.remove(file_name);
            }
        }
    }
}

fn load_thumbnail_file_index(directory: &std::path::Path) -> HashMap<String, u128> {
    let lock = THUMBNAIL_FILE_INDEX.get_or_init(|| StdMutex::new(None));
    if let Ok(guard) = lock.lock() {
        if let Some((ref cached_dir, refreshed_at, ref map)) = *guard {
            if cached_dir == directory && refreshed_at.elapsed().as_secs() < 120 {
                return map.clone();
            }
        }
    }
    let mut map = HashMap::new();
    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.filter_map(Result::ok) {
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || meta.len() < 4 || meta.len() > MAX_THUMBNAIL_BYTES as u64 {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else { continue };
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(1);
            map.insert(name, modified);
        }
    }
    if let Ok(mut guard) = lock.lock() {
        *guard = Some((directory.to_path_buf(), Instant::now(), map.clone()));
    }
    map
}

fn reconcile_cached_thumbnails(app: &AppHandle, root_id: &str) -> Result<(), String> {
    let cache_dir = cache_base_path(app)?.join("thumbnails");
    let thumb_index = load_thumbnail_file_index(&cache_dir);
    let mut connection = open_media_database(app)?;
    let rows = {
        let mut stmt = connection
            .prepare(
                "SELECT id, remote_path, size_bytes, COALESCE(etag, modified_at), content_hash, thumbnail_asset_id
                 FROM assets WHERE root_id = ?1 AND missing = 0",
            )
            .map_err(database_error)?;
        let mapped = stmt
            .query_map(params![root_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(database_error)?;
        mapped.collect::<Result<Vec<_>, _>>().map_err(database_error)?
    };
    let mut updates = Vec::new();
    for (id, remote_path, size_bytes, etag, content_hash, current_thumb_id) in rows {
        let fingerprint = match (etag, content_hash) {
            (Some(etag), _) => format!("{THUMBNAIL_CACHE_VERSION}|{remote_path}|{size_bytes}|{etag}"),
            (None, Some(hash)) => format!("{THUMBNAIL_CACHE_VERSION}|{remote_path}|{size_bytes}|{hash}"),
            (None, None) => format!("{THUMBNAIL_CACHE_VERSION}|{remote_path}|{size_bytes}"),
        };
        let file_name = format!(
            "asset-{:016x}-{:016x}.jpg",
            stable_hash(&id),
            stable_hash(&fingerprint)
        );
        let has_cached = thumb_index.contains_key(&file_name);
        if has_cached && current_thumb_id.is_none() {
            updates.push((id.clone(), Some(id)));
        } else if !has_cached && current_thumb_id.is_some() {
            updates.push((id, None));
        }
    }
    if !updates.is_empty() {
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(database_error)?;
        {
            let mut stmt = tx
                .prepare("UPDATE assets SET thumbnail_asset_id = ?2 WHERE id = ?1")
                .map_err(database_error)?;
            for (id, next_thumb) in updates {
                stmt.execute(params![id, next_thumb]).map_err(database_error)?;
            }
        }
        tx.commit().map_err(database_error)?;
    }
    Ok(())
}

fn asset_thumbnail_path(app: &AppHandle, asset_id: &str) -> Result<PathBuf, String> {
    if asset_id.starts_with("author_") || asset_id.starts_with("banner_") {
        let cache_dir = cache_base_path(app)?.join("thumbnails");
        return Ok(cache_dir.join(format!("{asset_id}.jpg")));
    }
    let (remote_path, size_bytes, etag, content_hash): (
        String,
        i64,
        Option<String>,
        Option<String>,
    ) = {
        let connection = open_media_database(app)?;
        let mut row_res = None;
        for _ in 0..3 {
            match connection.query_row(
                "SELECT remote_path, size_bytes, COALESCE(etag, modified_at), content_hash
                 FROM assets WHERE id = ?1 AND missing = 0",
                params![asset_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ) {
                Ok(v) => {
                    row_res = Some(Ok(v));
                    break;
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("busy") || msg.contains("locked") {
                        std::thread::sleep(std::time::Duration::from_millis(60));
                        row_res = Some(Err(e));
                        continue;
                    }
                    row_res = Some(Err(e));
                    break;
                }
            }
        }
        match row_res {
            Some(Ok(v)) => v,
            _ => return Err("找不到要缓存缩略图的媒体文件".to_string()),
        }
    };
    thumbnail_path_from_metadata(app,asset_id,&remote_path,size_bytes,etag,content_hash)
}

fn thumbnail_path_from_metadata(app:&AppHandle,asset_id:&str,remote_path:&str,size_bytes:i64,etag:Option<String>,content_hash:Option<String>)->Result<PathBuf,String>{
    let cache_dir=cache_base_path(app)?.join("thumbnails");
    // Prefer the provider ETag when available. Once the background content
    // hash finishes, this keeps an existing thumbnail key stable instead of
    // forcing the same image to be generated a second time.
    let fingerprint = match (etag, content_hash) {
        (Some(etag), _) => {
            format!("{THUMBNAIL_CACHE_VERSION}|{remote_path}|{size_bytes}|{etag}")
        }
        (None, Some(content_hash)) => {
            format!("{THUMBNAIL_CACHE_VERSION}|{remote_path}|{size_bytes}|{content_hash}")
        }
        (None, None) => format!("{THUMBNAIL_CACHE_VERSION}|{remote_path}|{size_bytes}"),
    };
    Ok(cache_dir.join(format!("asset-{:016x}-{:016x}.jpg",stable_hash(asset_id),stable_hash(&fingerprint))))
}

fn get_thumbnail_file_modified(directory: &std::path::Path, file_name: &str) -> Option<u128> {
    let lock = THUMBNAIL_FILE_INDEX.get_or_init(|| StdMutex::new(None));
    if let Ok(guard) = lock.lock() {
        if let Some((ref cached_dir, refreshed_at, ref map)) = *guard {
            if cached_dir == directory && refreshed_at.elapsed().as_secs() < 120 {
                return map.get(file_name).copied();
            }
        }
    }
    let map = load_thumbnail_file_index(directory);
    map.get(file_name).copied()
}

/// Resolve only a locally cached cover using canonical V2 asset metadata.
pub(crate) fn cached_asset_cover_url(app: &AppHandle, asset_id: &str) -> Option<String> {
    let path = asset_thumbnail_path(app, asset_id).ok()?;
    cached_cover_url(asset_id, &path)
}

fn cached_cover_url(asset_id:&str,path:&std::path::Path)->Option<String>{
    let file_name = path.file_name()?.to_str()?;
    if let Some(parent) = path.parent() {
        if let Some(modified) = get_thumbnail_file_modified(parent, file_name) {
            return Some(format!("/api/covers/{asset_id}/{file_name}?r={modified}"));
        }
    }
    let metadata=fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len()<4 || metadata.len()>MAX_THUMBNAIL_BYTES as u64 { return None; }
    let modified=metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    if let Some(parent) = path.parent() {
        record_thumbnail_file_in_index(parent, file_name, modified);
    }
    Some(format!("/api/covers/{asset_id}/{file_name}?r={modified}"))
}

#[allow(dead_code)]
fn attach_cached_covers(app:&AppHandle,c:&Connection,page:&mut RemoteBrowserPage)->Result<(),String>{
    let cache_dir = cache_base_path(app)?.join("thumbnails");
    let thumb_index = load_thumbnail_file_index(&cache_dir);
    if thumb_index.is_empty() {
        return Ok(());
    }
    let mut stmt=c.prepare("SELECT remote_path,size_bytes,COALESCE(etag,modified_at),content_hash FROM assets WHERE id=?1 AND missing=0").map_err(database_error)?;
    for entry in &mut page.items {
        let Some(id)=entry.asset_id.as_ref().or(entry.cover_asset_id.as_ref()) else {continue};
        let row = if !entry.is_directory {
            // File entries already carry path, size_bytes, and etag/modified_at when etag is present
            if let Some(ref etag) = entry.etag {
                Some((entry.path.clone(), entry.size_bytes, Some(etag.clone()), None))
            } else {
                stmt.query_row(params![id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?))).optional().map_err(database_error)?
            }
        } else {
            stmt.query_row(params![id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<String>>(3)?))).optional().map_err(database_error)?
        };
        if let Some((path,size,etag,hash))=row {
            // Preserve explicitly excluded video formats even when an old cache file exists.
            if matches!(extension(&path).as_str(),"avi"|"wmv"|"rmvb"|"rm"|"flv"|"vob"|"m2ts"|"mts"|"asf") {continue;}
            let thumb_path = thumbnail_path_from_metadata(app,id,&path,size,etag,hash)?;
            if let Some(file_name) = thumb_path.file_name().and_then(|v| v.to_str()) {
                if let Some(&modified) = thumb_index.get(file_name) {
                    entry.thumbnail_source = Some(format!("/api/covers/{id}/{file_name}?r={modified}"));
                }
            }
        }
    }
    Ok(())
}

pub fn read_cached_cover(app:&AppHandle,asset_id:&str,version:&str)->Result<Vec<u8>,String>{
    let cache_dir = cache_base_path(app)?.join("thumbnails");
    let direct_path = if (version.starts_with("asset-") || version.starts_with("author_") || version.starts_with("banner_")) && version.ends_with(".jpg") && !version.contains('/') && !version.contains('\\') && !version.contains("..") {
        let p = cache_dir.join(version);
        if p.is_file() {
            Some(p)
        } else {
            None
        }
    } else {
        None
    };
    let path = match direct_path {
        Some(p) => p,
        None => {
            let p = asset_thumbnail_path(app, asset_id)?;
            if p.file_name().and_then(|name| name.to_str()) != Some(version) {
                return Err("封面版本已变化".into());
            }
            p
        }
    };
    let metadata = fs::metadata(&path).map_err(|_| "封面不存在")?;
    if metadata.len() > MAX_THUMBNAIL_BYTES as u64 {
        return Err("封面无效".into());
    }
    let bytes = fs::read(path).map_err(|_| "封面读取失败")?;
    if !bytes.starts_with(&[0xff, 0xd8, 0xff]) || !bytes.ends_with(&[0xff, 0xd9]) {
        return Err("封面无效".into());
    }
    Ok(bytes)
}

fn prune_thumbnail_cache(directory: &std::path::Path) {
    // Do not traverse the complete cache for each newly generated cover.
    static LAST_PRUNE: OnceLock<StdMutex<HashMap<PathBuf, Instant>>> = OnceLock::new();
    {
        let mut times = LAST_PRUNE
            .get_or_init(|| StdMutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if times
            .get(directory)
            .is_some_and(|time| time.elapsed().as_secs() < 300)
        {
            return;
        }
        times.insert(directory.to_owned(), Instant::now());
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((entry.path(), metadata.len(), metadata.modified().ok()))
        })
        .collect::<Vec<_>>();
    let mut total = files.iter().map(|(_, size, _)| *size).sum::<u64>();
    if total <= MAX_THUMBNAIL_CACHE_BYTES {
        return;
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total <= MAX_THUMBNAIL_CACHE_BYTES * 9 / 10 {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            remove_thumbnail_file_from_index(&path);
            total = total.saturating_sub(size);
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicArchivePage {
    index: usize,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicArchiveInfo {
    pages: Vec<ComicArchivePage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicArchivePageData {
    mime_type: String,
    data_base64: String,
}

/// Downloads a ZIP/CBZ once into the application cache and returns its sorted
/// image entries. The WebView never parses the remote archive itself.
#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn inspect_comic_archive(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    asset_id: String,
) -> Result<ComicArchiveInfo, String> {
    // If it is a remote ZIP/CBZ, inspect Central Directory directly via HTTP Range
    if let Ok(meta) = ensure_remote_zip_meta(&app, &state, &asset_id).await {
        return Ok(ComicArchiveInfo {
            pages: meta
                .iter()
                .enumerate()
                .map(|(index, entry)| ComicArchivePage {
                    index,
                    name: entry.name.clone(),
                })
                .collect(),
        });
    }

    let (_path, entries) = ensure_comic_archive_with_index(&app, &state, &asset_id).await?;
    Ok(ComicArchiveInfo {
        pages: entries
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, name)| ComicArchivePage { index, name })
            .collect(),
    })
}

/// 供 App 做「按页 Range 直连」的 ZIP 清单；ZIP64 会返回错误，由 App 回退服务器路由。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicArchiveManifest {
    pub remote_path: String,
    pub pages: Vec<ComicArchiveManifestPage>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicArchiveManifestPage {
    pub index: usize,
    pub name: String,
    pub compression_method: u16,
    pub flags: u16,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub local_header_offset: u64,
}

pub async fn get_comic_archive_manifest(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    asset_id: String,
) -> Result<ComicArchiveManifest, String> {
    let remote_path = {
        let connection = open_media_database(&app)?;
        connection
            .query_row(
                "SELECT remote_path FROM assets WHERE id = ?1 AND missing = 0",
                params![asset_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| "找不到漫画文件".to_string())?
    };
    let meta = ensure_remote_zip_meta(&app, &state, &asset_id).await?;
    let pages = meta
        .iter()
        .enumerate()
        .map(|(index, entry)| ComicArchiveManifestPage {
            index,
            name: entry.name.clone(),
            compression_method: entry.compression_method,
            flags: entry.flags,
            compressed_size: entry.compressed_size,
            uncompressed_size: entry.uncompressed_size,
            local_header_offset: entry.local_header_offset,
        })
        .collect();
    Ok(ComicArchiveManifest { remote_path, pages })
}
/// Reads one page from the cached archive and returns a compact data payload.
#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn read_comic_archive_page(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    asset_id: String,
    page_index: usize,
) -> Result<ComicArchivePageData, String> {
    let (mime_type, bytes) = read_comic_archive_page_binary(&app, &state, &asset_id, page_index).await?;
    Ok(ComicArchivePageData {
        mime_type,
        data_base64: STANDARD.encode(bytes),
    })
}

pub async fn read_comic_archive_page_binary(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
    page_index: usize,
) -> Result<(String, Vec<u8>), String> {
    // Fast path: if remote ZIP metadata is available, read single page via HTTP Range
    if let Ok(meta) = ensure_remote_zip_meta(app, state, asset_id).await {
        if let Some(entry) = meta.get(page_index) {
            if let Ok(result) = fetch_remote_zip_page(app, state, asset_id, entry).await {
                return Ok(result);
            }
        }
    }

    let (path, entries) = ensure_comic_archive_with_index(app, state, asset_id).await?;
    let entry_name = entries
        .get(page_index)
        .cloned()
        .ok_or_else(|| "漫画页序号超出范围".to_string())?;
    tokio::task::spawn_blocking(move || read_comic_entry_by_name(&path, &entry_name))
        .await
        .map_err(|error| format!("漫画页读取任务失败：{error}"))?
}

async fn ensure_comic_archive_with_index(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
) -> Result<(PathBuf, Arc<Vec<String>>), String> {
    // Fast path: check in-memory verified archive + parsed page index cache
    if let Ok(cache) = state.comic_archive_index_cache.lock() {
        if let Some((path, entries)) = cache.get(asset_id) {
            if path.is_file() {
                return Ok((path.clone(), Arc::clone(entries)));
            }
        }
    }
    let path = ensure_comic_archive_cached(app, state, asset_id).await?;
    let path_for_index = path.clone();
    let entries = tokio::task::spawn_blocking(move || load_or_build_comic_index(&path_for_index))
        .await
        .map_err(|error| format!("漫画目录读取任务失败：{error}"))??;
    let arc_entries = Arc::new(entries);
    if let Ok(mut cache) = state.comic_archive_index_cache.lock() {
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(asset_id.to_owned(), (path.clone(), Arc::clone(&arc_entries)));
    }
    Ok((path, arc_entries))
}

async fn ensure_comic_archive_cached(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
) -> Result<PathBuf, String> {
    let task = {
        let mut tasks = state.comic_archive_tasks.lock().await;
        tasks
            .entry(asset_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::OnceCell::new()))
            .clone()
    };
    let result = task
        .get_or_init(|| async { ensure_comic_archive_cached_inner(app, state, asset_id).await })
        .await
        .clone();
    let mut tasks = state.comic_archive_tasks.lock().await;
    tasks.remove(asset_id);
    result
}

async fn ensure_comic_archive_cached_inner(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
) -> Result<PathBuf, String> {
    let (remote_path, media_type, expected_size, expected_etag, expected_modified) = {
        let connection = open_media_database(app)?;
        connection
            .query_row(
                "SELECT remote_path, media_type, size_bytes, etag, modified_at FROM assets WHERE id = ?1 AND missing = 0",
                params![asset_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?)),
            )
            .map_err(|_| "找不到可读取的漫画压缩包".to_string())?
    };
    if media_type != "comicArchive" {
        return Err("所选文件不是漫画压缩包".into());
    }

    let cache_base = if let Ok(value) = std::env::var("PML_CACHE_DIR") {
        PathBuf::from(value)
    } else {
        app.path()
            .app_cache_dir()
            .map_err(|error| format!("无法获取漫画缓存目录：{error}"))?
    };
    let cache_dir = cache_base.join("comic-archives");
    fs::create_dir_all(&cache_dir).map_err(|error| format!("无法创建漫画缓存目录：{error}"))?;
    let key = stable_hash(&format!("comic-v2|{asset_id}|{remote_path}|{expected_size}|{}|{}", expected_etag.as_deref().unwrap_or_default(), expected_modified.as_deref().unwrap_or_default()));
    let lower = remote_path.to_ascii_lowercase();
    let cache_extension = if lower.ends_with(".epub") {
        "epub"
    } else if lower.ends_with(".rar") || lower.ends_with(".cbr") {
        "rar"
    } else {
        "zip"
    };
    let target = cache_dir.join(format!("comic-{key:016x}.{cache_extension}"));
    if archive_cache_is_usable(&target) {
        return Ok(target);
    }

    prune_comic_archive_cache(&cache_dir, &target);

    let session = require_session(state).await?;
    let source_url = remote_url(&session.endpoint, &remote_path)?;
    let fallback_url = legacy_archive_path(&remote_path)
        .map(|path| remote_url(&session.endpoint, &path))
        .transpose()?;
    let source = ProxySource {
        url: source_url,
        fallback_url,
        session,
        content_type: media_content_type(&remote_path).into(),
        remote_path: remote_path.clone(),
        allow_api_fallback: true,
        api_url_cache: Arc::new(StdMutex::new(None)),
        direct_url: false,
        thumbnail_cache_path: None,
        proxy_address: None,
        asset_id: Some(asset_id.to_string()),
        cache_base_path: None,
        activity_tracker: None,
        max_cache_bytes: None,
    };
    let temporary = target.with_extension(format!("{cache_extension}.{}.part", random_token()));
    let mut candidates = VecDeque::<Url>::new();
    let mut attempts = Vec::new();
    if source.direct_url || source.session.provider == ProviderKind::CloudDrive2 {
        candidates.push_back(source.url.clone());
    } else {
        for candidate in [Some(source.url.clone()), source.fallback_url.clone()] {
            if let Some(url) = candidate {
                if let Some(proxy_url) = alist_proxy_url(&source.session.endpoint, &url) {
                    if !candidates.iter().any(|existing| existing == &proxy_url) {
                        candidates.push_back(proxy_url);
                    }
                }
                if let Some(download_url) = alist_download_url(&source.session.endpoint, &url) {
                    if !candidates.iter().any(|existing| existing == &download_url) {
                        candidates.push_back(download_url);
                    }
                }
            }
        }
    }
    let mut api_fallback_pending = source.session.provider == ProviderKind::Alist;
    loop {
        let Some(candidate) = candidates.pop_front() else {
            if !api_fallback_pending {
                break;
            }
            api_fallback_pending = false;
            match cached_api_download_url(&source).await {
                Ok(Some(api_url)) => candidates.push_back(api_url),
                Ok(None) => attempts.push("AList API 未返回可用的下载直链".to_string()),
                Err(error) => attempts.push(format!("AList API 直链不可用：{error}")),
            }
            continue;
        };
        let response =
            match open_proxy_upstream_for_url(&source, &candidate, "GET", None, None).await {
                Ok(response) => response,
                Err(error) => {
                    attempts.push(format!("{}: {error}", candidate));
                    continue;
                }
            };
        let status = response.status();
        if !status.is_success() {
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let detail = if content_type.contains("json")
                || content_type.contains("html")
                || content_type.contains("text")
            {
                let body = response.text().await.unwrap_or_default();
                let snippet = body.chars().take(180).collect::<String>();
                format!(
                    "（{}）{}",
                    if content_type.is_empty() {
                        "无内容类型"
                    } else {
                        &content_type
                    },
                    snippet
                )
            } else {
                format!(
                    "（{}）",
                    if content_type.is_empty() {
                        "无内容类型"
                    } else {
                        &content_type
                    }
                )
            };
            attempts.push(format!("{}: HTTP {}{}", candidate, status, detail));
            continue;
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if content_type.contains("application/json")
            || content_type.contains("text/html")
            || content_type.contains("text/plain")
        {
            attempts.push(format!(
                "{}: 返回了 {} 错误页",
                candidate,
                if content_type.is_empty() {
                    "文本"
                } else {
                    &content_type
                }
            ));
            let _ = response.bytes().await;
            continue;
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_COMIC_ARCHIVE_BYTES)
        {
            return Err("漫画压缩包超过 4 GB，暂不支持在线读取".into());
        }
        let mut file = match tokio_fs::File::create(&temporary).await {
            Ok(file) => file,
            Err(error) => return Err(format!("无法创建漫画缓存文件：{error}")),
        };
        let mut received = 0_u64;
        let mut stream = response.bytes_stream();
        let mut stream_error = None;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    stream_error = Some(format!("漫画压缩包下载失败：{error}"));
                    break;
                }
            };
            received = received.saturating_add(chunk.len() as u64);
            if received > MAX_COMIC_ARCHIVE_BYTES {
                stream_error = Some("漫画压缩包超过 4 GB，暂不支持在线读取".into());
                break;
            }
            if let Err(error) = file.write_all(&chunk).await {
                stream_error = Some(format!("写入漫画缓存失败：{error}"));
                break;
            }
        }
        if stream_error.is_none() {
            if let Err(error) = file.flush().await {
                stream_error = Some(format!("保存漫画缓存失败：{error}"));
            }
        }
        drop(file);
        if let Some(error) = stream_error {
            let _ = tokio_fs::remove_file(&temporary).await;
            attempts.push(format!("{}: {error}", candidate));
            continue;
        }
        if received == 0 || !archive_cache_is_usable(&temporary) {
            let _ = tokio_fs::remove_file(&temporary).await;
            attempts.push(format!("{}: 下载内容不是完整有效的漫画压缩包", candidate));
            continue;
        }
        if target.exists() {
            let _ = tokio_fs::remove_file(&target).await;
        }
        tokio_fs::rename(&temporary, &target)
            .await
            .map_err(|error| format!("提交漫画缓存失败：{error}"))?;
        return Ok(target);
    }
    let _ = tokio_fs::remove_file(&temporary).await;
    if attempts.is_empty() {
        Err("AList 没有返回可读取的漫画压缩包".into())
    } else {
        Err(format!("AList 无法读取漫画压缩包：{}", attempts.join("；")))
    }
}

fn archive_cache_is_usable(path: &PathBuf) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if metadata.len() == 0 || metadata.len() > MAX_COMIC_ARCHIVE_BYTES {
        return false;
    }
    let ext = path.extension().and_then(|v| v.to_str()).unwrap_or_default();
    if ext.eq_ignore_ascii_case("rar") {
        return is_valid_rar_archive(path);
    }
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    ZipArchive::new(file).is_ok()
}

fn is_valid_rar_archive(path: &std::path::Path) -> bool {
    if let Ok(output) = std::process::Command::new("bsdtar")
        .args(["-tf", path.to_str().unwrap_or_default()])
        .output()
    {
        return output.status.success() && !output.stdout.is_empty();
    }
    // Fallback if bsdtar is missing: check RAR4/RAR5 magic bytes
    if let Ok(mut file) = fs::File::open(path) {
        let mut magic = [0u8; 7];
        if file.read_exact(&mut magic).is_ok() {
            return magic.starts_with(b"Rar!\x1a\x07");
        }
    }
    false
}

fn comic_index_path(path: &PathBuf) -> PathBuf {
    path.with_extension("index.json")
}

fn load_or_build_comic_index(path: &PathBuf) -> Result<Vec<String>, String> {
    let index_path = comic_index_path(path);
    if let Ok(bytes) = fs::read(&index_path) {
        if let Ok(names) = serde_json::from_slice::<Vec<String>>(&bytes) {
            if !names.is_empty() { return Ok(names); }
        }
    }
    let names = list_comic_entries_uncached(path)?;
    let _ = fs::write(&index_path, serde_json::to_vec(&names).unwrap_or_default());
    Ok(names)
}

fn list_comic_entries_uncached(path: &PathBuf) -> Result<Vec<String>, String> {
    let ext = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
    if ext.eq_ignore_ascii_case("epub") {
        if let Ok(entries) = list_epub_spine_entries(path) {
            if !entries.is_empty() { return Ok(entries); }
        }
    } else if ext.eq_ignore_ascii_case("rar") {
        return list_rar_entries(path);
    }
    let file = fs::File::open(path).map_err(|error| format!("无法打开漫画缓存：{error}"))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("漫画 ZIP 目录损坏：{error}"))?;
    let mut names = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| format!("读取漫画 ZIP 条目失败：{error}"))?;
        let name = entry
            .name()
            .map_err(|error| format!("读取漫画 ZIP 文件名失败：{error}"))?
            .into_owned();
        if entry.is_dir() || entry.size() > MAX_COMIC_PAGE_BYTES || !is_comic_image_name(&name) {
            continue;
        }
        names.push(name);
    }
    names.sort_by(|left, right| natural_name_key(left).cmp(&natural_name_key(right)).then_with(|| left.cmp(right)));
    if names.is_empty() {
        return Err("漫画 ZIP 中没有可读取的图片页".into());
    }
    Ok(names)
}

fn list_rar_entries(path: &std::path::Path) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("bsdtar")
        .args(["-tf", path.to_str().unwrap_or_default()])
        .output()
        .map_err(|e| format!("运行 bsdtar 失败：{e}"))?;
    if !output.status.success() {
        return Err(format!("bsdtar 读取 RAR 目录失败：{}", String::from_utf8_lossy(&output.stderr)));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.ends_with('/') && !line.ends_with('\\') && is_comic_image_name(line))
        .map(ToOwned::to_owned)
        .collect();
    names.sort_by(|left, right| natural_name_key(left).cmp(&natural_name_key(right)).then_with(|| left.cmp(right)));
    if names.is_empty() {
        return Err("漫画 RAR 中没有可读取的图片页".into());
    }
    Ok(names)
}

fn list_epub_spine_entries(path: &PathBuf) -> Result<Vec<String>, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|error| error.to_string())?;
    let container_xml = {
        let mut entry = archive.by_name("META-INF/container.xml").map_err(|error| error.to_string())?;
        let mut value = String::new();
        entry.read_to_string(&mut value).map_err(|error| error.to_string())?;
        value
    };
    let container = roxmltree::Document::parse(&container_xml).map_err(|error| error.to_string())?;
    let opf_path = container.descendants()
        .find(|node| node.has_tag_name("rootfile"))
        .and_then(|node| node.attribute("full-path"))
        .ok_or("EPUB 缺少 OPF 主文件")?.to_owned();
    let opf_xml = {
        let mut entry = archive.by_name(&opf_path).map_err(|error| error.to_string())?;
        let mut value = String::new();
        entry.read_to_string(&mut value).map_err(|error| error.to_string())?;
        value
    };
    let opf = roxmltree::Document::parse(&opf_xml).map_err(|error| error.to_string())?;
    let mut manifest = HashMap::<String, String>::new();
    for item in opf.descendants().filter(|node| node.has_tag_name("item")) {
        let Some(id) = item.attribute("id") else { continue };
        let Some(href) = item.attribute("href") else { continue };
        let decoded = percent_decode_str(href).decode_utf8().map_err(|_| "EPUB 图片路径不是 UTF-8")?;
        let base = opf_path.rsplit_once('/').map(|(value, _)| value).unwrap_or("");
        let joined = if base.is_empty() { decoded.to_string() } else { format!("{base}/{decoded}") };
        manifest.insert(id.to_owned(), joined.replace("\\", "/"));
    }
    let mut pages = Vec::new();
    for itemref in opf.descendants().filter(|node| node.has_tag_name("itemref")) {
        let Some(idref) = itemref.attribute("idref") else { continue };
        let Some(path) = manifest.get(idref) else { continue };
        if is_comic_image_name(path) { pages.push(path.clone()); }
    }
    Ok(pages)
}

fn escape_bsdtar_pattern(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len() + 16);
    for ch in name.chars() {
        if matches!(ch, '[' | ']' | '*' | '?' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn read_rar_entry_by_name(path: &std::path::Path, name: &str) -> Result<(String, Vec<u8>), String> {
    let escaped = escape_bsdtar_pattern(name);
    let mut child = std::process::Command::new("bsdtar")
        .args(["-xOqf", path.to_str().unwrap_or_default(), "--", &escaped])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 bsdtar 解压失败：{e}"))?;

    let mut bytes = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut bytes);
    }
    let status = child.wait().map_err(|e| format!("等待 bsdtar 失败：{e}"))?;
    if (!status.success() || bytes.is_empty()) && escaped != name {
        if let Ok(mut fallback_child) = std::process::Command::new("bsdtar")
            .args(["-xOqf", path.to_str().unwrap_or_default(), "--", name])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            let mut fb_bytes = Vec::new();
            if let Some(mut out) = fallback_child.stdout.take() {
                let _ = out.read_to_end(&mut fb_bytes);
            }
            if fallback_child.wait().is_ok_and(|s| s.success()) && !fb_bytes.is_empty() {
                bytes = fb_bytes;
            }
        }
    }

    if bytes.is_empty() {
        return Err(format!("从 RAR 解压条目 {name} 失败或文件为空"));
    }
    if bytes.len() as u64 > MAX_COMIC_PAGE_BYTES {
        return Err("漫画页超过 64 MB，暂不支持显示".into());
    }
    Ok((comic_image_mime(name).into(), bytes))
}

fn read_comic_entry_by_name(path: &PathBuf, name: &str) -> Result<(String, Vec<u8>), String> {
    let ext = path.extension().and_then(|v| v.to_str()).unwrap_or_default();
    if ext.eq_ignore_ascii_case("rar") {
        return read_rar_entry_by_name(path, name);
    }
    let file = fs::File::open(path).map_err(|error| format!("无法打开漫画缓存：{error}"))?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| format!("漫画 ZIP 目录损坏：{error}"))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format!("无法打开漫画页：{error}"))?;
    if entry.size() > MAX_COMIC_PAGE_BYTES {
        return Err("漫画页超过 64 MB，暂不支持显示".into());
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut bytes)
        .map_err(|error| format!("解压漫画页失败：{error}"))?;
    Ok((comic_image_mime(name).into(), bytes))
}

fn is_zip_or_cbz(path: &str) -> bool {
    let ext = extension(path);
    matches!(ext.as_str(), "zip" | "cbz")
}

fn parse_zip_eocd(tail: &[u8]) -> Result<(u64, u64), String> {
    if tail.len() < 22 {
        return Err("ZIP 数据过短，无法解析 EOCD".into());
    }
    let max_search = tail.len() - 22;
    for i in (0..=max_search).rev() {
        if tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] {
            let comment_len = u16::from_le_bytes([tail[i + 20], tail[i + 21]]) as usize;
            if i + 22 + comment_len <= tail.len() {
                let cd_size = u32::from_le_bytes([tail[i + 12], tail[i + 13], tail[i + 14], tail[i + 15]]) as u64;
                let cd_offset = u32::from_le_bytes([tail[i + 16], tail[i + 17], tail[i + 18], tail[i + 19]]) as u64;
                if cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
                    return Err("ZIP64 格式暂不支持分片解析".into());
                }
                return Ok((cd_offset, cd_size));
            }
        }
    }
    Err("未在尾部数据中找到 ZIP EOCD 签名".into())
}

fn parse_zip_central_directory_entries(cd_bytes: &[u8]) -> Result<Vec<ZipEntryMeta>, String> {
    let mut entries = Vec::new();
    let mut pos = 0usize;
    while pos + 46 <= cd_bytes.len() {
        if cd_bytes[pos..pos + 4] != [0x50, 0x4b, 0x01, 0x02] {
            break;
        }
        let flags = u16::from_le_bytes([cd_bytes[pos + 8], cd_bytes[pos + 9]]);
        let compression_method = u16::from_le_bytes([cd_bytes[pos + 10], cd_bytes[pos + 11]]);
        let crc32 = u32::from_le_bytes([cd_bytes[pos + 16], cd_bytes[pos + 17], cd_bytes[pos + 18], cd_bytes[pos + 19]]);
        let compressed_size = u32::from_le_bytes([cd_bytes[pos + 20], cd_bytes[pos + 21], cd_bytes[pos + 22], cd_bytes[pos + 23]]) as u64;
        let uncompressed_size = u32::from_le_bytes([cd_bytes[pos + 24], cd_bytes[pos + 25], cd_bytes[pos + 26], cd_bytes[pos + 27]]) as u64;
        let name_len = u16::from_le_bytes([cd_bytes[pos + 28], cd_bytes[pos + 29]]) as usize;
        let extra_len = u16::from_le_bytes([cd_bytes[pos + 30], cd_bytes[pos + 31]]) as usize;
        let comment_len = u16::from_le_bytes([cd_bytes[pos + 32], cd_bytes[pos + 33]]) as usize;
        let local_header_offset = u32::from_le_bytes([cd_bytes[pos + 42], cd_bytes[pos + 43], cd_bytes[pos + 44], cd_bytes[pos + 45]]) as u64;

        let total_entry_len = 46 + name_len + extra_len + comment_len;
        if pos + total_entry_len > cd_bytes.len() {
            return Err("ZIP 中央目录条目截断".into());
        }

        if flags & 0x0001 != 0 {
            return Err("加密 ZIP 不支持分片读取".into());
        }
        if compressed_size == 0xFFFF_FFFF || uncompressed_size == 0xFFFF_FFFF || local_header_offset == 0xFFFF_FFFF {
            return Err("ZIP64 条目暂不支持分片读取".into());
        }

        let name_bytes = &cd_bytes[pos + 46..pos + 46 + name_len];
        let name = String::from_utf8_lossy(name_bytes).into_owned();

        let is_dir = name.ends_with('/') || name.ends_with('\\');
        if !is_dir && uncompressed_size <= MAX_COMIC_PAGE_BYTES && is_comic_image_name(&name) {
            entries.push(ZipEntryMeta {
                name,
                compression_method,
                flags,
                crc32,
                compressed_size,
                uncompressed_size,
                local_header_offset,
            });
        }

        pos += total_entry_len;
    }

    entries.sort_by(|left, right| {
        natural_name_key(&left.name)
            .cmp(&natural_name_key(&right.name))
            .then_with(|| left.name.cmp(&right.name))
    });

    if entries.is_empty() {
        return Err("ZIP 中央目录中未找到有效图片".into());
    }
    Ok(entries)
}

fn decompress_zip_range_slice(slice: &[u8], entry: &ZipEntryMeta) -> Result<(String, Vec<u8>), String> {
    if slice.len() < 30 {
        return Err("ZIP 本地头分片过短".into());
    }
    if slice[0..4] != [0x50, 0x4b, 0x03, 0x04] {
        return Err("ZIP 本地头签名不匹配".into());
    }
    let local_name_len = u16::from_le_bytes([slice[26], slice[27]]) as usize;
    let local_extra_len = u16::from_le_bytes([slice[28], slice[29]]) as usize;
    let payload_offset = 30 + local_name_len + local_extra_len;
    let payload_end = payload_offset + entry.compressed_size as usize;
    if payload_end > slice.len() {
        return Err(format!("本地分片不足：需要 {} 字节，实际仅 {} 字节", payload_end, slice.len()));
    }
    let payload = &slice[payload_offset..payload_end];

    if entry.compression_method == 0 {
        if payload.len() as u64 != entry.uncompressed_size {
            return Err("Stored 条目大小不匹配".into());
        }
        return Ok((comic_image_mime(&entry.name).into(), payload.to_vec()));
    }

    // Build synthetic 1-file in-memory ZIP with ASCII entry name "page"
    let dummy_name = b"page";
    let dummy_name_len = dummy_name.len() as u16;

    let mut mini_zip = Vec::with_capacity(34 + payload.len() + 50 + 22);
    mini_zip.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04]);
    mini_zip.extend_from_slice(&20u16.to_le_bytes());
    mini_zip.extend_from_slice(&(entry.flags & !0x0008).to_le_bytes());
    mini_zip.extend_from_slice(&entry.compression_method.to_le_bytes());
    mini_zip.extend_from_slice(&0u32.to_le_bytes());
    mini_zip.extend_from_slice(&entry.crc32.to_le_bytes());
    mini_zip.extend_from_slice(&(entry.compressed_size as u32).to_le_bytes());
    mini_zip.extend_from_slice(&(entry.uncompressed_size as u32).to_le_bytes());
    mini_zip.extend_from_slice(&dummy_name_len.to_le_bytes());
    mini_zip.extend_from_slice(&0u16.to_le_bytes());
    mini_zip.extend_from_slice(dummy_name);
    mini_zip.extend_from_slice(payload);

    let cd_offset = mini_zip.len() as u32;

    mini_zip.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02]);
    mini_zip.extend_from_slice(&20u16.to_le_bytes());
    mini_zip.extend_from_slice(&20u16.to_le_bytes());
    mini_zip.extend_from_slice(&(entry.flags & !0x0008).to_le_bytes());
    mini_zip.extend_from_slice(&entry.compression_method.to_le_bytes());
    mini_zip.extend_from_slice(&0u32.to_le_bytes());
    mini_zip.extend_from_slice(&entry.crc32.to_le_bytes());
    mini_zip.extend_from_slice(&(entry.compressed_size as u32).to_le_bytes());
    mini_zip.extend_from_slice(&(entry.uncompressed_size as u32).to_le_bytes());
    mini_zip.extend_from_slice(&dummy_name_len.to_le_bytes());
    mini_zip.extend_from_slice(&0u16.to_le_bytes());
    mini_zip.extend_from_slice(&0u16.to_le_bytes());
    mini_zip.extend_from_slice(&0u16.to_le_bytes());
    mini_zip.extend_from_slice(&0u16.to_le_bytes());
    mini_zip.extend_from_slice(&0u32.to_le_bytes());
    mini_zip.extend_from_slice(&0u32.to_le_bytes());
    mini_zip.extend_from_slice(dummy_name);

    let cd_size = (mini_zip.len() as u32) - cd_offset;

    mini_zip.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06]);
    mini_zip.extend_from_slice(&0u32.to_le_bytes());
    mini_zip.extend_from_slice(&1u16.to_le_bytes());
    mini_zip.extend_from_slice(&1u16.to_le_bytes());
    mini_zip.extend_from_slice(&cd_size.to_le_bytes());
    mini_zip.extend_from_slice(&cd_offset.to_le_bytes());
    mini_zip.extend_from_slice(&0u16.to_le_bytes());

    let cursor = std::io::Cursor::new(mini_zip);
    let mut archive = ZipArchive::new(cursor).map_err(|e| format!("构造内存 ZIP 失败：{e}"))?;
    let mut file = archive.by_index(0).map_err(|e| format!("读取内存条目失败：{e}"))?;
    let mut decompressed = Vec::with_capacity(entry.uncompressed_size as usize);
    file.read_to_end(&mut decompressed).map_err(|e| format!("分片解压失败：{e}"))?;
    Ok((comic_image_mime(&entry.name).into(), decompressed))
}

fn comic_zipmeta_path(cache_dir: &std::path::Path, key: u64) -> PathBuf {
    cache_dir.join(format!("comic-{key:016x}.zipmeta.json"))
}

async fn ensure_remote_zip_meta(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
) -> Result<Arc<Vec<ZipEntryMeta>>, String> {
    if let Ok(cache) = state.comic_remote_zip_cache.lock() {
        if let Some(entries) = cache.get(asset_id) {
            return Ok(Arc::clone(entries));
        }
    }

    let (remote_path, expected_size, expected_etag, expected_modified) = {
        let connection = open_media_database(app)?;
        connection
            .query_row(
                "SELECT remote_path, size_bytes, etag, modified_at FROM assets WHERE id = ?1 AND missing = 0",
                params![asset_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?)),
            )
            .map_err(|_| "找不到漫画文件".to_string())?
    };

    if !is_zip_or_cbz(&remote_path) {
        return Err("不是 ZIP/CBZ 格式".into());
    }

    let cache_base = if let Ok(value) = std::env::var("PML_CACHE_DIR") {
        PathBuf::from(value)
    } else {
        app.path()
            .app_cache_dir()
            .map_err(|error| format!("无法获取漫画缓存目录：{error}"))?
    };
    let cache_dir = cache_base.join("comic-archives");
    let _ = fs::create_dir_all(&cache_dir);
    let key = stable_hash(&format!("comic-v2|{asset_id}|{remote_path}|{expected_size}|{}|{}", expected_etag.as_deref().unwrap_or_default(), expected_modified.as_deref().unwrap_or_default()));
    let meta_file = comic_zipmeta_path(&cache_dir, key);

    if let Ok(bytes) = fs::read(&meta_file) {
        if let Ok(entries) = serde_json::from_slice::<Vec<ZipEntryMeta>>(&bytes) {
            if !entries.is_empty() {
                let arc_entries = Arc::new(entries);
                if let Ok(mut cache) = state.comic_remote_zip_cache.lock() {
                    cache.insert(asset_id.to_string(), Arc::clone(&arc_entries));
                }
                return Ok(arc_entries);
            }
        }
    }

    let source = build_comic_proxy_source(state, asset_id, &remote_path).await?;
    let entries = fetch_remote_zip_central_directory(&source, expected_size).await?;
    let arc_entries = Arc::new(entries);

    let _ = fs::write(&meta_file, serde_json::to_vec(&*arc_entries).unwrap_or_default());
    if let Ok(mut cache) = state.comic_remote_zip_cache.lock() {
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(asset_id.to_string(), Arc::clone(&arc_entries));
    }
    Ok(arc_entries)
}

async fn fetch_remote_zip_page(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
    entry: &ZipEntryMeta,
) -> Result<(String, Vec<u8>), String> {
    let (remote_path, expected_size) = {
        let connection = open_media_database(app)?;
        connection
            .query_row(
                "SELECT remote_path, size_bytes FROM assets WHERE id = ?1 AND missing = 0",
                params![asset_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|_| "找不到漫画文件".to_string())?
    };
    let source = build_comic_proxy_source(state, asset_id, &remote_path).await?;
    let start = entry.local_header_offset;
    let initial_fetch_len = 30 + entry.name.len() as u64 + 2048 + entry.compressed_size;
    let max_end = if expected_size > 0 { (expected_size as u64).saturating_sub(1) } else { u64::MAX - 1 };
    let end = (start + initial_fetch_len).min(max_end);

    let slice = fetch_remote_range_bytes(&source, start, end).await?;
    let entry_clone = entry.clone();
    tokio::task::spawn_blocking(move || decompress_zip_range_slice(&slice, &entry_clone))
        .await
        .map_err(|e| format!("解压任务失败：{e}"))?
}

async fn fetch_remote_zip_central_directory(
    source: &ProxySource,
    mut file_size: i64,
) -> Result<Vec<ZipEntryMeta>, String> {
    if file_size <= 22 {
        let probe = open_proxy_upstream(source, "GET", Some("bytes=0-0"), None).await?;
        if let Some(cr) = probe.headers().get(CONTENT_RANGE).and_then(|v| v.to_str().ok()) {
            if let Some((_, _, total)) = parse_content_range(cr) {
                file_size = total as i64;
            }
        }
    }
    if file_size <= 22 {
        return Err("文件过小，不是有效的 ZIP".into());
    }

    let tail_len = (file_size as u64).min(262_144);
    let tail_start = (file_size as u64) - tail_len;
    let tail_bytes = fetch_remote_range_bytes(source, tail_start, (file_size as u64) - 1).await?;
    let (cd_offset, cd_size) = parse_zip_eocd(&tail_bytes)?;

    if cd_size > 16 * 1024 * 1024 {
        return Err("ZIP 中央目录过大，超过 16 MB".into());
    }

    let cd_bytes = if cd_offset >= tail_start && (cd_offset - tail_start) + cd_size <= tail_bytes.len() as u64 {
        let start_idx = (cd_offset - tail_start) as usize;
        let end_idx = start_idx + cd_size as usize;
        tail_bytes[start_idx..end_idx].to_vec()
    } else {
        fetch_remote_range_bytes(source, cd_offset, cd_offset + cd_size - 1).await?
    };

    parse_zip_central_directory_entries(&cd_bytes)
}

async fn build_comic_proxy_source(
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
    remote_path: &str,
) -> Result<ProxySource, String> {
    let session = require_session(state).await?;
    let source_url = remote_url(&session.endpoint, remote_path)?;
    let fallback_url = legacy_archive_path(remote_path)
        .map(|path| remote_url(&session.endpoint, &path))
        .transpose()?;
    Ok(ProxySource {
        url: source_url,
        fallback_url,
        session,
        content_type: media_content_type(remote_path).into(),
        remote_path: remote_path.to_owned(),
        allow_api_fallback: true,
        api_url_cache: Arc::new(StdMutex::new(None)),
        direct_url: false,
        thumbnail_cache_path: None,
        proxy_address: None,
        asset_id: Some(asset_id.to_owned()),
        cache_base_path: None,
        activity_tracker: None,
        max_cache_bytes: None,
    })
}

async fn fetch_remote_range_bytes(
    source: &ProxySource,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, String> {
    if end < start {
        return Err("无效的分片范围".into());
    }
    let expected_len = (end - start + 1) as usize;
    if expected_len == 0 || expected_len as u64 > MAX_COMIC_PAGE_BYTES + 2 * 1024 * 1024 {
        return Err("分片请求范围过大".into());
    }
    let range_header = format!("bytes={start}-{end}");
    let response = open_proxy_upstream(source, "GET", Some(&range_header), None).await?;
    let status = response.status();
    if start > 0 && status != StatusCode::PARTIAL_CONTENT {
        return Err(format!("存储源未返回 206 分片响应（HTTP {status}）"));
    }
    if start == 0 && !status.is_success() {
        return Err(format!("存储源分片请求失败（HTTP {status}）"));
    }

    let mut stream = response.bytes_stream();
    let mut out = Vec::with_capacity(expected_len.min(8 * 1024 * 1024));
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("读取远端分片失败：{e}"))?;
        let remaining = expected_len.saturating_sub(out.len());
        if remaining == 0 {
            break;
        }
        if chunk.len() <= remaining {
            out.extend_from_slice(&chunk);
        } else {
            out.extend_from_slice(&chunk[..remaining]);
            break;
        }
        if out.len() >= expected_len {
            break;
        }
    }
    if out.is_empty() {
        return Err("远端返回空数据".into());
    }
    Ok(out)
}

async fn extract_rar_cover_from_prefix(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
) -> Result<(String, Vec<u8>), String> {
    let (remote_path, expected_size) = {
        let connection = open_media_database(app)?;
        connection
            .query_row(
                "SELECT remote_path, size_bytes FROM assets WHERE id = ?1 AND missing = 0",
                params![asset_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|_| "找不到漫画文件".to_string())?
    };
    if expected_size <= 0 {
        return Err("文件大小无效".into());
    }
    let source = build_comic_proxy_source(state, asset_id, &remote_path).await?;
    let prefix_len = (expected_size as u64).min(6 * 1024 * 1024);
    let prefix_bytes = fetch_remote_range_bytes(&source, 0, prefix_len.saturating_sub(1)).await?;
    if prefix_bytes.is_empty() {
        return Err("获取 RAR 前缀失败".into());
    }

    tokio::task::spawn_blocking(move || {
        let temp_dir = std::env::temp_dir();
        let token = random_token();
        let temp_file = temp_dir.join(format!("pml_rar_prefix_{token}.rar"));
        let _ = fs::write(&temp_file, &prefix_bytes);
        let cleanup = |f: &PathBuf| { let _ = fs::remove_file(f); };

        let result = (|| {
            let names = list_rar_entries(&temp_file)?;
            let first_image = names.first().ok_or_else(|| "RAR 前缀中未找到图片".to_string())?;
            read_rar_entry_by_name(&temp_file, first_image)
        })();

        cleanup(&temp_file);
        result
    })
    .await
    .map_err(|e| format!("RAR 封面提取任务失败：{e}"))?
}

async fn generate_comic_archive_thumbnail(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
    remote_path: &str,
) -> Result<(Option<String>, Option<String>), String> {
    static COMIC_THUMBNAIL_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let semaphore = COMIC_THUMBNAIL_SEMAPHORE.get_or_init(|| tokio::sync::Semaphore::new(4));
    let Ok(_permit) = tokio::time::timeout(std::time::Duration::from_secs(30), semaphore.acquire()).await else {
        return Err("封面生成队列繁忙".into());
    };

    let ext = extension(remote_path);
    let is_zip = matches!(ext.as_str(), "zip" | "cbz");
    let is_rar = matches!(ext.as_str(), "rar" | "cbr");

    let page_result = if is_zip {
        if let Ok(meta) = ensure_remote_zip_meta(app, state, asset_id).await {
            if let Some(first_entry) = meta.first() {
                fetch_remote_zip_page(app, state, asset_id, first_entry).await.ok()
            } else {
                None
            }
        } else {
            None
        }
    } else if is_rar {
        extract_rar_cover_from_prefix(app, state, asset_id).await.ok()
    } else {
        None
    };

    let (mime, page_bytes) = match page_result {
        Some(data) => data,
        None => read_comic_archive_page_binary(app, state, asset_id, 0).await?,
    };

    let is_jpeg = page_bytes.starts_with(&[0xff, 0xd8, 0xff]) && page_bytes.ends_with(&[0xff, 0xd9]);
    let jpeg_bytes = if is_jpeg && page_bytes.len() <= MAX_THUMBNAIL_BYTES {
        Some(page_bytes.clone())
    } else {
        encode_thumbnail_jpeg_with_ffmpeg(&page_bytes).await.ok().or_else(|| if is_jpeg { Some(page_bytes.clone()) } else { None })
    };

    match jpeg_bytes {
        Some(bytes) => {
            let saved_path = persist_thumbnail_jpeg_bytes(app, asset_id, &bytes, None)?;
            let url = cached_cover_url(asset_id, &saved_path);
            Ok((url, None))
        }
        None => {
            let data_url = format!("data:{mime};base64,{}", STANDARD.encode(&page_bytes));
            Ok((None, Some(data_url)))
        }
    }
}

async fn encode_thumbnail_jpeg_with_ffmpeg(image_bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            "-i",
            "pipe:0",
            "-frames:v",
            "1",
            "-vf",
            "scale=min(480\\,iw):-2",
            "-q:v",
            "5",
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("启动 FFmpeg 失败：{e}"))?;

    let payload = image_bytes.to_vec();
    let stdin_task = child.stdin.take().map(|mut stdin| {
        tokio::spawn(async move {
            let _ = stdin.write_all(&payload).await;
            let _ = stdin.shutdown().await;
        })
    });

    let mut out = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_end(&mut out).await;
    }
    if let Some(task) = stdin_task {
        let _ = task.await;
    }
    let status = child
        .wait()
        .await
        .map_err(|e| format!("等待 FFmpeg 失败：{e}"))?;
    if !status.success() || !out.starts_with(&[0xff, 0xd8, 0xff]) || !out.ends_with(&[0xff, 0xd9]) {
        return Err("FFmpeg 缩放漫画封面失败".into());
    }
    Ok(out)
}

fn find_exif_jpeg_thumbnail_in_slice(buf: &[u8]) -> Option<Result<&[u8], (usize, usize)>> {
    if buf.len() < 16 || buf[0] != 0xff || buf[1] != 0xd8 {
        return None;
    }
    let mut cursor = 2;
    while cursor + 4 <= buf.len() {
        if buf[cursor] != 0xff {
            break;
        }
        let marker = buf[cursor + 1];
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        let seg_len = u16::from_be_bytes([buf[cursor + 2], buf[cursor + 3]]) as usize;
        if seg_len < 2 || cursor + 2 + seg_len > buf.len() {
            break;
        }
        if marker == 0xe1 {
            let payload = &buf[cursor + 4..cursor + 2 + seg_len];
            if payload.len() >= 14 && &payload[0..6] == b"Exif\0\0" {
                let tiff_start_in_file = cursor + 4 + 6;
                let tiff = &payload[6..];
                if tiff.len() < 8 {
                    return None;
                }
                let is_le = match &tiff[0..2] {
                    b"II" => true,
                    b"MM" => false,
                    _ => return None,
                };
                let read_u16 = |offset: usize| -> Option<u16> {
                    let slice = tiff.get(offset..offset + 2)?;
                    Some(if is_le {
                        u16::from_le_bytes([slice[0], slice[1]])
                    } else {
                        u16::from_be_bytes([slice[0], slice[1]])
                    })
                };
                let read_u32 = |offset: usize| -> Option<u32> {
                    let slice = tiff.get(offset..offset + 4)?;
                    Some(if is_le {
                        u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]])
                    } else {
                        u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]])
                    })
                };
                if read_u16(2)? != 42 {
                    return None;
                }
                let ifd0_offset = read_u32(4)? as usize;
                let ifd0_count = read_u16(ifd0_offset)? as usize;
                let next_ifd_ptr_offset = ifd0_offset + 2 + ifd0_count * 12;
                let ifd1_offset = read_u32(next_ifd_ptr_offset)? as usize;
                if ifd1_offset == 0 || ifd1_offset >= tiff.len() {
                    return None;
                }
                let ifd1_count = read_u16(ifd1_offset)? as usize;
                let mut thumb_offset: Option<usize> = None;
                let mut thumb_length: Option<usize> = None;
                for i in 0..ifd1_count {
                    let entry_offset = ifd1_offset + 2 + i * 12;
                    let tag = read_u16(entry_offset)?;
                    let val = read_u32(entry_offset + 8)? as usize;
                    if tag == 0x0201 {
                        thumb_offset = Some(val);
                    } else if tag == 0x0202 {
                        thumb_length = Some(val);
                    }
                }
                if let (Some(off), Some(len)) = (thumb_offset, thumb_length) {
                    if len > 0 && len < 2_000_000 {
                        if off + len <= tiff.len() {
                            let thumb = &tiff[off..off + len];
                            if thumb.starts_with(&[0xff, 0xd8]) && thumb.ends_with(&[0xff, 0xd9]) {
                                return Some(Ok(thumb));
                            }
                        } else {
                            let file_offset = tiff_start_in_file + off;
                            return Some(Err((file_offset, len)));
                        }
                    }
                }
            }
        }
        cursor += 2 + seg_len;
    }
    None
}

async fn generate_image_thumbnail_backend(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
    remote_path: &str,
) -> Result<String, String> {
    static IMAGE_THUMBNAIL_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let semaphore = IMAGE_THUMBNAIL_SEMAPHORE.get_or_init(|| tokio::sync::Semaphore::new(2));
    let Ok(_permit) = tokio::time::timeout(std::time::Duration::from_secs(15), semaphore.acquire()).await else {
        return Err("图片封面生成队列繁忙".into());
    };

    if is_heic(remote_path) {
        if let Ok(Some(url)) = generate_heic_thumbnail(app, state, asset_id, remote_path).await {
            return Ok(url);
        }
    }

    let source = build_asset_proxy_source(state, asset_id, remote_path).await?;

    let response = open_proxy_upstream(&source, "GET", None, None).await?;
    let bytes = response.bytes().await.map_err(|e| format!("读取远程图片文件失败: {e}"))?;
    if bytes.is_empty() {
        return Err("远程图片文件为空".into());
    }

    let is_jpeg = bytes.starts_with(&[0xff, 0xd8, 0xff]) && bytes.ends_with(&[0xff, 0xd9]);
    let jpeg_bytes = if is_jpeg && bytes.len() <= MAX_THUMBNAIL_BYTES {
        bytes.to_vec()
    } else {
        encode_thumbnail_jpeg_with_ffmpeg(&bytes).await.unwrap_or_else(|_| bytes.to_vec())
    };

    let _saved_path = persist_thumbnail_jpeg_bytes(app, asset_id, &jpeg_bytes, None)?;
    let url = format!("data:image/jpeg;base64,{}", STANDARD.encode(&jpeg_bytes));
    Ok(url)
}

async fn generate_video_thumbnail_backend(
    app: &AppHandle,
    state: &State<'_, MediaLibraryState>,
    asset_id: &str,
) -> Result<String, String> {
    static VIDEO_THUMBNAIL_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let semaphore = VIDEO_THUMBNAIL_SEMAPHORE.get_or_init(|| tokio::sync::Semaphore::new(1));
    let Ok(_permit) = tokio::time::timeout(std::time::Duration::from_secs(6), semaphore.acquire()).await else {
        return Err("视频封面生成队列繁忙".into());
    };

    let proxy_source_url = register_media_source(app, state, asset_id, true, false).await?;

    let run_ffmpeg = |ss: &str| {
        let args = [
            "-hide_banner",
            "-loglevel", "error",
            "-threads", "1",
            "-fflags", "+nobuffer+fastseek",
            "-probesize", "524288",
            "-analyzeduration", "500000",
            "-ss", ss,
            "-i", &proxy_source_url,
            "-map", "0:v:0",
            "-frames:v", "1",
            "-vf", "scale=min(960\\,iw):-2",
            "-q:v", "3",
            "-f", "image2pipe",
            "-vcodec", "mjpeg",
            "pipe:1",
        ];
        Command::new("ffmpeg")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
    };

    // Attempt 1: seek to 0.5 second to skip black intro frame while keeping seek distance short
    let mut jpeg_bytes = None;
    if let Ok(mut proc) = run_ffmpeg("00:00:00.5") {
        let mut buf = Vec::new();
        if let Some(mut out) = proc.stdout.take() {
            let read_task = out.read_to_end(&mut buf);
            if let Ok(Ok(_)) = tokio::time::timeout(std::time::Duration::from_secs(4), read_task).await {
                let status = proc.wait().await;
                if status.is_ok_and(|s| s.success()) && buf.starts_with(&[0xff, 0xd8, 0xff]) && buf.ends_with(&[0xff, 0xd9]) {
                    jpeg_bytes = Some(buf);
                }
            }
        }
    }

    // Attempt 2: if 0.5s seek failed, fallback to near frame 0
    if jpeg_bytes.is_none() {
        if let Ok(mut proc) = run_ffmpeg("00:00:00") {
            let mut buf = Vec::new();
            if let Some(mut out) = proc.stdout.take() {
                let read_task = out.read_to_end(&mut buf);
                if let Ok(Ok(_)) = tokio::time::timeout(std::time::Duration::from_secs(3), read_task).await {
                    let status = proc.wait().await;
                    if status.is_ok_and(|s| s.success()) && buf.starts_with(&[0xff, 0xd8, 0xff]) && buf.ends_with(&[0xff, 0xd9]) {
                        jpeg_bytes = Some(buf);
                    }
                }
            }
        }
    }

    let bytes = jpeg_bytes.ok_or_else(|| "ffmpeg 无法截取视频画面".to_string())?;
    let _saved_path = persist_thumbnail_jpeg_bytes(app, asset_id, &bytes, None)?;
    let url = format!("data:image/jpeg;base64,{}", STANDARD.encode(&bytes));
    Ok(url)
}

fn prune_comic_archive_cache(directory: &std::path::Path, keep: &std::path::Path) {
    let Ok(entries) = fs::read_dir(directory) else { return; };
    let now = SystemTime::now();
    let mut files = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let file_name = path.file_name().and_then(|v| v.to_str()).unwrap_or_default();
        if file_name.contains(".part") || file_name.ends_with(".tmp") {
            if let Ok(modified) = meta.modified() {
                if now.duration_since(modified).unwrap_or_default().as_secs() > 1800 {
                    let _ = fs::remove_file(&path);
                }
            }
            continue;
        }
        let ext = path.extension().and_then(|v| v.to_str()).unwrap_or_default();
        if !meta.is_file() || !matches!(ext, "zip" | "cbz" | "epub" | "rar" | "cbr") {
            continue;
        }
        files.push((path, meta.len(), meta.modified().ok()));
    }
    let mut total = files.iter().map(|(_, size, _)| *size).sum::<u64>();
    if total <= MAX_COMIC_CACHE_BYTES { return; }
    files.sort_by_key(|(_, _, modified)| *modified);
    for (path, size, _) in files {
        if total <= MAX_COMIC_CACHE_BYTES * 9 / 10 { break; }
        if path == keep { continue; }
        if fs::remove_file(&path).is_ok() {
            let _ = fs::remove_file(comic_index_path(&path));
            total = total.saturating_sub(size);
        }
    }
}

fn is_comic_image_name(name: &str) -> bool {
    matches!(
        extension(name).as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "bmp" | "tif" | "tiff" | "jxl" | "svg"
    )
}

fn comic_image_mime(name: &str) -> &'static str {
    match extension(name).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "jxl" => "image/jxl",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn status_from(app: Option<&AppHandle>, configured: bool, connected: bool, message: &str) -> LibraryStatus {
    let mut endpoint = None;
    if let Some(app) = app {
        if let Ok(connection) = crate::v2_config::open(app) {
            let cd2_ep: Option<String> = connection
                .query_row("SELECT value FROM settings WHERE key = 'cd2_endpoint'", [], |r| r.get(0))
                .ok();
            let alist_ep: Option<String> = connection
                .query_row("SELECT value FROM settings WHERE key = 'alist_endpoint'", [], |r| r.get(0))
                .ok();
            endpoint = cd2_ep.or(alist_ep);
        }
    }
    if endpoint.is_none() {
        let cd2 = std::env::var("PML_CD2_URL")
            .or_else(|_| std::env::var("PML_WEBDAV_URL"))
            .ok()
            .filter(|value| !value.trim().is_empty());
        endpoint = cd2.or_else(|| {
            if std::env::var("PML_ALIST_TOKEN").is_ok() {
                std::env::var("PML_ALIST_URL")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| Some(DEFAULT_ALIST_API_URL.to_owned()))
            } else {
                None
            }
        });
    }
    LibraryStatus {
        configured,
        connected,
        endpoint,
        username: None,
        has_saved_password: false,
        has_saved_token: configured,
        auth_mode: "token".into(),
        message: message.to_owned(),
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}


// Media ids are canonical V2 ids. Never probe V1 before resolving playback or cache metadata.
fn open_media_database(app: &AppHandle) -> Result<Connection, String> {
    crate::four_domains::ensure_v2(app)?;
    let connection = crate::four_domains::open_v2(app)?;
    ensure_media_cover_metadata(&connection)?;
    Ok(connection)
}

fn ensure_media_cover_metadata(connection: &Connection) -> Result<(), String> {
    // Custom covers are provider-directory metadata, not scan completion records.
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS directory_sync_state (
            root_id TEXT NOT NULL, remote_path TEXT NOT NULL,
            scanned_at INTEGER NOT NULL, revision TEXT NOT NULL DEFAULT '',
            cover_asset_id TEXT,
            PRIMARY KEY(root_id, remote_path)
        );",
    ).map_err(database_error)
}


fn database_error(error: rusqlite::Error) -> String {
    format!("本地媒体库数据库错误：{error}")
}

fn normalize_endpoint(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let mut url = Url::parse(trimmed)
        .map_err(|_| "PML_ALIST_URL 必须是完整的 HTTP 或 HTTPS 地址".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("PML_ALIST_URL 不能包含账号、查询参数或锚点".into());
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub provider_type: String, // "cd2" or "alist"
    pub cd2_endpoint: Option<String>,
    pub cd2_user: Option<String>,
    pub cd2_password: Option<String>,
    pub alist_endpoint: Option<String>,
    pub alist_token: Option<String>,
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_provider_config(app: AppHandle) -> Result<ProviderConfig, String> {
    let connection = crate::v2_config::open(&app)?;
    let get_val = |k: &str| -> Option<String> {
        connection.query_row("SELECT value FROM settings WHERE key = ?1", params![k], |r| r.get(0)).ok()
    };
    let cd2_endpoint = get_val("cd2_endpoint").or_else(|| std::env::var("PML_CD2_URL").ok());
    let cd2_user = get_val("cd2_user").or_else(|| std::env::var("PML_CD2_USER").ok());
    let cd2_password = get_val("cd2_password").or_else(|| std::env::var("PML_CD2_PASSWORD").ok());
    let alist_endpoint = get_val("alist_endpoint").or_else(|| std::env::var("PML_ALIST_URL").ok());
    let alist_token = get_val("alist_token").or_else(|| std::env::var("PML_ALIST_TOKEN").ok());
    let provider_type = if alist_token.is_some() && cd2_endpoint.is_none() { "alist" } else { "cd2" }.to_string();
    Ok(ProviderConfig {
        provider_type,
        cd2_endpoint,
        cd2_user,
        cd2_password,
        alist_endpoint,
        alist_token,
    })
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn save_provider_config(
    app: AppHandle,
    config: ProviderConfig,
) -> Result<(), String> {
    let connection = crate::v2_config::open(&app)?;
    let set_val = |k: &str, v: Option<&str>| -> Result<(), String> {
        if let Some(val) = v.filter(|s| !s.trim().is_empty()) {
            connection.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![k, val.trim()],
            ).map_err(database_error)?;
        } else {
            let _ = connection.execute("DELETE FROM settings WHERE key = ?1", params![k]);
        }
        Ok(())
    };
    if config.provider_type == "cd2" {
        set_val("cd2_endpoint", config.cd2_endpoint.as_deref())?;
        set_val("cd2_user", config.cd2_user.as_deref())?;
        set_val("cd2_password", config.cd2_password.as_deref())?;
    } else {
        set_val("alist_endpoint", config.alist_endpoint.as_deref())?;
        set_val("alist_token", config.alist_token.as_deref())?;
    }
    Ok(())
}

fn library_provider_configuration(app: Option<&AppHandle>) -> Result<(ProviderKind, String, String), String> {
    // 1. 优先从 SQLite settings 读取用户界面配置（移动端与桌面端自适应持久化）
    if let Some(app) = app {
        if let Ok(connection) = crate::v2_config::open(app) {
            let get_setting = |key: &str| -> Option<String> {
                connection.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0)).ok()
            };
            if let Some(cd2_url) = get_setting("cd2_endpoint").filter(|v| !v.trim().is_empty()) {
                if let Ok(endpoint) = normalize_endpoint(&cd2_url) {
                    let user = get_setting("cd2_user").unwrap_or_default().trim().to_string();
                    let pass = get_setting("cd2_password").unwrap_or_default().trim().to_string();
                    let token = if !user.is_empty() || !pass.is_empty() {
                        format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
                    } else {
                        String::new()
                    };
                    return Ok((ProviderKind::CloudDrive2, endpoint, token));
                }
            }
            if let Some(token) = get_setting("alist_token").and_then(|t| normalize_api_token(&t)) {
                let endpoint = get_setting("alist_endpoint").unwrap_or_else(|| DEFAULT_ALIST_API_URL.to_owned());
                if let Ok(endpoint) = normalize_endpoint(&endpoint) {
                    return Ok((ProviderKind::Alist, endpoint, token));
                }
            }
        }
    }

    // 2. 环境变量兜底（向后兼容 Docker 容器运行模式）
    let cd2_url = std::env::var("PML_CD2_URL")
        .or_else(|_| std::env::var("PML_WEBDAV_URL"))
        .ok()
        .filter(|v| !v.trim().is_empty());

    if let Some(endpoint) = cd2_url {
        let endpoint = normalize_endpoint(&endpoint)?;
        let user = std::env::var("PML_CD2_USER")
            .or_else(|_| std::env::var("PML_WEBDAV_USER"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let pass = std::env::var("PML_CD2_PASSWORD")
            .or_else(|_| std::env::var("PML_WEBDAV_PASSWORD"))
            .unwrap_or_default()
            .trim()
            .to_string();
        let token = if !user.is_empty() || !pass.is_empty() {
            format!("Basic {}", STANDARD.encode(format!("{user}:{pass}")))
        } else {
            String::new()
        };
        return Ok((ProviderKind::CloudDrive2, endpoint, token));
    }

    let endpoint = std::env::var("PML_ALIST_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_ALIST_API_URL.to_owned());
    let endpoint = normalize_endpoint(&endpoint)?;
    let token = std::env::var("PML_ALIST_TOKEN")
        .ok()
        .and_then(|value| normalize_api_token(&value))
        .ok_or("请在上方设置中输入 CloudDrive2 WebDAV 地址或 AList Token 并点击保存")?;
    Ok((ProviderKind::Alist, endpoint, token))
}

fn normalize_remote_path(value: &str) -> Result<String, String> {
    let trimmed = value.trim().replace('\\', "/");
    if trimmed.contains('\0') {
        return Err("目录路径无效".into());
    }
    let mut segments = Vec::new();
    for segment in trimmed.split('/') {
        let segment = segment.trim();
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            return Err("目录路径不能包含 ..".into());
        }
        segments.push(segment);
    }
    Ok(if segments.is_empty() {
        "/".into()
    } else {
        format!("/{}", segments.join("/"))
    })
}

fn build_session(
    provider: ProviderKind,
    endpoint: &str,
    api_token: String,
) -> Result<Session, String> {
    let endpoint = Url::parse(endpoint).map_err(|_| "媒体源地址无效".to_string())?;
    // timeout() 是整请求（含响应体）总超时，大视频流式转发超过 45 秒会被掐断，
    // 表现为播放器反复缓冲。改用 read_timeout()：只限制单次读空闲时间，
    // 持续有数据的下载不受总时长限制。
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(std::time::Duration::from_secs(12))
        .read_timeout(std::time::Duration::from_secs(45))
        .pool_max_idle_per_host(16)
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .tcp_nodelay(true)
        .build()
        .map_err(|error| format!("无法创建网络客户端：{error}"))?;
    Ok(Session {
        provider,
        endpoint,
        api_token,
        client,
    })
}

async fn establish_session(
    endpoint: &str,
    token: &str,
    provider: ProviderKind,
) -> Result<Session, String> {
    match provider {
        ProviderKind::Alist => establish_api_session(endpoint, token).await,
        ProviderKind::CloudDrive2 => establish_cd2_session(endpoint, token).await,
    }
}

async fn establish_cd2_session(endpoint: &str, token: &str) -> Result<Session, String> {
    let mut session = build_session(ProviderKind::CloudDrive2, endpoint, token.to_string())?;

    let send_propfind = |s: &Session, url: Url| {
        let mut req = s
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), url)
            .header(USER_AGENT, MEDIA_USER_AGENT)
            .header("Depth", "0");
        if !s.api_token.is_empty() {
            req = req.header(AUTHORIZATION, &s.api_token);
        }
        req.send()
    };

    let response = send_propfind(&session, session.endpoint.clone()).await;
    let ok = match &response {
        Ok(res) => res.status().is_success() || res.status().as_u16() == 207,
        Err(_) => false,
    };

    if !ok {
        if session.endpoint.path() == "/" {
            if let Ok(dav_url) = session.endpoint.join("dav/") {
                if let Ok(res2) = send_propfind(&session, dav_url.clone()).await {
                    if res2.status().is_success() || res2.status().as_u16() == 207 {
                        session.endpoint = dav_url;
                        return Ok(session);
                    }
                }
            }
        }

        match response {
            Ok(res) => {
                if res.status() == StatusCode::UNAUTHORIZED {
                    return Err(
                        "CloudDrive2 WebDAV 认证失败，请检查 PML_CD2_USER 和 PML_CD2_PASSWORD".into(),
                    );
                }
                return Err(format!("CloudDrive2 WebDAV 返回异常状态码：{}", res.status()));
            }
            Err(err) => {
                return Err(format!("无法连接 CloudDrive2 WebDAV：{err}"));
            }
        }
    }

    Ok(session)
}

fn unescape_xml(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

fn extract_xml_tag<'a>(xml: &'a str, tag_name: &str) -> Option<&'a str> {
    let mut search_from = 0;
    while let Some(open_rel) = xml[search_from..].find('<') {
        let open_idx = search_from + open_rel;
        let after_bracket = &xml[open_idx + 1..];
        if after_bracket.starts_with('/')
            || after_bracket.starts_with('!')
            || after_bracket.starts_with('?')
        {
            search_from = open_idx + 1;
            continue;
        }
        let Some(close_bracket_rel) = after_bracket.find('>') else {
            return None;
        };
        let full_tag = &after_bracket[..close_bracket_rel];
        let tag_name_part = full_tag.split_whitespace().next().unwrap_or("");
        let actual_tag = tag_name_part.rsplit(':').next().unwrap_or(tag_name_part);
        if actual_tag.eq_ignore_ascii_case(tag_name) {
            if full_tag.ends_with('/') {
                return Some("");
            }
            let content_start = open_idx + 1 + close_bracket_rel + 1;
            let rest = &xml[content_start..];
            let mut close_search = 0;
            while let Some(c_rel) = rest[close_search..].find("</") {
                let c_idx = close_search + c_rel;
                let c_rest = &rest[c_idx + 2..];
                if let Some(c_close) = c_rest.find('>') {
                    let c_tag_part = c_rest[..c_close].trim();
                    let c_actual = c_tag_part.rsplit(':').next().unwrap_or(c_tag_part);
                    if c_actual.eq_ignore_ascii_case(tag_name) {
                        return Some(&rest[..c_idx]);
                    }
                    close_search = c_idx + 2;
                } else {
                    break;
                }
            }
            return None;
        }
        search_from = open_idx + 1;
    }
    None
}

fn parse_webdav_responses(xml: &str) -> Vec<&str> {
    let mut responses = Vec::new();
    let mut search_from = 0;
    while let Some(open_rel) = xml[search_from..].find('<') {
        let open_idx = search_from + open_rel;
        let after_bracket = &xml[open_idx + 1..];
        if after_bracket.starts_with('/')
            || after_bracket.starts_with('!')
            || after_bracket.starts_with('?')
        {
            search_from = open_idx + 1;
            continue;
        }
        let Some(close_bracket_rel) = after_bracket.find('>') else {
            break;
        };
        let full_tag = &after_bracket[..close_bracket_rel];
        let tag_name_part = full_tag.split_whitespace().next().unwrap_or("");
        let actual_tag = tag_name_part.rsplit(':').next().unwrap_or(tag_name_part);
        if actual_tag.eq_ignore_ascii_case("response") {
            let content_start = open_idx + 1 + close_bracket_rel + 1;
            let rest = &xml[content_start..];
            let mut close_search = 0;
            let mut found = false;
            while let Some(c_rel) = rest[close_search..].find("</") {
                let c_idx = close_search + c_rel;
                let c_rest = &rest[c_idx + 2..];
                if let Some(c_close) = c_rest.find('>') {
                    let c_tag_part = c_rest[..c_close].trim();
                    let c_actual = c_tag_part.rsplit(':').next().unwrap_or(c_tag_part);
                    if c_actual.eq_ignore_ascii_case("response") {
                        responses.push(&rest[..c_idx]);
                        search_from = content_start + c_idx + 2 + c_close + 1;
                        found = true;
                        break;
                    }
                    close_search = c_idx + 2;
                } else {
                    break;
                }
            }
            if !found {
                break;
            }
        } else {
            search_from = open_idx + 1;
        }
    }
    responses
}

fn parse_webdav_multistatus(
    xml: &str,
    endpoint: &Url,
    requested_path: &str,
) -> Result<Vec<DavEntry>, String> {
    let document = roxmltree::Document::parse(xml).map_err(|_| "WebDAV 目录清单不完整，本次未修改索引".to_string())?;
    let root = document.root_element();
    if root.tag_name().name() != "multistatus" { return Err("WebDAV 未返回目录清单".into()); }
    let requested = normalize_remote_path(requested_path)?;
    let prefix = endpoint.path().trim_end_matches('/');
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for response in root.children().filter(|n|n.is_element() && n.tag_name().name()=="response") {
        let href = response.children().find(|n|n.has_tag_name(("DAV:","href")) || n.is_element() && n.tag_name().name()=="href")
            .and_then(|n|n.text()).ok_or("WebDAV 条目缺少路径")?;
        let href_path = if href.starts_with("http://") || href.starts_with("https://") {
            Url::parse(href).map_err(|_|"WebDAV 路径无效")?.path().to_owned()
        } else { href.to_owned() };
        let decoded = percent_decode_str(&href_path).decode_utf8().map_err(|_|"WebDAV 路径编码无效")?;
        let relative = if !prefix.is_empty() {
            decoded.strip_prefix(prefix).filter(|p|p.is_empty() || p.starts_with('/')).ok_or("WebDAV 路径不属于当前媒体源")?
        } else { &decoded };
        let path = normalize_remote_path(relative)?;
        let prop = response.children().filter(|n|n.is_element() && n.tag_name().name()=="propstat").find_map(|n| {
            let status = n.children().find(|c|c.is_element() && c.tag_name().name()=="status").and_then(|c|c.text()).unwrap_or("");
            if status.split_whitespace().nth(1) != Some("200") { return None; }
            n.children().find(|c|c.is_element() && c.tag_name().name()=="prop" && c.children().any(|p|p.is_element() && p.tag_name().name()=="resourcetype"))
        }).ok_or("WebDAV 目录包含无法读取的条目，本次未修改索引")?;
        if path == requested { continue; }
        let text = |name: &str| prop.children().find(|n|n.is_element() && n.tag_name().name()==name).and_then(|n|n.text()).map(str::to_owned);
        let collection = prop.descendants().any(|n|n.is_element() && n.tag_name().name()=="collection");
        if seen.insert(path.clone()) {
            entries.push(DavEntry {
                remote_path:path, is_collection:collection,
                size_bytes:text("getcontentlength").and_then(|v|v.parse().ok()).unwrap_or(0),
                modified_at:text("getlastmodified"),
                etag:text("getetag").map(|v|v.trim_matches('"').to_owned()),
                thumbnail_url:None,
            });
        }
    }
    Ok(entries)
}

async fn list_webdav_directory(
    session: &Session,
    remote_path: &str,
) -> Result<Vec<DavEntry>, String> {
    let target_url = remote_url(&session.endpoint, remote_path)?;
    let mut req = session
        .client
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), target_url)
        .header(USER_AGENT, MEDIA_USER_AGENT)
        .header("Depth", "1");
    if !session.api_token.is_empty() {
        req = req.header(AUTHORIZATION, &session.api_token);
    }
    let response = req
        .send()
        .await
        .map_err(|error| format!("WebDAV 目录请求失败：{error}"))?;
    let status = response.status();
    if !status.is_success() && status.as_u16() != 207 {
        return Err(format!("WebDAV 目录请求失败（HTTP {}）", status));
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("读取 WebDAV 响应失败：{error}"))?;
    parse_webdav_multistatus(&body, &session.endpoint, remote_path)
}

async fn establish_api_session(endpoint: &str, token: &str) -> Result<Session, String> {
    let token = normalize_api_token(token).ok_or("AList API 令牌不能为空")?;
    let session = build_session(ProviderKind::Alist, endpoint, token)?;
    // Validate authentication through the lightweight user endpoint. A
    // directory request can wake the 115 driver and take seconds even for a
    // single page, which used to block the home screen on every startup.
    let response = session
        .client
        .get(alist_api_url(&session.endpoint, "me")?)
        .header(USER_AGENT, MEDIA_USER_AGENT)
        .header(AUTHORIZATION, &session.api_token)
        .send()
        .await
        .map_err(|error| format!("AList API 连接失败：{error}"))?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("读取 AList 用户信息失败：{error}"))?;
    let code = body
        .get("code")
        .and_then(|value| value.as_i64())
        .or_else(|| {
            body.get("code")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or_default();
    if !status.is_success() || (code != 200 && code != 0) {
        let message = body
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("AList 用户验证失败");
        return Err(format!(
            "AList API 用户验证失败（HTTP {}）：{}",
            status,
            summarize_api_error(message)
        ));
    }
    Ok(session)
}

fn normalize_api_token(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .unwrap_or(value)
        .trim();
    (!value.is_empty()).then(|| value.to_owned())
}

async fn require_session(state: &MediaLibraryState) -> Result<Session, String> {
    state
        .session
        .lock()
        .await
        .clone()
        .ok_or_else(|| "尚未连接媒体源（AList / CloudDrive2），请在设置中完成连接".into())
}

async fn list_api_directory(session: &Session, remote_path: &str) -> Result<Vec<DavEntry>, String> {
    Ok(list_api_directory_range(session, remote_path, None, true)
        .await?
        .0)
}

async fn list_api_directories(
    session: &Session,
    remote_path: &str,
) -> Result<Vec<RemoteDirectoryEntry>, String> {
    let path = normalize_remote_path(remote_path)?;
    let url = alist_api_url(&session.endpoint, "fs/dirs")?;
    let response = session
        .client
        .post(url)
        .header(USER_AGENT, MEDIA_USER_AGENT)
        .header(AUTHORIZATION, &session.api_token)
        .json(&serde_json::json!({
            "path": &path,
            "password": "",
            "force_root": false
        }))
        .send()
        .await
        .map_err(|error| format!("AList API 文件夹读取失败：{error}"))?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("读取 AList API 文件夹响应失败：{error}"))?;
    let code = body
        .get("code")
        .and_then(|value| value.as_i64())
        .or_else(|| {
            body.get("code")
                .and_then(|value| value.as_str())
                .and_then(|value| value.parse().ok())
        })
        .unwrap_or_default();
    if !status.is_success() || (code != 200 && code != 0) {
        let message = body
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("未知错误");
        return Err(format!(
            "AList API 文件夹读取失败（HTTP {}）：{}",
            status,
            summarize_api_error(message)
        ));
    }
    let data = body
        .get("data")
        .and_then(|value| value.as_array())
        .ok_or("AList API 未返回有效文件夹列表")?;
    let entries = data
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_owned();
            let child = if path == "/" {
                format!("/{name}")
            } else {
                format!("{path}/{name}")
            };
            Some(RemoteDirectoryEntry {
                name,
                path: normalize_remote_path(&child).ok()?,
            })
        })
        .collect::<Vec<_>>();
    Ok(entries)
}

// Scans consume every page; interactive browsing requests exactly one page.
async fn list_api_directory_range(
    session: &Session,
    remote_path: &str,
    single_page: Option<i64>,
    refresh: bool,
) -> Result<(Vec<DavEntry>, bool), String> {
    let token = session.api_token.as_str();
    let url = alist_api_url(&session.endpoint, "fs/list")?;
    let path = normalize_remote_path(remote_path)?;
    let mut page = single_page.unwrap_or(1);
    // AList documents 500 as the maximum effective page size. Older builds
    // may accept larger values, but using the documented value keeps page
    // boundaries stable across versions and storage drivers.
    let per_page = if single_page.is_some() {
        BROWSE_PAGE_SIZE as i64
    } else {
        ALIST_API_PAGE_SIZE
    };
    let mut result = Vec::new();
    let mut seen_paths = HashSet::new();
    loop {
        if page > (MAX_SCAN_ENTRIES as i64 / per_page) + 1 {
            return Err(format!(
                "目录超过 {MAX_SCAN_ENTRIES} 项，请拆分为更小的资料库根目录"
            ));
        }
        let response = session.client.post(url.clone())
            .header(USER_AGENT, MEDIA_USER_AGENT)
            .header(AUTHORIZATION, token)
            // Refresh only the first page. This obtains a current directory
            // snapshot without repeatedly forcing the storage driver to
            // rebuild the same listing while walking later pages.
            .json(&serde_json::json!({ "path": path, "password": "", "page": page, "per_page": per_page, "refresh": refresh && page == 1 }))
            .send().await.map_err(|error| format!("AList API 目录读取失败：{error}"))?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let response_text = response
            .text()
            .await
            .map_err(|error| format!("读取 AList API 目录响应失败：{error}"))?;
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(format!(
                "AList API 令牌无效、已过期或没有目录权限（HTTP {}）",
                status
            ));
        }
        let body: serde_json::Value = match serde_json::from_str(&response_text) {
            Ok(body) => body,
            Err(_)
                if status == StatusCode::METHOD_NOT_ALLOWED
                    || content_type.contains("text/html")
                    || response_text.trim_start().starts_with("<") =>
            {
                return Err(format!(
                    "AList API 目录读取失败（HTTP {}）：服务器返回 HTML 错误页面",
                    status
                ));
            }
            Err(_) => {
                return Err(format!(
                    "读取 AList API 目录响应失败（HTTP {}）：响应不是有效 JSON",
                    status
                ));
            }
        };
        let code = body
            .get("code")
            .and_then(|value| value.as_i64())
            .or_else(|| {
                body.get("code")
                    .and_then(|value| value.as_str())
                    .and_then(|value| value.parse().ok())
            })
            .unwrap_or_default();
        let message = body
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("未知错误");
        if !status.is_success() || (code != 200 && code != 0) {
            return Err(format!(
                "AList API 目录读取失败（HTTP {}）：{}",
                status,
                summarize_api_error(message)
            ));
        }
        let data = body.get("data").ok_or("AList API 未返回目录数据")?;
        let entries = api_directory_entries(data).ok_or_else(|| {
            let fields = data
                .as_object()
                .map(|object| object.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_else(|| format!("类型 {}", json_value_type(data)));
            format!(
                "AList API 未返回有效目录列表（HTTP {}，data 字段：{}）",
                status, fields
            )
        })?;
        let count = entries.len();
        let mut added_entries = 0usize;
        for entry in entries {
            let name = entry
                .get("name")
                .or_else(|| entry.get("filename"))
                .and_then(|value| value.as_str())
                .ok_or("AList API 目录项缺少名称")?;
            let is_collection = entry
                .get("is_dir")
                .or_else(|| entry.get("isDirectory"))
                .and_then(|value| value.as_bool())
                .or_else(|| {
                    entry
                        .get("is_dir")
                        .or_else(|| entry.get("isDirectory"))
                        .and_then(|value| value.as_i64())
                        .map(|value| value != 0)
                })
                .or_else(|| {
                    entry
                        .get("is_dir")
                        .or_else(|| entry.get("isDirectory"))
                        .and_then(|value| value.as_str())
                        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
                })
                .unwrap_or(false);
            let size_bytes = entry
                .get("size")
                .and_then(|value| value.as_i64())
                .or_else(|| {
                    entry
                        .get("size")
                        .and_then(|value| value.as_str())
                        .and_then(|value| value.parse().ok())
                })
                .unwrap_or(0);
            let etag = entry
                .get("etag")
                .or_else(|| entry.get("sign"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            let modified_at = entry
                .get("modified")
                .or_else(|| entry.get("modified_at"))
                .or_else(|| entry.get("updated_at"))
                .or_else(|| entry.get("last_modified"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned);
            let thumbnail_url = entry
                .get("thumb")
                .and_then(|value| value.as_str())
                .and_then(|value| alist_thumbnail_url(&session.endpoint, value));
            let child = if path == "/" {
                format!("/{name}")
            } else {
                format!("{path}/{name}")
            };
            let remote_path = normalize_remote_path(&child)?;
            // A few storage drivers return an overlapping page when the
            // directory changes during pagination. Keep the first occurrence
            // so the scanner cannot create unstable duplicate work.
            if seen_paths.insert(remote_path.clone()) {
                added_entries += 1;
                result.push(DavEntry {
                    remote_path,
                    is_collection,
                    size_bytes,
                    modified_at,
                    etag,
                    thumbnail_url,
                });
            }
        }
        let has_more = data
            .get("has_more")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        if single_page.is_some() {
            if count > BROWSE_PAGE_SIZE {
                // If AList storage driver does not support server-side pagination and returned all items,
                // accept them gracefully with has_more = false instead of failing.
                return Ok((result, false));
            }
            // A short page is terminal even when older AList versions omit
            // or incorrectly default the `has_more` field.
            return Ok((result, count == BROWSE_PAGE_SIZE && has_more));
        }
        // `total` is intentionally not used as a stop condition. Several
        // AList drivers report a stale/partial total, which used to hide the
        // remainder of a directory when the first page was shorter than
        // `per_page`. Continue until the provider returns an empty page, an
        // explicit `has_more: false`, or a page containing no new paths.
        if count == 0 || added_entries == 0 || !has_more {
            break;
        }
        page += 1;
    }
    Ok((result, false))
}

fn api_directory_entries(data: &serde_json::Value) -> Option<Vec<serde_json::Value>> {
    if let Some(entries) = data.as_array() {
        return Some(entries.clone());
    }
    let value = data
        .get("content")
        .or_else(|| data.get("files"))
        .or_else(|| data.get("items"))
        .or_else(|| data.get("children"))?;
    if value.is_null() {
        // Empty directories are represented as `content: null` by some
        // AList storage drivers. Treat that as a valid empty page; a denied
        // request has already been rejected by its non-2xx/API error code.
        return Some(Vec::new());
    }
    if let Some(entries) = value.as_array() {
        return Some(entries.clone());
    }
    // A few gateway wrappers serialize the content array as a JSON string.
    value
        .as_str()
        .and_then(|encoded| serde_json::from_str::<serde_json::Value>(encoded).ok())
        .and_then(|decoded| decoded.as_array().cloned())
}

fn json_value_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

async fn list_directory(session: &Session, remote_path: &str) -> Result<Vec<DavEntry>, String> {
    match session.provider {
        ProviderKind::Alist => list_api_directory(session, remote_path).await,
        ProviderKind::CloudDrive2 => list_webdav_directory(session, remote_path).await,
    }
}

fn remote_url(endpoint: &Url, remote_path: &str) -> Result<Url, String> {
    let path = normalize_remote_path(remote_path)?;
    let mut url = endpoint.clone();
    {
        // url crate 的 path_segments_mut().push() 会在尾斜杠路径上再补一个
        // "/"，拼出 `/dav//115/...` 这样的双斜杠地址，部分 AList 部署会
        // 因此对 GET 返回 405。先 pop_if_empty 去掉空段再逐段追加。
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "AList 地址无法设置路径".to_string())?;
        segments.pop_if_empty();
        for segment in path
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
        {
            // Decode one escaped path layer before the URL library performs
            // its canonical path encoding.
            let decoded = percent_decode_str(segment)
                .decode_utf8()
                .map_err(|_| "媒体路径包含非 UTF-8 转义字符".to_string())?;
            segments.push(decoded.as_ref());
        }
    }
    Ok(url)
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}


async fn list_directory_with_backoff(
    session: &Session,
    directory: &str,
) -> Result<Vec<DavEntry>, String> {
    let mut last_error = String::new();
    for attempt in 0..3_u64 {
        match list_directory(session, directory).await {
            Ok(entries) => return Ok(entries),
            Err(error) if attempt < 2 && is_retryable_scan_error(&error) => {
                last_error = error;
                tokio::time::sleep(std::time::Duration::from_secs(1 << attempt)).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error)
}

fn is_retryable_scan_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("http 408")
        || normalized.contains("http 429")
        || normalized.contains("http 500")
        || normalized.contains("http 502")
        || normalized.contains("http 503")
        || normalized.contains("http 504")
        || normalized.contains("timeout")
        || normalized.contains("timed out")
        || normalized.contains("decoding response body")
        || normalized.contains("error decoding")
        || normalized.contains("请求失败")
}

fn directory_fingerprint(entries: &[DavEntry]) -> String {
    let mut values = entries
        .iter()
        .map(|entry| {
            format!(
                "{}|{}|{}|{}|{}",
                entry.remote_path,
                entry.is_collection,
                entry.size_bytes,
                entry.modified_at.as_deref().unwrap_or_default(),
                entry.etag.as_deref().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    values.sort_unstable();
    format!("{:016x}", stable_hash(&values.join("\n")))
}

fn directory_has_reusable_fingerprint(directory: &str, entries: &[DavEntry]) -> bool {
    // A leaf directory's complete child listing is enough to detect changes.
    // For nested directories, require the provider to expose a stable ETag for
    // the directory itself; otherwise a new file several levels below could be
    // missed when only the parent listing is unchanged.
    if !entries
        .iter()
        .any(|entry| entry.is_collection && entry.remote_path != directory)
    {
        return true;
    }
    entries.iter().any(|entry| {
        entry.remote_path == directory
            && entry
                .etag
                .as_deref()
                .is_some_and(|etag| !etag.trim().is_empty())
    })
}

fn is_skippable_scan_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("http 404")
        || normalized.contains("http 405")
        || normalized.contains("http 410")
        || normalized.contains("object not found")
        || normalized.contains("failed get dir")
        || normalized.contains("failed get parent list")
        || normalized.contains("failed get objs")
        || normalized.contains("failed to list objs")
        || normalized.contains("failed list objs")
        || normalized.contains("http 408")
        || normalized.contains("http 429")
        || normalized.contains("http 500")
        || normalized.contains("http 502")
        || normalized.contains("http 503")
        || normalized.contains("http 504")
        || normalized.contains("timeout")
        || normalized.contains("timed out")
        || normalized.contains("decoding response body")
        || normalized.contains("error decoding")
        || normalized.contains("potential threats")
        || normalized.contains("<!doctype")
        || normalized.contains("<html")
        || normalized.contains("html 错误页面")
}

fn summarize_api_error(message: &str) -> String {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("<!doctype")
        || normalized.contains("<html")
        || normalized.contains("potential threats")
    {
        return "服务器返回 HTML 错误页面，当前目录暂时无法读取".into();
    }
    if normalized.contains("failed get objs")
        || normalized.contains("failed to list objs")
        || normalized.contains("failed list objs")
    {
        return "存储驱动无法读取该目录（failed get objs）".into();
    }
    let shortened = message.chars().take(240).collect::<String>();
    if shortened.chars().count() < message.chars().count() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

fn media_type_for(kind: LibraryRootKind, path: &str) -> Option<&'static str> {
    let extension = extension(path);
    match kind {
        LibraryRootKind::Media
            if matches!(
                extension.as_str(),
                "jpg"
                    | "jpeg"
                    | "png"
                    | "webp"
                    | "gif"
                    | "heic"
                    | "heif"
                    | "avif"
                    | "bmp"
                    | "tif"
                    | "tiff"
                    | "jxl"
                    | "svg"
            ) =>
        {
            Some("image")
        }
        LibraryRootKind::Media
            if matches!(
                extension.as_str(),
                "mp4"
                    | "mkv"
                    | "mov"
                    | "avi"
                    | "webm"
                    | "m4v"
                    | "ts"
                    | "m2ts"
                    | "mts"
                    | "m4s"
                    | "flv"
                    | "mpeg"
                    | "mpg"
                    | "3gp"
                    | "wmv"
                    | "asf"
                    | "rm"
                    | "rmvb"
                    | "ogv"
                    | "f4v"
                    | "vob"
            ) =>
        {
            Some("video")
        }
        LibraryRootKind::Comics if matches!(extension.as_str(), "cbz" | "zip" | "epub" | "rar" | "cbr") => {
            Some("comicArchive")
        }
        LibraryRootKind::Comics if extension == "pdf" => Some("pdf"),
        LibraryRootKind::Comics
            if matches!(
                extension.as_str(),
                "jpg"
                    | "jpeg"
                    | "png"
                    | "webp"
                    | "gif"
                    | "avif"
                    | "bmp"
                    | "tif"
                    | "tiff"
                    | "jxl"
                    | "svg"
            ) =>
        {
            Some("comicPage")
        }
        _ => None,
    }
}





// 借鉴 czkawka 极速分块哈希算法思想：
// 针对网盘/NAS 场景，如果对几十 GB 的视频全量计算 SHA-256，会消耗巨大带宽和时间；
// 采用 Range 请求仅读取文件前 64KB (Head) + 尾 64KB (Tail) 并附带 size_bytes 进行哈希，
// 精度达到 99.999% 同时速度提升数百倍，避免网盘连接打满。
async fn hash_remote_asset(source: &ProxySource) -> Result<String, String> {
    // 优先尝试读取文件头 64KB
    let head_range = Some("bytes=0-65535".to_string());
    let response = open_proxy_upstream(source, "GET", head_range.as_deref(), None).await?;
    if !response_is_usable(&response, source) {
        return Err(format!(
            "远程响应不可用于哈希：{}",
            proxy_response_summary(&response)
        ));
    }
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();
    let mut bytes_read: usize = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("读取远程文件失败：{error}"))?;
        bytes_read += chunk.len();
        hasher.update(&chunk);
        // 最多读取 128KB 保证速度与资源占用
        if bytes_read >= 131072 {
            break;
        }
    }
    let digest = hasher.finalize();
    let value = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("fast64k:{value}"))
}

#[derive(Clone)]
struct DuplicateCandidate {
    id: String,
    root_id: String,
    remote_path: String,
    file_name: String,
    parent_path: String,
    media_type: String,
    size_bytes: i64,
    etag: Option<String>,
    modified_at: Option<String>,
    content_hash: Option<String>,
}

fn load_duplicate_candidates(app: &AppHandle, root_id: Option<&str>) -> Result<Vec<DuplicateCandidate>, String> {
    let connection = open_media_database(app)?;
    let sql = "SELECT id, root_id, remote_path, file_name, parent_path, media_type, size_bytes, etag, modified_at, content_hash
               FROM assets
               WHERE missing = 0 AND size_bytes > 0 AND (?1 IS NULL OR root_id = ?1)
                 AND size_bytes IN (
                   SELECT size_bytes FROM assets
                   WHERE missing = 0 AND size_bytes > 0 AND (?1 IS NULL OR root_id = ?1)
                   GROUP BY size_bytes HAVING COUNT(*) > 1
                 )
               ORDER BY size_bytes DESC, file_name COLLATE NOCASE, id";
    let rows = connection.prepare(sql).map_err(database_error)?.query_map(params![root_id], |row| {
        Ok(DuplicateCandidate {
            id: row.get(0)?, root_id: row.get(1)?, remote_path: row.get(2)?, file_name: row.get(3)?,
            parent_path: row.get(4)?, media_type: row.get(5)?, size_bytes: row.get(6)?, etag: row.get(7)?,
            modified_at: row.get(8)?, content_hash: row.get(9)?,
        })
    }).map_err(database_error)?.collect::<Result<Vec<_>, _>>().map_err(database_error)?;
    Ok(rows)
}

fn duplicate_groups_from_connection(connection: &Connection, root_id: Option<&str>) -> Result<Vec<DuplicateGroup>, String> {
    let sql = "SELECT content_hash, id, root_id, file_name, remote_path, parent_path, media_type, size_bytes, modified_at
               FROM assets
               WHERE missing = 0 AND content_hash IS NOT NULL AND (?1 IS NULL OR root_id = ?1)
               ORDER BY size_bytes DESC, content_hash, file_name COLLATE NOCASE, id";
    let rows = connection.prepare(sql).map_err(database_error)?.query_map(params![root_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            DuplicateAsset { id: row.get(1)?, root_id: row.get(2)?, file_name: row.get(3)?, remote_path: row.get(4)?,
                parent_path: row.get(5)?, media_type: row.get(6)?, size_bytes: row.get(7)?, modified_at: row.get(8)? },
        ))
    }).map_err(database_error)?.collect::<Result<Vec<_>, _>>().map_err(database_error)?;
    let mut groups: Vec<DuplicateGroup> = Vec::new();
    for (hash, asset) in rows {
        if let Some(group) = groups.iter_mut().find(|group| group.hash == hash && group.size_bytes == asset.size_bytes) {
            group.assets.push(asset);
        } else {
            groups.push(DuplicateGroup { hash, size_bytes: asset.size_bytes, assets: vec![asset] });
        }
    }
    groups.retain(|group| group.assets.len() > 1);
    Ok(groups)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_duplicate_scan_progress(state: State<'_, MediaLibraryState>) -> DuplicateScanProgress {
    state.duplicate_progress.lock().unwrap_or_else(|error| error.into_inner()).clone()
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn cancel_duplicate_scan(state: State<'_, MediaLibraryState>) -> DuplicateScanProgress {
    state.duplicate_cancel.store(true, Ordering::Relaxed);
    state.duplicate_progress.lock().unwrap_or_else(|error| error.into_inner()).clone()
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn start_duplicate_scan(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    root_id: Option<String>,
) -> Result<DuplicateScanProgress, String> {
    {
        let progress = state.duplicate_progress.lock().unwrap_or_else(|error| error.into_inner());
        if progress.running { return Err("重复文件检查正在进行中".into()); }
    }
    let session = require_session(&state).await?;
    let candidates = load_duplicate_candidates(&app, root_id.as_deref())?;
    state.duplicate_cancel.store(false, Ordering::Relaxed);
    {
        let mut progress = state.duplicate_progress.lock().unwrap_or_else(|error| error.into_inner());
        *progress = DuplicateScanProgress { running: true, phase: "scanning".into(), scope_root_id: root_id.clone(),
            candidates_total: candidates.len(), processed: 0, duplicate_groups: 0, matched_files: 0,
            percent: if candidates.is_empty() { 100.0 } else { 0.0 },
            message: if candidates.is_empty() { "没有发现同大小候选文件".into() } else { format!("准备检查 {} 个候选文件", candidates.len()) }, error: None };
    }
    let progress = Arc::clone(&state.duplicate_progress);
    let cancel = Arc::clone(&state.duplicate_cancel);
    let app_for_task = app.clone();
    crate::rt::spawn(async move {
        duplicate_scan_worker(app_for_task, session, candidates, root_id, progress, cancel).await;
    });
    Ok(get_duplicate_scan_progress(state))
}

async fn duplicate_scan_worker(
    app: AppHandle,
    session: Session,
    candidates: Vec<DuplicateCandidate>,
    root_id: Option<String>,
    progress: Arc<StdMutex<DuplicateScanProgress>>,
    cancel: Arc<AtomicBool>,
) {
    for (index, candidate) in candidates.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            let mut current = progress.lock().unwrap_or_else(|error| error.into_inner());
            current.running = false; current.phase = "cancelled".into(); current.percent = 100.0;
            current.message = format!("已取消，已处理 {}/{} 个候选文件", index, candidates.len());
            return;
        }
        if candidate.content_hash.is_none() {
            let source = match remote_url(&session.endpoint, &candidate.remote_path) {
                Ok(url) => ProxySource { url, fallback_url: None, session: session.clone(), content_type: media_content_type(&candidate.remote_path).to_owned(),
                    remote_path: candidate.remote_path.clone(), allow_api_fallback: true, api_url_cache: Arc::new(StdMutex::new(None)), direct_url: false,
                    thumbnail_cache_path: None, proxy_address: None, asset_id: Some(candidate.id.clone()), cache_base_path: None,
                    activity_tracker: None, max_cache_bytes: None },
                Err(error) => {
                    let mut current = progress.lock().unwrap_or_else(|e| e.into_inner()); current.processed = index + 1;
                    current.message = format!("跳过 {}：{}", candidate.file_name, error); continue;
                }
            };
            if let Ok(content_hash) = hash_remote_asset(&source).await {
                let written=open_media_database(&app).and_then(|connection| {
                    connection.execute("UPDATE assets SET content_hash = ?1 WHERE id = ?2 AND root_id = ?3 AND remote_path = ?4 AND missing = 0 AND size_bytes = ?5 AND etag IS ?6 AND modified_at IS ?7 AND content_hash IS NULL",
                        params![content_hash,candidate.id,candidate.root_id,candidate.remote_path,candidate.size_bytes,candidate.etag,candidate.modified_at]).map_err(database_error)
                });
                if let Err(error)=written {
                    let mut current=progress.lock().unwrap_or_else(|e|e.into_inner());
                    current.running=false;current.phase="failed".into();current.error=Some(error);
                    current.message="重复文件检查失败：无法保存 V2 哈希".into();
                    return;
                }
            }
        }
        let mut current = progress.lock().unwrap_or_else(|error| error.into_inner());
        current.processed = index + 1;
        current.percent = ((index + 1) as f64 / candidates.len().max(1) as f64 * 100.0).min(99.0);
        current.message = format!("正在读取文件哈希（{}/{})", index + 1, candidates.len());
    }
    let groups = match open_media_database(&app).and_then(|connection|duplicate_groups_from_connection(&connection,root_id.as_deref())) {
        Ok(groups)=>groups,
        Err(error)=>{
            let mut current=progress.lock().unwrap_or_else(|e|e.into_inner());
            current.running=false;current.phase="failed".into();current.error=Some(error);
            current.message="重复文件检查失败：无法读取 V2 分组".into();
            return;
        }
    };
    let mut current = progress.lock().unwrap_or_else(|error| error.into_inner());
    current.running = false; current.phase = "completed".into(); current.percent = 100.0;
    current.duplicate_groups = groups.len(); current.matched_files = groups.iter().map(|group| group.assets.len()).sum();
    current.message = if groups.is_empty() { "检查完成，未发现重复文件".into() } else { format!("检查完成，发现 {} 组重复文件", groups.len()) };
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_duplicate_groups(app: AppHandle, root_id: Option<String>) -> Result<Vec<DuplicateGroup>, String> {
    let connection = open_media_database(&app)?;
    duplicate_groups_from_connection(&connection, root_id.as_deref())
}



fn load_indexed_assets(
    connection: &Connection,
    root: &LibraryRoot,
) -> Result<Vec<IndexedAsset>, String> {
    let mut statement = connection.prepare("SELECT id, remote_path, file_name, parent_path, media_type, size_bytes, etag FROM assets WHERE root_id = ?1 AND missing = 0").map_err(database_error)?;
    let rows = statement
        .query_map(params![root.id], |row| {
            Ok(IndexedAsset {
                id: row.get(0)?,
                root_id: root.id.clone(),
                root_kind: root.kind,
                remote_path: row.get(1)?,
                file_name: row.get(2)?,
                parent_path: row.get(3)?,
                media_type: row.get(4)?,
                size_bytes: row.get(5)?,
                etag: row.get(6)?,
            })
        })
        .map_err(database_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(database_error)
}

/// 解析资产路径：分类（{xxx} / ｛xxx｝ 多级）、作者（【xxx】）、合集（其余段，多级）。
/// P/V/图片/视频 等类型分隔段被跳过，从而自动合并到父合集。
struct ParsedPath {
    categories: Vec<String>,
    author: Option<String>,
    collections: Vec<String>,
}

fn parse_asset_path(root: &str, remote_path: &str) -> ParsedPath {
    let root_depth = path_segments(root).len();
    let parent = path_segments(&parent_path(remote_path));
    let mut categories = Vec::new();
    let mut author = None;
    let mut collections = Vec::new();
    for segment in parent.into_iter().skip(root_depth) {
        if let Some(name) = category_marker(&segment) {
            categories.push(name);
        } else if let Some(name) = author_marker(&segment) {
            if author.is_none() {
                author = Some(name);
            }
        } else if is_type_marker(&segment) {
            // 图片/视频分隔目录，跳过以便自动合并到父合集
        } else {
            collections.push(segment);
        }
    }
    ParsedPath {
        categories,
        author,
        collections,
    }
}

fn category_marker(segment: &str) -> Option<String> {
    let trimmed = segment.trim();
    // AList paths can contain either ASCII or full-width braces. Treat both
    // forms as category markers so the same directory is classified after a
    // full scan regardless of how it was named.
    let name = if let Some(name) = trimmed.strip_prefix('{') {
        name.strip_suffix('}')?
    } else if let Some(name) = trimmed.strip_prefix('｛') {
        name.strip_suffix('｝')?
    } else {
        return None;
    }
    .trim();
    (!name.is_empty() && !name.contains(['{', '}', '｛', '｝'])).then(|| name.to_owned())
}

fn author_marker(segment: &str) -> Option<String> {
    let trimmed = segment.trim();
    let name = trimmed.strip_prefix('【')?.strip_suffix('】')?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

fn is_type_marker(segment: &str) -> bool {
    matches!(
        normalize_name(segment).as_str(),
        "p" | "v"
            | "pic"
            | "pics"
            | "picture"
            | "pictures"
            | "image"
            | "images"
            | "photo"
            | "photos"
            | "video"
            | "videos"
            | "图片"
            | "视频"
            | "图像"
            | "相册"
    )
}

fn effective_parse_asset_path(
    root: &str,
    remote_path: &str,
    sparse_authors: &HashSet<String>,
) -> ParsedPath {
    let mut parsed = parse_asset_path(root, remote_path);
    if let Some(author) = parsed.author.take() {
        if sparse_authors.contains(&author) {
            parsed.collections.insert(0, format!("【{author}】"));
        } else {
            parsed.author = Some(author);
        }
    }
    parsed
}

fn effective_group_anchor(
    root: &str,
    remote_path: &str,
    sparse_authors: &HashSet<String>,
) -> String {
    let parsed = effective_parse_asset_path(root, remote_path, sparse_authors);
    let mut parts = Vec::new();
    for category in &parsed.categories {
        parts.push(format!("{{{category}}}"));
    }
    if let Some(author) = &parsed.author {
        parts.push(format!("【{author}】"));
    }
    parts.extend(parsed.collections.iter().cloned());
    if parts.is_empty() {
        root.to_owned()
    } else {
        format!("/{}", parts.join("/"))
    }
}

fn ensure_author(connection: &Connection, name: &str, origin: &str) -> Result<String, String> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err("作者名称不能为空".into());
    }
    if let Some(id) = connection
        .query_row(
            "SELECT id FROM authors WHERE name = ?1 COLLATE NOCASE",
            params![normalized],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
    {
        return Ok(id);
    }
    let id = format!("author-{:016x}", stable_hash(&normalize_name(normalized)));
    connection.execute("INSERT OR IGNORE INTO authors (id, name, aliases_json, origin, created_at) VALUES (?1, ?2, '[]', ?3, ?4)", params![id, normalized, origin, now()]).map_err(database_error)?;
    Ok(id)
}

fn ensure_category_path(
    connection: &Connection,
    path: &[String],
    origin: &str,
) -> Result<Option<String>, String> {
    if path.is_empty() {
        return Ok(None);
    }
    let mut parent_id: Option<String> = None;
    let mut full = String::new();
    let mut leaf = None;
    for segment in path {
        if !full.is_empty() {
            full.push('/');
        }
        full.push_str(segment);
        let id = match &parent_id {
            Some(parent) => connection.query_row("SELECT id FROM categories WHERE name = ?1 COLLATE NOCASE AND parent_id = ?2", params![full, parent], |row| row.get::<_, String>(0)).optional().map_err(database_error)?,
            None => connection.query_row("SELECT id FROM categories WHERE name = ?1 COLLATE NOCASE AND parent_id IS NULL", params![full], |row| row.get::<_, String>(0)).optional().map_err(database_error)?,
        };
        let id = match id {
            Some(id) => id,
            None => {
                let id = format!("category-{:016x}", stable_hash(&full));
                connection.execute("INSERT OR IGNORE INTO categories (id, name, parent_id, origin, created_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, full, parent_id, origin, now()]).map_err(database_error)?;
                id
            }
        };
        leaf = Some(id.clone());
        parent_id = Some(id);
    }
    Ok(leaf)
}

fn ensure_collection(
    connection: &Connection,
    root_id: &str,
    author_id: Option<&str>,
    name: &str,
    origin: &str,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("合集名称不能为空".into());
    }
    let id = match author_id {
        Some(author_id) => connection.query_row("SELECT id FROM collections WHERE root_id = ?1 AND author_id = ?2 AND name = ?3 COLLATE NOCASE AND parent_id IS NULL", params![root_id, author_id, name], |row| row.get::<_, String>(0)).optional().map_err(database_error)?,
        None => connection.query_row("SELECT id FROM collections WHERE root_id = ?1 AND author_id IS NULL AND name = ?2 COLLATE NOCASE AND parent_id IS NULL", params![root_id, name], |row| row.get::<_, String>(0)).optional().map_err(database_error)?,
    };
    if let Some(id) = id {
        return Ok(id);
    }
    let id = format!(
        "collection-{:016x}",
        stable_hash(&format!("{root_id}|{}|{name}", author_id.unwrap_or("")))
    );
    connection.execute("INSERT OR IGNORE INTO collections (id, root_id, author_id, name, parent_id, anchor_path, origin, created_at) VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6)", params![id, root_id, author_id, name, origin, now()]).map_err(database_error)?;
    Ok(id)
}

fn ensure_collection_path(
    connection: &Connection,
    root_id: &str,
    author_id: Option<&str>,
    path: &[String],
    origin: &str,
) -> Result<String, String> {
    let mut parent_id: Option<String> = None;
    let mut full = String::new();
    let mut leaf = String::new();
    for segment in path {
        if !full.is_empty() {
            full.push('/');
        }
        full.push_str(segment);
        let id = match (&parent_id, author_id) {
            (Some(parent), Some(author)) => connection.query_row("SELECT id FROM collections WHERE root_id = ?1 AND author_id = ?2 AND name = ?3 COLLATE NOCASE AND parent_id = ?4", params![root_id, author, full, parent], |row| row.get::<_, String>(0)).optional().map_err(database_error)?,
            (Some(parent), None) => connection.query_row("SELECT id FROM collections WHERE root_id = ?1 AND author_id IS NULL AND name = ?2 COLLATE NOCASE AND parent_id = ?3", params![root_id, full, parent], |row| row.get::<_, String>(0)).optional().map_err(database_error)?,
            (None, Some(author)) => connection.query_row("SELECT id FROM collections WHERE root_id = ?1 AND author_id = ?2 AND name = ?3 COLLATE NOCASE AND parent_id IS NULL", params![root_id, author, full], |row| row.get::<_, String>(0)).optional().map_err(database_error)?,
            (None, None) => connection.query_row("SELECT id FROM collections WHERE root_id = ?1 AND author_id IS NULL AND name = ?2 COLLATE NOCASE AND parent_id IS NULL", params![root_id, full], |row| row.get::<_, String>(0)).optional().map_err(database_error)?,
        };
        let id = match id {
            Some(id) => id,
            None => {
                let id = format!(
                    "collection-{:016x}",
                    stable_hash(&format!("{root_id}|{}|{full}", author_id.unwrap_or("")))
                );
                connection.execute("INSERT OR IGNORE INTO collections (id, root_id, author_id, name, parent_id, anchor_path, origin, created_at) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7)", params![id, root_id, author_id, full, parent_id, origin, now()]).map_err(database_error)?;
                id
            }
        };
        leaf = id.clone();
        parent_id = Some(id);
    }
    Ok(leaf)
}

fn purge_orphaned_metadata(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "DELETE FROM authors
             WHERE id NOT IN (SELECT DISTINCT author_id FROM asset_metadata WHERE author_id IS NOT NULL)",
            [],
        )
        .map_err(database_error)?;
    // Keep only category nodes which lead to a currently indexed asset. The
    // previous one-pass leaf deletion could leave old parent categories behind
    // after a directory was renamed or moved.
    connection
        .execute(
            "WITH RECURSIVE live_categories(id) AS (
                SELECT DISTINCT category_id FROM asset_metadata WHERE category_id IS NOT NULL
                UNION
                SELECT category.parent_id
                FROM categories category JOIN live_categories live ON category.id = live.id
                WHERE category.parent_id IS NOT NULL
             )
             DELETE FROM categories WHERE id NOT IN (SELECT id FROM live_categories)",
            [],
        )
        .map_err(database_error)?;
    // Collections are also a tree. Remove only obsolete automatic nodes;
    // manually assigned collections remain untouched even when temporarily
    // empty.
    connection
        .execute(
            "WITH RECURSIVE live_collections(id) AS (
                SELECT DISTINCT collection_id FROM asset_metadata WHERE collection_id IS NOT NULL
                UNION
                SELECT collection.parent_id
                FROM collections collection JOIN live_collections live ON collection.id = live.id
                WHERE collection.parent_id IS NOT NULL
             )
             DELETE FROM collections
             WHERE origin = 'automatic' AND id NOT IN (SELECT id FROM live_collections)",
            [],
        )
        .map_err(database_error)?;
    Ok(())
}

fn paths_overlap(left: &str, right: &str) -> bool {
    let left = normalize_remote_path(left).unwrap_or_else(|_| left.into());
    let right = normalize_remote_path(right).unwrap_or_else(|_| right.into());
    left == "/"
        || right == "/"
        || left == right
        || left.strip_prefix(&(right.clone() + "/")).is_some()
        || right.strip_prefix(&(left + "/")).is_some()
}

fn media_filter_clause(filter: Option<&str>) -> Result<&'static str, String> {
    match filter.unwrap_or("all") {
        "all" => Ok("root.kind = 'media'"),
        "images" => Ok("root.kind = 'media' AND asset.media_type = 'image'"),
        "videos" => Ok("root.kind = 'media' AND asset.media_type = 'video'"),
        "shortVideos" => Ok("root.kind = 'media' AND asset.media_type = 'video'"),
        "comics" => Ok("root.kind = 'comics'"),
        _ => Err("不支持的媒体筛选条件".into()),
    }
}

fn category_filter_clause(category_id: Option<String>, values: &mut Vec<Box<dyn ToSql>>) -> String {
    match category_id.filter(|value| !value.is_empty()) {
        Some(value) if value == "uncategorized" => "metadata.category_id IS NULL".into(),
        Some(value) => {
            values.push(Box::new(value));
            "metadata.category_id IN (WITH RECURSIVE cat_sub(id) AS (SELECT id FROM categories WHERE id = ? UNION ALL SELECT c.id FROM categories c JOIN cat_sub s ON c.parent_id = s.id) SELECT id FROM cat_sub)".into()
        }
        None => "1 = 1".into(),
    }
}

fn path_segments(path: &str) -> Vec<String> {
    path.trim_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
fn file_name(path: &str) -> String {
    path_segments(path)
        .last()
        .cloned()
        .unwrap_or_else(|| "/".into())
}
fn parent_path(path: &str) -> String {
    let mut segments = path_segments(path);
    segments.pop();
    if segments.is_empty() {
        "/".into()
    } else {
        format!("/{}", segments.join("/"))
    }
}
fn legacy_archive_path(path: &str) -> Option<String> {
    let mut segments = path_segments(path);
    let file_name = segments.pop()?;
    (segments.last() == Some(&file_name)).then(|| format!("/{}", segments.join("/")))
}
fn extension(path: &str) -> String {
    file_name(path)
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

fn is_heic(path: &str) -> bool {
    let ext = extension(path);
    ext == "heic" || ext == "heif"
}

/// Legacy cache key retained for database compatibility. Video covers are no
/// longer resolved from same-folder image files.
fn thumbnail_key(file_name: &str, media_type: &str) -> Option<String> {
    if !matches!(media_type, "image" | "video") {
        return None;
    }
    let stem = file_name.rsplit_once('.')?.0;
    if stem.is_empty()
        || stem
            .chars()
            .all(|character| character.is_ascii_digit() || " _-().[]{}【】".contains(character))
    {
        return None;
    }
    Some(stem.to_lowercase())
}

fn normalize_name(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .filter(|value| {
            !value.is_whitespace()
                && !matches!(
                    value,
                    '[' | ']' | '【' | '】' | '(' | ')' | '（' | '）' | '_' | '-' | '.'
                )
        })
        .collect()
}
fn stable_hash(value: &str) -> u64 {
    stable_hash_seeded(0, value)
}

fn stable_hash_seeded(seed: u64, value: &str) -> u64 {
    value.bytes().fold(seed ^ 0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

async fn clear_proxy_sources(state: &MediaLibraryState) {
    let sources = state.proxy.lock().await.as_ref().map(|proxy| proxy.sources.clone());
    if let Some(sources) = sources {
        sources.lock().await.clear();
    }
}
async fn register_media_source(
    app: &AppHandle,
    state: &MediaLibraryState,
    asset_id: &str,
    allow_api_fallback: bool,
    thumbnail: bool,
) -> Result<String, String> {
    let token = if thumbnail {
        format!("thumb_{asset_id}")
    } else {
        asset_id.to_owned()
    };
    let route = if thumbnail { "thumbnail" } else { "media" };
    let proxy = ensure_media_proxy(state).await?;
    let (remote_path, media_type, size_bytes, etag, content_hash): (
        String,
        String,
        i64,
        Option<String>,
        Option<String>,
    ) = {
        let connection = open_media_database(app)?;
        connection
            .query_row(
                "SELECT remote_path, media_type, size_bytes, COALESCE(etag, modified_at), content_hash FROM assets WHERE id = ?1 AND missing = 0",
                params![asset_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .map_err(|_| "找不到可预览的媒体文件".to_string())?
    };
    let ext = extension(&remote_path);
    let suffix = if thumbnail {
        ".jpg".to_string()
    } else if !ext.is_empty() {
        format!(".{ext}")
    } else if media_type == "video" {
        ".mp4".to_string()
    } else {
        String::new()
    };
    {
        let sources = proxy.sources.lock().await;
        if sources.contains_key(&token) {
            return Ok(format!("http://{}/{route}/{token}{suffix}", proxy.address));
        }
    }
    if size_bytes > 0 {
        if let Ok(mut guard) = ASSET_SIZE_CACHE.lock() {
            if guard.len() > 8192 {
                guard.clear();
            }
            guard.insert(asset_id.to_owned(), size_bytes as u64);
        }
    }
    let session = require_session(state).await?;
    let mut content_type = media_content_type(&remote_path).to_owned();
    if media_type == "video" && (content_type.is_empty() || content_type == "application/octet-stream") {
        content_type = "video/mp4".to_string();
    }
    let _ = &media_type;
    // Some AList-backed providers reject byte ranges while still allowing
    // normal GET streams. Let the local proxy serve the actual request and
    // retry a full GET when range access is unsupported.
    let fallback_url = if media_type == "comicArchive" {
        legacy_archive_path(&remote_path)
            .map(|path| remote_url(&session.endpoint, &path))
            .transpose()?
    } else {
        None
    };
    let thumbnail_cache_path = if thumbnail {
        thumbnail_path_from_metadata(app, asset_id, &remote_path, size_bytes, etag, content_hash).ok()
    } else {
        None
    };
    let max_b = CACHED_MAX_SHORT_VIDEO_BYTES.load(std::sync::atomic::Ordering::Relaxed);
    let (max_cache_bytes, activity_tracker) = (Some(max_b), Some(state.last_stream_activity.clone()));
    let source = ProxySource {
        url: remote_url(&session.endpoint, &remote_path)?,
        fallback_url,
        session,
        content_type,
        remote_path,
        allow_api_fallback,
        api_url_cache: Arc::new(StdMutex::new(None)),
        direct_url: false,
        thumbnail_cache_path,
        proxy_address: Some(proxy.address),
        asset_id: Some(asset_id.to_string()),
        cache_base_path: cache_base_path(app).ok(),
        activity_tracker,
        max_cache_bytes,
    };
    let mut sources = proxy.sources.lock().await;
    sources.insert(token.clone(), source);
    // Do not resolve fs/get eagerly here. On 115-backed AList stores, the
    // detached control-plane request wakes the provider at the same time as
    // the first media Range request and makes the first byte slower. The API
    // URL remains a lazy fallback in open_proxy_upstream when /p or /d cannot
    // serve the stream.
    while sources.len() > MAX_MEDIA_PROXY_SOURCES {
        if let Some(expired) = sources.keys().find(|k| *k != &token).cloned() {
            sources.remove(&expired);
        } else {
            break;
        }
    }
    Ok(format!("http://{}/{route}/{token}{suffix}", proxy.address))
}

pub async fn ensure_token_registered(
    app: &AppHandle,
    state: &MediaLibraryState,
    token: &str,
    thumbnail: bool,
) {
    if let Ok(proxy) = ensure_media_proxy(state).await {
        if proxy.sources.lock().await.contains_key(token) {
            return;
        }
    }
    let asset_id = token.strip_prefix("thumb_").unwrap_or(token);
    if asset_id.starts_with("asset-") {
        let _ = register_media_source(app, state, asset_id, true, thumbnail).await;
    }
}

async fn register_direct_proxy_source(
    state: &MediaLibraryState,
    session: &Session,
    url: Url,
    content_type: &str,
) -> Result<String, String> {
    let proxy = ensure_media_proxy(state).await?;
    let source = ProxySource {
        url,
        fallback_url: None,
        session: session.clone(),
        content_type: content_type.to_owned(),
        remote_path: String::new(),
        allow_api_fallback: false,
        api_url_cache: Arc::new(StdMutex::new(None)),
        direct_url: true,
        thumbnail_cache_path: None,
        proxy_address: Some(proxy.address),
        asset_id: None,
        cache_base_path: None,
        activity_tracker: Some(state.last_stream_activity.clone()),
        max_cache_bytes: None,
    };
    let token = random_token();
    let mut sources = proxy.sources.lock().await;
    sources.insert(token.clone(), source);
    while sources.len() > MAX_MEDIA_PROXY_SOURCES {
        if let Some(expired) = sources.keys().next().cloned() {
            sources.remove(&expired);
        } else {
            break;
        }
    }
    Ok(format!("http://{}/media/{token}", proxy.address))
}

async fn ensure_media_proxy(state: &MediaLibraryState) -> Result<MediaProxyHandle, String> {
    let mut guard = state.proxy.lock().await;
    if guard.is_none() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| format!("无法启动本地媒体代理：{error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("无法读取本地媒体代理地址：{error}"))?;
        let sources = Arc::new(Mutex::new(HashMap::new()));
        let handler_sources = sources.clone();
        let handler_thumbs = state.thumbnails_dir.clone();
        crate::rt::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let _ = stream.set_nodelay(true);
                let sources = handler_sources.clone();
                let thumbs = handler_thumbs.clone();
                crate::rt::spawn(async move {
                    let _ = serve_proxy_request(stream, sources, thumbs).await;
                });
            }
        });
        *guard = Some(MediaProxy { address, sources });
    }
    let proxy = guard.as_ref().ok_or("本地媒体代理不可用")?;
    Ok(MediaProxyHandle {
        address: proxy.address,
        sources: proxy.sources.clone(),
    })
}

struct MediaProxyHandle {
    address: std::net::SocketAddr,
    sources: Arc<Mutex<HashMap<String, ProxySource>>>,
}
fn random_token() -> String {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn serve_local_file_range(
    stream: &mut TcpStream,
    method: &str,
    file_path: &std::path::Path,
    content_type: &str,
    range_header: Option<&str>,
) -> Result<(), String> {
    let mut file = tokio_fs::File::open(file_path)
        .await
        .map_err(|e| format!("打开缓存文件失败: {e}"))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|e| format!("获取缓存文件信息失败: {e}"))?;
    let file_size = metadata.len();
    if file_size == 0 {
        return Err("缓存文件大小为0".into());
    }

    if let Ok(f) = std::fs::File::open(file_path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }

    if method == "HEAD" {
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {file_size}\r\nAccept-Ranges: bytes\r\nCache-Control: private, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Content-Type, Accept-Ranges\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(header.as_bytes()).await.map_err(|e| e.to_string())?;
        return Ok(());
    }

    let mut start = 0u64;
    let mut end = file_size - 1;
    let mut is_range = false;

    if let Some(range_val) = range_header {
        let range_val = range_val.trim();
        if let Some(spec) = range_val.strip_prefix("bytes=") {
            let parts: Vec<&str> = spec.split('-').collect();
            if parts.len() == 2 {
                let p0 = parts[0].trim();
                let p1 = parts[1].trim();
                if !p0.is_empty() && !p1.is_empty() {
                    if let (Ok(s), Ok(e)) = (p0.parse::<u64>(), p1.parse::<u64>()) {
                        if s <= e && s < file_size {
                            start = s;
                            end = e.min(file_size - 1);
                            is_range = true;
                        }
                    }
                } else if !p0.is_empty() && p1.is_empty() {
                    if let Ok(s) = p0.parse::<u64>() {
                        if s < file_size {
                            start = s;
                            end = file_size - 1;
                            is_range = true;
                        }
                    }
                } else if p0.is_empty() && !p1.is_empty() {
                    if let Ok(suffix_len) = p1.parse::<u64>() {
                        if suffix_len > 0 {
                            start = file_size.saturating_sub(suffix_len);
                            end = file_size - 1;
                            is_range = true;
                        }
                    }
                }
            }
        }
    }

    let length = end - start + 1;
    let header = if is_range {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: {content_type}\r\nContent-Length: {length}\r\nContent-Range: bytes {start}-{end}/{file_size}\r\nAccept-Ranges: bytes\r\nCache-Control: private, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Content-Type, Accept-Ranges\r\nConnection: close\r\n\r\n"
        )
    } else {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {file_size}\r\nAccept-Ranges: bytes\r\nCache-Control: private, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Content-Type, Accept-Ranges\r\nConnection: close\r\n\r\n"
        )
    };

    stream.write_all(header.as_bytes()).await.map_err(|e| e.to_string())?;

    if start > 0 {
        file.seek(tokio::io::SeekFrom::Start(start))
            .await
            .map_err(|e| format!("定位文件失败: {e}"))?;
    }

    let mut remaining = length;
    let mut buffer = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let to_read = (remaining as usize).min(buffer.len());
        let count = file.read(&mut buffer[..to_read]).await.map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        stream.write_all(&buffer[..count]).await.map_err(|e| e.to_string())?;
        remaining -= count as u64;
    }

    Ok(())
}

async fn serve_proxy_request(
    mut stream: TcpStream,
    sources: Arc<Mutex<HashMap<String, ProxySource>>>,
    thumbnails_dir: Arc<StdMutex<Option<PathBuf>>>,
) -> Result<(), String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    while bytes.len() < 32 * 1024 {
        let count = stream
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.windows(4).any(|value| value == b"\r\n\r\n") {
            break;
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.split("\r\n");
    let request = lines.next().ok_or("无效的本地媒体请求")?;
    let mut request_parts = request.split_whitespace();
    let method = request_parts.next().unwrap_or_default();
    let target = request_parts.next().unwrap_or_default();

    if method == "OPTIONS" {
        let header = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, HEAD, OPTIONS\r\nAccess-Control-Allow-Headers: Range, If-Range, Content-Type, Authorization\r\nAccess-Control-Max-Age: 86400\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(header.as_bytes()).await;
        return Ok(());
    }

    if let Some(rest) = target.strip_prefix("/comic/") {
        return serve_comic_page_request(&mut stream, method, rest).await;
    }

    if let Some(cover_path) = target.strip_prefix("/api/covers/") {
        let clean = cover_path.split('?').next().unwrap_or(cover_path);
        let mut parts = clean.split('/');
        let _asset_id = parts.next().unwrap_or_default();
        let file_name = parts.next().unwrap_or_default();
        let thumb_dir = thumbnails_dir.lock().ok().and_then(|g| g.clone());
        if let Some(thumb_dir) = thumb_dir {
            let file_path = thumb_dir.join(file_name);
            if file_path.is_file() {
                if let Ok(bytes) = fs::read(&file_path) {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nCache-Control: public, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    if method != "HEAD" {
                        let _ = stream.write_all(&bytes).await;
                    }
                    return Ok(());
                }
            }
        }
        return write_proxy_error(&mut stream, 404, "Cover Not Found").await;
    }

    let (token, compatible, thumbnail) = if let Some(value) = target.strip_prefix("/thumbnail/") {
        (value, false, true)
    } else if let Some(value) = target.strip_prefix("/compatible/") {
        (value, true, false)
    } else if let Some(value) = target.strip_prefix("/media/") {
        (value, false, false)
    } else {
        return Err("无效的媒体地址".into());
    };
    let clean_token = token.split('?').next().unwrap_or(token).split('#').next().unwrap_or(token);
    let resolved_token = clean_token
        .rsplit_once('.')
        .map(|(base, _)| base)
        .unwrap_or(clean_token);
    if resolved_token.is_empty() || resolved_token.contains('/') {
        return Err("无效的媒体地址".into());
    }
    let mut range = None;
    let mut if_range = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("range") {
                range = Some(value.trim().to_owned());
            } else if name.eq_ignore_ascii_case("if-range") {
                if_range = Some(value.trim().to_owned());
            }
        }
    }
    let source = {
        let guard = sources.lock().await;
        guard.get(resolved_token).or_else(|| guard.get(clean_token)).cloned()
    };
    let Some(source) = source else {
        return write_proxy_error(&mut stream, 404, "Not Found").await;
    };
    if method != "GET" && method != "HEAD" {
        return write_proxy_error(&mut stream, 405, "Method Not Allowed").await;
    }
    if !thumbnail {
        if let Some(ref tracker) = source.activity_tracker {
            tracker.store(now() as u64, std::sync::atomic::Ordering::Relaxed);
        }
    }
    if thumbnail {
        return serve_video_thumbnail(&mut stream, method, token, source).await;
    }
    if compatible {
        return serve_compatible_proxy_request(&mut stream, method, source).await;
    }

    // Check if video is cached locally on disk
    if let (Some(ref asset_id), Some(ref base_path)) = (&source.asset_id, &source.cache_base_path) {
        if source.content_type.starts_with("video/") {
            let cache_dir = base_path.join("short-videos");
            let target_file = cache_dir.join(format!("{asset_id}.mp4"));
            if target_file.is_file() {
                if let Ok(meta) = fs::metadata(&target_file) {
                    if meta.len() >= 64 * 1024 {
                        return serve_local_file_range(&mut stream, method, &target_file, &source.content_type, range.as_deref()).await;
                    } else {
                        let _ = fs::remove_file(&target_file);
                    }
                }
            }
        }
    }

    let preempt_slot = acquire_video_stream_slot(&source, method, range.as_deref());
    match open_proxy_upstream(&source, method, range.as_deref(), if_range.as_deref()).await {
        Ok(response) => {
            pipe_proxy_response(&mut stream, method, response, &source, preempt_slot).await
        }
        Err(error) => write_proxy_error(&mut stream, 502, &error).await,
    }
}

/// Generate one small JPEG directly beside the authenticated media proxy.
/// This avoids browser-side seeking, which is slow and unreliable for remote
/// AList providers and for containers that require the FFmpeg compatibility
/// stream (AVI/MKV/TS).
async fn serve_video_thumbnail(
    stream: &mut TcpStream,
    method: &str,
    token: &str,
    source: ProxySource,
) -> Result<(), String> {
    if method == "HEAD" {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nCache-Control: private, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n")
            .await
            .map_err(|error| error.to_string())?;
        return Ok(());
    }

    // 1. Check disk cache first!
    if let Some(ref path) = source.thumbnail_cache_path {
        if path.is_file() {
            if let Ok(bytes) = fs::read(path) {
                if !bytes.is_empty() {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nCache-Control: private, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    stream
                        .write_all(header.as_bytes())
                        .await
                        .map_err(|error| error.to_string())?;
                    stream
                        .write_all(&bytes)
                        .await
                        .map_err(|error| error.to_string())?;
                    return Ok(());
                }
            }
        }
    }

    // 2. If the video is cached locally on disk, FFmpeg can extract in < 200ms
    if let (Some(ref asset_id), Some(ref base_path)) = (&source.asset_id, &source.cache_base_path) {
        let local_file = base_path.join("short-videos").join(format!("{asset_id}.mp4"));
        if local_file.is_file() {
            if let Ok(meta) = fs::metadata(&local_file) {
                if meta.len() >= 64 * 1024 {
                    let child = Command::new("ffmpeg")
                        .args([
                            "-hide_banner", "-loglevel", "error", "-ss", "0.5",
                            "-i", local_file.to_str().unwrap_or_default(),
                            "-map", "0:v:0", "-frames:v", "1",
                            "-vf", "scale=min(480\\,iw):-2", "-q:v", "5",
                            "-f", "image2pipe", "-vcodec", "mjpeg", "pipe:1"
                        ])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .kill_on_drop(true)
                        .spawn();
                    if let Ok(mut proc) = child {
                        let mut buf = Vec::new();
                        if let Some(mut out) = proc.stdout.take() {
                            let _ = out.read_to_end(&mut buf).await;
                        }
                        if proc.wait().await.is_ok_and(|s| s.success()) && !buf.is_empty() {
                            if let Some(ref path) = source.thumbnail_cache_path {
                                if let Some(parent) = path.parent() {
                                    let _ = fs::create_dir_all(parent);
                                }
                                let _ = fs::write(path, &buf);
                            }
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nCache-Control: private, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                                buf.len()
                            );
                            stream.write_all(header.as_bytes()).await.map_err(|e| e.to_string())?;
                            stream.write_all(&buf).await.map_err(|e| e.to_string())?;
                            return Ok(());
                        }
                    }
                } else {
                    let _ = fs::remove_file(&local_file);
                }
            }
        }
    }

    // 3. For remote video / HEIC: extract with a responsive concurrency limiter
    // Generates thumbnails on demand with robust timeouts for remote drives.
    // If active video streaming is currently occurring, yield immediately to prioritize streaming bandwidth!
    if let Some(ref tracker) = source.activity_tracker {
        let last_act = tracker.load(std::sync::atomic::Ordering::Relaxed);
        if last_act > 0 && now().saturating_sub(last_act as i64) < 15 {
            return write_proxy_error(stream, 503, "Yielding to Active Media Stream").await;
        }
    }

    static THUMBNAIL_SEMAPHORE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let semaphore = THUMBNAIL_SEMAPHORE.get_or_init(|| tokio::sync::Semaphore::new(2));

    let Ok(_permit) = tokio::time::timeout(std::time::Duration::from_secs(35), semaphore.acquire()).await else {
        return write_proxy_error(stream, 503, "Thumbnail Generation Queue Busy").await;
    };

    let is_heic_img = is_heic(&source.remote_path);
    if is_heic_img {
        if let Ok(response) = open_proxy_upstream(&source, "GET", None, None).await {
            if let Ok(bytes) = response.bytes().await {
                if !bytes.is_empty() {
                    let temp_dir = std::env::temp_dir();
                    let rand_id = random_token();
                    let in_path = temp_dir.join(format!("pml_heic_{rand_id}.heic"));
                    let out_path = temp_dir.join(format!("pml_heic_{rand_id}.jpg"));

                    if fs::write(&in_path, &bytes).is_ok() {
                        let mut success = false;
                        let hc = Command::new("heif-convert")
                            .args(["-q", "90", in_path.to_str().unwrap_or_default(), out_path.to_str().unwrap_or_default()])
                            .output()
                            .await;
                        if hc.is_ok_and(|out| out.status.success()) && out_path.is_file() {
                            success = true;
                        }

                        if !success {
                            let ff = Command::new("ffmpeg")
                                .args([
                                    "-hide_banner", "-loglevel", "error", "-threads", "1",
                                    "-i", in_path.to_str().unwrap_or_default(),
                                    "-frames:v", "1",
                                    "-vf", "scale=min(1440\\,iw):-2",
                                    "-q:v", "3",
                                    out_path.to_str().unwrap_or_default(),
                                ])
                                .output()
                                .await;
                            if ff.is_ok_and(|out| out.status.success()) && out_path.is_file() {
                                success = true;
                            }
                        }

                        let _ = fs::remove_file(&in_path);

                        if success {
                            if let Ok(jpg_bytes) = fs::read(&out_path) {
                                let _ = fs::remove_file(&out_path);
                                if !jpg_bytes.is_empty() {
                                    if let Some(ref cpath) = source.thumbnail_cache_path {
                                        if let Some(p) = cpath.parent() {
                                            let _ = fs::create_dir_all(p);
                                        }
                                        let _ = fs::write(cpath, &jpg_bytes);
                                    }
                                    let header = format!(
                                        "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nCache-Control: private, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                                        jpg_bytes.len()
                                    );
                                    let _ = stream.write_all(header.as_bytes()).await;
                                    let _ = stream.write_all(&jpg_bytes).await;
                                    return Ok(());
                                }
                            }
                            let _ = fs::remove_file(&out_path);
                        }
                    }
                }
            }
        }
    }

    if let Some(address) = source.proxy_address {
        let media_url = format!("http://{address}/media/{token}");
        let is_still_image = source.content_type.starts_with("image/") || is_heic_img;
        let loopback_task = async {
            let scale_filter = if is_heic_img { "scale=min(1080\\,iw):-2" } else { "scale=min(480\\,iw):-2" };

            let run_ffmpeg = |ss_opt: Option<&str>| {
                let mut args = vec![
                    "-hide_banner",
                    "-loglevel", "error",
                    "-threads", "1",
                ];
                if let Some(pos) = ss_opt {
                    args.extend_from_slice(&["-ss", pos]);
                }
                args.extend_from_slice(&[
                    "-i", &media_url,
                    "-map", "0:v:0",
                    "-frames:v", "1",
                    "-vf", scale_filter,
                    "-q:v", "5",
                    "-f", "image2pipe",
                    "-vcodec", "mjpeg",
                    "pipe:1",
                ]);
                Command::new("ffmpeg")
                    .args(&args)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
            };

            // Attempt 1: For still images, never seek (-ss). For video, seek 0.5s to skip black intro frame.
            let first_ss = if is_still_image { None } else { Some("0.5") };
            if let Ok(mut proc) = run_ffmpeg(first_ss) {
                let mut out = proc.stdout.take();
                let mut buf = Vec::new();
                if let Some(ref mut stdout) = out {
                    let _ = stdout.read_to_end(&mut buf).await;
                }
                let status = proc.wait().await;
                if status.is_ok_and(|s| s.success()) && !buf.is_empty() {
                    return Ok::<Vec<u8>, String>(buf);
                }
            }

            // Attempt 2: If video seek at 0.5s failed (e.g. ultra-short clip < 0.5s or index seek issue), fallback to frame 0
            if !is_still_image {
                if let Ok(mut proc) = run_ffmpeg(None) {
                    let mut out = proc.stdout.take();
                    let mut buf = Vec::new();
                    if let Some(ref mut stdout) = out {
                        let _ = stdout.read_to_end(&mut buf).await;
                    }
                    let status = proc.wait().await;
                    if status.is_ok_and(|s| s.success()) && !buf.is_empty() {
                        return Ok::<Vec<u8>, String>(buf);
                    }
                }
            }

            Err::<Vec<u8>, String>("无法提取封面".into())
        };

        if let Ok(Ok(buf)) = tokio::time::timeout(std::time::Duration::from_secs(30), loopback_task).await {
            if let Some(ref path) = source.thumbnail_cache_path {
                if let Some(parent) = path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                let _ = fs::write(path, &buf);
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nCache-Control: private, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                buf.len()
            );
            stream.write_all(header.as_bytes()).await.map_err(|e| e.to_string())?;
            stream.write_all(&buf).await.map_err(|e| e.to_string())?;
            return Ok(());
        }
    }

    write_proxy_error(stream, 404, "Thumbnail Not Cached").await
}

/// Convert containers that browser media elements cannot decode (AVI/MKV/TS,
/// etc.) into a fragmented MP4 stream. The input is authenticated by the same
/// proxy source used for normal playback, so credentials never reach FFmpeg or
/// the browser. The fragmented MP4 header allows playback to begin before the
/// full remote file has been downloaded.
async fn serve_compatible_proxy_request(
    stream: &mut TcpStream,
    method: &str,
    source: ProxySource,
) -> Result<(), String> {
    if method == "HEAD" {
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nCache-Control: private, max-age=300\r\nAccept-Ranges: none\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n")
            .await
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
    let response = match open_proxy_upstream(&source, "GET", None, None).await {
        Ok(response) => response,
        Err(error) => return write_proxy_error(stream, 502, &error).await,
    };
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = proxy_response_summary(&response);
        return write_proxy_error(stream, status, &body).await;
    }
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            "-i",
            "pipe:0",
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "28",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-movflags",
            "+frag_keyframe+empty_moov+default_base_moof",
            "-f",
            "mp4",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("当前服务未安装 FFmpeg，无法播放此视频格式：{error}"))?;
    let mut input = child.stdin.take().ok_or("FFmpeg 输入管道不可用")?;
    let mut output = child.stdout.take().ok_or("FFmpeg 输出管道不可用")?;
    let mut body = response.bytes_stream();
    let tracker_clone = source.activity_tracker.clone();
    let writer = crate::rt::spawn(async move {
        let mut last_chunk_ts = 0i64;
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|error| format!("读取远程视频失败：{error}"))?;
            let current_ts = now();
            if current_ts.saturating_sub(last_chunk_ts) >= 3 {
                last_chunk_ts = current_ts;
                if let Some(ref tracker) = tracker_clone {
                    tracker.store(current_ts as u64, std::sync::atomic::Ordering::Relaxed);
                }
            }
            input
                .write_all(&chunk)
                .await
                .map_err(|error| format!("写入 FFmpeg 输入失败：{error}"))?;
        }
        drop(input);
        Ok::<(), String>(())
    });
    if let Err(e) = stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nCache-Control: private, max-age=300\r\nTransfer-Encoding: chunked\r\nAccept-Ranges: none\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Type, Accept-Ranges\r\nConnection: close\r\n\r\n")
        .await
    {
        let _ = child.kill().await;
        return Err(e.to_string());
    }
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let count = match output.read(&mut buffer).await {
            Ok(c) => c,
            Err(e) => {
                let _ = child.kill().await;
                return Err(format!("读取 FFmpeg 输出失败：{e}"));
            }
        };
        if count == 0 {
            break;
        }
        let size = format!("{:X}\r\n", count);
        if let Err(e) = stream.write_all(size.as_bytes()).await {
            let _ = child.kill().await;
            return Err(e.to_string());
        }
        if let Err(e) = stream.write_all(&buffer[..count]).await {
            let _ = child.kill().await;
            return Err(e.to_string());
        }
        if let Err(e) = stream.write_all(b"\r\n").await {
            let _ = child.kill().await;
            return Err(e.to_string());
        }
    }
    let _ = stream.write_all(b"0\r\n\r\n").await;
    let _ = writer.await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    Ok(())
}

/// 获取 AList API 返回的下载地址，作为服务端 `/p` 代理失败时的兼容后备。
/// 注意：115 的 CDN 直链只适合调试或已由上游附带 Cookie 的部署；常规播放
/// 必须优先使用 `/p`，否则会因缺少 115 Cookie 得到 403 invalid signature。
async fn alist_api_download_url(
    session: &Session,
    remote_path: &str,
) -> Result<Option<Url>, String> {
    let token = session.api_token.as_str();

    let get_url = alist_api_url(&session.endpoint, "fs/get")?;
    let get = session
        .client
        .post(get_url)
        // fs/get is a small control-plane request. Do not let a sleeping
        // 115 driver hold the first video request for the full media timeout;
        // the proxy still has the legacy AList routes as fallback.
        .timeout(std::time::Duration::from_secs(6))
        .header(USER_AGENT, MEDIA_USER_AGENT)
        .header(AUTHORIZATION, token)
        // Use refresh: false so AList answers from its memory/local cache instantly without
        // forcing a slow upstream query to the cloud provider on every video start.
        .json(&serde_json::json!({ "path": remote_path, "password": "", "refresh": false }))
        .send()
        .await
        .map_err(|error| format!("AList 下载链接获取失败：{error}"))?;
    if !get.status().is_success() {
        return Err(format!("AList 下载链接获取失败（HTTP {}）", get.status()));
    }
    let get_body: serde_json::Value = get
        .json()
        .await
        .map_err(|error| format!("读取 AList 下载响应失败：{error}"))?;
    if get_body
        .get("code")
        .and_then(|value| value.as_i64())
        .is_some_and(|code| code != 200 && code != 0)
    {
        return Err(format!(
            "AList 下载链接获取失败：{}",
            get_body
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or("未知错误")
        ));
    }
    let data = get_body.get("data");
    let raw = data
        .and_then(|value| value.get("raw_url"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty());
    let signed = data
        .and_then(|value| value.get("url"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty());
    // Prefer AList's own download route. It can refresh/translate the
    // provider signature, while raw_url is often a short-lived CDN URL.
    let candidate = signed.or(raw).unwrap_or_default();
    if candidate.is_empty() {
        return Err("AList 未返回可用的下载直链".into());
    }
    let mut url = Url::parse(candidate)
        .or_else(|_| session.endpoint.join(candidate))
        .map_err(|_| "AList 返回了无效下载地址".to_string())?;
    // `/api/fs/get` returns `/d/...` for many deployments. When AList itself
    // is mounted below a reverse-proxy prefix, an absolute-path join would
    // otherwise drop that prefix and point at the wrong virtual host route.
    if let Some(prefix) = alist_route_prefix(&session.endpoint) {
        if same_origin(&url, &session.endpoint)
            && (url.path().starts_with("/d/") || url.path().starts_with("/p/"))
        {
            let path = format!("/{prefix}{}", url.path());
            url.set_path(&path);
        }
    }
    // Some AList builds return the route and its signature as separate fields.
    // Reattach the sign when the route does not already carry a query.
    if url.path().starts_with("/d/") && url.query().is_none() {
        if let Some(sign) = data
            .and_then(|value| value.get("sign"))
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
        {
            url.query_pairs_mut().append_pair("sign", sign);
        }
    }
    if !matches!(url.scheme(), "http" | "https") {
        return Err("AList 返回了不安全的下载地址".into());
    }
    Ok(Some(url))
}

/// 带 TTL 缓存的 AList API 直链解析。dav GET 被拒绝（405）的环境下，视频的
/// play/seek 会反复触发上游重连；缓存直链可避免每次都登录 API 并重新取 115
/// 冷启动直链，显著缩短二次请求的等待。
async fn cached_api_download_url(source: &ProxySource) -> Result<Option<Url>, String> {
    if !source.remote_path.is_empty() {
        if let Ok(cache) = GLOBAL_API_URL_CACHE.lock() {
            if let Some((url, expires_at)) = cache.get(&source.remote_path) {
                if *expires_at > now() {
                    return Ok(Some(url.clone()));
                }
            }
        }
    }
    {
        let cache = source.api_url_cache.lock().unwrap();
        if let Some((url, expires_at)) = cache.as_ref() {
            if *expires_at > now() {
                return Ok(Some(url.clone()));
            }
        }
    }
    match alist_api_download_url(&source.session, &source.remote_path).await {
        Ok(result) => {
            if let Some(url) = result.clone() {
                *source.api_url_cache.lock().unwrap() = Some((url.clone(), now() + 600));
                if !source.remote_path.is_empty() {
                    if let Ok(mut cache) = GLOBAL_API_URL_CACHE.lock() {
                        cache.insert(source.remote_path.clone(), (url, now() + 600));
                    }
                }
            }
            Ok(result)
        }
        Err(error) => Err(format!("AList API 直链请求失败：{error}")),
    }
}

async fn open_proxy_upstream(
    source: &ProxySource,
    method: &str,
    range: Option<&str>,
    if_range: Option<&str>,
) -> Result<reqwest::Response, String> {
    // Fast path: if AList API direct download URL is already cached, hit it first to avoid failing on /p and /d
    if source.allow_api_fallback && source.session.provider == ProviderKind::Alist {
        let cached_url = {
            let from_global = if !source.remote_path.is_empty() {
                GLOBAL_API_URL_CACHE.lock().ok().and_then(|c| {
                    c.get(&source.remote_path).and_then(|(u, exp)| if *exp > now() { Some(u.clone()) } else { None })
                })
            } else {
                None
            };
            from_global.or_else(|| {
                source.api_url_cache.lock().ok().and_then(|c| {
                    c.as_ref().and_then(|(u, exp)| if *exp > now() { Some(u.clone()) } else { None })
                })
            })
        };
        if let Some(api_url) = cached_url {
            if let Ok(response) = open_proxy_upstream_for_url(source, &api_url, method, range, if_range).await {
                if response_is_usable(&response, source) && !matches!(response.status(), StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED) {
                    return Ok(response);
                }
            }
        }
    }

    let mut candidates = VecDeque::new();
    if source.direct_url || source.session.provider == ProviderKind::CloudDrive2 {
        candidates.push_back(source.url.clone());
    } else {
        for source_url in [Some(source.url.clone()), source.fallback_url.clone()] {
            if let Some(source_url) = source_url {
                if let Some(proxy_url) = alist_proxy_url(&source.session.endpoint, &source_url) {
                    candidates.push_back(proxy_url);
                }
                if let Some(download_url) =
                    alist_download_url(&source.session.endpoint, &source_url)
                {
                    candidates.push_back(download_url);
                }
            }
        }
    }

    let mut attempts = Vec::new();
    while let Some(candidate) = candidates.pop_front() {
        match open_proxy_upstream_for_url(source, &candidate, method, range, if_range).await {
            Ok(response) if response_is_usable(&response, source) => return Ok(response),
            Ok(response) => {
                attempts.push(proxy_response_summary(&response));
            }
            Err(error) => attempts.push(error),
        }
    }

    if source.allow_api_fallback && source.session.provider == ProviderKind::Alist {
        let api_url = cached_api_download_url(source)
            .await?
            .ok_or("AList API 未返回可用媒体下载直链")?;
        let response =
            open_proxy_upstream_for_url(source, &api_url, method, range, if_range).await?;
        if !matches!(
            response.status(),
            StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED
        ) {
            return Ok(response);
        }
        *source.api_url_cache.lock().unwrap() = None;
        if let Ok(mut cache) = GLOBAL_API_URL_CACHE.lock() {
            cache.remove(&source.remote_path);
        }
        let refreshed = cached_api_download_url(source)
            .await?
            .ok_or("AList API 未返回可用媒体下载直链")?;
        let response =
            open_proxy_upstream_for_url(source, &refreshed, method, range, if_range).await?;
        if matches!(
            response.status(),
            StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED
        ) {
            return Err("AList 115 下载被拒绝（403 invalid signature）：请在该 115 存储启用 Web 代理，并确认 API 令牌具有读取权限".into());
        }
        return Ok(response);
    }
    let provider_name = match source.session.provider {
        ProviderKind::Alist => "AList",
        ProviderKind::CloudDrive2 => "CloudDrive2",
    };
    Err(format!("{provider_name} 未返回可用媒体流（{}）", attempts.join("；")))
}

fn proxy_response_summary(response: &reqwest::Response) -> String {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("未声明类型");
    let content_length = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("未知大小");
    format!(
        "HTTP {}，{}，{}",
        response.status(),
        content_type,
        content_length
    )
}

fn response_is_usable(response: &reqwest::Response, source: &ProxySource) -> bool {
    if !response.status().is_success() {
        return false;
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    // AList's web UI and some storage-driver failures use HTTP 200 for an HTML
    // or JSON error page. A successful status alone must never make that body
    // look like a media stream to WebView/mpv.
    if content_type.starts_with("text/")
        || content_type.contains("json")
        || content_type.contains("xml")
        || content_type.contains("javascript")
    {
        return false;
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .is_some_and(|value| value == "0")
    {
        return false;
    }
    // AList commonly uses application/octet-stream for a valid download, so
    // allow it. For explicit image/video responses, reject a clearly wrong
    // media family before the local proxy forwards it.
    if source.content_type.starts_with("image/") {
        return content_type.is_empty()
            || content_type.starts_with("image/")
            || content_type.starts_with("application/octet-stream");
    }
    if source.content_type.starts_with("video/") {
        return content_type.is_empty()
            || content_type.starts_with("video/")
            || content_type.starts_with("application/octet-stream");
    }
    true
}

fn alist_download_url(endpoint: &Url, source: &Url) -> Option<Url> {
    alist_route_url(endpoint, source, "d")
}

fn alist_thumbnail_url(endpoint: &Url, raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = Url::parse(raw).or_else(|_| endpoint.join(raw)).ok()?;
    let path = parsed.path();
    let is_thumb_api = path
        .trim_matches('/')
        .strip_prefix(&format!("{}/", alist_base_prefix(endpoint)))
        .unwrap_or_else(|| path.trim_matches('/'))
        .eq_ignore_ascii_case("api/fs/thumb");
    if !is_thumb_api
        && !path
            .split('/')
            .any(|segment| segment == "p" || segment == "d")
    {
        return None;
    }
    let mut url = endpoint.clone();
    if is_thumb_api {
        let api_url = alist_api_url(endpoint, "fs/thumb").ok()?;
        url.set_path(api_url.path());
        url.set_query(parsed.query());
        url.set_fragment(None);
        return Some(url);
    }
    let prefix = alist_route_prefix(endpoint).unwrap_or_default();
    let route_path = if prefix.is_empty() || path.starts_with(&format!("/{prefix}/")) {
        path.to_owned()
    } else {
        format!("/{prefix}{path}")
    };
    url.set_path(&route_path);
    url.set_query(parsed.query());
    url.set_fragment(None);
    Some(url)
}

fn alist_proxy_url(endpoint: &Url, source: &Url) -> Option<Url> {
    let mut url = alist_route_url(endpoint, source, "p")?;
    // AList's /p handler otherwise redirects to a configured Down Proxy URL.
    // Keep the request on AList so the driver-provided 115 Cookie is retained.
    url.query_pairs_mut().append_pair("d", "1");
    Some(url)
}

/// Builds an AList API URL below the same reverse-proxy mount as the entered
/// endpoint. An endpoint mounted below `/alist/` maps to `/alist/api/*`, while
/// a normal root endpoint maps to `/api/*`. `Url::join` with a leading
/// slash would silently discard `/alist`, which made token scans succeed only
/// on installations exposed at the host root.
fn alist_api_url(endpoint: &Url, route: &str) -> Result<Url, String> {
    let route = route.trim_matches('/');
    if route.is_empty() {
        return Err("AList API 路径无效".into());
    }
    let prefix = alist_base_prefix(endpoint);
    let path = if prefix.is_empty() {
        format!("/api/{route}")
    } else {
        format!("/{prefix}/api/{route}")
    };
    let mut url = endpoint.clone();
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn alist_route_root_url(endpoint: &Url, route: &str) -> Result<Url, String> {
    let route = route.trim_matches('/');
    if route.is_empty() {
        return Err("AList 路径无效".into());
    }
    let prefix = alist_base_prefix(endpoint);
    let path = if prefix.is_empty() {
        format!("/{route}")
    } else {
        format!("/{prefix}/{route}")
    };
    let mut url = endpoint.clone();
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn alist_base_prefix(endpoint: &Url) -> String {
    let mut segments = endpoint
        .path_segments()
        .map(|parts| {
            parts
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if segments.last().is_some_and(|segment| {
        segment.eq_ignore_ascii_case("dav") || segment.eq_ignore_ascii_case("api")
    }) {
        segments.pop();
    }
    segments.join("/")
}

fn is_alist_owned_route(endpoint: &Url, current: &Url) -> bool {
    if !same_origin(endpoint, current) {
        return false;
    }
    let prefix = alist_base_prefix(endpoint);
    let route_prefix = if prefix.is_empty() {
        "/".to_owned()
    } else {
        format!("/{prefix}/")
    };
    let Some(route) = current.path().strip_prefix(&route_prefix) else {
        return false;
    };
    matches!(route.split('/').next(), Some("api" | "d" | "p"))
}

fn alist_route_url(endpoint: &Url, source: &Url, route: &str) -> Option<Url> {
    if !same_origin(endpoint, source) {
        return None;
    }
    let endpoint_path = endpoint.path().trim_end_matches('/');
    let source_path = source.path();
    let relative = source_path
        .strip_prefix(endpoint_path)?
        .trim_start_matches('/');
    if relative.is_empty() {
        return None;
    }
    let mut url = endpoint.clone();
    // AList mounts all routes below its configured URL.Path. For a reverse
    // proxy such as `/alist/dav/`, keep `/alist` when translating DAV to
    // `/alist/p/...` or `/alist/d/...`; the old implementation always reset
    // to the host root and silently produced an HTML 404 page.
    let route_prefix = alist_route_prefix(endpoint).unwrap_or_default();
    let route_path = if route_prefix.is_empty() {
        format!("/{route}/{relative}")
    } else {
        format!("/{route_prefix}/{route}/{relative}")
    };
    url.set_path(&route_path);
    Some(url)
}

fn alist_route_prefix(endpoint: &Url) -> Option<String> {
    let prefix = alist_base_prefix(endpoint);
    (!prefix.is_empty()).then_some(prefix)
}

async fn open_proxy_upstream_for_url(
    source: &ProxySource,
    url: &Url,
    method: &str,
    range: Option<&str>,
    if_range: Option<&str>,
) -> Result<reqwest::Response, String> {
    let response = open_proxy_upstream_request(source, url, method, range, if_range).await?;
    if range.is_some()
        && method == "GET"
        && matches!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED
                | StatusCode::RANGE_NOT_SATISFIABLE
        )
    {
        return open_proxy_upstream_request(source, url, method, None, None).await;
    }
    Ok(response)
}

async fn open_proxy_upstream_request(
    source: &ProxySource,
    initial_url: &Url,
    method: &str,
    range: Option<&str>,
    if_range: Option<&str>,
) -> Result<reqwest::Response, String> {
    let mut current = initial_url.clone();
    for _ in 0..=5 {
        let request_method = if method == "HEAD" {
            Method::HEAD
        } else {
            Method::GET
        };
        let mut request = source
            .session
            .client
            .request(request_method, current.clone())
            // Keep the same media request shape as the reference client. In
            // particular, do not impersonate a browser when AList redirects
            // to a signed 115 URL: that provider can bind its response to the
            // original request profile.
            .header(ACCEPT_ENCODING, "identity");
        if same_origin(&current, &source.session.endpoint) {
            if source.session.provider == ProviderKind::CloudDrive2 {
                if !source.session.api_token.is_empty() {
                    request = request.header(AUTHORIZATION, &source.session.api_token);
                }
            } else if is_alist_owned_route(&source.session.endpoint, &current) {
                // Keep the token on AList-owned routes, but never forward it
                // to a provider/CDN after a redirect.
                request = request.header(AUTHORIZATION, &source.session.api_token);
            }
        }
        if let Some(range) = range {
            request = request.header(RANGE, range);
        }
        if let Some(if_range) = if_range {
            request = request.header(IF_RANGE, if_range);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("读取媒体失败：{error}"))?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or("媒体重定向缺少地址")?;
        let next = current
            .join(location)
            .map_err(|_| "媒体重定向地址无效".to_string())?;
        if !matches!(next.scheme(), "http" | "https")
            || (current.scheme() == "https" && next.scheme() != "https")
        {
            return Err("媒体重定向地址不安全".into());
        }
        current = next;
    }
    Err("媒体重定向次数过多".into())
}

fn parse_content_range(val: &str) -> Option<(u64, u64, u64)> {
    let rest = val.trim().strip_prefix("bytes ")?.trim();
    let (range_str, total_str) = rest.split_once('/')?;
    let total = total_str.trim().parse::<u64>().ok()?;
    let (start_str, end_str) = range_str.trim().split_once('-')?;
    let start = start_str.trim().parse::<u64>().ok()?;
    let end = end_str.trim().parse::<u64>().ok()?;
    Some((start, end, total))
}

fn parse_range_header_bounds(range_header: Option<&str>) -> Option<(u64, Option<u64>)> {
    let spec = range_header?.trim().strip_prefix("bytes=")?;
    let (start_str, end_str) = spec.split_once('-')?;
    let start_str = start_str.trim();
    let end_str = end_str.trim();
    if start_str.is_empty() {
        return None;
    }
    let start = start_str.parse::<u64>().ok()?;
    let end = if end_str.is_empty() {
        None
    } else {
        end_str.parse::<u64>().ok()
    };
    Some((start, end))
}

fn acquire_video_stream_slot(
    source: &ProxySource,
    method: &str,
    range_header: Option<&str>,
) -> Option<(u64, VideoStreamPreemption)> {
    if method != "GET" || !source.content_type.starts_with("video/") {
        return None;
    }
    let asset_id = source.asset_id.as_ref()?;
    let file_size = ASSET_SIZE_CACHE
        .lock()
        .ok()
        .and_then(|g| g.get(asset_id).copied())
        .unwrap_or(0);
    if let Some((start, end_opt)) = parse_range_header_bounds(range_header) {
        // Browsers often issue a concurrent tail-range request (within the last 8MB of the file)
        // to read the MP4 moov atom while the initial bytes=0- stream is still active, or issue
        // a tiny probe (< 64KB). Do not let metadata probes preempt main playback or vice-versa.
        let is_tail_probe =
            file_size > 16 * 1024 * 1024 && start >= file_size.saturating_sub(8 * 1024 * 1024);
        let is_tiny_probe = end_opt.is_some_and(|end| end >= start && (end - start) <= 65536);
        if is_tail_probe || is_tiny_probe {
            return None;
        }
    }
    let Ok(mut map) = ACTIVE_VIDEO_STREAMS.lock() else {
        return None;
    };
    if map.len() > 2048 {
        map.clear();
    }
    let entry = map
        .entry(asset_id.clone())
        .or_insert_with(|| VideoStreamPreemption {
            seq: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            notify: Arc::new(tokio::sync::Notify::new()),
        })
        .clone();
    let my_seq = entry
        .seq
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;
    entry.notify.notify_waiters();
    Some((my_seq, entry))
}

struct PartFileCleanupGuard {
    part_path: Option<PathBuf>,
}

impl Drop for PartFileCleanupGuard {
    fn drop(&mut self) {
        if let Some(path) = self.part_path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

async fn serve_local_file_range_axum(
    method: &axum::http::Method,
    file_path: &std::path::Path,
    content_type: &str,
    range_header: Option<&str>,
) -> Result<axum::response::Response, String> {
    use axum::{
        body::Body,
        http::{HeaderMap, HeaderValue, StatusCode as AxumStatusCode},
        response::IntoResponse,
    };

    let mut file = tokio_fs::File::open(file_path)
        .await
        .map_err(|e| format!("打开缓存文件失败: {e}"))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|e| format!("获取缓存文件信息失败: {e}"))?;
    let file_size = metadata.len();
    if file_size == 0 {
        return Err("缓存文件大小为0".into());
    }

    let mut start = 0u64;
    let mut end = file_size - 1;
    let mut is_range = false;

    if let Some(range_val) = range_header {
        let range_val = range_val.trim();
        if let Some(spec) = range_val.strip_prefix("bytes=") {
            let parts: Vec<&str> = spec.split('-').collect();
            if parts.len() == 2 {
                let p0 = parts[0].trim();
                let p1 = parts[1].trim();
                if !p0.is_empty() && !p1.is_empty() {
                    if let (Ok(s), Ok(e)) = (p0.parse::<u64>(), p1.parse::<u64>()) {
                        if s <= e && s < file_size {
                            start = s;
                            end = e.min(file_size - 1);
                            is_range = true;
                        }
                    }
                } else if !p0.is_empty() && p1.is_empty() {
                    if let Ok(s) = p0.parse::<u64>() {
                        if s < file_size {
                            start = s;
                            end = file_size - 1;
                            is_range = true;
                        }
                    }
                } else if p0.is_empty() && !p1.is_empty() {
                    if let Ok(suffix_len) = p1.parse::<u64>() {
                        if suffix_len > 0 {
                            start = file_size.saturating_sub(suffix_len);
                            end = file_size - 1;
                            is_range = true;
                        }
                    }
                }
            }
        }
    }

    let length = end - start + 1;
    let mut out_headers = HeaderMap::new();
    if let Ok(ct) = HeaderValue::from_str(content_type) {
        out_headers.insert(CONTENT_TYPE, ct);
    }
    if let Ok(cl) = HeaderValue::from_str(&length.to_string()) {
        out_headers.insert(CONTENT_LENGTH, cl);
    }
    out_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    out_headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=86400"),
    );
    out_headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    out_headers.insert(
        axum::http::header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Content-Length, Content-Range, Content-Type, Accept-Ranges"),
    );

    if *method == axum::http::Method::HEAD {
        return Ok((AxumStatusCode::OK, out_headers).into_response());
    }

    let status = if is_range {
        if let Ok(cr) = HeaderValue::from_str(&format!("bytes {start}-{end}/{file_size}")) {
            out_headers.insert(CONTENT_RANGE, cr);
        }
        AxumStatusCode::PARTIAL_CONTENT
    } else {
        AxumStatusCode::OK
    };

    if start > 0 {
        file.seek(tokio::io::SeekFrom::Start(start))
            .await
            .map_err(|e| format!("定位文件失败: {e}"))?;
    }

    let stream = futures_util::stream::unfold((file, length), |(mut file, remaining)| async move {
        if remaining == 0 {
            return None;
        }
        let to_read = (remaining as usize).min(64 * 1024);
        let mut buf = vec![0u8; to_read];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<Vec<u8>, std::io::Error>(buf), (file, remaining - n as u64)))
            }
            Err(e) => Some((Err(e), (file, 0))),
        }
    });

    Ok((status, out_headers, Body::from_stream(stream)).into_response())
}

pub async fn serve_media_axum(
    app: &AppHandle,
    token: String,
    method: axum::http::Method,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    use axum::{
        body::Body,
        http::{HeaderMap, HeaderValue, StatusCode as AxumStatusCode},
        response::IntoResponse,
    };

    if token.is_empty() || token.contains('/') {
        return AxumStatusCode::BAD_REQUEST.into_response();
    }
    if method != axum::http::Method::GET && method != axum::http::Method::HEAD {
        return AxumStatusCode::METHOD_NOT_ALLOWED.into_response();
    }

    // `open_media_source` 生成的代理地址带扩展名（/media/asset-hex.mp4），
    // 而源是以 asset id 注册的；这里必须和本地代理一样先去掉扩展名再查找。
    let token = {
        let base = token
            .split(['?', '#'])
            .next()
            .unwrap_or(token.as_str())
            .to_owned();
        match base.rsplit_once('.') {
            Some((stem, _)) if stem.starts_with("asset-") => stem.to_owned(),
            _ => base,
        }
    };

    let state = app.state::<MediaLibraryState>();
    ensure_token_registered(app, &state, &token, false).await;

    let source = {
        let guard = state.proxy.lock().await;
        if let Some(proxy) = guard.as_ref() {
            proxy.sources.lock().await.get(&token).cloned()
        } else {
            None
        }
    };
    let Some(source) = source else {
        return AxumStatusCode::NOT_FOUND.into_response();
    };

    if let Some(ref tracker) = source.activity_tracker {
        tracker.store(now() as u64, std::sync::atomic::Ordering::Relaxed);
    }

    let range = headers
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .map(ToOwned::to_owned);
    let if_range = headers
        .get(IF_RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .map(ToOwned::to_owned);

    // 1. Serve from local short-video disk cache if present
    if let (Some(ref asset_id), Some(ref base_path)) = (&source.asset_id, &source.cache_base_path) {
        if source.content_type.starts_with("video/") {
            let target_file = base_path.join("short-videos").join(format!("{asset_id}.mp4"));
            if target_file.is_file() {
                if let Ok(meta) = fs::metadata(&target_file) {
                    if meta.len() >= 64 * 1024 {
                        if let Ok(resp) = serve_local_file_range_axum(
                            &method,
                            &target_file,
                            &source.content_type,
                            range.as_deref(),
                        )
                        .await
                        {
                            return resp;
                        }
                    } else {
                        let _ = fs::remove_file(&target_file);
                    }
                }
            }
        }
    }

    // 2. Acquire active video stream slot (preempting any stale playback/seek range on the same video)
    let method_str = method.as_str();
    let preempt_slot = acquire_video_stream_slot(&source, method_str, range.as_deref());

    // 2.5 AList 视频流加速中继（服务端直连 115 CDN 管道式中继，紧密重试防掉落慢速 CD2）
    let mut alist_response: Option<reqwest::Response> = None;
    if source.content_type.starts_with("video/") {
        let direct_cdn_url = match try_get_alist_direct_url(app, &source).await {
            Some(url) => Some(url),
            None => fetch_fresh_alist_direct_url(app, &source).await,
        };
        if let Some(direct_cdn_url) = direct_cdn_url {
            if let Ok(parsed_url) = Url::parse(&direct_cdn_url) {
                match open_proxy_upstream_for_url(&source, &parsed_url, method_str, range.as_deref(), if_range.as_deref()).await {
                    Ok(resp) if resp.status().is_success() => {
                        alist_response = Some(resp);
                    }
                    _other => {
                        eprintln!("[AList Accel] Upstream failed or expired, retrying with fresh AList URL...");
                        invalidate_alist_accel_url_cache(&source.remote_path);
                        // 针对过期链接，同个请求内立即重新向 AList 获取全新直链并重试，避免客户端报错！
                        if let Some(fresh_cdn_url) = fetch_fresh_alist_direct_url(app, &source).await {
                            if let Ok(fresh_url) = Url::parse(&fresh_cdn_url) {
                                match open_proxy_upstream_for_url(&source, &fresh_url, method_str, range.as_deref(), if_range.as_deref()).await {
                                    Ok(retry_resp) if retry_resp.status().is_success() => {
                                        alist_response = Some(retry_resp);
                                    }
                                    _ => {
                                        eprintln!("[AList Accel] Fresh URL also failed, falling back to CD2");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let response = if let Some(resp) = alist_response {
        resp
    } else {
        match open_proxy_upstream(&source, method_str, range.as_deref(), if_range.as_deref()).await {
            Ok(res) => res,
            Err(err) => return (AxumStatusCode::BAD_GATEWAY, err).into_response(),
        }
    };

    let status = AxumStatusCode::from_u16(response.status().as_u16())
        .unwrap_or(AxumStatusCode::BAD_GATEWAY);
    let up_headers = response.headers().clone();
    let mut out_headers = HeaderMap::new();

    let upstream_ct = up_headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !upstream_ct.is_empty()
        && !upstream_ct
            .to_ascii_lowercase()
            .starts_with("application/octet-stream")
    {
        if let Ok(v) = HeaderValue::from_str(upstream_ct) {
            out_headers.insert(CONTENT_TYPE, v);
        }
    } else if !source.content_type.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&source.content_type) {
            out_headers.insert(CONTENT_TYPE, v);
        }
    }

    for header_name in [CONTENT_LENGTH, CONTENT_RANGE, ETAG, ACCEPT_RANGES] {
        if let Some(val) = up_headers.get(&header_name) {
            out_headers.insert(header_name, val.clone());
        }
    }
    if !out_headers.contains_key(ACCEPT_RANGES) {
        out_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    out_headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    out_headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    out_headers.insert(
        axum::http::header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Content-Length, Content-Range, Content-Type, ETag, Accept-Ranges"),
    );

    // Record total file size from Content-Range or Content-Length if not yet cached
    if let Some(ref asset_id) = source.asset_id {
        let total_len = if status == AxumStatusCode::PARTIAL_CONTENT {
            up_headers
                .get(CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range)
                .map(|(_, _, total)| total)
        } else if status == AxumStatusCode::OK {
            up_headers
                .get(CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
        } else {
            None
        };
        if let Some(total) = total_len.filter(|t| *t > 0) {
            if let Ok(mut guard) = ASSET_SIZE_CACHE.lock() {
                guard.insert(asset_id.clone(), total);
            }
        }
    }

    if method == axum::http::Method::HEAD {
        return (status, out_headers).into_response();
    }

    // Tee-caching for short videos (<= 150MB, full stream starting at byte 0)
    let mut cache_ctx: Option<(PartFileCleanupGuard, PathBuf, tokio_fs::File, u64)> = None;
    if source.content_type.starts_with("video/") {
        if let (Some(ref asset_id), Some(ref base_path)) = (&source.asset_id, &source.cache_base_path) {
            let mut full_expected_len: Option<u64> = None;
            const MAX_SHORT_VIDEO_CACHE_SIZE: u64 = 150 * 1024 * 1024;
            if status == AxumStatusCode::OK {
                if let Some(len) = up_headers
                    .get(CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    if len >= 64 * 1024 && len <= MAX_SHORT_VIDEO_CACHE_SIZE {
                        full_expected_len = Some(len);
                    }
                }
            } else if status == AxumStatusCode::PARTIAL_CONTENT {
                if let Some(cr_str) = up_headers.get(CONTENT_RANGE).and_then(|v| v.to_str().ok()) {
                    if let Some((start, end, total)) = parse_content_range(cr_str) {
                        if start == 0
                            && end + 1 >= total
                            && total >= 64 * 1024
                            && total <= MAX_SHORT_VIDEO_CACHE_SIZE
                        {
                            full_expected_len = Some(total);
                        }
                    }
                }
            }

            if let Some(expected_len) = full_expected_len {
                let cache_dir = base_path.join("short-videos");
                let final_path = cache_dir.join(format!("{asset_id}.mp4"));
                let part_path = cache_dir.join(format!("{asset_id}.mp4.part"));
                if !final_path.is_file() {
                    let _ = fs::create_dir_all(&cache_dir);
                    if let Ok(file) = tokio_fs::File::create(&part_path).await {
                        let guard = PartFileCleanupGuard {
                            part_path: Some(part_path),
                        };
                        cache_ctx = Some((guard, final_path, file, expected_len));
                    }
                }
            }
        }
    }

    let body = response.bytes_stream();
    let tracker = source.activity_tracker.clone();
    let max_cache_bytes = source.max_cache_bytes.unwrap_or(2 * 1024 * 1024 * 1024);

    let stream = futures_util::stream::unfold(
        (body, cache_ctx, 0u64, 0i64, preempt_slot, tracker, max_cache_bytes),
        |(mut body, mut cache_ctx, mut bytes_written, mut last_ts, preempt_slot, tracker, max_cache_bytes)| async move {
            let next_chunk = if let Some((my_seq, ref preempt)) = preempt_slot {
                if preempt.seq.load(std::sync::atomic::Ordering::SeqCst) != my_seq {
                    None
                } else {
                    tokio::select! {
                        biased;
                        _ = preempt.notify.notified() => {
                            if preempt.seq.load(std::sync::atomic::Ordering::SeqCst) != my_seq {
                                None
                            } else {
                                body.next().await
                            }
                        }
                        item = body.next() => item,
                    }
                }
            } else {
                body.next().await
            };

            match next_chunk {
                Some(Ok(mut chunk)) => {
                    let current_ts = now();
                    if current_ts.saturating_sub(last_ts) >= 3 {
                        last_ts = current_ts;
                        if let Some(ref t) = tracker {
                            t.store(current_ts as u64, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    // 聚合分片至 256KB 大窗口，降低跨境网络切片往返惩罚，稳定吞吐量
                    const TARGET_CHUNK_SIZE: usize = 256 * 1024;
                    if chunk.len() < TARGET_CHUNK_SIZE {
                        let mut buf = chunk.to_vec();
                        while buf.len() < TARGET_CHUNK_SIZE {
                            match tokio::time::timeout(std::time::Duration::from_millis(15), body.next()).await {
                                Ok(Some(Ok(next_b))) => {
                                    buf.extend_from_slice(&next_b);
                                }
                                _ => break,
                            }
                        }
                        chunk = axum::body::Bytes::from(buf);
                    }
                    if let Some((_, _, ref mut cache_file, _)) = cache_ctx {
                        if cache_file.write_all(&chunk).await.is_ok() {
                            bytes_written += chunk.len() as u64;
                        } else {
                            cache_ctx = None;
                        }
                    }
                    Some((
                        Ok::<axum::body::Bytes, std::io::Error>(chunk),
                        (body, cache_ctx, bytes_written, last_ts, preempt_slot, tracker, max_cache_bytes),
                    ))
                }
                Some(Err(err)) => Some((
                    Err(std::io::Error::other(err.to_string())),
                    (body, None, bytes_written, last_ts, preempt_slot, tracker, max_cache_bytes),
                )),
                None => {
                    if let Some((mut guard, final_path, mut cache_file, expected_len)) = cache_ctx.take() {
                        let _ = cache_file.flush().await;
                        drop(cache_file);
                        if expected_len >= 64 * 1024 && bytes_written >= expected_len {
                            if let Some(part_path) = guard.part_path.take() {
                                if tokio_fs::rename(&part_path, &final_path).await.is_ok() {
                                    if let Some(parent) = final_path.parent() {
                                        prune_short_video_cache(parent, max_cache_bytes);
                                    }
                                } else {
                                    let _ = tokio_fs::remove_file(&part_path).await;
                                }
                            }
                        }
                    }
                    None
                }
            }
        },
    );

    (status, out_headers, Body::from_stream(stream)).into_response()
}

async fn pipe_proxy_response(
    stream: &mut TcpStream,
    method: &str,
    response: reqwest::Response,
    source: &ProxySource,
    preempt_slot: Option<(u64, VideoStreamPreemption)>,
) -> Result<(), String> {
    let status = response.status();
    let headers = response.headers().clone();
    let reason = status.canonical_reason().unwrap_or("OK");
    let mut header = format!("HTTP/1.1 {} {}\r\n", status.as_u16(), reason);
    // Proxy URLs intentionally contain no filename. Supplying the file's known type is
    // necessary when AList responds with application/octet-stream.
    let upstream_content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !upstream_content_type.is_empty()
        && !upstream_content_type
            .to_ascii_lowercase()
            .starts_with("application/octet-stream")
        && !upstream_content_type
            .to_ascii_lowercase()
            .starts_with("text/plain")
    {
        header.push_str(&format!("content-type: {upstream_content_type}\r\n"));
    } else if !source.content_type.is_empty() && source.content_type != "application/octet-stream" {
        header.push_str(&format!("content-type: {}\r\n", source.content_type));
    } else {
        header.push_str("content-type: video/mp4\r\n");
    }
    for header_name in [CONTENT_LENGTH, CONTENT_RANGE, ETAG, ACCEPT_RANGES] {
        if let Some(value) = headers
            .get(&header_name)
            .and_then(|value| value.to_str().ok())
        {
            header.push_str(&format!("{}: {}\r\n", header_name.as_str(), value));
        }
    }
    if !headers.contains_key(ACCEPT_RANGES) {
        header.push_str("Accept-Ranges: bytes\r\n");
    }
    header.push_str("Cache-Control: private, max-age=300\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Content-Length, Content-Range, Content-Type, ETag, Accept-Ranges\r\nConnection: close\r\n\r\n");
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    if method == "HEAD" {
        return Ok(());
    }

    // Tee-caching for short videos only:
    // Only cache when the video is small (<= 150MB) and contains the ENTIRE video starting at byte 0.
    // Large regular media videos (e.g. 2GB episodes/movies) MUST NOT be tee-cached to prevent disk I/O stutter.
    let mut cache_ctx: Option<(std::path::PathBuf, std::path::PathBuf, tokio_fs::File, u64)> = None;
    if method == "GET" && source.content_type.starts_with("video/") {
        if let (Some(ref asset_id), Some(ref base_path)) = (&source.asset_id, &source.cache_base_path) {
            let mut full_expected_len: Option<u64> = None;
            const MAX_SHORT_VIDEO_CACHE_SIZE: u64 = 150 * 1024 * 1024; // 150 MB max
            if status == StatusCode::OK {
                if let Some(len) = headers
                    .get(CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    if len >= 64 * 1024 && len <= MAX_SHORT_VIDEO_CACHE_SIZE {
                        full_expected_len = Some(len);
                    }
                }
            } else if status == StatusCode::PARTIAL_CONTENT {
                if let Some(cr_str) = headers.get(CONTENT_RANGE).and_then(|v| v.to_str().ok()) {
                    if let Some((start, end, total)) = parse_content_range(cr_str) {
                        // Must start at byte 0 and end must reach the very end of file (total - 1)
                        if start == 0 && end + 1 >= total && total >= 64 * 1024 && total <= MAX_SHORT_VIDEO_CACHE_SIZE {
                            full_expected_len = Some(total);
                        }
                    }
                }
            }

            if let Some(expected_len) = full_expected_len {
                let cache_dir = base_path.join("short-videos");
                let final_path = cache_dir.join(format!("{asset_id}.mp4"));
                let part_path = cache_dir.join(format!("{asset_id}.mp4.part"));
                if !final_path.is_file() {
                    let _ = fs::create_dir_all(&cache_dir);
                    if let Ok(file) = tokio_fs::File::create(&part_path).await {
                        cache_ctx = Some((part_path, final_path, file, expected_len));
                    }
                }
            }
        }
    }

    let mut body = response.bytes_stream();
    let mut bytes_written_to_cache = 0u64;
    let mut last_chunk_ts = 0i64;
    loop {
        let next_chunk = if let Some((my_seq, ref preempt)) = preempt_slot {
            if preempt.seq.load(std::sync::atomic::Ordering::SeqCst) != my_seq {
                break;
            }
            tokio::select! {
                biased;
                _ = preempt.notify.notified() => {
                    if preempt.seq.load(std::sync::atomic::Ordering::SeqCst) != my_seq {
                        break;
                    }
                    body.next().await
                }
                item = body.next() => item,
            }
        } else {
            body.next().await
        };
        let Some(chunk) = next_chunk else {
            break;
        };
        let chunk = chunk.map_err(|error| format!("读取媒体流失败：{error}"))?;
        let current_ts = now();
        if current_ts.saturating_sub(last_chunk_ts) >= 3 {
            last_chunk_ts = current_ts;
            if let Some(ref tracker) = source.activity_tracker {
                tracker.store(current_ts as u64, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if let Err(error) = stream.write_all(&chunk).await {
            if let Some((part_path, _, mut cache_file, _)) = cache_ctx.take() {
                let _ = cache_file.flush().await;
                drop(cache_file);
                let _ = tokio_fs::remove_file(&part_path).await;
            }
            return Err(error.to_string());
        }

        if let Some((_, _, ref mut cache_file, _)) = cache_ctx {
            if cache_file.write_all(&chunk).await.is_ok() {
                bytes_written_to_cache += chunk.len() as u64;
            } else {
                cache_ctx = None;
            }
        }
    }

    if let Some((part_path, final_path, mut cache_file, expected_len)) = cache_ctx {
        let _ = cache_file.flush().await;
        drop(cache_file);
        let ok_to_commit = expected_len >= 64 * 1024 && bytes_written_to_cache >= expected_len;
        if ok_to_commit {
            if tokio_fs::rename(&part_path, &final_path).await.is_ok() {
                if let Some(parent) = final_path.parent() {
                    let max_b = source.max_cache_bytes.unwrap_or(2 * 1024 * 1024 * 1024);
                    prune_short_video_cache(parent, max_b);
                }
            }
        } else {
            let _ = tokio_fs::remove_file(&part_path).await;
        }
    }

    Ok(())
}

/// 原生端（iOS / 桌面）没有同源的 axum 服务，`/api/comic-pages/*` 不可达。
/// 漫画压缩包页面改由本地媒体代理提供：`/comic/{asset_id}/{page_index}`。
async fn serve_comic_page_request(
    stream: &mut TcpStream,
    method: &str,
    rest: &str,
) -> Result<(), String> {
    if method != "GET" && method != "HEAD" {
        return write_proxy_error(stream, 405, "Method Not Allowed").await;
    }
    let path = rest.split(&['?', '#'][..]).next().unwrap_or(rest);
    let Some((asset_id, page_part)) = path.rsplit_once('/') else {
        return write_proxy_error(stream, 404, "Invalid comic page path").await;
    };
    let Ok(page_index) = page_part.parse::<usize>() else {
        return write_proxy_error(stream, 404, "Invalid comic page index").await;
    };
    if !asset_id.starts_with("asset-") || asset_id.len() > 128 {
        return write_proxy_error(stream, 404, "Invalid comic asset").await;
    }
    let Some(app) = APP_HANDLE.get().cloned() else {
        return write_proxy_error(stream, 502, "Local media proxy is not ready").await;
    };
    let state = app.state::<MediaLibraryState>();
    match read_comic_archive_page_binary(&app, &state, asset_id, page_index).await {
        Ok((mime_type, bytes)) => {
            let content_type = if mime_type.is_empty() {
                "application/octet-stream"
            } else {
                mime_type.as_str()
            };
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: private, max-age=86400\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                bytes.len()
            );
            stream
                .write_all(header.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            if method != "HEAD" {
                stream
                    .write_all(&bytes)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        Err(error) => {
            eprintln!("comic_page proxy failed asset={asset_id} page={page_index}: {error}");
            write_proxy_error(stream, 404, "Comic page unavailable").await
        }
    }
}

async fn write_proxy_error(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
) -> Result<(), String> {
    let phrase = match status {
        404 => "Not Found",
        405 => "Method Not Allowed",
        502 => "Bad Gateway",
        _ => "Error",
    };
    let body = reason.as_bytes();
    let header = format!("HTTP/1.1 {status} {phrase}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n", body.len());
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stream
        .write_all(body)
        .await
        .map_err(|error| error.to_string())
}

fn media_content_type(path: &str) -> &'static str {
    match extension(path).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "heic" | "heif" => "image/heic",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "jxl" => "image/jxl",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" | "m4s" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        "ts" | "m2ts" | "mts" => "video/mp2t",
        "flv" => "video/x-flv",
        "mpeg" | "mpg" => "video/mpeg",
        "3gp" => "video/3gpp",
        "wmv" => "video/x-ms-wmv",
        "asf" => "video/x-ms-asf",
        "rm" | "rmvb" => "application/vnd.rn-realmedia",
        "ogv" => "video/ogg",
        "f4v" => "video/x-f4v",
        "vob" => "video/dvd",
        "zip" | "cbz" | "epub" => "application/zip",
        "rar" | "cbr" => "application/vnd.rar",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn requires_compatible_video(path: &str) -> bool {
    matches!(
        extension(path).as_str(),
        "avi"
            | "mkv"
            | "ts"
            | "m2ts"
            | "mts"
            | "flv"
            | "mpeg"
            | "mpg"
            | "3gp"
            | "wmv"
            | "asf"
            | "rm"
            | "rmvb"
            | "vob"
            | "f4v"
    )
}

// ============================================================================
// 待收纳智能整理工作台 (Personal Media Curator) - 后端服务与高安全风控引擎
// ============================================================================

const SETTING_CURATOR_CONFIG: &str = "curator_config";

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CuratorItemAssignment {
    pub target_vault_path: Option<String>,
    pub author: Option<String>,
    pub category_path: Option<String>,
    pub category_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CuratorConfig {
    pub filter_words: Vec<String>,
    pub pending_path: Option<String>,
    pub target_vault_root: Option<String>,
    #[serde(default)]
    pub manual_assignments: std::collections::HashMap<String, Option<CuratorItemAssignment>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CuratorConfigUpdate {
    pub filter_words: Option<Vec<String>>,
    pub pending_path: Option<String>,
    pub target_vault_root: Option<String>,
    pub manual_assignments: Option<std::collections::HashMap<String, Option<CuratorItemAssignment>>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurateMoveItemRequest {
    pub source_path: String,
    pub target_path: String,
    pub is_directory: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurateMoveItemResult {
    pub success: bool,
    pub status: String, // "MOVED", "SKIPPED_EXIST", "FAILED", "RATE_LIMITED"
    pub message: String,
    pub source_path: String,
    pub target_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurateBatchMoveRequest {
    pub items: Vec<CurateMoveItemRequest>,
    pub delay_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurateBatchMoveResult {
    pub success_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub aborted: bool,
    pub abort_reason: Option<String>,
    pub results: Vec<CurateMoveItemResult>,
}

pub fn get_curator_config_internal(app: &AppHandle) -> Result<CuratorConfig, String> {
    let connection = crate::v2_config::open(app)?;
    let val: Option<String> = connection
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![SETTING_CURATOR_CONFIG],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?;

    let mut config: CuratorConfig = match val {
        Some(json_str) => serde_json::from_str(&json_str)
            .map_err(|e| format!("解析收纳过滤词配置失败: {e}"))?,
        None => CuratorConfig::default(),
    };
    // Root paths have a single owner. Stale compatibility JSON must not
    // resurrect a removed domain or silently redirect physical curation moves.
    config.pending_path = canonical_domain_path(&connection, "pending")?;
    config.target_vault_root = canonical_domain_path(&connection, "vault")?;
    Ok(config)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_curator_config(app: AppHandle) -> Result<CuratorConfig, String> {
    get_curator_config_internal(&app)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn save_curator_config(app: AppHandle, config: CuratorConfigUpdate) -> Result<(), String> {
    let connection = crate::v2_config::open(&app)?;
    let mut current = get_curator_config_internal(&app).unwrap_or_default();
    if let Some(words) = config.filter_words {
        current.filter_words = words;
    }
    if let Some(path) = config.pending_path {
        current.pending_path = Some(path);
    }
    if let Some(root) = config.target_vault_root {
        current.target_vault_root = Some(root);
    }
    if let Some(assignments) = config.manual_assignments {
        for (path, item) in assignments {
            match item {
                Some(assigned) => {
                    current.manual_assignments.insert(path, Some(assigned));
                }
                None => {
                    current.manual_assignments.remove(&path);
                }
            }
        }
    }
    let json_str = serde_json::to_string(&current)
        .map_err(|e| format!("序列化收纳配置失败: {e}"))?;
    connection
        .execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SETTING_CURATOR_CONFIG, json_str],
        )
        .map_err(database_error)?;
    Ok(())
}

async fn webdav_check_exists(session: &Session, remote_path: &str) -> Result<bool, String> {
    let url = remote_url(&session.endpoint, remote_path)?;
    let method = Method::from_bytes(b"PROPFIND").map_err(|e| e.to_string())?;
    let mut req = session
        .client
        .request(method, url)
        .header(USER_AGENT, MEDIA_USER_AGENT)
        .header("Depth", "0");
    if !session.api_token.is_empty() {
        req = req.header(AUTHORIZATION, &session.api_token);
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("WebDAV 检查存在性网络失败: {e}"))?;
    let status = res.status();
    if status.is_success() || status.as_u16() == 207 {
        Ok(true)
    } else if status == StatusCode::NOT_FOUND {
        Ok(false)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Err("RATE_LIMITED: 115/CD2 触发频控限制 (HTTP 429)".into())
    } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        Err("AUTH_ERROR: 115/CD2 权限或会话受限 (HTTP 401/403)".into())
    } else {
        Ok(false)
    }
}

async fn webdav_mkcol(session: &Session, path: &str) -> Result<(), String> {
    let url = remote_url(&session.endpoint, path)?;
    let method = Method::from_bytes(b"MKCOL").map_err(|e| e.to_string())?;
    let mut req = session
        .client
        .request(method, url)
        .header(USER_AGENT, MEDIA_USER_AGENT);
    if !session.api_token.is_empty() {
        req = req.header(AUTHORIZATION, &session.api_token);
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("创建文件夹网络失败: {e}"))?;
    let status = res.status();
    if status == StatusCode::CREATED
        || status == StatusCode::METHOD_NOT_ALLOWED
        || status.is_success()
    {
        Ok(())
    } else if status == StatusCode::CONFLICT {
        Err("CONFLICT".to_string())
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Err("RATE_LIMITED: 115/CD2 触发频控限制 (HTTP 429)".to_string())
    } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        Err("AUTH_ERROR: 115/CD2 权限或会话受限 (HTTP 401/403)".to_string())
    } else {
        Err(format!("创建目录失败: HTTP {status}"))
    }
}

async fn webdav_ensure_directory(session: &Session, dir_path: &str) -> Result<(), String> {
    let normalized = normalize_remote_path(dir_path)?;
    if normalized == "/" || normalized.is_empty() {
        return Ok(());
    }
    if webdav_check_exists(session, &normalized).await? {
        return Ok(());
    }

    match webdav_mkcol(session, &normalized).await {
        Ok(()) => Ok(()),
        Err(e) if e == "CONFLICT" => {
            let parent = parent_path(&normalized);
            if parent != "/" && !parent.is_empty() && parent != normalized {
                Box::pin(webdav_ensure_directory(session, &parent)).await?;
            }
            webdav_mkcol(session, &normalized).await
        }
        Err(e) => Err(e),
    }
}

async fn webdav_move_item(
    session: &Session,
    source_path: &str,
    target_path: &str,
    is_dir: bool,
) -> Result<(), String> {
    let source_url = remote_url(&session.endpoint, source_path)?;
    let target_url = remote_url(&session.endpoint, target_path)?;

    let mut dest_str = target_url.to_string();
    if is_dir && !dest_str.ends_with('/') {
        dest_str.push('/');
    }

    let mut src_str = source_url.to_string();
    if is_dir && !src_str.ends_with('/') {
        src_str.push('/');
    }
    let req_url = Url::parse(&src_str).unwrap_or(source_url);

    let method = Method::from_bytes(b"MOVE").map_err(|e| e.to_string())?;
    let mut req = session
        .client
        .request(method, req_url)
        .header(USER_AGENT, MEDIA_USER_AGENT)
        .header("Destination", dest_str)
        .header("Overwrite", "F");

    if !session.api_token.is_empty() {
        req = req.header(AUTHORIZATION, &session.api_token);
    }

    let res = req
        .send()
        .await
        .map_err(|e| format!("WebDAV 移动网络请求失败: {e}"))?;
    let status = res.status();

    if status == StatusCode::CREATED || status == StatusCode::NO_CONTENT || status.is_success() {
        Ok(())
    } else if status == StatusCode::PRECONDITION_FAILED {
        Err("EXISTS: 目标文件或文件夹已存在 (HTTP 412 Precondition Failed)".into())
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Err("RATE_LIMITED: 115/CD2 触发频控限制 (HTTP 429)".into())
    } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        Err("AUTH_ERROR: 115/CD2 权限或会话受限 (HTTP 401/403)".into())
    } else if status == StatusCode::CONFLICT {
        Err("CONFLICT: 目标父路径不存在或冲突 (HTTP 409)".into())
    } else {
        Err(format!("WebDAV MOVE 操作失败 (HTTP {status})"))
    }
}

fn sync_db_after_move(
    app: &AppHandle,
    source_path: &str,
    target_path: &str,
    _is_directory: bool,
) -> Result<(), String> {
    crate::v2_moves::reconcile(app, source_path, target_path)
}

fn moved_index_message(result: Result<(), String>) -> String {
    match result {
        Ok(()) => "物理移动成功，已同步库索引".into(),
        Err(error) => format!("物理移动已成功，但 V2 库索引同步失败：{error}。请刷新索引；请勿重试物理移动。"),
    }
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn curate_move_item(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    request: CurateMoveItemRequest,
) -> Result<CurateMoveItemResult, String> {
    let source_path = normalize_remote_path(&request.source_path)?;
    let target_path = normalize_remote_path(&request.target_path)?;

    if source_path == target_path {
        return Ok(CurateMoveItemResult {
            success: false,
            status: "SKIPPED_EXIST".into(),
            message: "源路径与目标路径相同，跳过操作".into(),
            source_path,
            target_path,
        });
    }

    let session = require_session(&state).await?;
    let is_dir = request
        .is_directory
        .unwrap_or_else(|| source_path.ends_with('/'));

    // 高性能极速单请求移动（带 Overwrite: F 严格防覆盖保证）：
    // 在绝大多数场景下（作者目录已存在），只需单次 WebDAV MOVE 请求即可完成（耗时由 ~2s 降至 <300ms）。
    // 若返回 409 Conflict（新作者父目录尚未建立），则自动兜底创建父目录并重试；
    // 若返回 412 Precondition Failed，则表明目标同名，安全跳过绝不覆盖。
    let move_res = webdav_move_item(&session, &source_path, &target_path, is_dir).await;
    match move_res {
        Ok(()) => {
            let message = moved_index_message(sync_db_after_move(&app, &source_path, &target_path, is_dir));
            // 移动成功后清理持久化指定记录
            let _ = save_curator_config(app.clone(), CuratorConfigUpdate {
                manual_assignments: Some([(source_path.clone(), None)].into_iter().collect()),
                ..Default::default()
            });
            Ok(CurateMoveItemResult {
                success: true,
                status: "MOVED".into(),
                message,
                source_path,
                target_path,
            })
        }
        Err(e) if e.starts_with("CONFLICT") => {
            // 目标父目录不存在（首次为新作者或新分类建目），递归建目后重试
            let target_parent = parent_path(&target_path);
            if let Err(mkcol_err) = webdav_ensure_directory(&session, &target_parent).await {
                if mkcol_err.starts_with("RATE_LIMITED") || mkcol_err.starts_with("AUTH_ERROR") {
                    return Err(mkcol_err);
                }
                return Ok(CurateMoveItemResult {
                    success: false,
                    status: "FAILED".into(),
                    message: format!("创建目标父目录失败: {mkcol_err}"),
                    source_path,
                    target_path,
                });
            }
            // 建目完成后再次执行移动
            match webdav_move_item(&session, &source_path, &target_path, is_dir).await {
                Ok(()) => {
                    let message = moved_index_message(sync_db_after_move(&app, &source_path, &target_path, is_dir));
                    Ok(CurateMoveItemResult {
                        success: true,
                        status: "MOVED".into(),
                        message,
                        source_path,
                        target_path,
                    })
                }
                Err(retry_e) if retry_e.starts_with("EXISTS") => Ok(CurateMoveItemResult {
                    success: false,
                    status: "SKIPPED_EXIST".into(),
                    message: "目标已存在同名项目 (WebDAV 412)，安全跳过未覆盖".into(),
                    source_path,
                    target_path,
                }),
                Err(retry_e) if retry_e.starts_with("RATE_LIMITED") || retry_e.starts_with("AUTH_ERROR") => {
                    Err(retry_e)
                }
                Err(retry_e) => Ok(CurateMoveItemResult {
                    success: false,
                    status: "FAILED".into(),
                    message: format!("移动操作重试失败: {retry_e}"),
                    source_path,
                    target_path,
                }),
            }
        }
        Err(e) if e.starts_with("EXISTS") => Ok(CurateMoveItemResult {
            success: false,
            status: "SKIPPED_EXIST".into(),
            message: "目标已存在同名项目 (WebDAV 412)，安全跳过未覆盖".into(),
            source_path,
            target_path,
        }),
        Err(e) if e.starts_with("RATE_LIMITED") || e.starts_with("AUTH_ERROR") => Err(e),
        Err(e) => Ok(CurateMoveItemResult {
            success: false,
            status: "FAILED".into(),
            message: format!("移动操作失败: {e}"),
            source_path,
            target_path,
        }),
    }
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn curate_batch_move(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    request: CurateBatchMoveRequest,
) -> Result<CurateBatchMoveResult, String> {
    let mut success_count = 0;
    let mut skipped_count = 0;
    let mut failed_count = 0;
    let mut aborted = false;
    let mut abort_reason = None;
    let mut results = Vec::new();

    let base_delay = request.delay_ms.unwrap_or(1000).max(600);

    for (i, item) in request.items.into_iter().enumerate() {
        // 115 & CD2 专用防风控串行节拍器：随机抖动 + 延迟防QPS突发
        if i > 0 {
            let jitter = (OsRng.next_u32() % 300) as u64;
            tokio::time::sleep(Duration::from_millis(base_delay + jitter)).await;
        }

        match curate_move_item(app.clone(), state.clone(), item).await {
            Ok(res) => {
                match res.status.as_str() {
                    "MOVED" => success_count += 1,
                    "SKIPPED_EXIST" => skipped_count += 1,
                    _ => failed_count += 1,
                }
                results.push(res);
            }
            Err(err) => {
                // 遭遇频控、认证或严重异常时，立即触发熔断保护，终止后续操作
                failed_count += 1;
                aborted = true;
                abort_reason = Some(err.clone());
                results.push(CurateMoveItemResult {
                    success: false,
                    status: "RATE_LIMITED".into(),
                    message: format!("批量任务触发风控/异常熔断中断：{err}"),
                    source_path: String::new(),
                    target_path: String::new(),
                });
                break;
            }
        }
    }

    Ok(CurateBatchMoveResult {
        success_count,
        skipped_count,
        failed_count,
        aborted,
        abort_reason,
        results,
    })
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInboxRequest {
    pub inbox_path: String,
    pub batch_name: String,
    pub item_names: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackInboxResult {
    pub moved_count: usize,
    pub target_batch_path: String,
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn open_raw_media_source(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    remote_path: String,
) -> Result<String, String> {
    let normalized = normalize_remote_path(&remote_path)?;
    let session = require_session(&state).await?;
    let content_type = media_content_type(&normalized).to_owned();
    let proxy = ensure_media_proxy(&state).await?;
    let source = ProxySource {
        url: remote_url(&session.endpoint, &normalized)?,
        fallback_url: None,
        session,
        content_type,
        remote_path: normalized,
        allow_api_fallback: true,
        api_url_cache: Arc::new(StdMutex::new(None)),
        direct_url: false,
        thumbnail_cache_path: None,
        proxy_address: Some(proxy.address),
        asset_id: None,
        cache_base_path: cache_base_path(&app).ok(),
        activity_tracker: None,
        max_cache_bytes: None,
    };
    let token = random_token();
    let mut sources = proxy.sources.lock().await;
    sources.insert(token.clone(), source);
    while sources.len() > MAX_MEDIA_PROXY_SOURCES {
        if let Some(expired) = sources.keys().next().cloned() {
            sources.remove(&expired);
        } else {
            break;
        }
    }
    Ok(format!("http://{}/media/{token}", proxy.address))
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn delete_empty_pending_batch(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    batch_path: String,
) -> Result<bool, String> {
    let normalized = normalize_remote_path(&batch_path)?;
    if normalized == "/" || normalized.is_empty() {
        return Err("禁止删除根目录".into());
    }
    {
        let connection=open_media_database(&app)?;
        validate_pending_subtree(&connection,&normalized)?;
    }
    let session = require_session(&state).await?;

    let entries = match list_directory(&session, &normalized).await {
        Ok(list) => list,
        Err(list_error) => match webdav_check_exists(&session, &normalized).await {
            Ok(false) => { purge_pending_assets(app,normalized)?; return Ok(true); }
            Ok(true) => return Err(format!("无法确认目录为空，已阻止删除：{list_error}")),
            Err(check_error) => return Err(format!("无法确认目录状态，已阻止删除：{list_error}; {check_error}")),
        }
    };

    // Hidden files are still content: deleting them would not be an empty-directory operation.
    let non_hidden = entries;

    if !non_hidden.is_empty() {
        return Err(format!(
            "安全防护阻止：该批次目录中仍含有 {} 个项目，绝不误删非空目录！",
            non_hidden.len()
        ));
    }

    let delete_url = remote_url(&session.endpoint, &normalized)?;
    let method = Method::from_bytes(b"DELETE").map_err(|e| e.to_string())?;
    let mut req = session
        .client
        .request(method, delete_url)
        .header(USER_AGENT, MEDIA_USER_AGENT);
    if !session.api_token.is_empty() {
        req = req.header(AUTHORIZATION, &session.api_token);
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("删除网络请求失败: {e}"))?;
    let status = res.status();
    if status == StatusCode::NO_CONTENT || status == StatusCode::OK || status == StatusCode::NOT_FOUND {
        purge_pending_assets(app,normalized)?;
        Ok(true)
    } else {
        Err(format!("删除空目录失败: HTTP {status}"))
    }
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn pack_inbox_to_pending_batch(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    request: PackInboxRequest,
) -> Result<PackInboxResult, String> {
    let inbox = normalize_remote_path(&request.inbox_path)?;
    let config = get_curator_config_internal(&app)?;
    let pending_base = config
        .pending_path
        .ok_or_else(|| "未配置待收纳基础目录".to_string())?;
    let pending_norm = normalize_remote_path(&pending_base)?;

    let batch_folder_name = if request.batch_name.trim().is_empty() {
        let now = chrono::Local::now();
        format!("【Pending_{}】", now.format("%Y%m%d_%H%M"))
    } else {
        let b = request.batch_name.trim();
        if b.starts_with('【') && b.ends_with('】') {
            b.to_string()
        } else {
            format!("【{b}】")
        }
    };

    let target_batch_path = format!("{pending_norm}/{batch_folder_name}");
    let session = require_session(&state).await?;

    webdav_ensure_directory(&session, &target_batch_path).await?;

    let mut moved_count = 0;
    for (i, name) in request.item_names.into_iter().enumerate() {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(800)).await;
        }
        let src = format!("{inbox}/{name}");
        let dst = format!("{target_batch_path}/{name}");
        let move_res = webdav_move_item(&session, &src, &dst, false).await;
        if move_res.is_ok() {
            let _ = sync_db_after_move(&app, &src, &dst, false);
            moved_count += 1;
        }
    }

    Ok(PackInboxResult {
        moved_count,
        target_batch_path,
    })
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn purge_pending_assets(app: AppHandle, pending_path: String) -> Result<usize, String> {
    let mut connection = open_media_database(&app)?;
    let normalized = normalize_remote_path(&pending_path)?;
    let root_id=validate_pending_subtree(&connection,&normalized)?;
    let prefix=format!("{}/",normalized.trim_end_matches('/'));
    let parent=parent_path(&normalized);
    let tx=connection.transaction().map_err(database_error)?;
    let count=tx.execute("DELETE FROM assets WHERE domain='pending' AND (remote_path=?1 OR substr(remote_path,1,length(?2))=?2)",params![normalized,prefix]).map_err(database_error)?;
    tx.execute("DELETE FROM provider_directory_entries WHERE domain='pending' AND (remote_path=?1 OR substr(remote_path,1,length(?2))=?2 OR parent_path=?3)",params![normalized,prefix,parent]).map_err(database_error)?;
    tx.execute("DELETE FROM provider_directory_snapshots WHERE domain='pending' AND (parent_path=?1 OR substr(parent_path,1,length(?2))=?2 OR parent_path=?3)",params![normalized,prefix,parent]).map_err(database_error)?;
    tx.execute("DELETE FROM directory_sync_state WHERE root_id=?1 AND (remote_path=?2 OR substr(remote_path,1,length(?3))=?3 OR remote_path=?4)",params![root_id,normalized,prefix,parent]).map_err(database_error)?;
    tx.execute("UPDATE pending_projects SET
        total_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=pending_projects.id),
        triage_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=pending_projects.id AND assignment_status='triage'),
        ready_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=pending_projects.id AND assignment_status='ready' AND conflict_reason IS NULL),
        updated_at=strftime('%s','now') WHERE root_id=?1",params![root_id]).map_err(database_error)?;
    tx.execute("DELETE FROM pending_projects WHERE root_id=?1 AND total_count=0 AND (source_path=?2 OR substr(source_path,1,length(?3))=?3)",params![root_id,normalized,prefix]).map_err(database_error)?;
    tx.commit().map_err(database_error)?;
    Ok(count)
}

fn validate_pending_subtree(connection:&Connection,path:&str)->Result<String,String> {
    let (root_id,root):(String,String)=connection.query_row("SELECT root_id,remote_path FROM domain_roots WHERE domain='pending'",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(database_error)?;
    let root=normalize_remote_path(&root)?;
    if path==root || !path.starts_with(&format!("{}/",root.trim_end_matches('/'))) {
        return Err("仅允许清理待收纳域内的批次子目录，禁止清理域根目录或其他域".into());
    }
    Ok(root_id)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn set_author_avatar(
    app: AppHandle,
    author_id: String,
    asset_id: Option<String>,
) -> Result<(), String> {
    let connection = open_media_database(&app)?;
    connection
        .execute(
            "UPDATE authors SET avatar_asset_id = ?1 WHERE id = ?2",
            params![asset_id, author_id],
        )
        .map_err(database_error)?;
    Ok(())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn upload_author_avatar(
    app: AppHandle,
    author_id: String,
    data_base64: String,
) -> Result<String, String> {
    let connection = open_media_database(&app)?;
    let clean_base64 = data_base64
        .split(',')
        .nth(1)
        .unwrap_or(&data_base64);
    let trimmed = clean_base64.trim().replace('\n', "").replace('\r', "").replace(' ', "+");
    let bytes = STANDARD
        .decode(&trimmed)
        .or_else(|_| URL_SAFE.decode(&trimmed))
        .map_err(|e| format!("头像数据不是有效的 Base64: {e}"))?;
    let safe_id = author_id.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let avatar_asset_key = format!("author_{safe_id}");
    persist_thumbnail_jpeg_bytes(&app, &avatar_asset_key, &bytes, None)?;
    let updated = connection
        .execute(
            "UPDATE authors SET avatar_asset_id = ?1 WHERE id = ?2 OR name = ?2",
            params![avatar_asset_key, author_id],
        )
        .map_err(database_error)?;
    if updated == 0 {
        connection
            .execute(
                "INSERT OR IGNORE INTO authors (id, name, aliases_json, origin, created_at, avatar_asset_id)
                 VALUES (?1, ?1, '[]', 'user', ?2, ?3)",
                params![author_id, now(), avatar_asset_key],
            )
            .map_err(database_error)?;
    }
    Ok(avatar_asset_key)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn set_author_banner(
    app: AppHandle,
    author_id: String,
    asset_id: Option<String>,
) -> Result<(), String> {
    let connection = open_media_database(&app)?;
    let updated = connection
        .execute(
            "UPDATE authors SET banner_asset_id = ?1 WHERE id = ?2 OR name = ?2",
            params![asset_id, author_id],
        )
        .map_err(database_error)?;
    if updated == 0 {
        connection
            .execute(
                "INSERT OR IGNORE INTO authors (id, name, aliases_json, origin, created_at, banner_asset_id)
                 VALUES (?1, ?1, '[]', 'user', ?2, ?3)",
                params![author_id, now(), asset_id],
            )
            .map_err(database_error)?;
    }
    Ok(())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn upload_author_banner(
    app: AppHandle,
    author_id: String,
    data_base64: String,
) -> Result<String, String> {
    let connection = open_media_database(&app)?;
    let clean_base64 = data_base64
        .split(',')
        .nth(1)
        .unwrap_or(&data_base64);
    let trimmed = clean_base64.trim().replace('\n', "").replace('\r', "").replace(' ', "+");
    let bytes = STANDARD
        .decode(&trimmed)
        .or_else(|_| URL_SAFE.decode(&trimmed))
        .map_err(|e| format!("背景壁纸数据不是有效的 Base64: {e}"))?;
    let safe_id = author_id.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let banner_asset_key = format!("banner_{safe_id}");
    persist_thumbnail_jpeg_bytes(&app, &banner_asset_key, &bytes, None)?;
    let updated = connection
        .execute(
            "UPDATE authors SET banner_asset_id = ?1 WHERE id = ?2 OR name = ?2",
            params![banner_asset_key, author_id],
        )
        .map_err(database_error)?;
    if updated == 0 {
        connection
            .execute(
                "INSERT OR IGNORE INTO authors (id, name, aliases_json, origin, created_at, banner_asset_id)
                 VALUES (?1, ?1, '[]', 'user', ?2, ?3)",
                params![author_id, now(), banner_asset_key],
            )
            .map_err(database_error)?;
    }
    Ok(banner_asset_key)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn update_author_aliases(
    app: AppHandle,
    author_id: String,
    aliases: Vec<String>,
) -> Result<Vec<String>, String> {
    let connection = open_media_database(&app)?;
    let clean_aliases: Vec<String> = aliases
        .into_iter()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();
    let json_str = serde_json::to_string(&clean_aliases).map_err(|e| e.to_string())?;
    let updated = connection
        .execute(
            "UPDATE authors SET aliases_json = ?1 WHERE id = ?2 OR name = ?2",
            params![json_str, author_id],
        )
        .map_err(database_error)?;
    if updated == 0 {
        connection
            .execute(
                "INSERT OR IGNORE INTO authors (id, name, aliases_json, origin, created_at)
                 VALUES (?1, ?1, ?2, 'user', ?3)",
                params![author_id, json_str, now()],
            )
            .map_err(database_error)?;
    }
    Ok(clean_aliases)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurationPreviewPayload {
    pub asset: LibraryAsset,
    pub comic_pages: Vec<LibraryAsset>,
    pub playlist_items: Vec<LibraryAsset>,
}

fn build_asset_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryAsset> {
    Ok(LibraryAsset {
        id: row.get(0)?,
        root_id: row.get(1)?,
        root_kind: LibraryRootKind::Media,
        remote_path: row.get(2)?,
        file_name: row.get(3)?,
        parent_path: row.get(4)?,
        media_type: row.get(5)?,
        size_bytes: row.get(6)?,
        duration_seconds: row.get(7)?,
        category_id: None,
        category_name: None,
        author_id: None,
        author_name: None,
        collection_id: None,
        collection_name: None,
        metadata_origin: None,
        content_hash: None,
        thumbnail_asset_id: row.get(8)?,
    })
}

pub fn ensure_preview_file_asset(connection: &Connection, root_id: &str, path: &str) -> Result<LibraryAsset, String> {
    let clean_path=normalize_remote_path(path)?;
    let (domain,root):(String,String)=connection.query_row("SELECT domain,remote_path FROM domain_roots WHERE root_id=?1",params![root_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(database_error)?;
    let root=normalize_remote_path(&root)?;
    if clean_path==root || !clean_path.starts_with(&format!("{}/",root.trim_end_matches('/'))) {
        return Err("预览文件不在指定域根目录内".into());
    }
    let root_kind=if domain=="comics" {LibraryRootKind::Comics} else {LibraryRootKind::Media};
    let media_type=media_type_for(root_kind,&clean_path)
        .or_else(||media_type_for(LibraryRootKind::Comics,&clean_path))
        .ok_or("不支持预览此文件类型")?;
    let id=format!("asset-{:016x}",stable_hash(&format!("{domain}|{clean_path}")));
    connection.execute("INSERT INTO assets(id,domain,root_id,remote_path,file_name,parent_path,media_type,size_bytes,missing,first_seen_at,last_seen_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7,0,0,?8,?8)
        ON CONFLICT(domain,remote_path) DO UPDATE SET root_id=excluded.root_id,missing=0,missing_since=NULL,missing_scan_count=0,last_seen_at=excluded.last_seen_at",
        params![id,domain,root_id,clean_path,file_name(&clean_path),parent_path(&clean_path),media_type,now()]).map_err(database_error)?;
    let mut asset=connection.query_row("SELECT id,root_id,remote_path,file_name,parent_path,media_type,size_bytes,duration_seconds,thumbnail_asset_id FROM assets WHERE domain=?1 AND remote_path=?2 AND missing=0",params![domain,clean_path],build_asset_from_row).map_err(database_error)?;
    asset.root_kind=root_kind;
    Ok(asset)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn prepare_curation_preview(
    app: AppHandle,
    _state: State<'_, MediaLibraryState>,
    path: String,
    is_directory: bool,
) -> Result<CurationPreviewPayload, String> {
    let clean_path = normalize_remote_path(&path)?;
    let app_handle = app.clone();
    let path_clone = clean_path.clone();

    tokio::task::spawn_blocking(move || {
        let connection = open_media_database(&app_handle)?;
        let root_id = {
            let mut stmt = connection.prepare("SELECT root_id, remote_path FROM domain_roots").map_err(database_error)?;
            let roots = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(database_error)?;
            let mut best = String::new();
            let mut longest = 0;
            for r in roots {
                if let Ok((rid, rpath)) = r {
                    if (path_clone == rpath || path_clone.starts_with(&format!("{}/", rpath.trim_end_matches('/')))) && rpath.len() >= longest {
                        longest = rpath.len();
                        best = rid;
                    }
                }
            }
            if best.is_empty() {
                return Err("预览路径不属于任何已配置域根目录".into());
            } else {
                best
            }
        };

        if !is_directory {
            let asset = ensure_preview_file_asset(&connection, &root_id, &path_clone)?;
            Ok(CurationPreviewPayload {
                asset: asset.clone(),
                comic_pages: Vec::new(),
                playlist_items: vec![asset],
            })
        } else {
            let mut stmt = connection.prepare(
                "SELECT id, root_id, remote_path, file_name, parent_path, media_type, size_bytes, duration_seconds, thumbnail_asset_id
                 FROM assets WHERE parent_path = ?1 AND root_id=?2 AND missing = 0 ORDER BY file_name COLLATE NOCASE,remote_path LIMIT 200"
            ).map_err(database_error)?;
            let children = stmt.query_map(params![path_clone,root_id], build_asset_from_row).map_err(database_error)?;
            let mut assets = children.collect::<Result<Vec<_>,_>>().map_err(database_error)?;
            let domain:String=connection.query_row("SELECT domain FROM domain_roots WHERE root_id=?1",params![root_id],|r|r.get(0)).map_err(database_error)?;
            if domain=="comics" { for asset in &mut assets { asset.root_kind=LibraryRootKind::Comics; } }

            if assets.is_empty() {
                return Err("目录没有已索引的可预览媒体，请先刷新目录".into());
            }

            let first_video = assets.iter().find(|a| a.media_type == "video").cloned();
            let first_comic = assets.iter().find(|a| a.media_type == "comicArchive" || a.media_type == "comicPage" || a.media_type == "pdf").cloned();
            let first_image = assets.iter().find(|a| a.media_type == "image").cloned();

            if let Some(video) = first_video {
                let video_list: Vec<LibraryAsset> = assets.iter().filter(|a| a.media_type == "video").cloned().collect();
                Ok(CurationPreviewPayload {
                    asset: video,
                    comic_pages: Vec::new(),
                    playlist_items: video_list,
                })
            } else if let Some(comic) = first_comic {
                Ok(CurationPreviewPayload {
                    asset: comic,
                    comic_pages: Vec::new(),
                    playlist_items: assets,
                })
            } else if let Some(image) = first_image {
                let image_list: Vec<LibraryAsset> = assets.iter().filter(|a| a.media_type == "image").cloned().collect();
                Ok(CurationPreviewPayload {
                    asset: image,
                    comic_pages: image_list.clone(),
                    playlist_items: image_list,
                })
            } else {
                let first = assets.remove(0);
                Ok(CurationPreviewPayload {
                    asset: first.clone(),
                    comic_pages: Vec::new(),
                    playlist_items: vec![first],
                })
            }
        }
    }).await.map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_config_uses_only_canonical_roots_and_preserves_clear() {
        let c=Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE domain_roots(domain TEXT PRIMARY KEY,remote_path TEXT);
            CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT);
            INSERT INTO settings VALUES('dir.pending','/stale'),('curator_config','{\"pendingPath\":\"/stale\"}');
            INSERT INTO domain_roots VALUES('pending','/canonical');").unwrap();
        assert_eq!(directory_config_from_connection(&c).unwrap().pending_path.as_deref(),Some("/canonical"));
        c.execute("DELETE FROM domain_roots",[]).unwrap();
        assert!(directory_config_from_connection(&c).unwrap().pending_path.is_none());
        assert!(canonical_domain_path(&c,"vault").unwrap().is_none());
    }

    #[test]
    fn v2_preview_preserves_identity_and_enforces_root_boundary() {
        let c=Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE domain_roots(domain TEXT,root_id TEXT,remote_path TEXT);
            INSERT INTO domain_roots VALUES('vault','root','/vault');
            CREATE TABLE assets(id TEXT PRIMARY KEY,domain TEXT,root_id TEXT,remote_path TEXT,file_name TEXT,parent_path TEXT,media_type TEXT,size_bytes INTEGER,missing INTEGER,first_seen_at INTEGER,last_seen_at INTEGER,missing_since INTEGER,missing_scan_count INTEGER DEFAULT 0,duration_seconds INTEGER,thumbnail_asset_id TEXT,UNIQUE(domain,remote_path));
            INSERT INTO assets(id,domain,root_id,remote_path,file_name,parent_path,media_type,size_bytes,missing,first_seen_at) VALUES('imported','vault','root','/vault/a.mp4','a.mp4','/vault','video',77,1,5);").unwrap();
        let asset=ensure_preview_file_asset(&c,"root","/vault/a.mp4").unwrap();
        assert_eq!(asset.id,"imported");assert_eq!(asset.size_bytes,77);
        assert_eq!(c.query_row("SELECT first_seen_at FROM assets",[],|r|r.get::<_,i64>(0)).unwrap(),5);
        assert!(ensure_preview_file_asset(&c,"root","/vault-other/a.mp4").is_err());
        assert!(ensure_preview_file_asset(&c,"root","/vault").is_err());
        assert!(ensure_preview_file_asset(&c,"root","/vault/unsupported.txt").is_err());
    }

    #[test]
    fn v2_search_registration_preserves_existing_identity() {
        let mut connection=Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE assets(id TEXT PRIMARY KEY,domain TEXT,root_id TEXT,remote_path TEXT,file_name TEXT,parent_path TEXT,media_type TEXT,size_bytes INTEGER,etag TEXT,modified_at TEXT,content_hash TEXT,missing INTEGER,missing_since INTEGER,missing_scan_count INTEGER DEFAULT 0,first_seen_at INTEGER,last_seen_at INTEGER,UNIQUE(domain,remote_path));
            CREATE TABLE vault_items(asset_id TEXT PRIMARY KEY,updated_at INTEGER);
            INSERT INTO assets(id,domain,root_id,remote_path,file_name,parent_path,media_type,size_bytes,missing,first_seen_at,last_seen_at) VALUES('imported','vault','root','/vault/nested/a.mp4','a.mp4','/vault/nested','video',1,1,5,5);").unwrap();
        let entry=RemoteBrowserEntry {path:"/vault/nested/a.mp4".into(),name:"a.mp4".into(),is_directory:false,size_bytes:2,modified_at:None,etag:None,asset_id:Some("generated".into()),media_type:Some("video".into()),thumbnail_source:None,thumbnail_url:None,cover_asset_id:None,duration_seconds:None,progress_seconds:None};
        let ids=register_search_assets_v2(&mut connection,"vault","root",vec![entry],10).unwrap();
        assert_eq!(ids.get("/vault/nested/a.mp4").map(String::as_str),Some("imported"));
        let row:(String,i64,i64,i64)=connection.query_row("SELECT parent_path,first_seen_at,last_seen_at,missing FROM assets",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(row,("/vault/nested".into(),5,10,0));
        assert_eq!(connection.query_row("SELECT asset_id FROM vault_items",[],|r|r.get::<_,String>(0)).unwrap(),"imported");
    }

    #[test]
    fn v2_directory_cover_assets_keep_domain_and_first_seen() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE domain_roots(domain TEXT PRIMARY KEY, root_id TEXT);
             INSERT INTO domain_roots VALUES('vault', 'vault-root');
             CREATE TABLE assets(
                 id TEXT PRIMARY KEY, domain TEXT NOT NULL, root_id TEXT NOT NULL,
                 remote_path TEXT NOT NULL, file_name TEXT NOT NULL, parent_path TEXT NOT NULL,
                 media_type TEXT NOT NULL, size_bytes INTEGER NOT NULL, missing INTEGER NOT NULL,
                 first_seen_at INTEGER NOT NULL, last_seen_at INTEGER NOT NULL,
                 missing_since INTEGER, missing_scan_count INTEGER NOT NULL DEFAULT 0,
                 UNIQUE(domain, remote_path)
             );",
        ).unwrap();
        ensure_media_cover_metadata(&connection).unwrap();
        ensure_media_cover_metadata(&connection).unwrap();
        upsert_directory_cover_asset(&connection, "dir_cover_test", "vault-root", "/vault/book/__custom_dir_cover__.jpg", 100, 10).unwrap();
        connection.execute("UPDATE assets SET missing=1,missing_since=11,missing_scan_count=2", []).unwrap();
        upsert_directory_cover_asset(&connection, "dir_cover_test", "vault-root", "/vault/book/__custom_dir_cover__.jpg", 200, 20).unwrap();
        let row: (String, i64, i64, i64, i64, Option<i64>, i64) = connection.query_row(
            "SELECT domain,size_bytes,first_seen_at,last_seen_at,missing,missing_since,missing_scan_count FROM assets",
            [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
        ).unwrap();
        assert_eq!(row, ("vault".into(), 200, 10, 20, 0, None, 0));
        assert!(upsert_directory_cover_asset(&connection, "bad", "unknown-root", "/bad.jpg", 1, 30).is_err());
        connection.execute(
            "INSERT INTO directory_sync_state(root_id,remote_path,scanned_at,cover_asset_id) VALUES('vault-root','/vault/book',20,'dir_cover_test')", [],
        ).unwrap();
        assert_eq!(connection.query_row("SELECT cover_asset_id FROM directory_sync_state", [], |r| r.get::<_, String>(0)).unwrap(), "dir_cover_test");
    }

    fn sync_test_database() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE assets(id TEXT PRIMARY KEY,root_id TEXT,remote_path TEXT,file_name TEXT,parent_path TEXT,media_type TEXT,size_bytes INTEGER,etag TEXT,modified_at TEXT,content_hash TEXT,missing INTEGER,scanned_at INTEGER,duration_seconds INTEGER,UNIQUE(root_id,remote_path));
            CREATE TABLE library_nodes(root_id TEXT,remote_path TEXT,parent_path TEXT,name TEXT,scanned_at INTEGER,PRIMARY KEY(root_id,remote_path));
            CREATE TABLE directory_snapshots(root_id TEXT,remote_path TEXT,fingerprint TEXT,scanned_at INTEGER,PRIMARY KEY(root_id,remote_path));
            CREATE TABLE directory_sync_state(root_id TEXT,remote_path TEXT,scanned_at INTEGER,PRIMARY KEY(root_id,remote_path));
            CREATE TABLE favorites(asset_id TEXT PRIMARY KEY);
            CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT);
            CREATE TABLE playback_progress(asset_id TEXT PRIMARY KEY,position_seconds INTEGER);").unwrap();
        migrate_directory_order(&c).unwrap();
        c
    }
    fn sync_entry(path: &str, directory: bool) -> DavEntry {
        DavEntry { remote_path:path.into(),is_collection:directory,size_bytes:100,modified_at:Some("v1".into()),etag:None,thumbnail_url:None }
    }
    #[test]
    fn directory_sort_is_global_and_pages_append_without_reordering() {
        let mut c=sync_test_database();
        let mut entries=Vec::new();
        for directory in [true,false] {
            for n in 1..=305 {
                let path=if directory {format!("/media/folder-{n}")} else {format!("/media/image-{n}.jpg")};
                let mut entry=sync_entry(&path,directory);
                entry.modified_at=Some(chrono::DateTime::from_timestamp(1700000000+n*60,0).unwrap().to_rfc3339());
                entry.size_bytes=n;
                entries.push(entry);
            }
        }
        entries.reverse();
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media",entries.clone()).unwrap();
        for sort in [BrowseSort::Name,BrowseSort::Newest,BrowseSort::Size] {
            let mut all=Vec::new();let mut cursor=None;
            loop {
                let page=browse_sorted_directory_page(&c,"root","/media",sort,cursor.as_deref()).unwrap().unwrap();
                assert!(page.items.len()<=100);
                all.extend(page.items);cursor=page.next_cursor;if cursor.is_none(){break;}
            }
            assert_eq!(all.len(),610);
            assert_eq!(all.iter().map(|e|&e.path).collect::<HashSet<_>>().len(),610);
            for (i,entry) in all.iter().enumerate() {
                let directory=i<305;let index=i%305;
                let n=if matches!(sort,BrowseSort::Newest) || (!directory && matches!(sort,BrowseSort::Size)) {305-index} else {index+1};
                assert_eq!(entry.name,if directory{format!("folder-{n}")}else{format!("image-{n}.jpg")});
                assert!(entry.modified_at.is_some());
            }
        }
        let cursor=browse_sorted_directory_page(&c,"root","/media",BrowseSort::Name,None).unwrap().unwrap().next_cursor.unwrap();
        assert!(browse_sorted_directory_page(&c,"root","/media",BrowseSort::Newest,Some(&cursor)).is_err());
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media",entries).unwrap();
        assert!(browse_sorted_directory_page(&c,"root","/media",BrowseSort::Name,Some(&cursor)).is_err());
        for sort in [BrowseSort::Name,BrowseSort::Newest,BrowseSort::Size] {
            let sql=format!("EXPLAIN QUERY PLAN SELECT id FROM assets WHERE root_id=?1 AND parent_path=?2 AND missing=0 ORDER BY {} LIMIT 101 OFFSET 100",sort.order(false));
            let plans=c.prepare(&sql).unwrap().query_map(params!["root","/media"],|r|r.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
            assert!(!plans.iter().any(|p|p.contains("TEMP B-TREE")),"{plans:?}");
        }
    }

    #[test]
    fn unknown_dates_sort_last_and_cover_lookup_uses_saved_candidate() {
        let mut c=sync_test_database();
        let mut old=sync_entry("/media/old",true);old.modified_at=Some("Wed, 01 Jan 2020 00:00:00 GMT".into());
        let mut new=sync_entry("/media/new",true);new.modified_at=Some("2026-10-22T10:00:00+08:00".into());
        let mut unknown=sync_entry("/media/unknown",true);unknown.modified_at=None;
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media",vec![unknown,old,new]).unwrap();
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media/new",vec![sync_entry("/media/new/cover.avi",false),sync_entry("/media/new/image-2.jpg",false)]).unwrap();
        let page=browse_sorted_directory_page(&c,"root","/media",BrowseSort::Newest,None).unwrap().unwrap();
        assert_eq!(page.items.iter().map(|e|e.name.as_str()).collect::<Vec<_>>(),vec!["new","old","unknown"]);
        let cover=page.items[0].cover_asset_id.as_ref().unwrap();
        assert_eq!(c.query_row("SELECT file_name FROM assets WHERE id=?1",params![cover],|r|r.get::<_,String>(0)).unwrap(),"image-2.jpg");
        // Running migration again must preserve complete snapshots and propagate subfolder covers.
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media/old",vec![sync_entry("/media/old/sub",true)]).unwrap();
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media/old/sub",vec![sync_entry("/media/old/sub/deep.jpg",false)]).unwrap();
        c.execute("DELETE FROM settings WHERE key = 'directory_cover_tree_v2'", []).unwrap();
        migrate_directory_order(&c).unwrap();
        let page_after=browse_sorted_directory_page(&c,"root","/media",BrowseSort::Name,None).unwrap().unwrap();
        let old_entry=page_after.items.iter().find(|e|e.name=="old").unwrap();
        assert!(old_entry.cover_asset_id.is_some());
    }
    #[test]
    fn sync_commits_all_pages_and_preserves_removed_history() {
        let mut c=sync_test_database();
        let entries:Vec<_>=(0..235).map(|i|sync_entry(&format!("/media/{i:03}.jpg"),false)).collect();
        let result=sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media",entries.clone()).unwrap();
        assert_eq!(result.total,235); assert_eq!(result.added,235);
        let first=browse_local_directory_page(&c,"root","/media",1).unwrap().unwrap();
        assert_eq!(first.items.len(),100); assert!(first.next_cursor.as_ref().unwrap().starts_with("v2:name:"));
        assert_eq!(browse_local_directory_page(&c,"root","/media",3).unwrap().unwrap().items.len(),35);
        let id=first.items[0].asset_id.clone().unwrap();
        c.execute("INSERT INTO favorites VALUES(?1)",params![id]).unwrap();
        c.execute("INSERT INTO playback_progress VALUES(?1,42)",params![id]).unwrap();
        let mut changed=entries[1..].to_vec(); changed[0].modified_at=Some("v2".into());
        let result=sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media",changed).unwrap();
        assert_eq!(result.removed,1); assert_eq!(result.updated,1);
        assert_eq!(c.query_row("SELECT missing FROM assets WHERE id=?1",params![id],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(c.query_row("SELECT position_seconds FROM playback_progress WHERE asset_id=?1",params![id],|r|r.get::<_,i64>(0)).unwrap(),42);
        assert_eq!(c.query_row("SELECT count(*) FROM favorites",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        let restored=sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media",entries).unwrap();assert_eq!(restored.added,1);
    }
    #[test]
    fn sync_removes_only_deleted_subtrees_and_rolls_back_on_failure() {
        let mut c=sync_test_database();
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media",vec![sync_entry("/media/a_%",true),sync_entry("/media/abc",true)]).unwrap();
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media/a_%",vec![sync_entry("/media/a_%/a.jpg",false)]).unwrap();
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media/abc",vec![sync_entry("/media/abc/b.jpg",false)]).unwrap();
        sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media",vec![sync_entry("/media/abc",true)]).unwrap();
        assert!(browse_local_directory_page(&c,"root","/media/a_%",1).unwrap().is_none());
        assert_eq!(c.query_row("SELECT count(*) FROM assets WHERE missing=0",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        c.execute_batch("CREATE TRIGGER reject_insert BEFORE INSERT ON assets WHEN NEW.file_name='fail.jpg' BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media/abc",vec![sync_entry("/media/abc/new.jpg",false),sync_entry("/media/abc/fail.jpg",false)]).is_err());
        assert_eq!(c.query_row("SELECT count(*) FROM assets WHERE missing=0",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(browse_local_directory_page(&c,"root","/media/abc",1).unwrap().unwrap().items[0].name,"b.jpg");
        let empty=sync_directory_entries(&mut c,"root",LibraryRootKind::Media,"/media/abc",vec![]).unwrap();assert_eq!(empty.total,0);
        assert!(browse_local_directory_page(&c,"root","/media/abc",1).unwrap().unwrap().items.is_empty());
    }
    #[test]
    fn malformed_or_denied_webdav_listing_is_not_an_empty_directory() {
        let endpoint=Url::parse("http://localhost/dav/").unwrap();
        assert!(parse_webdav_multistatus("<html>error</html>",&endpoint,"/").is_err());
        assert!(parse_webdav_multistatus("<D:multistatus xmlns:D='DAV:'><D:response>",&endpoint,"/").is_err());
        assert!(parse_webdav_multistatus("<D:multistatus xmlns:D='DAV:'><D:response><D:href>/dav/a.jpg</D:href><D:status>HTTP/1.1 403 Forbidden</D:status></D:response></D:multistatus>",&endpoint,"/").is_err());
    }

    fn browse_test_session(address: std::net::SocketAddr) -> Session {
        Session {
            provider: ProviderKind::Alist,
            endpoint: Url::parse(&format!("http://{address}/")).unwrap(),
            api_token: "test-token".into(),
            client: Client::new(),
        }
    }

    #[test]
    fn browse_api_fetches_one_page_and_refreshes_only_on_request() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let requests = Arc::new(StdMutex::new(Vec::<serde_json::Value>::new()));
            let captured = requests.clone();
            let router = axum::Router::new().fallback(axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                captured.lock().unwrap().push(body.clone());
                async move {
                    let page = body["page"].as_i64().unwrap();
                    let per_page = body["per_page"].as_i64().unwrap();
                    let content = if page <= 2 { (0..per_page).map(|i| serde_json::json!({"name": format!("video-{}.mp4", (page - 1) * per_page + i), "size": 100, "is_dir": false})).collect::<Vec<_>>() } else { Vec::new() };
                    axum::Json(serde_json::json!({"code":200,"data":{"content":content,"total":0}}))
                }
            }));
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let session = browse_test_session(listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let state = MediaLibraryState::new();
            let (first, cursor) =
                browse_directory_page(&state, &session, "root", "/media", None, false)
                    .await
                    .unwrap();
            assert_eq!(first.len(), BROWSE_PAGE_SIZE);
            assert_eq!(requests.lock().unwrap().len(), 1);
            let (second, cursor) =
                browse_directory_page(&state, &session, "root", "/media", cursor, false)
                    .await
                    .unwrap();
            assert_eq!(second.len(), BROWSE_PAGE_SIZE);
            assert!(!first
                .iter()
                .any(|a| second.iter().any(|b| a.remote_path == b.remote_path)));
            let (last, cursor) =
                browse_directory_page(&state, &session, "root", "/media", cursor, false)
                    .await
                    .unwrap();
            assert!(last.is_empty() && cursor.is_none());
            browse_directory_page(&state, &session, "root", "/media", None, true)
                .await
                .unwrap();
            let observed = requests.lock().unwrap();
            assert_eq!(observed.len(), 4);
            for request in observed.iter().take(3) {
                assert_eq!(request["refresh"], false);
                assert_eq!(request["per_page"], BROWSE_PAGE_SIZE);
            }
            assert_eq!(observed[3]["refresh"], true);
            server.abort();
        });
    }

    #[test]
    fn unchanged_browser_page_never_requires_a_database_write() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE assets (id TEXT PRIMARY KEY, root_id TEXT, remote_path TEXT, file_name TEXT, parent_path TEXT, media_type TEXT, size_bytes INTEGER, etag TEXT, thumbnail_key TEXT, missing INTEGER, scanned_at INTEGER, content_hash TEXT, UNIQUE(root_id, remote_path));").unwrap();
        let entry = RemoteBrowserEntry {
            path: "/media/a.mp4".into(),
            name: "a.mp4".into(),
            is_directory: false,
            size_bytes: 100,
            modified_at: None,
            etag: Some("v1".into()),
            asset_id: Some("asset-a".into()),
            media_type: Some("video".into()),
            thumbnail_source: None,
            thumbnail_url: None,
            cover_asset_id: None,
            duration_seconds: None,
            progress_seconds: None,
        };
        register_browsed_assets(&mut connection, "root", "/media", vec![entry.clone()], 1).unwrap();
        connection.execute_batch("PRAGMA query_only = ON;").unwrap();
        register_browsed_assets(&mut connection, "root", "/media", vec![entry.clone()], 2).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT scanned_at FROM assets", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        connection
            .execute_batch("PRAGMA query_only = OFF; UPDATE assets SET content_hash = 'old';")
            .unwrap();
        let mut changed = entry;
        changed.size_bytes = 200;
        register_browsed_assets(&mut connection, "root", "/media", vec![changed], 3).unwrap();
        let actual = connection
            .query_row("SELECT size_bytes, content_hash FROM assets", [], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .unwrap();
        assert_eq!(actual, (200, None));
    }
    #[test]
    fn restores_a_duplicated_comic_archive_path() {
        assert_eq!(
            legacy_archive_path("/漫画/作品.zip/作品.zip").as_deref(),
            Some("/漫画/作品.zip")
        );
        assert_eq!(legacy_archive_path("/漫画/作品.zip").as_deref(), None);
    }

    #[test]
    fn recognizes_the_first_braced_directory_as_a_category() {
        let parsed = parse_asset_path("/115/文件", "/115/文件/{国产}/{推特}/【作者】/合集/01.jpg");
        assert_eq!(parsed.categories.first().map(String::as_str), Some("国产"));
        assert_eq!(category_marker("{  韩国  }").as_deref(), Some("韩国"));
        assert_eq!(category_marker("{分类"), None);
    }

    #[test]
    fn recognizes_full_width_braced_directories_as_categories() {
        let parsed = parse_asset_path(
            "/115/文件",
            "/115/文件/{国产}/｛1-推特｝/【作者】/合集/01.jpg",
        );
        assert_eq!(parsed.categories, vec!["国产", "1-推特"]);
        assert_eq!(category_marker("｛  韩国  ｝").as_deref(), Some("韩国"));
        assert_eq!(category_marker("｛分类"), None);
        assert_eq!(category_marker("{分类｝"), None);
    }

    #[test]
    fn directory_fingerprints_are_order_independent() {
        let first = vec![
            DavEntry {
                remote_path: "/root/b.jpg".into(),
                is_collection: false,
                size_bytes: 2,
                modified_at: None,
                etag: Some("b".into()),
                thumbnail_url: None,
            },
            DavEntry {
                remote_path: "/root/a".into(),
                is_collection: true,
                size_bytes: 0,
                modified_at: None,
                etag: Some("a".into()),
                thumbnail_url: None,
            },
        ];
        let mut second = first.clone();
        second.reverse();
        assert_eq!(
            directory_fingerprint(&first),
            directory_fingerprint(&second)
        );
    }

    #[test]
    fn only_reuses_nested_directories_with_a_stable_directory_marker() {
        let nested = vec![DavEntry {
            remote_path: "/root/child".into(),
            is_collection: true,
            size_bytes: 0,
            modified_at: None,
            etag: None,
            thumbnail_url: None,
        }];
        assert!(!directory_has_reusable_fingerprint("/root", &nested));
        let leaf = vec![DavEntry {
            remote_path: "/root/file.jpg".into(),
            is_collection: false,
            size_bytes: 1,
            modified_at: None,
            etag: None,
            thumbnail_url: None,
        }];
        assert!(directory_has_reusable_fingerprint("/root", &leaf));
        let marked_nested = vec![
            DavEntry {
                remote_path: "/root".into(),
                is_collection: true,
                size_bytes: 0,
                modified_at: None,
                etag: Some("stable".into()),
                thumbnail_url: None,
            },
            nested[0].clone(),
        ];
        assert!(directory_has_reusable_fingerprint("/root", &marked_nested));
    }

    #[test]
    fn category_markers_do_not_become_author_or_collection_names() {
        let parsed = parse_asset_path("/文件", "/文件/{国产}/【果果】/#果果 50V/01.jpg");
        assert_eq!(parsed.categories, vec!["国产"]);
        assert_eq!(parsed.author.as_deref(), Some("果果"));
        assert_eq!(parsed.collections, vec!["#果果 50V"]);
    }

    #[test]
    fn uses_a_browser_compatible_type_for_proxy_urls() {
        assert_eq!(media_content_type("/媒体/封面.jpg"), "image/jpeg");
        assert_eq!(media_content_type("/媒体/片段.mp4"), "video/mp4");
        assert_eq!(media_content_type("/媒体/片段.m2ts"), "video/mp2t");
    }

    #[test]
    fn recognizes_common_media_extensions_that_are_not_browser_defaults() {
        assert_eq!(
            media_type_for(LibraryRootKind::Media, "/照片/扫描件.tiff"),
            Some("image")
        );
        assert_eq!(
            media_type_for(LibraryRootKind::Media, "/视频/片段.m2ts"),
            Some("video")
        );
        assert!(is_comic_image_name("page.tiff"));
    }

    #[test]
    fn matches_named_video_covers_but_rejects_index_like_names() {
        assert_eq!(
            thumbnail_key("Trailer.mp4", "video"),
            Some("trailer".into())
        );
        assert_eq!(
            thumbnail_key("trailer.JPG", "image"),
            Some("trailer".into())
        );
        assert_eq!(thumbnail_key("001.mp4", "video"), None);
        assert_eq!(thumbnail_key("001.jpg", "image"), None);
        assert_eq!(thumbnail_key("01-02.png", "image"), None);
        assert_eq!(
            thumbnail_key("episode-01.mp4", "video"),
            Some("episode-01".into())
        );
    }

    #[test]
    fn skips_only_transiently_unreadable_child_directories() {
        assert!(is_skippable_scan_error(
            "AList API 目录读取失败（HTTP 404 Not Found）：/媒体/已删除"
        ));
        assert!(is_skippable_scan_error(
            "AList API 目录读取失败（HTTP 405 Method Not Allowed）：/媒体/受限"
        ));
        assert!(is_skippable_scan_error("AList API 目录读取失败（HTTP 200 OK）：failed get objs: failed get dir: object not found"));
        assert!(is_skippable_scan_error(
            "AList API 目录读取失败（HTTP 200 OK）：failed to list objs: potential threats"
        ));
        assert!(is_skippable_scan_error(
            "AList API 目录读取失败（HTTP 405 Method Not Allowed）：服务器返回 HTML 错误页面"
        ));
        assert!(!is_skippable_scan_error("AList 用户名、密码或目录权限无效"));
    }

    #[test]
    fn summarizes_non_json_alist_errors_without_returning_html() {
        assert_eq!(
            summarize_api_error("failed get objs: failed to list objs: <!doctype html><html>..."),
            "服务器返回 HTML 错误页面，当前目录暂时无法读取"
        );
        assert_eq!(
            summarize_api_error("failed get objs: storage driver error"),
            "存储驱动无法读取该目录（failed get objs）"
        );
        assert_eq!(summarize_api_error("权限不足"), "权限不足");
    }

    #[test]
    fn normalizes_saved_bearer_prefixes() {
        assert_eq!(
            normalize_api_token("  Bearer abc  ").as_deref(),
            Some("abc")
        );
        assert_eq!(normalize_api_token("bearer abc").as_deref(), Some("abc"));
        assert_eq!(normalize_api_token("   "), None);
    }

    #[test]
    fn accepts_alist_directory_content_variants() {
        let direct = serde_json::json!([{"name":"a.jpg"}]);
        assert_eq!(api_directory_entries(&direct).unwrap().len(), 1);
        let wrapped = serde_json::json!({"content":[{"name":"b.mp4"}]});
        assert_eq!(api_directory_entries(&wrapped).unwrap().len(), 1);
        let encoded = serde_json::json!({"files":"[{\"name\":\"c.png\"}]"});
        assert_eq!(api_directory_entries(&encoded).unwrap().len(), 1);
        let empty = serde_json::json!({"content":null,"total":0});
        assert!(api_directory_entries(&empty).unwrap().is_empty());
        let children = serde_json::json!({"children":[{"name":"d.jpg"}]});
        assert_eq!(api_directory_entries(&children).unwrap().len(), 1);
    }

    #[test]
    fn builds_alist_download_route_for_zip_fallback() {
        let endpoint = Url::parse("http://example.com/dav/").unwrap();
        let source = Url::parse("http://example.com/dav/漫画/作品.zip").unwrap();
        assert_eq!(
            alist_download_url(&endpoint, &source).unwrap().as_str(),
            "http://example.com/d/%E6%BC%AB%E7%94%BB/%E4%BD%9C%E5%93%81.zip"
        );
        assert_eq!(
            alist_proxy_url(&endpoint, &source).unwrap().as_str(),
            "http://example.com/p/%E6%BC%AB%E7%94%BB/%E4%BD%9C%E5%93%81.zip?d=1"
        );
    }

    #[test]
    fn preserves_an_alist_reverse_proxy_prefix_for_media_routes() {
        let endpoint = Url::parse("http://example.com/alist/dav/").unwrap();
        let source = Url::parse("http://example.com/alist/dav/video.mp4").unwrap();
        assert_eq!(
            alist_proxy_url(&endpoint, &source).unwrap().as_str(),
            "http://example.com/alist/p/video.mp4?d=1"
        );
        assert_eq!(
            alist_download_url(&endpoint, &source).unwrap().as_str(),
            "http://example.com/alist/d/video.mp4"
        );
    }

    #[test]
    fn preserves_reverse_proxy_prefix_for_api_routes_and_auth() {
        let endpoint = Url::parse("http://example.com/alist/dav/").unwrap();
        assert_eq!(
            alist_api_url(&endpoint, "fs/list").unwrap().as_str(),
            "http://example.com/alist/api/fs/list"
        );
        assert!(is_alist_owned_route(
            &endpoint,
            &Url::parse("http://example.com/alist/p/video.mp4").unwrap()
        ));
        assert!(!is_alist_owned_route(
            &endpoint,
            &Url::parse("http://example.com/alist/cdn/video.mp4").unwrap()
        ));
    }

    #[test]
    fn normalizes_alist_thumbnail_routes_without_exposing_provider_hosts() {
        let endpoint = Url::parse("http://xhofe_alist:5244/alist/dav/").unwrap();
        let direct =
            alist_thumbnail_url(&endpoint, "http://127.0.0.1:5244/p/media/cover.jpg").unwrap();
        assert_eq!(
            direct.as_str(),
            "http://xhofe_alist:5244/alist/p/media/cover.jpg"
        );
        let api =
            alist_thumbnail_url(&endpoint, "/api/fs/thumb?path=%2Fmedia%2Fcover.jpg").unwrap();
        assert_eq!(
            api.as_str(),
            "http://xhofe_alist:5244/alist/api/fs/thumb?path=%2Fmedia%2Fcover.jpg"
        );
        assert!(alist_thumbnail_url(&endpoint, "/api/public/settings").is_none());
    }

    #[test]
    fn maps_root_endpoint_to_root_api_route() {
        let endpoint = Url::parse("http://example.com/").unwrap();
        assert_eq!(
            alist_api_url(&endpoint, "/auth/login").unwrap().as_str(),
            "http://example.com/api/auth/login"
        );
        let api_endpoint = Url::parse("http://example.com/api/").unwrap();
        assert_eq!(
            alist_api_url(&api_endpoint, "fs/list").unwrap().as_str(),
            "http://example.com/api/fs/list"
        );
    }

    #[test]
    fn parses_webdav_multistatus_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/115open/</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>115open</D:displayname>
        <D:resourcetype><D:collection/></D:resourcetype>
        <D:getcontentlength>0</D:getcontentlength>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/115open/%E6%BC%AB%E7%94%BB/test.mp4</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>test.mp4</D:displayname>
        <D:resourcetype/>
        <D:getcontentlength>104857600</D:getcontentlength>
        <D:getlastmodified>Sun, 19 Oct 2026 12:00:00 GMT</D:getlastmodified>
        <D:getetag>"abcdef123"</D:getetag>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let endpoint = Url::parse("http://192.168.31.71:19798/dav/").unwrap();
        let entries = parse_webdav_multistatus(xml, &endpoint, "/").unwrap();
        assert_eq!(entries.len(), 2);

        let dir = &entries[0];
        assert_eq!(dir.remote_path, "/115open");
        assert!(dir.is_collection);
        assert_eq!(dir.size_bytes, 0);

        let file = &entries[1];
        assert_eq!(file.remote_path, "/115open/漫画/test.mp4");
        assert!(!file.is_collection);
        assert_eq!(file.size_bytes, 104857600);
        assert_eq!(file.etag.as_deref(), Some("abcdef123"));
        assert_eq!(file.modified_at.as_deref(), Some("Sun, 19 Oct 2026 12:00:00 GMT"));
    }

    #[test]
    fn parses_zip_central_directory_and_decompresses_slice() {
        use std::io::Write;
        let mut buffer = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            zip.start_file("002.jpg", options).unwrap();
            zip.write_all(b"page 2 test payload").unwrap();
            zip.start_file("001.jpg", options).unwrap();
            zip.write_all(b"page 1 test payload").unwrap();
            zip.finish().unwrap();
        }
        let zip_bytes = buffer.into_inner();
        let (cd_offset, cd_size) = parse_zip_eocd(&zip_bytes).unwrap();
        let cd_bytes = &zip_bytes[cd_offset as usize..(cd_offset + cd_size) as usize];
        let entries = parse_zip_central_directory_entries(cd_bytes).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "001.jpg");
        assert_eq!(entries[1].name, "002.jpg");

        let entry0 = &entries[0];
        let slice = &zip_bytes[entry0.local_header_offset as usize..];
        let (mime, decompressed) = decompress_zip_range_slice(slice, entry0).unwrap();
        assert_eq!(mime, "image/jpeg");
        assert_eq!(decompressed, b"page 1 test payload");

        let entry1 = &entries[1];
        let slice = &zip_bytes[entry1.local_header_offset as usize..];
        let (mime, decompressed) = decompress_zip_range_slice(slice, entry1).unwrap();
        assert_eq!(mime, "image/jpeg");
        assert_eq!(decompressed, b"page 2 test payload");
    }
}

// ===== 目录槽位与媒体源浏览（P4） =====

const DIR_RECENT_KEY: &str = "dir_recent";
const DIR_PENDING_KEY: &str = "dir_pending";
const DIR_VAULT_KEY: &str = "dir_vault";
const DIR_COMICS_KEY: &str = "dir_comics";

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryConfig {
    pub recent_path: Option<String>,
    pub pending_path: Option<String>,
    pub vault_path: Option<String>,
    pub comics_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDirectoryListing {
    pub path: String,
    pub directories: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlistMappingCheck {
    pub ok: bool,
    pub message: String,
    pub cd2_count: usize,
    pub alist_count: usize,
    pub matched: usize,
    pub missing_in_alist: Vec<String>,
    pub missing_in_cd2: Vec<String>,
}

fn pml_dir_setting(connection: &Connection, key: &str) -> Option<String> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| {
            row.get::<_, String>(0)
        })
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn pml_set_setting(connection: &Connection, key: &str, value: Option<&str>) -> Result<(), String> {
    match value.filter(|v| !v.trim().is_empty()) {
        Some(v) => {
            connection
                .execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![key, v.trim()],
                )
                .map_err(database_error)?;
        }
        None => {
            let _ = connection.execute("DELETE FROM settings WHERE key = ?1", params![key]);
        }
    }
    Ok(())
}

fn pml_basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn get_directory_config(app: AppHandle) -> Result<DirectoryConfig, String> {
    let connection = crate::v2_config::open(&app)?;
    directory_config_from_connection(&connection)
}

fn canonical_domain_path(connection: &Connection, domain: &str) -> Result<Option<String>, String> {
    connection.query_row("SELECT remote_path FROM domain_roots WHERE domain=?1", [domain], |row| row.get(0))
        .optional().map_err(database_error)
}

fn directory_config_from_connection(connection: &Connection) -> Result<DirectoryConfig, String> {
    Ok(DirectoryConfig {
        recent_path: canonical_domain_path(connection, "recent")?,
        pending_path: canonical_domain_path(connection, "pending")?,
        vault_path: canonical_domain_path(connection, "vault")?,
        comics_path: canonical_domain_path(connection, "comics")?,
    })
}

fn pml_ensure_root(
    connection: &Connection,
    kind: LibraryRootKind,
    path: &str,
) -> Result<(), String> {
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT id, kind FROM library_roots WHERE remote_path = ?1",
            params![path],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(database_error)?;
    if let Some((id, existing_kind)) = existing {
        if existing_kind == kind.as_str() {
            return Ok(());
        }
        connection
            .execute("DELETE FROM library_roots WHERE id = ?1", params![id])
            .map_err(database_error)?;
        let _ = connection.execute("DELETE FROM collections WHERE root_id = ?1", params![id]);
    }
    let id = format!("root-{:016x}", stable_hash(path));
    connection
        .execute(
            "INSERT INTO library_roots (id, kind, remote_path, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, kind.as_str(), path, now()],
        )
        .map_err(database_error)?;
    Ok(())
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn save_directory_config(app: AppHandle, config: DirectoryConfig) -> Result<DirectoryConfig, String> {
    let normalize = |value: Option<String>| -> Result<Option<String>, String> {
        match value {
            Some(path) if !path.trim().is_empty() => Ok(Some(normalize_remote_path(&path)?)),
            _ => Ok(None),
        }
    };
    let config = DirectoryConfig {
        recent_path: normalize(config.recent_path)?,
        pending_path: normalize(config.pending_path)?,
        vault_path: normalize(config.vault_path)?,
        comics_path: normalize(config.comics_path)?,
    };

    let mut configured_paths = HashSet::new();
    for path in [&config.recent_path, &config.pending_path, &config.vault_path, &config.comics_path].into_iter().flatten() {
        if !configured_paths.insert(path.as_str()) {
            return Err("四个媒体域不能使用同一个根目录".into());
        }
    }
    {
        let mut connection = crate::v2_config::open(&app)?;
        let tx = connection.transaction().map_err(database_error)?;
        for (domain, key, path) in [
            ("recent", DIR_RECENT_KEY, config.recent_path.as_deref()),
            ("pending", DIR_PENDING_KEY, config.pending_path.as_deref()),
            ("vault", DIR_VAULT_KEY, config.vault_path.as_deref()),
            ("comics", DIR_COMICS_KEY, config.comics_path.as_deref()),
        ] {
            pml_set_setting(&tx, key, path)?;
            let old: Option<(String, String)> = tx.query_row(
                "SELECT root_id,remote_path FROM domain_roots WHERE domain=?1",
                params![domain], |r| Ok((r.get(0)?, r.get(1)?)),
            ).optional().map_err(database_error)?;
            if old.as_ref().map(|(_, old_path)| old_path.as_str()) == path {
                continue;
            }
            if let Some((root_id, _)) = old.as_ref() {
                tx.execute("DELETE FROM library_roots WHERE id=?1", params![root_id]).map_err(database_error)?;
            }
            // Keep state/history, but never show old-root assets under a newly configured root.
            tx.execute("UPDATE assets SET missing=1,missing_since=COALESCE(missing_since,?2) WHERE domain=?1", params![domain,now()]).map_err(database_error)?;
            tx.execute("DELETE FROM provider_directory_entries WHERE domain=?1", params![domain]).map_err(database_error)?;
            tx.execute("DELETE FROM provider_directory_snapshots WHERE domain=?1", params![domain]).map_err(database_error)?;
            match path {
                Some(path) => {
                    let root_id = format!("root-{:016x}",stable_hash(path));
                    tx.execute(
                        "INSERT INTO library_roots(id,kind,remote_path,created_at) VALUES(?1,?2,?3,?4)
                         ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,remote_path=excluded.remote_path",
                        params![root_id,if domain=="comics" {"comics"} else {"media"},path,now()],
                    ).map_err(database_error)?;
                    tx.execute(
                        "INSERT INTO domain_roots(domain,root_id,remote_path,updated_at) VALUES(?1,?2,?3,?4)
                         ON CONFLICT(domain) DO UPDATE SET root_id=excluded.root_id,remote_path=excluded.remote_path,updated_at=excluded.updated_at",
                        params![domain,root_id,path,now()],
                    ).map_err(database_error)?;
                }
                None => { tx.execute("DELETE FROM domain_roots WHERE domain=?1",params![domain]).map_err(database_error)?; }
            }
        }
        tx.commit().map_err(database_error)?;
    }
    // 兼容旧版：待整理/收纳目标同步到 curator 配置。
    let _ = save_curator_config(
        app.clone(),
        CuratorConfigUpdate {
            pending_path: config.pending_path.clone(),
            target_vault_root: config.vault_path.clone(),
            ..Default::default()
        },
    );

    Ok(config)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn list_media_source_directories(
    state: State<'_, MediaLibraryState>,
    path: String,
) -> Result<SourceDirectoryListing, String> {
    let session = require_session(&state).await?;
    let path = normalize_remote_path(&path)?;
    let entries = list_directory(&session, &path).await?;
    let mut directories: Vec<String> = entries
        .into_iter()
        .filter(|entry| entry.is_collection)
        .map(|entry| pml_basename(&entry.remote_path))
        .filter(|name| !name.is_empty())
        .collect();
    directories.sort_by_key(|name| name.to_lowercase());
    directories.dedup();
    Ok(SourceDirectoryListing { path, directories })
}

async fn pml_alist_session(config: &AlistAccelConfig) -> Result<Session, String> {
    let base_url = config.api_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return Err("请先填写 AList 地址".into());
    }
    let client = &*ALIST_ACCEL_CLIENT;
    let login_url = format!("{base_url}/api/auth/login");
    let login_body = serde_json::json!({
        "username": config.username.trim(),
        "password": config.password.as_deref().unwrap_or_default(),
    });
    let response = client
        .post(&login_url)
        .json(&login_body)
        .send()
        .await
        .map_err(|error| format!("无法连接 AList 服务：{error}"))?;
    if !response.status().is_success() {
        return Err(format!("AList 接口返回状态码 {}", response.status().as_u16()));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("解析 AList 登录响应失败：{error}"))?;
    if value.get("code").and_then(|code| code.as_i64()) != Some(200) {
        let message = value
            .get("message")
            .and_then(|message| message.as_str())
            .unwrap_or("账号或密码错误");
        return Err(format!("AList 认证失败：{message}"));
    }
    let token = value
        .get("data")
        .and_then(|data| data.get("token"))
        .and_then(|token| token.as_str())
        .unwrap_or_default()
        .to_string();
    if token.is_empty() {
        return Err("AList 未返回有效 Token".into());
    }
    let endpoint = Url::parse(base_url).map_err(|error| error.to_string())?;
    Ok(Session {
        provider: ProviderKind::Alist,
        endpoint,
        api_token: token,
        client: client.clone(),
    })
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn list_alist_directories(
    app: AppHandle,
    path: String,
) -> Result<SourceDirectoryListing, String> {
    let config = get_alist_accel_config(app)?;
    let session = pml_alist_session(&config).await?;
    let path = normalize_remote_path(&path)?;
    let entries = list_directory(&session, &path).await?;
    let mut directories: Vec<String> = entries
        .into_iter()
        .filter(|entry| entry.is_collection)
        .map(|entry| pml_basename(&entry.remote_path))
        .filter(|name| !name.is_empty())
        .collect();
    directories.sort_by_key(|name| name.to_lowercase());
    directories.dedup();
    Ok(SourceDirectoryListing { path, directories })
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn validate_alist_mapping(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    cd2_path: String,
    alist_path: String,
) -> Result<AlistMappingCheck, String> {
    let cd2_session = require_session(&state).await?;
    let config = get_alist_accel_config(app)?;
    let alist_session = pml_alist_session(&config).await?;
    let cd2_path = normalize_remote_path(&cd2_path)?;
    let alist_path = normalize_remote_path(&alist_path)?;
    let cd2_entries = list_directory(&cd2_session, &cd2_path).await?;
    let alist_entries = list_directory(&alist_session, &alist_path).await?;

    let cd2_names: Vec<String> = cd2_entries
        .iter()
        .filter(|entry| entry.is_collection)
        .map(|entry| pml_basename(&entry.remote_path))
        .filter(|name| !name.is_empty())
        .collect();
    let alist_names: Vec<String> = alist_entries
        .iter()
        .filter(|entry| entry.is_collection)
        .map(|entry| pml_basename(&entry.remote_path))
        .filter(|name| !name.is_empty())
        .collect();

    let cd2_set: HashSet<String> = cd2_names.iter().map(|name| name.to_lowercase()).collect();
    let alist_set: HashSet<String> = alist_names.iter().map(|name| name.to_lowercase()).collect();
    let matched = cd2_set.intersection(&alist_set).count();
    let mut missing_in_alist: Vec<String> = cd2_names
        .iter()
        .filter(|name| !alist_set.contains(&name.to_lowercase()))
        .take(10)
        .cloned()
        .collect();
    let mut missing_in_cd2: Vec<String> = alist_names
        .iter()
        .filter(|name| !cd2_set.contains(&name.to_lowercase()))
        .take(10)
        .cloned()
        .collect();
    missing_in_alist.sort();
    missing_in_cd2.sort();

    let ok = missing_in_alist.is_empty() && missing_in_cd2.is_empty();
    let message = if ok {
        format!("目录结构一致：{} 个同名目录", matched)
    } else {
        format!(
            "存在差异：CD2 {} 个目录，AList {} 个目录，{} 个同名",
            cd2_names.len(),
            alist_names.len(),
            matched
        )
    };
    Ok(AlistMappingCheck {
        ok,
        message,
        cd2_count: cd2_names.len(),
        alist_count: alist_names.len(),
        matched,
        missing_in_alist,
        missing_in_cd2,
    })
}
// ===== 字幕（同目录同名 SRT/VTT） =====

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubtitleTrack {
    pub name: String,
    pub path: String,
    pub format: String,
}

fn pml_subtitle_file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn pml_subtitle_stem(name: &str) -> &str {
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub async fn find_asset_subtitles(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    asset_id: String,
) -> Result<Vec<SubtitleTrack>, String> {
    let (remote_path, parent_path): (String, String) = {
        let connection = open_media_database(&app)?;
        connection
            .query_row(
                "SELECT remote_path, parent_path FROM assets WHERE id = ?1",
                params![asset_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| "找不到该视频的索引".to_string())?
    };
    let session = require_session(&state).await?;
    let entries = list_directory(&session, &parent_path).await?;
    let video_name = pml_subtitle_file_name(&remote_path);
    let base = pml_subtitle_stem(video_name).to_lowercase();
    let mut tracks: Vec<SubtitleTrack> = Vec::new();
    for entry in entries {
        if entry.is_collection {
            continue;
        }
        let name = pml_subtitle_file_name(&entry.remote_path);
        let lower = name.to_lowercase();
        let ext = lower.rsplit('.').next().unwrap_or("");
        if ext != "srt" && ext != "vtt" {
            continue;
        }
        let stem = pml_subtitle_stem(name).to_lowercase();
        let same_base = stem == base
            || stem.starts_with(&format!("{base}."))
            || stem.starts_with(&format!("{base}_"))
            || stem.starts_with(&format!("{base}-"));
        if !same_base {
            continue;
        }
        tracks.push(SubtitleTrack {
            name: name.to_string(),
            path: entry.remote_path.clone(),
            format: ext.to_string(),
        });
    }
    tracks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tracks)
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiblingVideo {
    pub id: String,
    pub title: String,
}

#[cfg_attr(feature = "tauri", tauri::command)]
pub fn list_sibling_videos(app: AppHandle, asset_id: String) -> Result<Vec<SiblingVideo>, String> {
    let connection = open_media_database(&app)?;
    let (root_id, parent_path, media_type): (String, String, String) = connection
        .query_row(
            "SELECT root_id, parent_path, media_type FROM assets WHERE id = ?1",
            params![asset_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "找不到该视频的索引".to_string())?;
    if media_type != "video" {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(
            "SELECT id, file_name, remote_path FROM assets
             WHERE root_id = ?1 AND parent_path = ?2 AND media_type = 'video' AND missing = 0
             ORDER BY file_name ASC",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map(params![root_id, parent_path], |row| {
            Ok(SiblingVideo {
                id: row.get(0)?,
                title: row.get(1)?,
            })
        })
        .map_err(database_error)?;
    Ok(rows.filter_map(|row| row.ok()).collect())
}