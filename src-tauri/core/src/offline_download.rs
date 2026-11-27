use crate::AppHandle;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_INBOX_PATH: &str = "/115open/PML_Inbox";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineDownloadConfig {
    pub inbox_path: String,
    pub trash_path: String,
    pub movie_dir: String,
    pub comic_dir: String,
    pub gallery_dir: String,
    pub creator_pending_dir: String,
    pub gallery_pending_dir: String,
    pub comic_pending_dir: String,
    pub collection_pending_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitOfflineMagnetRequest {
    pub magnet: String,
    pub title: Option<String>,
    pub target_dir: Option<String>,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineDownloadTask {
    pub id: String,
    pub info_hash: String,
    pub title: String,
    pub magnet: String,
    pub target_dir: String,
    pub category: String,
    pub status: String,
    pub message: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineQuotaInfo {
    pub total: i64,
    pub used: i64,
    pub left: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudOfflineFile {
    pub name: String,
    pub size_bytes: u64,
    pub size_text: String,
    pub url: String,
    /// 0/1 = downloading, 2 = completed, other = failed
    pub status: i32,
    pub status_category: String,
    pub info_hash: String,
    pub file_id: String,
    pub add_time: i64,
    pub percent_done: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfflineCloudStatusResponse {
    pub cloud_name: String,
    pub quota: OfflineQuotaInfo,
    pub page: i32,
    pub page_size: i32,
    pub page_count: i32,
    pub total_count: i64,
    pub items: Vec<CloudOfflineFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOfflineSubmitItemResult {
    pub input: String,
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOfflineSubmitResponse {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub target_dir: String,
    pub results: Vec<BatchOfflineSubmitItemResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxFileEntry {
    pub name: String,
    pub full_path: String,
    pub size_bytes: u64,
    pub size_text: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxFileRename {
    pub full_path: String,
    pub old_name: String,
    pub new_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxPlanItem {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
    pub total_size_bytes: u64,
    pub total_size_text: String,
    pub suggested_name: String,
    pub suggested_category: String,
    pub needs_clean: bool,
    pub flatten_source_dir: Option<String>,
    pub flatten_wrapper_dir: Option<String>,
    pub flatten_levels: usize,
    pub kept_files: Vec<InboxFileEntry>,
    pub junk_files: Vec<InboxFileEntry>,
    pub file_renames: Vec<InboxFileRename>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxScanResponse {
    pub inbox_path: String,
    pub trash_path: String,
    pub config: OfflineDownloadConfig,
    pub items: Vec<InboxPlanItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanInboxItemRequest {
    pub item_path: String,
    pub trash_paths: Vec<String>,
    pub file_renames: Option<Vec<InboxFileRename>>,
    pub flatten_source_dir: Option<String>,
    pub flatten_wrapper_dir: Option<String>,
    pub new_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanInboxItemResponse {
    pub ok: bool,
    pub moved_trash_count: usize,
    pub new_path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveInboxItemRequest {
    pub item_path: String,
    pub category: String,
    pub target_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveInboxItemResponse {
    pub ok: bool,
    pub dest_dir: String,
    pub message: String,
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn normalize_cloud_path(raw: &str, fallback: &str) -> String {
    let trimmed = raw.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    let with_slash = if trimmed.starts_with('/') {
        trimmed
    } else {
        format!("/{trimmed}")
    };
    let clean = with_slash.trim_end_matches('/').to_string();
    if clean.is_empty() {
        fallback.to_string()
    } else {
        clean
    }
}

fn ensure_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS offline_download_tasks (
            id TEXT PRIMARY KEY,
            info_hash TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT '',
            magnet TEXT NOT NULL,
            target_dir TEXT NOT NULL,
            category TEXT NOT NULL DEFAULT 'auto',
            status TEXT NOT NULL DEFAULT 'submitted',
            message TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_offline_tasks_created ON offline_download_tasks(created_at DESC);",
    )
    .map_err(|e| format!("初始化离线下载表失败：{e}"))
}

fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get::<_, String>(0))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn set_setting(conn: &Connection, key: &str, val: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO settings(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, val.trim()],
    )
    .map_err(|e| format!("保存配置失败：{e}"))?;
    Ok(())
}

pub fn get_offline_download_config(app: &AppHandle) -> Result<OfflineDownloadConfig, String> {
    let conn = crate::v2_config::open(app)?;
    ensure_schema(&conn)?;
    let inbox_path = normalize_cloud_path(
        &get_setting(&conn, "offline_inbox_path").unwrap_or_else(|| DEFAULT_INBOX_PATH.to_string()),
        DEFAULT_INBOX_PATH,
    );
    let trash_path = format!("{inbox_path}/_Trash");
    let movie_dir = get_setting(&conn, "offline_movie_dir").unwrap_or_default();
    let comic_dir = get_setting(&conn, "offline_comic_dir").unwrap_or_default();
    let gallery_dir = get_setting(&conn, "offline_gallery_dir").unwrap_or_default();
    let creator_pending_dir = get_setting(&conn, "offline_creator_pending_dir").unwrap_or_default();
    let gallery_pending_dir = get_setting(&conn, "offline_gallery_pending_dir").unwrap_or_else(|| gallery_dir.clone());
    let comic_pending_dir = get_setting(&conn, "offline_comic_pending_dir").unwrap_or_else(|| comic_dir.clone());
    let collection_pending_dir = get_setting(&conn, "offline_collection_pending_dir").unwrap_or_default();
    Ok(OfflineDownloadConfig {
        inbox_path,
        trash_path,
        movie_dir,
        comic_dir,
        gallery_dir,
        creator_pending_dir,
        gallery_pending_dir,
        comic_pending_dir,
        collection_pending_dir,
    })
}

pub fn save_offline_download_config(
    app: &AppHandle,
    inbox_path: &str,
    movie_dir: Option<&str>,
    comic_dir: Option<&str>,
    gallery_dir: Option<&str>,
    creator_pending_dir: Option<&str>,
    gallery_pending_dir: Option<&str>,
    comic_pending_dir: Option<&str>,
    collection_pending_dir: Option<&str>,
) -> Result<OfflineDownloadConfig, String> {
    let conn = crate::v2_config::open(app)?;
    ensure_schema(&conn)?;
    let clean_inbox = normalize_cloud_path(inbox_path, DEFAULT_INBOX_PATH);
    set_setting(&conn, "offline_inbox_path", &clean_inbox)?;
    if let Some(m) = movie_dir {
        set_setting(&conn, "offline_movie_dir", m.trim())?;
    }
    if let Some(c) = comic_dir {
        set_setting(&conn, "offline_comic_dir", c.trim())?;
    }
    if let Some(g) = gallery_dir {
        set_setting(&conn, "offline_gallery_dir", g.trim())?;
    }
    if let Some(v) = creator_pending_dir {
        set_setting(&conn, "offline_creator_pending_dir", v.trim())?;
    }
    if let Some(v) = gallery_pending_dir {
        set_setting(&conn, "offline_gallery_pending_dir", v.trim())?;
        set_setting(&conn, "offline_gallery_dir", v.trim())?;
    }
    if let Some(v) = comic_pending_dir {
        set_setting(&conn, "offline_comic_pending_dir", v.trim())?;
        set_setting(&conn, "offline_comic_dir", v.trim())?;
    }
    if let Some(v) = collection_pending_dir {
        set_setting(&conn, "offline_collection_pending_dir", v.trim())?;
    }
    get_offline_download_config(app)
}

fn resolve_cd2_grpc_credentials(conn: &Connection) -> Result<(String, String, String), String> {
    let raw_ep = get_setting(conn, "cd2_endpoint")
        .or_else(|| std::env::var("PML_CD2_URL").ok())
        .or_else(|| std::env::var("PML_CD2_ENDPOINT").ok())
        .unwrap_or_else(|| "http://127.0.0.1:19798".to_string());

    // Strip any /dav suffix to keep pure origin (e.g. http://127.0.0.1:19798)
    let origin = if let Ok(parsed) = reqwest::Url::parse(raw_ep.trim()) {
        let scheme = parsed.scheme();
        let host = parsed.host_str().unwrap_or("127.0.0.1");
        if let Some(port) = parsed.port() {
            format!("{scheme}://{host}:{port}")
        } else {
            format!("{scheme}://{host}")
        }
    } else {
        "http://127.0.0.1:19798".to_string()
    };

    let user = get_setting(conn, "cd2_user")
        .or_else(|| std::env::var("PML_CD2_USER").ok())
        .or_else(|| std::env::var("PML_WEBDAV_USER").ok())
        .unwrap_or_default()
        .trim()
        .to_string();
    let pass = get_setting(conn, "cd2_password")
        .or_else(|| std::env::var("PML_CD2_PASSWORD").ok())
        .or_else(|| std::env::var("PML_WEBDAV_PASSWORD").ok())
        .unwrap_or_default()
        .trim()
        .to_string();

    if user.is_empty() || pass.is_empty() {
        return Err("未检测到 CloudDrive2 账号密码（PML_CD2_USER / PML_CD2_PASSWORD）".to_string());
    }
    Ok((origin, user, pass))
}

// --- Minimal Zero-Dep Protobuf + gRPC-Web Helpers ---

fn pb_varint(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            b |= 0x80;
            out.push(b);
        } else {
            out.push(b);
            break;
        }
    }
}

fn pb_str(field_num: u32, s: &str, out: &mut Vec<u8>) {
    let bytes = s.as_bytes();
    pb_varint(((field_num as u64) << 3) | 2, out);
    pb_varint(bytes.len() as u64, out);
    out.extend_from_slice(bytes);
}

fn grpc_web_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(0u8);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

struct GrpcWebResponse {
    message: Vec<u8>,
    grpc_status: i32,
    grpc_message: String,
}

fn parse_grpc_web_body(body: &[u8], hdr_status: Option<i32>, hdr_msg: Option<String>) -> GrpcWebResponse {
    let mut message = Vec::new();
    let mut grpc_status = hdr_status.unwrap_or(0);
    let mut grpc_message = hdr_msg.unwrap_or_default();

    let mut pos = 0usize;
    while pos + 5 <= body.len() {
        let flag = body[pos];
        let len = u32::from_be_bytes([body[pos + 1], body[pos + 2], body[pos + 3], body[pos + 4]]) as usize;
        pos += 5;
        if pos + len > body.len() {
            break;
        }
        let slice = &body[pos..pos + len];
        pos += len;
        if (flag & 0x80) != 0 {
            // Trailer frame (ASCII HTTP headers separated by \r\n)
            if let Ok(txt) = std::str::from_utf8(slice) {
                for line in txt.lines() {
                    if let Some((k, v)) = line.split_once(':') {
                        let kl = k.trim().to_ascii_lowercase();
                        let val = v.trim();
                        if kl == "grpc-status" {
                            if let Ok(code) = val.parse::<i32>() {
                                grpc_status = code;
                            }
                        } else if kl == "grpc-message" && !val.is_empty() {
                            grpc_message = percent_encoding::percent_decode_str(val)
                                .decode_utf8_lossy()
                                .to_string();
                        }
                    }
                }
            }
        } else if message.is_empty() {
            message.extend_from_slice(slice);
        }
    }

    GrpcWebResponse {
        message,
        grpc_status,
        grpc_message,
    }
}

fn extract_pb_string_field(msg: &[u8], target_field: u32) -> Option<String> {
    let mut i = 0usize;
    while i < msg.len() {
        let (tag, next) = read_varint(msg, i)?;
        i = next;
        let field_num = (tag >> 3) as u32;
        let wire_type = tag & 7;
        match wire_type {
            0 => {
                let (_, n2) = read_varint(msg, i)?;
                i = n2;
            }
            2 => {
                let (len, n2) = read_varint(msg, i)?;
                i = n2;
                let end = i.checked_add(len as usize)?;
                if end > msg.len() {
                    return None;
                }
                if field_num == target_field {
                    return Some(String::from_utf8_lossy(&msg[i..end]).to_string());
                }
                i = end;
            }
            1 => i = i.checked_add(8)?,
            5 => i = i.checked_add(4)?,
            _ => return None,
        }
    }
    None
}

fn extract_pb_bool_field(msg: &[u8], target_field: u32) -> Option<bool> {
    let mut i = 0usize;
    while i < msg.len() {
        let (tag, next) = read_varint(msg, i)?;
        i = next;
        let field_num = (tag >> 3) as u32;
        let wire_type = tag & 7;
        match wire_type {
            0 => {
                let (val, n2) = read_varint(msg, i)?;
                if field_num == target_field {
                    return Some(val != 0);
                }
                i = n2;
            }
            2 => {
                let (len, n2) = read_varint(msg, i)?;
                i = n2.checked_add(len as usize)?;
            }
            1 => i = i.checked_add(8)?,
            5 => i = i.checked_add(4)?,
            _ => return None,
        }
    }
    None
}

fn read_varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let mut val = 0u64;
    let mut shift = 0u32;
    while pos < buf.len() && shift < 64 {
        let b = buf[pos];
        pos += 1;
        val |= ((b & 0x7f) as u64) << shift;
        if (b & 0x80) == 0 {
            return Some((val, pos));
        }
        shift += 7;
    }
    None
}

async fn cd2_grpc_call(
    client: &reqwest::Client,
    origin: &str,
    method: &str,
    payload: &[u8],
    token: Option<&str>,
) -> Result<GrpcWebResponse, String> {
    let url = format!("{origin}/clouddrive.CloudDriveFileSrv/{method}");
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/grpc-web+proto")
        .header("X-Grpc-Web", "1")
        .body(grpc_web_frame(payload));
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("连接 CloudDrive2 ({origin}) 失败：{e}"))?;
    let status = resp.status();
    let hdr_status = resp
        .headers()
        .get("grpc-status")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i32>().ok());
    let hdr_msg = resp
        .headers()
        .get("grpc-message")
        .and_then(|v| v.to_str().ok())
        .map(|s| percent_encoding::percent_decode_str(s).decode_utf8_lossy().to_string());
    let bytes = resp.bytes().await.map_err(|e| format!("读取 CloudDrive2 响应失败：{e}"))?;
    if !status.is_success() {
        return Err(format!("CloudDrive2 返回 HTTP {status}"));
    }
    Ok(parse_grpc_web_body(&bytes, hdr_status, hdr_msg))
}

async fn cd2_login(client: &reqwest::Client, origin: &str, user: &str, pass: &str) -> Result<String, String> {
    let mut payload = Vec::new();
    pb_str(1, user, &mut payload);
    pb_str(2, pass, &mut payload);
    let res = cd2_grpc_call(client, origin, "GetToken", &payload, None).await?;
    if res.grpc_status != 0 {
        return Err(format!("CloudDrive2 登录失败：{}", res.grpc_message));
    }
    let ok = extract_pb_bool_field(&res.message, 1).unwrap_or(false);
    if !ok {
        let err = extract_pb_string_field(&res.message, 2).unwrap_or_else(|| "账号或密码错误".to_string());
        return Err(format!("CloudDrive2 鉴权失败：{err}"));
    }
    extract_pb_string_field(&res.message, 3)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "CloudDrive2 未返回有效 Token".to_string())
}

async fn cd2_ensure_folder(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    folder_path: &str,
) -> Result<(), String> {
    // Split e.g. /115open/PML_Inbox -> parent="/115open", name="PML_Inbox"
    let clean = folder_path.trim_end_matches('/');
    let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        return Ok(());
    }
    // Create step by step from depth 2 (never touch root cloud mount)
    let mut current_parent = format!("/{}", parts[0]);
    for segment in &parts[1..] {
        let mut payload = Vec::new();
        pb_str(1, &current_parent, &mut payload);
        pb_str(2, segment, &mut payload);
        // CreateFolder is idempotent (returns error or already exists if present)
        let _ = cd2_grpc_call(client, origin, "CreateFolder", &payload, Some(token)).await;
        current_parent = format!("{current_parent}/{segment}");
    }
    Ok(())
}

fn normalize_magnet_url(raw: &str) -> Result<(String, String), String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("请输入磁力链接或资源地址".to_string());
    }
    if s.to_ascii_lowercase().starts_with("magnet:?") {
        let hash = extract_btih(s);
        return Ok((s.to_string(), hash));
    }
    if s.to_ascii_lowercase().starts_with("ed2k://")
        || s.to_ascii_lowercase().starts_with("http://")
        || s.to_ascii_lowercase().starts_with("https://")
    {
        return Ok((s.to_string(), String::new()));
    }
    // Bare 32-40 char infoHash
    if (s.len() == 40 || s.len() == 32) && s.chars().all(|c| c.is_ascii_alphanumeric()) {
        let lower = s.to_ascii_lowercase();
        return Ok((format!("magnet:?xt=urn:btih:{lower}"), lower));
    }
    Err("链接格式不支持，请粘贴 magnet:? 开头链接或 40 位 InfoHash".to_string())
}

fn extract_btih(magnet: &str) -> String {
    let lower = magnet.to_ascii_lowercase();
    if let Some(pos) = lower.find("btih:") {
        let tail = &lower[pos + 5..];
        let hash: String = tail.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        return hash;
    }
    String::new()
}

pub fn submit_offline_magnet(
    app: &AppHandle,
    req: SubmitOfflineMagnetRequest,
) -> Result<OfflineDownloadTask, String> {
    let (magnet_url, info_hash) = normalize_magnet_url(&req.magnet)?;
    let cfg = get_offline_download_config(app)?;
    let target_dir = normalize_cloud_path(
        req.target_dir.as_deref().unwrap_or(&cfg.inbox_path),
        &cfg.inbox_path,
    );
    let category = req
        .category
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("auto")
        .to_string();
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if !info_hash.is_empty() {
                format!("磁力任务 {}", &info_hash[..info_hash.len().min(10)])
            } else {
                "离线下载任务".to_string()
            }
        });

    let (origin, user, pass) = {
        let conn = crate::v2_config::open(app)?;
        resolve_cd2_grpc_credentials(&conn)?
    };

    let magnet_clone = magnet_url.clone();
    let target_clone = target_dir.clone();
    let submit_res: Result<String, String> = crate::rt::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;

        let token = cd2_login(&client, &origin, &user, &pass).await?;
        cd2_ensure_folder(&client, &origin, &token, &target_clone).await?;

        let mut payload = Vec::new();
        pb_str(1, &magnet_clone, &mut payload);
        pb_str(2, &target_clone, &mut payload);
        let res = cd2_grpc_call(&client, &origin, "AddOfflineFiles", &payload, Some(&token)).await?;
        if res.grpc_status != 0 {
            let detail = if res.grpc_message.is_empty() {
                format!("gRPC 错误码 {}", res.grpc_status)
            } else {
                res.grpc_message
            };
            return Err(format!("提交离线下载失败：{detail}"));
        }
        // FileOperationResult: field 1 = success (bool), field 2 = errorMessage (string)
        let op_ok = extract_pb_bool_field(&res.message, 1).unwrap_or(true);
        let op_msg = extract_pb_string_field(&res.message, 2).unwrap_or_default();
        if !op_ok && !op_msg.is_empty() {
            return Err(format!("网盘拒绝离线任务：{op_msg}"));
        }
        Ok(if op_msg.is_empty() {
            "已提交到 115 网盘离线队列".to_string()
        } else {
            op_msg
        })
    });

    let msg = submit_res?;
    let created_at = now_ts();
    let id = format!("off-{created_at}-{}", &info_hash[..info_hash.len().min(8)]);
    let task = OfflineDownloadTask {
        id: id.clone(),
        info_hash: info_hash.clone(),
        title: title.clone(),
        magnet: magnet_url.clone(),
        target_dir: target_dir.clone(),
        category: category.clone(),
        status: "submitted".to_string(),
        message: Some(msg.clone()),
        created_at,
    };

    let conn = crate::v2_config::open(app)?;
    ensure_schema(&conn)?;
    conn.execute(
        "INSERT INTO offline_download_tasks(id, info_hash, title, magnet, target_dir, category, status, message, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![id, info_hash, title, magnet_url, target_dir, category, "submitted", msg, created_at],
    )
    .map_err(|e| format!("记录离线任务失败：{e}"))?;

    Ok(task)
}

pub fn list_offline_download_tasks(app: &AppHandle, limit: i64) -> Result<Vec<OfflineDownloadTask>, String> {
    let conn = crate::v2_config::open(app)?;
    ensure_schema(&conn)?;
    let lim = limit.clamp(1, 100);
    let mut stmt = conn
        .prepare(
            "SELECT id, info_hash, title, magnet, target_dir, category, status, message, created_at
             FROM offline_download_tasks
             ORDER BY created_at DESC
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![lim], |r| {
            Ok(OfflineDownloadTask {
                id: r.get(0)?,
                info_hash: r.get(1)?,
                title: r.get(2)?,
                magnet: r.get(3)?,
                target_dir: r.get(4)?,
                category: r.get(5)?,
                status: r.get(6)?,
                message: r.get(7)?,
                created_at: r.get(8)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows.flatten() {
        out.push(r);
    }
    Ok(out)
}

pub fn delete_offline_download_task(app: &AppHandle, id: &str) -> Result<(), String> {
    let conn = crate::v2_config::open(app)?;
    ensure_schema(&conn)?;
    conn.execute("DELETE FROM offline_download_tasks WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn pb_int(field_num: u32, val: u64, out: &mut Vec<u8>) {
    pb_varint((field_num as u64) << 3, out);
    pb_varint(val, out);
}

#[derive(Debug)]
enum PbVal<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    F64(f64),
    F32(f32),
}

fn parse_pb_fields(buf: &[u8]) -> Vec<(u32, PbVal<'_>)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < buf.len() {
        let Some((tag, next)) = read_varint(buf, i) else { break };
        i = next;
        let field_num = (tag >> 3) as u32;
        let wire_type = tag & 7;
        match wire_type {
            0 => {
                let Some((v, n2)) = read_varint(buf, i) else { break };
                out.push((field_num, PbVal::Varint(v)));
                i = n2;
            }
            2 => {
                let Some((len, n2)) = read_varint(buf, i) else { break };
                i = n2;
                let Some(end) = i.checked_add(len as usize) else { break };
                if end > buf.len() { break; }
                out.push((field_num, PbVal::Bytes(&buf[i..end])));
                i = end;
            }
            1 => {
                if i + 8 > buf.len() { break; }
                let arr: [u8; 8] = buf[i..i + 8].try_into().unwrap();
                out.push((field_num, PbVal::F64(f64::from_le_bytes(arr))));
                i += 8;
            }
            5 => {
                if i + 4 > buf.len() { break; }
                let arr: [u8; 4] = buf[i..i + 4].try_into().unwrap();
                out.push((field_num, PbVal::F32(f32::from_le_bytes(arr))));
                i += 4;
            }
            _ => break,
        }
    }
    out
}

fn format_size_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "—".to_string();
    }
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut val = bytes as f64;
    let mut idx = 0usize;
    while val >= 1024.0 && idx < UNITS.len() - 1 {
        val /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{bytes} B")
    } else {
        format!("{val:.2} {}", UNITS[idx])
    }
}

async fn cd2_detect_cloud_account(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    preferred_mount: &str,
) -> Result<(String, String), String> {
    let mut payload = Vec::new();
    pb_str(1, "/", &mut payload);
    let res = cd2_grpc_call(client, origin, "GetSubFiles", &payload, Some(token)).await?;
    if res.grpc_status != 0 {
        return Err(format!("读取 CloudDrive2 挂载失败：{}", res.grpc_message));
    }
    let want_prefix = preferred_mount
        .trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let mut first_found: Option<(String, String)> = None;
    for (_, val) in parse_pb_fields(&res.message) {
        if let PbVal::Bytes(entry_bytes) = val {
            let mut mount_name = String::new();
            let mut cloud_name = String::new();
            let mut cloud_acct = String::new();
            for (sfn, sval) in parse_pb_fields(entry_bytes) {
                match (sfn, sval) {
                    (2, PbVal::Bytes(b)) => mount_name = String::from_utf8_lossy(b).to_string(),
                    (9, PbVal::Bytes(api_bytes)) => {
                        for (cfn, cval) in parse_pb_fields(api_bytes) {
                            match (cfn, cval) {
                                (1, PbVal::Bytes(b)) => cloud_name = String::from_utf8_lossy(b).to_string(),
                                (2, PbVal::Bytes(b)) => cloud_acct = String::from_utf8_lossy(b).to_string(),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            if !cloud_name.is_empty() && !cloud_acct.is_empty() {
                if !want_prefix.is_empty() && mount_name.to_lowercase() == want_prefix {
                    return Ok((cloud_name, cloud_acct));
                }
                if first_found.is_none() {
                    first_found = Some((cloud_name, cloud_acct));
                }
            }
        }
    }
    first_found.ok_or_else(|| "未在 CloudDrive2 中找到已挂载的云盘账号".to_string())
}

pub fn get_offline_cloud_status(app: &AppHandle, page: i32) -> Result<OfflineCloudStatusResponse, String> {
    let cfg = get_offline_download_config(app)?;
    let (origin, user, pass) = {
        let conn = crate::v2_config::open(app)?;
        resolve_cd2_grpc_credentials(&conn)?
    };
    let page_num = page.max(1) as u64;
    let inbox_path = cfg.inbox_path.clone();

    crate::rt::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(18))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;

        let token = cd2_login(&client, &origin, &user, &pass).await?;
        let (cloud_name, cloud_acct) = cd2_detect_cloud_account(&client, &origin, &token, &inbox_path).await?;

        // 1. Quota
        let mut q_payload = Vec::new();
        pb_str(1, &cloud_name, &mut q_payload);
        pb_str(2, &cloud_acct, &mut q_payload);
        let q_res = cd2_grpc_call(&client, &origin, "GetOfflineQuotaInfo", &q_payload, Some(&token)).await?;
        let mut quota = OfflineQuotaInfo { total: 0, used: 0, left: 0 };
        if q_res.grpc_status == 0 {
            for (fn_num, val) in parse_pb_fields(&q_res.message) {
                if let PbVal::Varint(v) = val {
                    match fn_num {
                        1 => quota.total = v as i64,
                        2 => quota.used = v as i64,
                        3 => quota.left = v as i64,
                        _ => {}
                    }
                }
            }
        }

        // 2. Paginated offline tasks
        let mut l_payload = Vec::new();
        pb_str(1, &cloud_name, &mut l_payload);
        pb_str(2, &cloud_acct, &mut l_payload);
        pb_int(3, page_num, &mut l_payload);
        let l_res = cd2_grpc_call(&client, &origin, "ListAllOfflineFiles", &l_payload, Some(&token)).await?;
        if l_res.grpc_status != 0 {
            return Err(format!("读取云下载任务失败：{}", l_res.grpc_message));
        }

        let mut resp_page = page_num as i32;
        let mut page_size = 30i32;
        let mut page_count = 1i32;
        let mut total_count = 0i64;
        let mut items = Vec::new();

        for (fn_num, val) in parse_pb_fields(&l_res.message) {
            match (fn_num, val) {
                (1, PbVal::Varint(v)) => resp_page = v as i32,
                (2, PbVal::Varint(v)) => page_size = v as i32,
                (3, PbVal::Varint(v)) => page_count = v as i32,
                (4, PbVal::Varint(v)) => total_count = v as i64,
                (6, PbVal::Bytes(entry_bytes)) => {
                    let mut name = String::new();
                    let mut size_bytes = 0u64;
                    let mut url = String::new();
                    let mut status = 0i32;
                    let mut info_hash = String::new();
                    let mut file_id = String::new();
                    let mut add_time = 0i64;
                    let mut percent_done = 0.0f64;

                    for (efn, eval) in parse_pb_fields(entry_bytes) {
                        match (efn, eval) {
                            (1, PbVal::Bytes(b)) => name = String::from_utf8_lossy(b).to_string(),
                            (2, PbVal::Varint(v)) => size_bytes = v,
                            (3, PbVal::Bytes(b)) => url = String::from_utf8_lossy(b).to_string(),
                            (4, PbVal::Varint(v)) => status = v as i32,
                            (5, PbVal::Bytes(b)) => info_hash = String::from_utf8_lossy(b).to_string(),
                            (6, PbVal::Bytes(b)) => file_id = String::from_utf8_lossy(b).to_string(),
                            (7, PbVal::Varint(v)) => add_time = v as i64,
                            (9, PbVal::F64(f)) => percent_done = f,
                            (9, PbVal::F32(f)) => percent_done = f as f64,
                            _ => {}
                        }
                    }
                    let status_category = match status {
                        2 => "completed",
                        0 | 1 => "downloading",
                        _ => "failed",
                    }
                    .to_string();
                    items.push(CloudOfflineFile {
                        name: if name.is_empty() { info_hash.clone() } else { name },
                        size_bytes,
                        size_text: format_size_bytes(size_bytes),
                        url,
                        status,
                        status_category,
                        info_hash,
                        file_id,
                        add_time,
                        percent_done: percent_done.clamp(0.0, 100.0),
                    });
                }
                _ => {}
            }
        }

        Ok(OfflineCloudStatusResponse {
            cloud_name,
            quota,
            page: resp_page,
            page_size,
            page_count,
            total_count,
            items,
        })
    })
}

pub fn submit_offline_magnets_batch(
    app: &AppHandle,
    urls_text: &str,
    target_dir: Option<&str>,
) -> Result<BatchOfflineSubmitResponse, String> {
    let cfg = get_offline_download_config(app)?;
    let clean_target = normalize_cloud_path(target_dir.unwrap_or(&cfg.inbox_path), &cfg.inbox_path);

    let mut lines: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for raw_line in urls_text.lines() {
        let s = raw_line.trim();
        if s.is_empty() {
            continue;
        }
        if seen.insert(s.to_ascii_lowercase()) {
            lines.push(s.to_string());
        }
    }
    if lines.is_empty() {
        return Err("请至少粘贴一条磁力链接或资源地址".to_string());
    }
    if lines.len() > 20 {
        return Err("为防止触发 115 风控，单次批量提交上限为 20 条链接".to_string());
    }

    let mut results = Vec::new();
    let mut succeeded = 0usize;
    let mut failed = 0usize;

    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            // Anti-risk serial pacing: 550ms between each submission
            std::thread::sleep(Duration::from_millis(550));
        }
        match submit_offline_magnet(
            app,
            SubmitOfflineMagnetRequest {
                magnet: line.clone(),
                title: None,
                target_dir: Some(clean_target.clone()),
                category: Some("auto".to_string()),
            },
        ) {
            Ok(task) => {
                succeeded += 1;
                results.push(BatchOfflineSubmitItemResult {
                    input: line.clone(),
                    ok: true,
                    message: task.message.unwrap_or_else(|| "已提交".to_string()),
                });
            }
            Err(err) => {
                failed += 1;
                let rate_limited = err.contains("频繁") || err.contains("风控") || err.contains("验证") || err.contains("429");
                results.push(BatchOfflineSubmitItemResult {
                    input: line.clone(),
                    ok: false,
                    message: err,
                });
                if rate_limited {
                    // Circuit breaker: stop immediately on rate limit
                    break;
                }
            }
        }
    }

    Ok(BatchOfflineSubmitResponse {
        total: lines.len(),
        succeeded,
        failed,
        target_dir: clean_target,
        results,
    })
}

// --- Inbox Scan, Safe Serial Clean (_Trash) & Archive ---

#[derive(Debug, Clone)]
struct Cd2Entry {
    name: String,
    full_path: String,
    size_bytes: u64,
    is_dir: bool,
}

async fn cd2_list_sub_files(
    client: &reqwest::Client,
    origin: &str,
    token: &str,
    dir_path: &str,
) -> Result<Vec<Cd2Entry>, String> {
    let mut payload = Vec::new();
    pb_str(1, dir_path, &mut payload);
    let res = cd2_grpc_call(client, origin, "GetSubFiles", &payload, Some(token)).await?;
    if res.grpc_status != 0 {
        return Err(format!("读取目录 {dir_path} 失败：{}", res.grpc_message));
    }
    let mut out = Vec::new();
    for (_, val) in parse_pb_fields(&res.message) {
        if let PbVal::Bytes(entry_bytes) = val {
            let mut name = String::new();
            let mut full_path = String::new();
            let mut size_bytes = 0u64;
            let mut file_type = 0u64;
            let mut is_dir_flag = false;
            for (efn, eval) in parse_pb_fields(entry_bytes) {
                match (efn, eval) {
                    (2, PbVal::Bytes(b)) => name = String::from_utf8_lossy(b).to_string(),
                    (3, PbVal::Bytes(b)) => full_path = String::from_utf8_lossy(b).to_string(),
                    (4, PbVal::Varint(v)) => size_bytes = v,
                    (5, PbVal::Varint(v)) => file_type = v,
                    (30, PbVal::Varint(v)) => is_dir_flag = v != 0,
                    _ => {}
                }
            }
            if !name.is_empty() && !full_path.is_empty() {
                out.push(Cd2Entry {
                    name,
                    full_path,
                    size_bytes,
                    is_dir: is_dir_flag || file_type == 0,
                });
            }
        }
    }
    Ok(out)
}

fn file_ext_lower(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

fn is_video_ext(ext: &str) -> bool {
    matches!(ext, "mp4" | "mkv" | "avi" | "wmv" | "mov" | "ts" | "m4v" | "flv" | "rmvb" | "webm")
}

fn is_image_ext(ext: &str) -> bool {
    matches!(ext, "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "bmp")
}

fn is_archive_ext(ext: &str) -> bool {
    matches!(ext, "zip" | "cbz" | "rar" | "cbr" | "7z")
}

fn looks_like_multipart_video(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.contains("part")
        || lower.contains("cd1")
        || lower.contains("cd2")
        || lower.contains("cd3")
        || lower.contains("ep")
        || lower.contains("v(")
        || lower.contains(" (1)")
        || lower.contains(" (2)")
        || lower.contains(" (3)")
        || lower.contains(" (4)")
        || lower.contains("(1).")
        || lower.contains("(2).")
        || lower.contains("(3).")
        || lower.contains("(4).")
    {
        return true;
    }
    false
}

fn is_ad_dir_name(dir_name: &str) -> bool {
    let lower = dir_name.to_ascii_lowercase();
    const AD_DIR_KEYWORDS: &[&str] = &[
        "约炮", "約炮", "同城上门", "同城上門", "同城约", "同城約", "同城交友",
        "全国可飞", "全國可飛", "品茶", "广告", "廣告", "宣传", "宣傳",
        "防走失", "最新地址", "永久地址", "备用地址", "備用地址", "发布器", "發布器",
    ];
    AD_DIR_KEYWORDS.iter().any(|k| lower.contains(k))
}

fn normalize_similarity_token(raw: &str) -> String {
    let mut s = raw.to_ascii_lowercase();
    // Strip trailing part/volume counters like (2V), (3P), (1), [2V]
    for (open, close) in [('(', ')'), ('[', ']'), ('（', '）'), ('【', '】')] {
        loop {
            let trimmed = s.trim_end();
            if trimmed.ends_with(close) {
                if let Some(idx) = trimmed.rfind(open) {
                    let inner = trimmed[idx + open.len_utf8()..trimmed.len() - close.len_utf8()].trim();
                    let is_count_tag = inner.len() <= 6
                        && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == ' ');
                    if is_count_tag {
                        s = trimmed[..idx].to_string();
                        continue;
                    }
                }
            }
            break;
        }
    }
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(c))
        .collect()
}

/// Returns true if `sub_name` is a redundant wrapper folder of `parent_name`
/// (e.g. `推特裸舞小美女【yum707玉米】8月VIP裸舞福利-3❤️最受不了乳摇了(2V)` inside `推特裸舞小美女【yum707玉米】8月VIP裸舞福利-3 最受不了乳摇了`).
fn is_redundant_same_named_subdir(parent_name: &str, sub_name: &str) -> bool {
    let a = normalize_similarity_token(&strip_site_watermarks(parent_name, false));
    let b = normalize_similarity_token(&strip_site_watermarks(sub_name, false));
    if a.chars().count() < 6 || b.chars().count() < 6 {
        return false;
    }
    if a == b || a.contains(&b) || b.contains(&a) {
        return true;
    }
    // Check shared prefix ratio (>= 70% of shorter normalized string)
    let min_len = a.chars().count().min(b.chars().count());
    let shared_prefix = a
        .chars()
        .zip(b.chars())
        .take_while(|(ca, cb)| ca == cb)
        .count();
    shared_prefix * 10 >= min_len * 7
}

fn stem_overlaps_folder(stem: &str, folder_core: &str) -> bool {
    let s = normalize_similarity_token(stem);
    let f = normalize_similarity_token(folder_core);
    if s.chars().count() < 4 || f.chars().count() < 4 {
        return false;
    }
    if s.contains(&f) || f.contains(&s) {
        return true;
    }
    let min_len = s.chars().count().min(f.chars().count());
    let shared_prefix = s
        .chars()
        .zip(f.chars())
        .take_while(|(ca, cb)| ca == cb)
        .count();
    shared_prefix >= 6 && shared_prefix * 2 >= min_len
}

fn extract_explicit_jav_number(raw: &str) -> Option<String> {
    let stripped = strip_site_watermarks(raw, false);
    let lower = stripped.to_ascii_lowercase();
    const NON_JAV_KEYWORDS: &[&str] = &[
        "推特", "twitter", "onlyfans", "fansly", "patreon", "网红", "網紅", "博主", "福利姬",
        "定制", "私拍", "裸舞", "微密", "秀人", "写真", "寫真", "套图", "套圖", "合集", "同人", "汉化", "漢化",
        "漫画", "漫畫", "图集", "圖集",
    ];
    if NON_JAV_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return None;
    }
    let num = crate::jav_scraper::extract_jav_number(&stripped)?;
    // Require the explicit hyphenated code to actually appear in the source name
    // (prevents creator IDs like "yum707" from being misread as "YUM-707").
    if stripped.to_ascii_uppercase().contains(&num) {
        Some(num)
    } else {
        None
    }
}

fn split_stem_ext(raw: &str) -> (String, Option<String>) {
    match raw.rsplit_once('.') {
        Some((st, ex)) if !ex.is_empty() && ex.len() <= 6 && ex.chars().all(|c| c.is_ascii_alphanumeric()) => {
            (st.to_string(), Some(ex.to_string()))
        }
        _ => (raw.to_string(), None),
    }
}

fn has_embedded_domain_token(text: &str) -> bool {
    let l = text.to_ascii_lowercase();
    const TLDS: &[&str] = &[
        ".com", ".net", ".org", ".la", ".cc", ".me", ".xyz", ".top", ".vip", ".tv", ".club", ".info", ".pro", ".io", ".cn",
    ];
    if l.contains("www.") || l.contains("http://") || l.contains("https://") {
        return true;
    }
    for tld in TLDS {
        let mut rest = l.as_str();
        while let Some(idx) = rest.find(tld) {
            let before = &rest[..idx];
            let after = &rest[idx + tld.len()..];
            let has_host_char = before
                .chars()
                .next_back()
                .map(|c| c.is_ascii_alphanumeric() || c == '-')
                .unwrap_or(false);
            let boundary_after = after
                .chars()
                .next()
                .map(|c| !c.is_ascii_alphanumeric())
                .unwrap_or(true);
            if has_host_char && boundary_after {
                return true;
            }
            rest = &rest[idx + tld.len()..];
        }
    }
    false
}

fn junk_reason_for_file(
    name: &str,
    full_path: &str,
    root_path: &str,
    size_bytes: u64,
    jav_code: Option<&str>,
    has_large_movie: bool,
    max_video_bytes: u64,
    has_any_real_video: bool,
    folder_core_name: &str,
    is_cross_folder_dup: bool,
) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    let (stem_raw, ext_opt) = split_stem_ext(name);
    let ext = ext_opt.as_deref().unwrap_or("").to_ascii_lowercase();

    if matches!(
        ext.as_str(),
        "txt" | "url" | "html" | "htm" | "mht" | "lnk" | "nfo" | "apk" | "exe" | "bat" | "chm" | "torrent"
    ) || lower == ".ds_store"
        || lower == "thumbs.db"
    {
        return Some("广告/说明文档");
    }

    // Check if file is nested inside an obvious ad subdirectory (e.g. "全国各地约炮 同城上门做爱/")
    if let Some(rel) = full_path.strip_prefix(root_path).map(|s| s.trim_start_matches('/')) {
        if let Some((parent_rel, _)) = rel.rsplit_once('/') {
            if parent_rel.split('/').any(is_ad_dir_name) {
                return Some("广告目录内文件");
            }
        }
    }

    const AD_KEYWORDS: &[&str] = &[
        "扫码", "掃碼", "扫描访问", "掃描訪問", "二维码", "二維碼", "qr码", "qr碼", "2048qr",
        "约炮", "約炮", "品茶", "同城上门", "同城上門", "同城约", "同城約", "同城交友", "同城一", "yq交友",
        "全国可飞", "全國可飛", "网址", "網址", "宣传", "宣傳", "广告", "廣告",
        "发布器", "發布器", "地址发布", "地址發布", "发布页", "發布頁",
        "最新地址", "最新位址", "永久地址", "备用地址", "備用地址", "防走失", "防屏蔽", "免责", "免責",
        "点关注", "點關注", "不迷路", "杏吧", "工口mh",
        "最新情报", "最新情報", "决胜负", "決勝負", "苍老师", "蒼老師", "强力推荐", "強力推薦", "三上悠亚", "三上悠亞",
        "18禁手游", "18禁手遊", "免费18禁", "免費18禁", "h手游", "h手遊",
        "核工厂", "核工廠", "色中色", "痴汉俱乐部", "痴漢俱樂部", "公仔箱",
        "草榴", "t66y", "1024社区", "1024社區", "2048社区", "2048社區", "2048地址", "sehuatang", "色花堂", "98堂",
        "uu直播", "美少女直播", "美女直播", "美女聊天", "线上美女", "線上美女", "台妹子", "花式表演", "聚合全网", "聚合全網",
        "信誉保证", "信譽保證", "20年信誉", "20年信譽", "服务全球", "服務全球", "等你来选", "等你來選",
        "激情随时看", "激情隨時看", "点击进入", "點擊進入", "有惊喜", "有驚喜",
        "点击访问", "點擊訪問", "免费看", "免費看", "一手资源", "一手資源", "每日更新", "最新最快",
        "ai脱衣", "ai脫衣", "一键脱衣", "一鍵脫衣", "ai换脸", "ai換臉", "视频换脸", "視頻換臉",
        "脱光衣服", "脫光衣服", "ai创作网站", "脱光心仪", "黃果短劇", "黄果短剧", "每日更新万部",
        "91porn", "a8uu.com", "a8nn.com", "xuu92", "98t.la", "sis001",
        "澳门", "澳門", "威尼斯人", "新葡京", "赌场", "賭場", "在线荷官", "在線荷官", "荷官", "博彩", "棋牌", "娱乐城", "娛樂城",
        "福利机置", "体育电竞", "體育電競",
        "裸聊", "一夜情", "加qq", "加微信", "微信号", "微信號", "电报群", "電報群", "飞机群", "飛機群",
        "精品资源免费下载", "免费下载", "免費下載", "资源免费", "資源免費",
    ];
    if name.contains("★★★") || AD_KEYWORDS.iter().any(|k| lower.contains(k) || compact.contains(k)) {
        return Some("疑似广告文件");
    }

    let stripped_file = strip_site_watermarks(name, true);
    let (stripped_stem, _) = split_stem_ext(&stripped_file);
    let matches_folder = stem_overlaps_folder(&stripped_stem, folder_core_name)
        || stem_overlaps_folder(&stem_raw, folder_core_name);

    // Embedded domain inside filename (e.g. "xxx suu37.com.mp4", "xxx auu32.com.gif") after removing legitimate leading site prefix
    if !matches_folder && has_embedded_domain_token(&stripped_stem) {
        let is_small_media = (is_video_ext(&ext) && size_bytes < 47_185_920)
            || ext == "gif"
            || (is_image_ext(&ext) && size_bytes < 5_242_880)
            || (is_archive_ext(&ext) && size_bytes < 10_485_760);
        if is_small_media {
            return Some("含推广域名的广告文件");
        }
    }

    // Small launcher / ad archives bundled inside video folders (e.g. _1024社区发布器 10.1版.zip, 2048地址发布器PC版.rar)
    if has_any_real_video && is_archive_ext(&ext) && size_bytes > 0 && size_bytes < 5_242_880 && !matches_folder {
        const ARCHIVE_AD_TOKENS: &[&str] = &[
            "发布", "發布", "地址", "社区", "社區", "pc版", "手机版", "手機版", "安卓", "1024", "2048", "app", "说明", "說明", "防走失", "网址", "網址",
        ];
        if ARCHIVE_AD_TOKENS.iter().any(|k| lower.contains(k)) {
            return Some("推广发布器压缩包");
        }
    }

    // Promo .gif banners inside video folders
    if has_any_real_video && ext == "gif" && size_bytes > 0 && size_bytes < 10_485_760 && !matches_folder {
        return Some("疑似推广动图广告");
    }

    // Cross-folder identical boilerplate spam across multiple inbox folders
    if is_cross_folder_dup
        && size_bytes > 0
        && size_bytes < 47_185_920
        && !looks_like_multipart_video(name)
        && !matches_folder
    {
        return Some("多目录重复推广文件");
    }

    // Unrelated intro/promo clips inside confirmed explicit JAV movie folders
    if let Some(code) = jav_code {
        if has_large_movie
            && is_video_ext(&ext)
            && size_bytes > 0
            && !looks_like_multipart_video(name)
            && !matches_folder
            && !name.to_ascii_uppercase().contains(code)
            && (size_bytes < 26_214_400
                || (size_bytes < 125_829_120 && size_bytes.saturating_mul(8) <= max_video_bytes))
        {
            return Some("疑似片头广告切片");
        }
    }

    None
}

fn looks_like_site_token(inner: &str) -> bool {
    let l = inner.to_ascii_lowercase();
    const TLDS: &[&str] = &[".com", ".net", ".org", ".la", ".cc", ".me", ".xyz", ".top", ".vip", ".tv", ".club", ".info", ".pro", ".io", ".cn"];
    l.contains("www.") || l.contains("http") || TLDS.iter().any(|t| l.contains(t))
}

fn strip_site_watermarks(raw: &str, is_file: bool) -> String {
    let (stem_orig, ext_opt) = if is_file {
        split_stem_ext(raw)
    } else {
        (raw.to_string(), None)
    };

    let mut s = stem_orig.trim().to_string();

    // 1. Strip bracketed site watermarks anywhere first: 【mitaocang.com】, [www.98T.la], (xxx.com)
    // Running this before leading domain stripping ensures "【98t.la】www.demo.com_title"
    // and "2048.vip-【98t.la】www.demo.com_title" strip both bracketed and unbracketed domains cleanly.
    for (open, close) in [('【', '】'), ('[', ']'), ('(', ')')] {
        let mut out = String::new();
        let mut rest = s.as_str();
        while let Some(o_idx) = rest.find(open) {
            out.push_str(&rest[..o_idx]);
            let after_open = &rest[o_idx + open.len_utf8()..];
            if let Some(c_idx) = after_open.find(close) {
                let inside = &after_open[..c_idx];
                if !looks_like_site_token(inside) {
                    out.push(open);
                    out.push_str(inside);
                    out.push(close);
                }
                rest = &after_open[c_idx + close.len_utf8()..];
            } else {
                out.push(open);
                rest = after_open;
                break;
            }
        }
        out.push_str(rest);
        s = out;
    }
    s = s
        .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '_' | '@' | '+'))
        .to_string();

    // 2. Strip leading domain prefixes separated by '@', '-', '_', '+', or whitespace
    // e.g. "2048.vip-推特..." -> "推特...", "hhd800.com@ABP-123" -> "ABP-123", "www.98T.la_xxx" -> "xxx"
    loop {
        let mut stripped_prefix = false;
        let bytes = s.as_bytes();
        let mut sep_pos: Option<(usize, usize)> = None;
        for (idx, ch) in s.char_indices() {
            if idx > 36 {
                break;
            }
            if matches!(ch, '@' | '-' | '_' | '+' | ' ') {
                let candidate = &s[..idx];
                if looks_like_site_token(candidate) {
                    let mut end = idx + ch.len_utf8();
                    while end < bytes.len() && matches!(bytes[end] as char, ' ' | '-' | '_' | '@' | '+') {
                        end += 1;
                    }
                    if end < s.len() {
                        sep_pos = Some((idx, end));
                    }
                }
                break;
            }
        }
        if let Some((_, after)) = sep_pos {
            let remainder = s[after..].trim().to_string();
            if !remainder.is_empty() {
                s = remainder;
                stripped_prefix = true;
            }
        }
        if !stripped_prefix {
            break;
        }
    }

    // 3. Clean stray leading/trailing punctuation and collapse double spaces
    let cleaned = s
        .trim_matches(|c: char| c == '$' || c == '-' || c == '_' || c.is_whitespace())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let final_stem = if cleaned.is_empty() { stem_orig } else { cleaned };
    match ext_opt {
        Some(ext) => format!("{final_stem}.{ext}"),
        None => final_stem,
    }
}

fn clean_display_name(raw: &str, suggested_category: &str) -> String {
    let stripped = strip_site_watermarks(raw, false);
    if suggested_category == "movie" {
        if let Some(num) = extract_explicit_jav_number(&stripped) {
            return num;
        }
    }
    stripped
}

pub fn scan_offline_inbox(app: &AppHandle) -> Result<InboxScanResponse, String> {
    let cfg = get_offline_download_config(app)?;
    let (origin, user, pass) = {
        let conn = crate::v2_config::open(app)?;
        resolve_cd2_grpc_credentials(&conn)?
    };
    let inbox_path = cfg.inbox_path.clone();
    let trash_path = cfg.trash_path.clone();
    let cfg_clone = cfg.clone();

    crate::rt::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;

        let token = cd2_login(&client, &origin, &user, &pass).await?;
        cd2_ensure_folder(&client, &origin, &token, &inbox_path).await?;
        let top_entries = cd2_list_sub_files(&client, &origin, &token, &inbox_path).await?;

        struct CollectedTopEntry {
            entry: Cd2Entry,
            all_files: Vec<Cd2Entry>,
            ad_dirs: Vec<Cd2Entry>,
            dir_children: std::collections::HashMap<String, Vec<Cd2Entry>>,
        }

        // Pass 1: collect directory trees and count cross-folder duplicate boilerplate files
        let mut collected: Vec<CollectedTopEntry> = Vec::new();
        let mut cross_folder_counts: std::collections::HashMap<(String, u64), usize> = std::collections::HashMap::new();

        for entry in top_entries {
            if entry.name == "_Trash" || entry.name.starts_with('.') {
                continue;
            }

            let mut all_files: Vec<Cd2Entry> = Vec::new();
            let mut ad_dirs: Vec<Cd2Entry> = Vec::new();
            let mut dir_children: std::collections::HashMap<String, Vec<Cd2Entry>> = std::collections::HashMap::new();
            if entry.is_dir {
                let mut queue: Vec<(String, u8)> = vec![(entry.full_path.clone(), 0)];
                while let Some((dir, depth)) = queue.pop() {
                    if depth > 2 {
                        continue;
                    }
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    if let Ok(children) = cd2_list_sub_files(&client, &origin, &token, &dir).await {
                        for ch in &children {
                            if ch.is_dir {
                                if is_ad_dir_name(&ch.name) {
                                    ad_dirs.push(ch.clone());
                                } else {
                                    queue.push((ch.full_path.clone(), depth + 1));
                                }
                            } else {
                                all_files.push(ch.clone());
                            }
                        }
                        dir_children.insert(dir, children);
                    }
                }
            } else {
                all_files.push(entry.clone());
            }

            if entry.is_dir {
                let mut seen_in_entry = std::collections::HashSet::new();
                for f in &all_files {
                    let key = (f.name.to_ascii_lowercase(), f.size_bytes);
                    if seen_in_entry.insert(key.clone()) {
                        *cross_folder_counts.entry(key).or_insert(0) += 1;
                    }
                }
            }

            collected.push(CollectedTopEntry {
                entry,
                all_files,
                ad_dirs,
                dir_children,
            });
        }

        let top_dir_count = collected.iter().filter(|c| c.entry.is_dir).count();
        let mut plan_items = Vec::new();

        // Pass 2: classify kept/junk files, wrapper subfolders, renames, and category suggestions
        for item in collected {
            let entry = item.entry;
            let all_files = item.all_files;
            let ad_dirs = item.ad_dirs;
            let dir_children = item.dir_children;

            let folder_core_name = strip_site_watermarks(&entry.name, !entry.is_dir);
            let has_main_video = all_files
                .iter()
                .any(|f| is_video_ext(&file_ext_lower(&f.name)) && f.size_bytes >= 52_428_800);
            let has_large_movie = all_files
                .iter()
                .any(|f| is_video_ext(&file_ext_lower(&f.name)) && f.size_bytes >= 314_572_800);
            let max_video_bytes: u64 = all_files
                .iter()
                .filter(|f| is_video_ext(&file_ext_lower(&f.name)))
                .map(|f| f.size_bytes)
                .max()
                .unwrap_or(0);
            let has_any_real_video = has_main_video
                || all_files.iter().any(|f| {
                    is_video_ext(&file_ext_lower(&f.name))
                        && (f.size_bytes >= 31_457_280 || stem_overlaps_folder(&strip_site_watermarks(&f.name, true), &folder_core_name))
                });
            let explicit_jav_num = extract_explicit_jav_number(&entry.name);

            let mut kept_files = Vec::new();
            let mut junk_files = Vec::new();
            let mut junk_set = std::collections::HashSet::new();
            let mut total_size = 0u64;

            for f in &all_files {
                total_size = total_size.saturating_add(f.size_bytes);
                let is_cross_folder_dup = top_dir_count >= 2
                    && cross_folder_counts
                        .get(&(f.name.to_ascii_lowercase(), f.size_bytes))
                        .copied()
                        .unwrap_or(0)
                        >= 2;
                if let Some(reason) = junk_reason_for_file(
                    &f.name,
                    &f.full_path,
                    &entry.full_path,
                    f.size_bytes,
                    explicit_jav_num.as_deref(),
                    has_large_movie,
                    max_video_bytes,
                    has_any_real_video,
                    &folder_core_name,
                    is_cross_folder_dup,
                ) {
                    junk_set.insert(f.full_path.clone());
                    junk_files.push(InboxFileEntry {
                        name: f.name.clone(),
                        full_path: f.full_path.clone(),
                        size_bytes: f.size_bytes,
                        size_text: format_size_bytes(f.size_bytes),
                        reason: reason.to_string(),
                    });
                } else {
                    kept_files.push(InboxFileEntry {
                        name: f.name.clone(),
                        full_path: f.full_path.clone(),
                        size_bytes: f.size_bytes,
                        size_text: format_size_bytes(f.size_bytes),
                        reason: String::new(),
                    });
                }
            }

            for d in &ad_dirs {
                junk_set.insert(d.full_path.clone());
                junk_files.push(InboxFileEntry {
                    name: format!("{}/", d.name),
                    full_path: d.full_path.clone(),
                    size_bytes: d.size_bytes,
                    size_text: "目录".to_string(),
                    reason: "广告子目录".to_string(),
                });
            }

            // Detect single-chain redundant wrapper folders OR single redundant same-named subfolder
            let mut flatten_source_dir: Option<String> = None;
            let mut flatten_wrapper_dir: Option<String> = None;
            let mut flatten_levels = 0usize;
            if entry.is_dir {
                let mut cur = entry.full_path.clone();
                let mut cur_name = entry.name.clone();
                while let Some(children) = dir_children.get(&cur) {
                    let sub_dirs: Vec<&Cd2Entry> = children
                        .iter()
                        .filter(|c| c.is_dir && !junk_set.contains(&c.full_path))
                        .collect();
                    let non_junk_files_here: Vec<&Cd2Entry> = children
                        .iter()
                        .filter(|c| !c.is_dir && !junk_set.contains(&c.full_path))
                        .collect();
                    if sub_dirs.len() == 1 {
                        let only_sub = sub_dirs[0];
                        let empty_shell = non_junk_files_here.is_empty();
                        let redundant_same = is_redundant_same_named_subdir(&cur_name, &only_sub.name)
                            && dir_children
                                .get(&only_sub.full_path)
                                .map(|inner| {
                                    let existing_names: std::collections::HashSet<String> = non_junk_files_here
                                        .iter()
                                        .map(|f| f.name.to_ascii_lowercase())
                                        .collect();
                                    !inner.iter().any(|c| existing_names.contains(&c.name.to_ascii_lowercase()))
                                })
                                .unwrap_or(false);
                        if empty_shell || redundant_same {
                            if flatten_wrapper_dir.is_none() {
                                flatten_wrapper_dir = Some(only_sub.full_path.clone());
                            }
                            cur = only_sub.full_path.clone();
                            cur_name = only_sub.name.clone();
                            flatten_source_dir = Some(cur.clone());
                            flatten_levels += 1;
                            continue;
                        }
                    }
                    break;
                }
            }

            // Detect kept files with website watermarks in their names
            let mut file_renames = Vec::new();
            for kf in &kept_files {
                let cleaned = strip_site_watermarks(&kf.name, true);
                if cleaned != kf.name && !cleaned.is_empty() {
                    file_renames.push(InboxFileRename {
                        full_path: kf.full_path.clone(),
                        old_name: kf.name.clone(),
                        new_name: cleaned,
                    });
                }
            }

            let kept_videos = kept_files.iter().filter(|f| is_video_ext(&file_ext_lower(&f.name))).count();
            let kept_images = kept_files.iter().filter(|f| is_image_ext(&file_ext_lower(&f.name))).count();
            let kept_comic_archives = kept_files
                .iter()
                .filter(|f| is_archive_ext(&file_ext_lower(&f.name)) && (kept_videos == 0 || f.size_bytes >= 2_097_152))
                .count();

            let lower_name = entry.name.to_ascii_lowercase();
            let has_jav_num = explicit_jav_num.is_some();
            let suggested_category = if has_jav_num && (has_main_video || kept_videos > 0) {
                "movie"
            } else if lower_name.contains("写真") || lower_name.contains("套图") || lower_name.contains("cos") || lower_name.contains("私房") {
                "gallery"
            } else if kept_comic_archives > 0 || (kept_videos == 0 && kept_images >= 8) {
                "comic"
            } else if has_main_video || kept_videos > 0 {
                "creator"
            } else {
                "collection"
            }
            .to_string();

            let suggested_name = clean_display_name(&entry.name, &suggested_category);
            let needs_clean = !junk_files.is_empty()
                || !file_renames.is_empty()
                || flatten_levels > 0
                || suggested_name != entry.name;

            plan_items.push(InboxPlanItem {
                name: entry.name,
                full_path: entry.full_path,
                is_dir: entry.is_dir,
                total_size_bytes: total_size,
                total_size_text: format_size_bytes(total_size),
                suggested_name,
                suggested_category,
                needs_clean,
                flatten_source_dir,
                flatten_wrapper_dir,
                flatten_levels,
                kept_files,
                junk_files,
                file_renames,
            });
        }

        Ok(InboxScanResponse {
            inbox_path,
            trash_path,
            config: cfg_clone,
            items: plan_items,
        })
    })
}

pub fn execute_inbox_clean(
    app: &AppHandle,
    req: CleanInboxItemRequest,
) -> Result<CleanInboxItemResponse, String> {
    let cfg = get_offline_download_config(app)?;
    let (origin, user, pass) = {
        let conn = crate::v2_config::open(app)?;
        resolve_cd2_grpc_credentials(&conn)?
    };
    let item_path = req.item_path.trim().trim_end_matches('/').to_string();
    if !item_path.starts_with(&cfg.inbox_path) {
        return Err("安全拦截：仅允许清洗中转收件箱内的项目".to_string());
    }
    let item_name = item_path.rsplit('/').next().unwrap_or("item").to_string();
    let trash_dir = format!("{}/{}", cfg.trash_path, item_name);

    crate::rt::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;

        let token = cd2_login(&client, &origin, &user, &pass).await?;
        let mut moved = 0usize;
        let mut renamed_files = 0usize;

        // 1. Move junk files into _Trash/<item_name> (ZERO permanent deletion, serial 450ms pacing)
        if !req.trash_paths.is_empty() {
            cd2_ensure_folder(&client, &origin, &token, &trash_dir).await?;
            for p in &req.trash_paths {
                let clean_p = p.trim();
                if clean_p.is_empty() || !clean_p.starts_with(&cfg.inbox_path) {
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(450)).await;
                let mut payload = Vec::new();
                pb_str(1, clean_p, &mut payload);
                pb_str(2, &trash_dir, &mut payload);
                let res = cd2_grpc_call(&client, &origin, "MoveFile", &payload, Some(&token)).await?;
                if res.grpc_status != 0 {
                    return Err(format!("移入回收站失败（已中止防风控）：{}", res.grpc_message));
                }
                moved += 1;
            }
        }

        // 2. Rename kept files to remove website watermarks
        if let Some(renames) = req.file_renames {
            for r in renames {
                let fp = r.full_path.trim();
                let nn = r.new_name.trim();
                if fp.is_empty() || nn.is_empty() || nn.contains('/') || !fp.starts_with(&cfg.inbox_path) {
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(450)).await;
                let mut payload = Vec::new();
                pb_str(1, fp, &mut payload);
                pb_str(2, nn, &mut payload);
                let res = cd2_grpc_call(&client, &origin, "RenameFile", &payload, Some(&token)).await?;
                if res.grpc_status == 0 {
                    renamed_files += 1;
                }
            }
        }

        // 3. Flatten single-chain wrapper folder (move inner children up to item_path, move wrapper to _Trash)
        if let (Some(src_dir), Some(wrapper_dir)) = (req.flatten_source_dir.as_deref(), req.flatten_wrapper_dir.as_deref()) {
            if src_dir.starts_with(&item_path) && wrapper_dir.starts_with(&item_path) {
                if let Ok(inner_children) = cd2_list_sub_files(&client, &origin, &token, src_dir).await {
                    for ch in inner_children {
                        tokio::time::sleep(Duration::from_millis(450)).await;
                        let mut payload = Vec::new();
                        pb_str(1, &ch.full_path, &mut payload);
                        pb_str(2, &item_path, &mut payload);
                        let _ = cd2_grpc_call(&client, &origin, "MoveFile", &payload, Some(&token)).await;
                    }
                    cd2_ensure_folder(&client, &origin, &token, &trash_dir).await?;
                    tokio::time::sleep(Duration::from_millis(450)).await;
                    let mut payload = Vec::new();
                    pb_str(1, wrapper_dir, &mut payload);
                    pb_str(2, &trash_dir, &mut payload);
                    let _ = cd2_grpc_call(&client, &origin, "MoveFile", &payload, Some(&token)).await;
                }
            }
        }

        // 4. Rename outer folder/file if requested
        let mut final_path = item_path.clone();
        if let Some(new_name) = req.new_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if new_name != item_name && !new_name.contains('/') {
                tokio::time::sleep(Duration::from_millis(450)).await;
                let mut payload = Vec::new();
                pb_str(1, &item_path, &mut payload);
                pb_str(2, new_name, &mut payload);
                let res = cd2_grpc_call(&client, &origin, "RenameFile", &payload, Some(&token)).await?;
                if res.grpc_status != 0 {
                    return Err(format!("重命名失败：{}", res.grpc_message));
                }
                if let Some(parent) = item_path.rsplit_once('/').map(|(p, _)| p) {
                    final_path = format!("{parent}/{new_name}");
                }
            }
        }

        Ok(CleanInboxItemResponse {
            ok: true,
            moved_trash_count: moved,
            new_path: final_path,
            message: format!("清洗完成：移入回收站 {moved} 项，文件去水印 {renamed_files} 项"),
        })
    })
}

pub fn archive_inbox_item(
    app: &AppHandle,
    req: ArchiveInboxItemRequest,
) -> Result<ArchiveInboxItemResponse, String> {
    let cfg = get_offline_download_config(app)?;
    let (origin, user, pass) = {
        let conn = crate::v2_config::open(app)?;
        resolve_cd2_grpc_credentials(&conn)?
    };
    let item_path = req.item_path.trim().trim_end_matches('/').to_string();
    if !item_path.starts_with(&cfg.inbox_path) {
        return Err("安全拦截：仅允许归档中转收件箱内的项目".to_string());
    }

    let raw_dest = req
        .target_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match req.category.as_str() {
            "movie" => cfg.movie_dir.clone(),
            "creator" => cfg.creator_pending_dir.clone(),
            "gallery" => cfg.gallery_pending_dir.clone(),
            "comic" => cfg.comic_pending_dir.clone(),
            "collection" => cfg.collection_pending_dir.clone(),
            _ => String::new(),
        });

    if raw_dest.trim().is_empty() {
        return Err("请先点击右上角「目标目录」配置该分类的网盘归档路径".to_string());
    }
    let dest_dir = normalize_cloud_path(&raw_dest, &raw_dest);

    crate::rt::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;

        let token = cd2_login(&client, &origin, &user, &pass).await?;
        cd2_ensure_folder(&client, &origin, &token, &dest_dir).await?;
        tokio::time::sleep(Duration::from_millis(450)).await;

        let mut payload = Vec::new();
        pb_str(1, &item_path, &mut payload);
        pb_str(2, &dest_dir, &mut payload);
        let res = cd2_grpc_call(&client, &origin, "MoveFile", &payload, Some(&token)).await?;
        if res.grpc_status != 0 {
            return Err(format!("归档移动失败：{}", res.grpc_message));
        }

        Ok(ArchiveInboxItemResponse {
            ok: true,
            dest_dir: dest_dir.clone(),
            message: format!("已归档至 {dest_dir}"),
        })
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxTrashItem {
    pub name: String,
    pub full_path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub size_text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxTrashGroup {
    pub group_name: String,
    pub group_path: String,
    pub suggested_restore_dir: String,
    pub restore_dir_exists: bool,
    pub items: Vec<InboxTrashItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxTrashResponse {
    pub trash_path: String,
    pub inbox_path: String,
    pub groups: Vec<InboxTrashGroup>,
    pub loose_items: Vec<InboxTrashItem>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreInboxTrashRequest {
    pub paths: Vec<String>,
    pub target_dir: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreInboxTrashResponse {
    pub ok: bool,
    pub restored_count: usize,
    pub target_dir: String,
    pub message: String,
}

pub fn list_inbox_trash(app: &AppHandle) -> Result<InboxTrashResponse, String> {
    let cfg = get_offline_download_config(app)?;
    let (origin, user, pass) = {
        let conn = crate::v2_config::open(app)?;
        resolve_cd2_grpc_credentials(&conn)?
    };
    let inbox_path = cfg.inbox_path.clone();
    let trash_path = cfg.trash_path.clone();

    crate::rt::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;

        let token = cd2_login(&client, &origin, &user, &pass).await?;
        cd2_ensure_folder(&client, &origin, &token, &trash_path).await?;

        let inbox_entries = cd2_list_sub_files(&client, &origin, &token, &inbox_path)
            .await
            .unwrap_or_default();
        let inbox_dirs: Vec<Cd2Entry> = inbox_entries
            .into_iter()
            .filter(|e| e.is_dir && e.name != "_Trash" && !e.name.starts_with('.'))
            .collect();

        tokio::time::sleep(Duration::from_millis(120)).await;
        let trash_entries = cd2_list_sub_files(&client, &origin, &token, &trash_path).await?;

        let mut groups = Vec::new();
        let mut loose_items = Vec::new();

        for entry in trash_entries {
            if entry.name.starts_with('.') {
                continue;
            }
            if entry.is_dir {
                tokio::time::sleep(Duration::from_millis(120)).await;
                let children = cd2_list_sub_files(&client, &origin, &token, &entry.full_path)
                    .await
                    .unwrap_or_default();
                let items: Vec<InboxTrashItem> = children
                    .into_iter()
                    .filter(|c| !c.name.starts_with('.'))
                    .map(|c| InboxTrashItem {
                        name: c.name,
                        full_path: c.full_path,
                        is_dir: c.is_dir,
                        size_bytes: c.size_bytes,
                        size_text: if c.is_dir {
                            "目录".to_string()
                        } else {
                            format_size_bytes(c.size_bytes)
                        },
                    })
                    .collect();

                if items.is_empty() {
                    continue;
                }

                let stripped_group = strip_site_watermarks(&entry.name, false);
                let matched_dir = inbox_dirs
                    .iter()
                    .find(|d| d.name == entry.name)
                    .or_else(|| {
                        inbox_dirs
                            .iter()
                            .find(|d| strip_site_watermarks(&d.name, false) == stripped_group)
                    });

                let (suggested_restore_dir, restore_dir_exists) = match matched_dir {
                    Some(d) => (d.full_path.clone(), true),
                    None => (format!("{inbox_path}/{stripped_group}"), false),
                };

                groups.push(InboxTrashGroup {
                    group_name: entry.name,
                    group_path: entry.full_path,
                    suggested_restore_dir,
                    restore_dir_exists,
                    items,
                });
            } else {
                loose_items.push(InboxTrashItem {
                    name: entry.name,
                    full_path: entry.full_path,
                    is_dir: false,
                    size_bytes: entry.size_bytes,
                    size_text: format_size_bytes(entry.size_bytes),
                });
            }
        }

        Ok(InboxTrashResponse {
            trash_path,
            inbox_path,
            groups,
            loose_items,
        })
    })
}

pub fn restore_inbox_trash(
    app: &AppHandle,
    req: RestoreInboxTrashRequest,
) -> Result<RestoreInboxTrashResponse, String> {
    let cfg = get_offline_download_config(app)?;
    let (origin, user, pass) = {
        let conn = crate::v2_config::open(app)?;
        resolve_cd2_grpc_credentials(&conn)?
    };
    let target_dir = normalize_cloud_path(&req.target_dir, &cfg.inbox_path);
    if !target_dir.starts_with(&cfg.inbox_path) || target_dir.starts_with(&cfg.trash_path) {
        return Err("安全拦截：仅允许还原到收件箱有效目录内".to_string());
    }
    let valid_paths: Vec<String> = req
        .paths
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty() && p.starts_with(&cfg.trash_path) && p != &cfg.trash_path)
        .collect();
    if valid_paths.is_empty() {
        return Err("请选择至少一个要还原的垃圾箱文件".to_string());
    }

    crate::rt::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;

        let token = cd2_login(&client, &origin, &user, &pass).await?;
        cd2_ensure_folder(&client, &origin, &token, &target_dir).await?;

        let mut restored = 0usize;
        for p in &valid_paths {
            tokio::time::sleep(Duration::from_millis(450)).await;
            let mut payload = Vec::new();
            pb_str(1, p, &mut payload);
            pb_str(2, &target_dir, &mut payload);
            let res = cd2_grpc_call(&client, &origin, "MoveFile", &payload, Some(&token)).await?;
            if res.grpc_status != 0 {
                return Err(format!("还原失败（已中止防风控）：{}", res.grpc_message));
            }
            restored += 1;
        }

        Ok(RestoreInboxTrashResponse {
            ok: true,
            restored_count: restored,
            target_dir: target_dir.clone(),
            message: format!("已安全还原 {restored} 项至 {target_dir}"),
        })
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cd2FolderEntry {
    pub name: String,
    pub full_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cd2FoldersResponse {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub folders: Vec<Cd2FolderEntry>,
}

pub fn list_cd2_folders(app: &AppHandle, raw_path: Option<String>) -> Result<Cd2FoldersResponse, String> {
    let (origin, user, pass) = {
        let conn = crate::v2_config::open(app)?;
        resolve_cd2_grpc_credentials(&conn)?
    };
    let mut target = raw_path.unwrap_or_default().trim().to_string();
    if target.is_empty() {
        target = "/115open".to_string();
    } else if !target.starts_with('/') {
        target = format!("/{target}");
    }
    if target.len() > 1 {
        target = target.trim_end_matches('/').to_string();
    }

    crate::rt::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;

        let token = cd2_login(&client, &origin, &user, &pass).await?;
        let entries = cd2_list_sub_files(&client, &origin, &token, &target).await?;
        let mut folders: Vec<Cd2FolderEntry> = entries
            .into_iter()
            .filter(|e| e.is_dir && !e.name.starts_with('.'))
            .map(|e| Cd2FolderEntry {
                name: e.name,
                full_path: e.full_path,
            })
            .collect();
        folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let parent_path = if target == "/" {
            None
        } else {
            match target.rsplit_once('/') {
                Some(("", _)) => Some("/".to_string()),
                Some((p, _)) => Some(p.to_string()),
                None => None,
            }
        };

        Ok(Cd2FoldersResponse {
            current_path: target,
            parent_path,
            folders,
        })
    })
}

pub fn create_cd2_folder(
    app: &AppHandle,
    parent_path: String,
    name: String,
) -> Result<Cd2FolderEntry, String> {
    let clean_name = name.trim().replace('/', "").replace('\\', "");
    if clean_name.is_empty() {
        return Err("文件夹名称不能为空".to_string());
    }
    let clean_parent = normalize_cloud_path(&parent_path, "/115open");
    let full_path = if clean_parent == "/" {
        format!("/{clean_name}")
    } else {
        format!("{clean_parent}/{clean_name}")
    };

    let (origin, user, pass) = {
        let conn = crate::v2_config::open(app)?;
        resolve_cd2_grpc_credentials(&conn)?
    };

    crate::rt::block_on(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(6))
            .build()
            .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;

        let token = cd2_login(&client, &origin, &user, &pass).await?;
        cd2_ensure_folder(&client, &origin, &token, &full_path).await?;
        Ok(Cd2FolderEntry {
            name: clean_name,
            full_path,
        })
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_leading_domain_dash_and_brackets() {
        assert_eq!(
            strip_site_watermarks("2048.vip-推特裸舞小美女【yum707玉米】8月VIP裸舞福利-2 掰穴大桃包小桃.mp4", true),
            "推特裸舞小美女【yum707玉米】8月VIP裸舞福利-2 掰穴大桃包小桃.mp4"
        );
        assert_eq!(
            strip_site_watermarks("hhd800.com@ABP-123.mp4", true),
            "ABP-123.mp4"
        );
        assert_eq!(
            strip_site_watermarks("【98t.la】www.demo.com_测试视频.mkv", true),
            "测试视频.mkv"
        );
    }

    #[test]
    fn detects_real_inbox_ads_and_preserves_main_videos() {
        let folder = "推特裸舞小美女【yum707玉米】8月VIP裸舞福利-2 掰穴大桃包小桃";
        let root = format!("/115open/PML_Inbox/{folder}");

        assert!(junk_reason_for_file(
            "2048.vip-推特裸舞小美女【yum707玉米】8月VIP裸舞福利-2 掰穴大桃包小桃.mp4",
            &format!("{root}/2048.vip-推特裸舞小美女【yum707玉米】8月VIP裸舞福利-2 掰穴大桃包小桃.mp4"),
            &root,
            462_724_450,
            None,
            true,
            462_724_450,
            true,
            folder,
            false,
        ).is_none());

        assert!(junk_reason_for_file(
            "V (2).mp4",
            &format!("{root}/sub/V (2).mp4"),
            &root,
            234_967_469,
            None,
            false,
            462_724_450,
            true,
            folder,
            false,
        ).is_none());

        for ad in [
            ("2048QR二维码.png", 19_530u64),
            ("2048地址发布器PC版.rar", 17_515),
            ("_1024社区发布器 10.1版.zip", 1_209_011),
            ("UU直播 20年信誉保证拥有海量美女等你来选 suu37.com.mp4", 23_480_443),
            ("台湾uu美少女直播 20年信誉保证服务全球 suu28.com.mp4", 23_498_796),
            ("台灣UU線上美女聊天 suu27.com.mp4", 23_472_234),
            ("同城一YQ交友.gif", 239_252),
            ("找美女看美女来点击进入有惊喜auu32.com.gif", 3_406_956),
            ("激情隨時看 suu26.com.mp4", 23_485_588),
        ] {
            let reason = junk_reason_for_file(
                ad.0,
                &format!("{root}/{}", ad.0),
                &root,
                ad.1,
                None,
                false,
                462_724_450,
                true,
                folder,
                true,
            );
            assert!(reason.is_some(), "expected junk for {}", ad.0);
        }

        // JAV folder promo leftovers (spaced text, ★★★ mobile game promo, unrelated intro clip < 1/8 max)
        for (main_name, folder_name, code, main_size, ad_name, ad_size) in [
            ("390JAC-172-uncensored-nyap2p.com.mp4", "390JAC-172-uncensored-HD", "390JAC-172", 3_492_693_024u64, "★★★櫻境物語-免费18禁手游.mp4", 51_834_116u64),
            ("390JAC-172-uncensored-nyap2p.com.mp4", "390JAC-172-uncensored-HD", "390JAC-172", 3_492_693_024, "工口MH-扫描访问.png", 989_472),
            ("390JAC-172-uncensored-nyap2p.com.mp4", "390JAC-172-uncensored-HD", "390JAC-172", 3_492_693_024, "杏吧社区-点关注不迷路.jpg", 278_195),
            ("4k2.meblk-673.mp4", "blk-673", "BLK-673", 5_330_590_991, "苍 老 师 强 力 推 荐.mp4", 77_146_172),
            ("nima-007.mp4", "nima-007", "NIMA-007", 5_920_167_105, "uuv79.mp4", 55_272_407),
        ] {
            let r_dir = format!("/115open/PML_Inbox/{folder_name}");
            assert!(junk_reason_for_file(
                main_name,
                &format!("{r_dir}/{main_name}"),
                &r_dir,
                main_size,
                Some(code),
                true,
                main_size,
                true,
                folder_name,
                false,
            ).is_none(), "main movie must stay: {main_name}");
            assert!(junk_reason_for_file(
                ad_name,
                &format!("{r_dir}/{ad_name}"),
                &r_dir,
                ad_size,
                Some(code),
                true,
                main_size,
                true,
                folder_name,
                false,
            ).is_some(), "expected junk for {ad_name}");
        }
    }

    #[test]
    fn redundant_same_named_subfolder_matches_with_emoji_and_count() {
        assert!(is_redundant_same_named_subdir(
            "推特裸舞小美女【yum707玉米】8月VIP裸舞福利-3 最受不了乳摇了",
            "推特裸舞小美女【yum707玉米】8月VIP裸舞福利-3❤️最受不了乳摇了(2V)"
        ));
        assert!(is_redundant_same_named_subdir(
            "推特裸舞小美女【yum707玉米】8月VIP裸舞福利-2 掰穴大桃包小桃",
            "推特裸舞小美女【yum707玉米】8月VIP裸舞福利-2掰穴大桃包小桃"
        ));
        assert!(!is_redundant_same_named_subdir(
            "合集大包",
            "第一集"
        ));
    }
}
