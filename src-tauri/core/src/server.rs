use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

static COMIC_PAGE_CACHE: LazyLock<Mutex<HashMap<String, (String, Vec<u8>)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

use axum::{
    body::Body,
    extract::{Path, State as AxumState},
    http::{
        header::{
            ACCEPT_RANGES, AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_TYPE, COOKIE, ETAG, IF_RANGE, RANGE, SET_COOKIE,
        },
        HeaderMap, HeaderValue, Method, Request, StatusCode,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(feature = "tauri")]
use tauri::Manager;
use crate::AppHandle;
use tower_http::services::ServeDir;

use crate::media_library::{self, MediaLibraryState};

#[derive(Clone)]
struct ServerState {
    app: AppHandle,
    auth: AuthConfig,
}

#[derive(Clone, Default)]
struct AuthConfig {
    username: Option<String>,
    password: Option<String>,
}

fn compute_session_token(username: &str, password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pml-auth-v1:");
    hasher.update(username.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|pair| {
            let mut parts = pair.trim().splitn(2, '=');
            let key = parts.next()?;
            let val = parts.next()?;
            if key == name {
                Some(val.to_string())
            } else {
                None
            }
        })
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginPayload {
    username: String,
    password: Option<String>,
    remember: Option<bool>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct FavoritesListRequest {
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncFavoritesRequest {
    asset_ids: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RootRequest {
    kind: media_library::LibraryRootKind,
    remote_path: String,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RootIdRequest {
    root_id: String,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ScanLibraryRequest {
    root_id: Option<String>,
    folder_path: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DuplicateScanRequest {
    root_id: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RemoteChildrenRequest {
    root_id: String,
    path: String,
    cursor: Option<String>,
    refresh: Option<bool>,
    sort: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SearchRemoteRequest {
    root_id: String,
    parent_path: String,
    keywords: String,
    cursor: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ShortVideoFeedRequest {
    cursor: Option<String>,
    limit: Option<i64>,
    author_id: Option<String>,
    max_duration_seconds: Option<i64>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AssetsRequest {
    filter: Option<String>,
    query: Option<String>,
    author_id: Option<String>,
    collection_id: Option<String>,
    category_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    parent_path: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CategoriesRequest {
    filter: Option<String>,
    parent_id: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AuthorsRequest {
    category_id: Option<String>,
    filter: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CollectionsRequest {
    author_id: Option<String>,
    parent_id: Option<String>,
    category_id: Option<String>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AssetIdRequest {
    asset_id: String,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct DurationRequest {
    asset_id: String,
    duration_seconds: f64,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ProgressRequest {
    asset_id: String,
    position_seconds: f64,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct HistoryRequest {
    limit: Option<i64>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ArchivePageRequest {
    asset_id: String,
    page_index: usize,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MetadataRequest {
    asset_id: String,
    author_name: Option<String>,
    collection_name: Option<String>,
}

pub fn start(app: AppHandle) {
    let port = std::env::var("PML_PORT")
        .or_else(|_| std::env::var("PORT"))
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(18080);
    let static_dir = std::env::var("PML_STATIC_DIR").unwrap_or_else(|_| "dist".to_owned());
    let auth = AuthConfig {
        username: std::env::var("PML_AUTH_USER")
            .ok()
            .filter(|value| !value.is_empty()),
        password: std::env::var("PML_AUTH_PASSWORD").ok(),
    };
    let state = ServerState { app, auth };
    crate::rt::spawn(async move {
        let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("personal-media-library HTTP server failed to bind port {port}: {error}");
                return;
            }
        };
        let router = build_router(state, static_dir);
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("personal-media-library HTTP server stopped: {error}");
        }
    });
}

/// 原生壳（iOS / 桌面）专用的内嵌 API：只监听回环地址，同步返回实际端口。
/// 与 `start` 的区别是不读环境变量里的端口、不暴露到局域网。
#[allow(dead_code)]
pub fn start_on_loopback(app: AppHandle, port: u16) -> Result<u16, String> {
    let static_dir = std::env::var("PML_STATIC_DIR").unwrap_or_else(|_| "dist".to_owned());
    let auth = AuthConfig {
        username: std::env::var("PML_AUTH_USER")
            .ok()
            .filter(|value| !value.is_empty()),
        password: std::env::var("PML_AUTH_PASSWORD").ok(),
    };
    let state = ServerState { app, auth };

    // 纯 Rust（iOS）模式：直接在当前 tokio 运行时里绑定，
    // 避免 std -> tokio 转换在非运行时上下文里失败。
    #[cfg(not(feature = "tauri"))]
    {
        let runtime = crate::rt::runtime();
        let listener = runtime
            .block_on(async move { tokio::net::TcpListener::bind(("127.0.0.1", port)).await })
            .map_err(|error| {
                let message = format!("本地 API 端口绑定失败：{error}");
                record_server_error(&message);
                message
            })?;
        let bound_port = listener
            .local_addr()
            .map(|address| address.port())
            .map_err(|error| format!("本地 API 端口读取失败：{error}"))?;
        crate::rt::spawn(async move {
            let router = build_router(state, static_dir);
            if let Err(error) = axum::serve(listener, router).await {
                record_server_error(&format!("本地 API 服务停止：{error}"));
            }
        });
        return Ok(bound_port);
    }

    #[cfg(feature = "tauri")]
    {
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", port))
            .map_err(|error| format!("本地 API 端口绑定失败：{error}"))?;
        std_listener
            .set_nonblocking(true)
            .map_err(|error| format!("本地 API 端口设置失败：{error}"))?;
        let bound_port = std_listener
            .local_addr()
            .map(|address| address.port())
            .unwrap_or(port);
        crate::rt::spawn(async move {
            let listener = match tokio::net::TcpListener::from_std(std_listener) {
                Ok(listener) => listener,
                Err(error) => {
                    record_server_error(&format!("本地 API 启动失败：{error}"));
                    return;
                }
            };
            let router = build_router(state, static_dir);
            if let Err(error) = axum::serve(listener, router).await {
                record_server_error(&format!("本地 API 服务停止：{error}"));
            }
        });
        Ok(bound_port)
    }
}

static LAST_SERVER_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn record_server_error(message: &str) {
    eprintln!("personal-media-library server error: {message}");
    if let Ok(mut slot) = LAST_SERVER_ERROR.lock() {
        *slot = Some(message.to_owned());
    }
}

pub fn last_server_error() -> Option<String> {
    LAST_SERVER_ERROR
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
}

fn build_router(state: ServerState, static_dir: String) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/covers/{asset_id}/{version}", get(cached_cover))
        .route("/api/comic-pages/{asset_id}/{page_index}", get(cached_comic_page))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/{command}", post(dispatch_command))
        .route("/media/{token}", get(proxy_media).head(proxy_media))
        .route(
            "/compatible/{token}",
            get(proxy_compatible_media).head(proxy_compatible_media),
        )
        .route(
            "/thumbnail/{token}",
            get(proxy_thumbnail).head(proxy_thumbnail),
        )
        .fallback_service(
            ServeDir::new(PathBuf::from(static_dir)).append_index_html_on_directories(true),
        )
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .layer(middleware::from_fn(cache_headers))
        .with_state(state)
}

async fn cache_headers(request: Request<Body>, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    let cache_policy = if path.starts_with("/media/") || path.starts_with("/compatible/") || path.starts_with("/thumbnail/") {
        "private, max-age=300"
    } else if path == "/" || path.ends_with(".html") || path == "/manifest.webmanifest" {
        "no-cache, no-store, must-revalidate"
    } else if path.starts_with("/assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    if !response.headers().contains_key(CACHE_CONTROL) {
    if let Ok(value) = cache_policy.parse() {
        response.headers_mut().insert(CACHE_CONTROL, value);
    }
    }
    response
}

async fn auth_middleware(
    AxumState(state): AxumState<ServerState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    if path == "/api/health" || path.starts_with("/api/auth/") {
        return next.run(request).await;
    }
    if !path.starts_with("/api/")
        && !path.starts_with("/media/")
        && !path.starts_with("/compatible/")
        && !path.starts_with("/thumbnail/")
    {
        return next.run(request).await;
    }

    let Some(username) = state.auth.username.as_deref() else {
        return next.run(request).await;
    };
    let password = state.auth.password.as_deref().unwrap_or_default();
    let expected_token = compute_session_token(username, password);
    let expected_basic = STANDARD.encode(format!("{username}:{password}"));

    let cookie_token = extract_cookie(request.headers(), "pml_session");
    let query_token = request.uri().query().and_then(|q| {
        q.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            if k == "token" || k == "auth" { Some(v.to_string()) } else { None }
        })
    });
    let auth_header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let bearer_token = auth_header.and_then(|value| value.strip_prefix("Bearer "));
    let basic_token = auth_header.and_then(|value| value.strip_prefix("Basic "));

    let is_valid = cookie_token.as_deref() == Some(&expected_token)
        || query_token.as_deref() == Some(&expected_token)
        || bearer_token == Some(&expected_token)
        || basic_token == Some(&expected_basic);

    if is_valid {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(CONTENT_TYPE, "application/json")],
            "{\"error\":\"需要登录\"}",
        )
            .into_response()
    }
}

async fn auth_login(
    AxumState(state): AxumState<ServerState>,
    Json(payload): Json<LoginPayload>,
) -> Response {
    let Some(expected_user) = state.auth.username.as_deref() else {
        return Json(json!({ "status": "ok", "authRequired": false })).into_response();
    };
    let expected_password = state.auth.password.as_deref().unwrap_or_default();
    let supplied_password = payload.password.as_deref().unwrap_or_default();

    if payload.username.trim() == expected_user && supplied_password == expected_password {
        let token = compute_session_token(expected_user, expected_password);
        let max_age_str = if payload.remember.unwrap_or(true) {
            "; Max-Age=2592000"
        } else {
            ""
        };
        let cookie_value = format!("pml_session={token}; Path=/; SameSite=Lax; HttpOnly{max_age_str}");
        let mut response = Json(json!({
            "status": "ok",
            "token": token,
            "username": expected_user,
            "authRequired": true
        })).into_response();
        if let Ok(val) = HeaderValue::from_str(&cookie_value) {
            response.headers_mut().insert(SET_COOKIE, val);
        }
        response
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "用户名或密码错误" })),
        ).into_response()
    }
}

async fn auth_logout() -> Response {
    let cookie_value = "pml_session=; Path=/; Max-Age=0; SameSite=Lax; HttpOnly";
    let mut response = Json(json!({ "status": "ok" })).into_response();
    if let Ok(val) = HeaderValue::from_str(cookie_value) {
        response.headers_mut().insert(SET_COOKIE, val);
    }
    response
}

async fn auth_status(
    AxumState(state): AxumState<ServerState>,
    request: Request<Body>,
) -> Response {
    let Some(expected_user) = state.auth.username.as_deref() else {
        return Json(json!({ "authenticated": true, "authRequired": false })).into_response();
    };
    let expected_password = state.auth.password.as_deref().unwrap_or_default();
    let expected_token = compute_session_token(expected_user, expected_password);
    let expected_basic = STANDARD.encode(format!("{expected_user}:{expected_password}"));

    let cookie_token = extract_cookie(request.headers(), "pml_session");
    let auth_header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let bearer_token = auth_header.and_then(|v| v.strip_prefix("Bearer "));
    let basic_token = auth_header.and_then(|v| v.strip_prefix("Basic "));

    let is_authenticated = cookie_token.as_deref() == Some(&expected_token)
        || bearer_token == Some(&expected_token)
        || basic_token == Some(&expected_basic);

    Json(json!({
        "authenticated": is_authenticated,
        "authRequired": true,
        "username": if is_authenticated { Some(expected_user) } else { None }
    })).into_response()
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

fn parse_args<T: DeserializeOwned>(payload: &Value) -> Result<T, Response> {
    serde_json::from_value(payload.clone())
        .map_err(|error| error_response(format!("请求参数无效：{error}")))
}

fn error_response(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message.into() })),
    )
        .into_response()
}

fn result_response<T: Serialize>(result: Result<T, String>) -> Response {
    match result {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => error_response(error),
    }
}

async fn blocking_result_response<T, F>(task: F) -> Response
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    match tokio::task::spawn_blocking(task).await {
        Ok(result) => result_response(result),
        Err(error) => error_response(format!("后台数据库任务失败：{error}")),
    }
}

fn rewrite_media_url(result: Result<String, String>) -> Result<String, String> {
    result.and_then(|url| {
        if url.starts_with("data:image/") || url.starts_with('/') {
            return Ok(url);
        }
        let parsed = url::Url::parse(&url).map_err(|_| "媒体代理地址无效".to_owned())?;
        let path = parsed.path();
        let (route, token) = if let Some(value) = path.strip_prefix("/media/") {
            ("media", value)
        } else if let Some(value) = path.strip_prefix("/compatible/") {
            ("compatible", value)
        } else if let Some(value) = path.strip_prefix("/thumbnail/") {
            ("thumbnail", value)
        } else {
            return Err("媒体代理令牌无效".to_owned());
        };
        if token.is_empty() || token.contains('/') {
            return Err("媒体代理令牌无效".to_owned());
        }
        Ok(format!("/{route}/{token}"))
    })
}

async fn dispatch_command(
    AxumState(context): AxumState<ServerState>,
    Path(command): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    let app = &context.app;
    match command.as_str() {
        "get_library_status" => result_response(
            media_library::get_library_status(app.clone(), app.state::<MediaLibraryState>()).await,
        ),
        "get_alist_diagnostics" => result_response(
            media_library::get_alist_diagnostics(app.clone(), app.state::<MediaLibraryState>())
                .await,
        ),
        "restore_alist_connection" => result_response(
            media_library::restore_alist_connection(app.clone(), app.state::<MediaLibraryState>())
                .await,
        ),
        "list_remote_directory" => {
            let path: String = match payload.get("path").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 path 参数"),
            };
            result_response(
                media_library::list_remote_directory(app.state::<MediaLibraryState>(), path).await,
            )
        }
        "list_remote_children" => {
            let args: RemoteChildrenRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::list_remote_children(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.root_id,
                    args.path,
                    args.cursor,
                    args.refresh,
                    args.sort,
                )
                .await,
            )
        }
        "search_remote_files" => {
            let args: SearchRemoteRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::search_remote_files(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.root_id,
                    args.parent_path,
                    args.keywords,
                    args.cursor,
                )
                .await,
            )
        }
        "list_library_roots" => {
            let app = app.clone();
            blocking_result_response(move || media_library::list_library_roots(app)).await
        }
        "list_library_nodes" => {
            let args: RootIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let parent_path = payload
                .get("parentPath")
                .and_then(Value::as_str)
                .unwrap_or("/");
            let app = app.clone();
            let root_id = args.root_id;
            let parent_path = parent_path.to_owned();
            blocking_result_response(move || {
                media_library::list_library_nodes(app, root_id, parent_path)
            })
            .await
        }
        "add_library_root" => {
            let args: RootRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::add_library_root(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.kind,
                    args.remote_path,
                )
                .await,
            )
        }
        "remove_library_root" => {
            let args: RootIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::remove_library_root(app, args.root_id))
                .await
        }
        "scan_library" => {
            let args: ScanLibraryRequest = parse_args(&payload).unwrap_or_default();
            result_response(
                media_library::scan_library(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.root_id,
                    args.folder_path,
                )
                .await,
            )
        }
        "start_scan" => {
            let args: ScanLibraryRequest = parse_args(&payload).unwrap_or_default();
            result_response(media_library::start_scan(
                app.clone(),
                app.state::<MediaLibraryState>(),
                args.root_id,
                args.folder_path,
            ))
        }
        "get_media_stream_info" => {
            let args: AssetIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::get_media_stream_info(app, args.asset_id)
            })
            .await
        }
        "get_scan_progress" => result_response(Ok(media_library::get_scan_progress(
            app.state::<MediaLibraryState>(),
        ))),
        "start_duplicate_scan" => {
            let args: DuplicateScanRequest = match parse_args(&payload) { Ok(value) => value, Err(error) => return error };
            result_response(media_library::start_duplicate_scan(app.clone(), app.state::<MediaLibraryState>(), args.root_id).await)
        }
        "get_duplicate_scan_progress" => result_response(Ok(media_library::get_duplicate_scan_progress(app.state::<MediaLibraryState>()))),
        "cancel_duplicate_scan" => result_response(Ok(media_library::cancel_duplicate_scan(app.state::<MediaLibraryState>()))),
        "list_duplicate_groups" => {
            let args: DuplicateScanRequest = match parse_args(&payload) { Ok(value) => value, Err(error) => return error };
            let app = app.clone();
            blocking_result_response(move || media_library::list_duplicate_groups(app, args.root_id)).await
        }
        "rescan_failed_directories" => result_response(
            media_library::rescan_failed_directories(app.clone(), app.state::<MediaLibraryState>())
                .await,
        ),
        "get_library_overview" => {
            let app = app.clone();
            blocking_result_response(move || media_library::get_library_overview(app)).await
        }
        "list_library_assets" => {
            let args: AssetsRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::list_library_assets(
                    app,
                    args.filter,
                    args.query,
                    args.author_id,
                    args.collection_id,
                    args.category_id,
                    args.limit,
                    args.offset,
                    args.parent_path,
                )
            })
            .await
        }
        "list_short_video_feed" => {
            let args: ShortVideoFeedRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::list_short_video_feed(
                    app,
                    args.cursor,
                    args.limit,
                    args.author_id,
                    args.max_duration_seconds,
                )
            })
            .await
        }
        "list_comic_folders" => {
            let args: RootIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let parent_path = payload
                .get("parentPath")
                .and_then(Value::as_str)
                .unwrap_or("/");
            let app = app.clone();
            let root_id = args.root_id;
            let parent_path = parent_path.to_owned();
            blocking_result_response(move || {
                media_library::list_comic_folders(app, root_id, parent_path)
            })
            .await
        }
        "list_library_categories" => {
            let args: CategoriesRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::list_library_categories(app, args.filter, args.parent_id)
            })
            .await
        }
        "list_library_authors" => {
            let args: AuthorsRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::list_library_authors(app, args.category_id, args.filter)
            })
            .await
        }
        "list_library_collections" => {
            let args: CollectionsRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::list_library_collections(
                    app,
                    args.author_id,
                    args.parent_id,
                    args.category_id,
                )
            })
            .await
        }
        "assign_asset_metadata" => {
            let args: MetadataRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::assign_asset_metadata(
                    app,
                    args.asset_id,
                    args.author_name,
                    args.collection_name,
                )
            })
            .await
        }
        "set_asset_duration" => {
            let args: DurationRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::set_asset_duration(app, args.asset_id, args.duration_seconds)
            })
            .await
        }
        "get_asset_playback_progress" => {
            let args: AssetIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::get_asset_playback_progress(app, args.asset_id)
            })
            .await
        }
        "set_asset_playback_progress" => {
            let args: ProgressRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::set_asset_playback_progress(
                    app,
                    args.asset_id,
                    args.position_seconds,
                )
            })
            .await
        }
        "list_playback_history" => {
            let args: HistoryRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::list_playback_history(app, args.limit))
                .await
        }
        "clear_playback_history" => {
            let app = app.clone();
            blocking_result_response(move || media_library::clear_playback_history(app)).await
        }
        "toggle_favorite" => {
            let args: AssetIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::toggle_favorite(app, args.asset_id)).await
        }
        "list_favorite_assets" => {
            let args: FavoritesListRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::list_favorite_assets(app, args.limit, args.offset)).await
        }
        "list_favorite_ids" => {
            let app = app.clone();
            blocking_result_response(move || media_library::list_favorite_ids(app)).await
        }
        "sync_favorites" => {
            let args: SyncFavoritesRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::sync_favorites(app, args.asset_ids)).await
        }
        "get_favorite_count" => {
            let app = app.clone();
            blocking_result_response(move || media_library::get_favorite_count(app)).await
        }
        "get_cache_stats" => {
            let app = app.clone();
            blocking_result_response(move || media_library::get_cache_stats(app)).await
        }
        "clear_cache" => {
            let target = payload
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let app = app.clone();
            blocking_result_response(move || media_library::clear_cache(app, target)).await
        }
        "get_automation_settings" => {
            let app = app.clone();
            blocking_result_response(move || media_library::get_automation_settings(app)).await
        }
        "set_automation_settings" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct SettingsPayload {
                settings: media_library::AutomationSettings,
            }
            let args: SettingsPayload = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::set_automation_settings(app, args.settings)).await
        }
        "list_sibling_videos" => {
        let asset_id: String = match payload.get("assetId").and_then(Value::as_str) {
            Some(value) => value.to_owned(),
            None => return error_response("缺少 assetId 参数"),
        };
        let app = app.clone();
        blocking_result_response(move || media_library::list_sibling_videos(app, asset_id)).await
    }    "find_asset_subtitles" => {
        let asset_id: String = match payload.get("assetId").and_then(Value::as_str) {
            Some(value) => value.to_owned(),
            None => return error_response("缺少 assetId 参数"),
        };
        result_response(media_library::find_asset_subtitles(app.clone(), app.state::<MediaLibraryState>(), asset_id).await)
    }    "get_directory_config" => {
        let app = app.clone();
        blocking_result_response(move || media_library::get_directory_config(app)).await
    }
    "save_directory_config" => {
        let args: media_library::DirectoryConfig = match parse_args(&payload) {
            Ok(value) => value,
            Err(error) => return error,
        };
        let app = app.clone();
        blocking_result_response(move || media_library::save_directory_config(app, args)).await
    }
    "list_source_directories" => {
        let path: String = match payload.get("path").and_then(Value::as_str) {
            Some(value) => value.to_owned(),
            None => return error_response("缺少 path 参数"),
        };
        result_response(media_library::list_media_source_directories(app.state::<MediaLibraryState>(), path).await)
    }
    "list_alist_directories" => {
        let path: String = match payload.get("path").and_then(Value::as_str) {
            Some(value) => value.to_owned(),
            None => return error_response("缺少 path 参数"),
        };
        result_response(media_library::list_alist_directories(app.clone(), path).await)
    }
    "validate_alist_mapping" => {
        let cd2_path: String = match payload.get("cd2Path").and_then(Value::as_str) {
            Some(value) => value.to_owned(),
            None => return error_response("缺少 cd2Path 参数"),
        };
        let alist_path: String = match payload.get("alistPath").and_then(Value::as_str) {
            Some(value) => value.to_owned(),
            None => return error_response("缺少 alistPath 参数"),
        };
        result_response(media_library::validate_alist_mapping(app.clone(), app.state::<MediaLibraryState>(), cd2_path, alist_path).await)
    }    "get_curator_config" => {
            let app = app.clone();
            blocking_result_response(move || media_library::get_curator_config(app)).await
        }
        "save_curator_config" => {
            let args: media_library::CuratorConfigUpdate = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::save_curator_config(app, args)).await
        }
        "curate_move_item" => {
            let args: media_library::CurateMoveItemRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::curate_move_item(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args,
                )
                .await,
            )
        }
        "curate_batch_move" => {
            let args: media_library::CurateBatchMoveRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::curate_batch_move(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args,
                )
                .await,
            )
        }
        "open_raw_media_source" => {
            let remote_path: String = match payload.get("remotePath").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 remotePath 参数"),
            };
            result_response(rewrite_media_url(
                media_library::open_raw_media_source(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    remote_path,
                )
                .await,
            ))
        }
        "delete_empty_pending_batch" => {
            let batch_path: String = match payload.get("batchPath").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 batchPath 参数"),
            };
            result_response(
                media_library::delete_empty_pending_batch(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    batch_path,
                )
                .await,
            )
        }
        "pack_inbox_to_pending_batch" => {
            let args: media_library::PackInboxRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::pack_inbox_to_pending_batch(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args,
                )
                .await,
            )
        }
        "purge_pending_assets" => {
            let pending_path: String = match payload.get("pendingPath").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 pendingPath 参数"),
            };
            let app = app.clone();
            blocking_result_response(move || media_library::purge_pending_assets(app, pending_path)).await
        }
        "set_author_avatar" => {
            let author_id: String = match payload.get("authorId").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 authorId 参数"),
            };
            let asset_id: Option<String> = payload.get("assetId").and_then(Value::as_str).map(|s| s.to_owned());
            let app = app.clone();
            blocking_result_response(move || media_library::set_author_avatar(app, author_id, asset_id)).await
        }
        "upload_author_avatar" => {
            let author_id: String = match payload.get("authorId").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 authorId 参数"),
            };
            let data_base64: String = match payload.get("dataBase64").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 dataBase64 参数"),
            };
            let app = app.clone();
            blocking_result_response(move || media_library::upload_author_avatar(app, author_id, data_base64)).await
        }
        "set_author_banner" => {
            let author_id: String = match payload.get("authorId").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 authorId 参数"),
            };
            let asset_id: Option<String> = payload.get("assetId").and_then(Value::as_str).map(|s| s.to_owned());
            let app = app.clone();
            blocking_result_response(move || media_library::set_author_banner(app, author_id, asset_id)).await
        }
        "upload_author_banner" => {
            let author_id: String = match payload.get("authorId").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 authorId 参数"),
            };
            let data_base64: String = match payload.get("dataBase64").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 dataBase64 参数"),
            };
            let app = app.clone();
            blocking_result_response(move || media_library::upload_author_banner(app, author_id, data_base64)).await
        }
        "update_author_aliases" => {
            let author_id: String = match payload.get("authorId").and_then(Value::as_str) {
                Some(value) => value.to_owned(),
                None => return error_response("缺少 authorId 参数"),
            };
            let aliases: Vec<String> = match payload.get("aliases").and_then(Value::as_array) {
                Some(arr) => arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(),
                None => Vec::new(),
            };
            let app = app.clone();
            blocking_result_response(move || media_library::update_author_aliases(app, author_id, aliases)).await
        }
        "get_provider_config" => {
            let app = app.clone();
            blocking_result_response(move || media_library::get_provider_config(app)).await
        }
        "save_provider_config" => {
            let target_val = payload.get("config").unwrap_or(&payload);
            let config: media_library::ProviderConfig = match parse_args(target_val) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::save_provider_config(app, config)).await
        }
        "get_alist_accel_config" => {
            let app = app.clone();
            blocking_result_response(move || media_library::get_alist_accel_config(app)).await
        }
        "set_alist_accel_config" => {
            let target_val = payload.get("config").unwrap_or(&payload);
            let config: media_library::AlistAccelConfig = match parse_args(target_val) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::set_alist_accel_config(app, config)).await
        }
        "test_alist_accel_connection" => {
            let target_val = payload.get("config").unwrap_or(&payload);
            let config: media_library::AlistAccelConfig = match parse_args(target_val) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(media_library::test_alist_accel_connection(config).await)
        }
        "prepare_curation_preview" => {
            let path: String = match payload.get("path").and_then(Value::as_str) {
                Some(p) => p.to_owned(),
                None => return error_response("缺少 path 参数"),
            };
            let is_directory: bool = payload.get("isDirectory").and_then(Value::as_bool).unwrap_or(false);
            let state = app.state::<MediaLibraryState>();
            result_response(media_library::prepare_curation_preview(app.clone(), state, path, is_directory).await)
        }
        "open_media_source" => {
            let args: AssetIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(rewrite_media_url(
                media_library::open_media_source(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.asset_id,
                )
                .await,
            ))
        }
        "open_media_thumbnail_source" => {
            let args: AssetIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(rewrite_media_url(
                media_library::open_media_thumbnail_source(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.asset_id,
                )
                .await,
            ))
        }
        "get_thumbnail_input" => {
            let args: AssetIdRequest = match parse_args(&payload) { Ok(v)=>v, Err(e)=>return e };
            let result = media_library::get_thumbnail_input(app.clone(),app.state::<MediaLibraryState>(),args.asset_id,payload.get("force").and_then(Value::as_bool)).await;
            result_response(result.and_then(|mut input| {
                input.source = input.source.map(|url|rewrite_media_url(Ok(url))).transpose()?;
                Ok(input)
            }))
        }
        "get_asset_thumbnail" => {
            let args: AssetIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || media_library::get_asset_thumbnail(app, args.asset_id))
                .await
        }
        "save_asset_thumbnail" => {
            #[derive(Deserialize, Default)]
            #[serde(rename_all = "camelCase")]
            struct ThumbnailRequest {
                asset_id: String,
                mime_type: String,
                data_base64: String,
                expected_version: Option<String>,
            }
            let args: ThumbnailRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            let app = app.clone();
            blocking_result_response(move || {
                media_library::save_asset_thumbnail(
                    app,
                    args.asset_id,
                    args.mime_type,
                    args.data_base64,
                    args.expected_version,
                )
            })
            .await
        }
        "get_comic_archive_manifest" => {
            let args: AssetIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::get_comic_archive_manifest(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.asset_id,
                )
                .await,
            )
        }        "inspect_comic_archive" => {
            let args: AssetIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::inspect_comic_archive(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.asset_id,
                )
                .await,
            )
        }
        "read_comic_archive_page" => {
            let args: ArchivePageRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::read_comic_archive_page(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.asset_id,
                    args.page_index,
                )
                .await,
            )
        }
        "set_comic_cover_page" => {
            let args: ArchivePageRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::set_comic_cover_page(
                    app.clone(),
                    app.state::<MediaLibraryState>(),
                    args.asset_id,
                    args.page_index,
                )
                .await,
            )
        }
        "set_directory_cover" => {
            #[derive(Deserialize, Default)]
            #[serde(rename_all = "camelCase")]
            struct SetDirCoverRequest {
                root_id: String,
                remote_path: String,
                cover_asset_id: String,
            }
            let args: SetDirCoverRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(media_library::set_directory_cover(
                app.clone(),
                args.root_id,
                args.remote_path,
                args.cover_asset_id,
            ))
        }
        "upload_custom_cover" => {
            #[derive(Deserialize, Default)]
            #[serde(rename_all = "camelCase")]
            struct UploadCoverRequest {
                root_id: Option<String>,
                target_type: String,
                target_id_or_path: String,
                mime_type: String,
                data_base64: String,
            }
            let args: UploadCoverRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(
                media_library::upload_custom_cover(
                    app.clone(),
                    args.root_id,
                    args.target_type,
                    args.target_id_or_path,
                    args.mime_type,
                    args.data_base64,
                )
                .await,
            )
        }
        _ => error_response(format!("不支持的 API：{command}")),
    }
}

async fn proxy_media(
    AxumState(context): AxumState<ServerState>,
    Path(token): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    media_library::serve_media_axum(&context.app, token, method, headers).await
}

async fn proxy_compatible_media(
    AxumState(context): AxumState<ServerState>,
    Path(token): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    proxy_media_route(context, token, "compatible", method, headers).await
}

async fn proxy_thumbnail(
    AxumState(context): AxumState<ServerState>,
    Path(token): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    proxy_media_route(context, token, "thumbnail", method, headers).await
}

static PROXY_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

async fn proxy_media_route(
    context: ServerState,
    token: String,
    route: &str,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let state = context.app.state::<MediaLibraryState>();
    media_library::ensure_token_registered(&context.app, &state, &token, route == "thumbnail").await;
    let address = state.proxy.lock().await.as_ref().map(|proxy| proxy.address);
    let Some(address) = address else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let url = format!("http://{address}/{route}/{token}");
    let client = &*PROXY_CLIENT;
    let request_method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut request = client.request(request_method, url);
    for (name, value) in [
        (RANGE, headers.get(RANGE)),
        (IF_RANGE, headers.get(IF_RANGE)),
    ] {
        if let Some(value) = value {
            request = request.header(name, value);
        }
    }
    let upstream = match request.send().await {
        Ok(response) => response,
        Err(error) => return (StatusCode::BAD_GATEWAY, error.to_string()).into_response(),
    };
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut output_headers = HeaderMap::new();
    for name in [
        CONTENT_TYPE,
        CONTENT_LENGTH,
        CONTENT_RANGE,
        ACCEPT_RANGES,
        ETAG,
    ] {
        if let Some(value) = upstream.headers().get(&name) {
            output_headers.insert(name, value.clone());
        }
    }
    // Media URLs are short-lived, tokenized local proxy routes. A small
    // private cache removes duplicate metadata/range requests while avoiding
    // persistence of a stream beyond the source token lifetime.
    output_headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=300"),
    );
    output_headers.insert(
        axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    output_headers.insert(
        axum::http::header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static(
            "Content-Length, Content-Range, Content-Type, ETag, Accept-Ranges",
        ),
    );
    if method == Method::HEAD {
        return (status, output_headers, Body::empty()).into_response();
    }
    let stream = upstream.bytes_stream().map(|chunk| {
        chunk.map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error.to_string()))
    });
    (status, output_headers, Body::from_stream(stream)).into_response()
}

async fn cached_cover(AxumState(context):AxumState<ServerState>,Path((asset_id,version)):Path<(String,String)>,headers:HeaderMap)->Response {
    let app=context.app.clone();
    let bytes=match tokio::task::spawn_blocking(move||media_library::read_cached_cover(&app,&asset_id,&version)).await {
        Ok(Ok(bytes))=>bytes,
        _=>return (StatusCode::NOT_FOUND,[(CACHE_CONTROL,"no-store")],"封面不存在或已更新").into_response(),
    };
    let etag=format!("\"{:x}\"",Sha256::digest(&bytes));
    let not_modified=headers.get("if-none-match").and_then(|v|v.to_str().ok())==Some(etag.as_str());
    let mut response=if not_modified {StatusCode::NOT_MODIFIED.into_response()} else {([(CONTENT_TYPE,"image/jpeg")],bytes).into_response()};
    response.headers_mut().insert(CACHE_CONTROL,HeaderValue::from_static("public, max-age=31536000, immutable"));
    if let Ok(value)=HeaderValue::from_str(&etag){response.headers_mut().insert(ETAG,value);}
    response
}

async fn cached_comic_page(
    AxumState(context): AxumState<ServerState>,
    Path((asset_id, page_index)): Path<(String, usize)>,
    headers: HeaderMap,
) -> Response {
    let app = context.app.clone();
    let state = app.state::<MediaLibraryState>();
    let cache_key = format!("{asset_id}|{page_index}");
    let cached = COMIC_PAGE_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(&cache_key).cloned());
    let result = match cached {
        Some(hit) => Ok(hit),
        None => {
            let fetched = media_library::read_comic_archive_page_binary(&app, &state, &asset_id, page_index).await;
            if let Ok(value) = &fetched {
                if let Ok(mut cache) = COMIC_PAGE_CACHE.lock() {
                    if cache.len() > 80 {
                        cache.clear();
                    }
                    cache.insert(cache_key.clone(), value.clone());
                }
            }
            fetched
        }
    };
    match result {
        Ok((mime_type, bytes)) => {
            let etag = format!("\"comic-{asset_id}-{page_index}-{}\"", bytes.len());
            let not_modified = headers
                .get("if-none-match")
                .and_then(|v| v.to_str().ok())
                == Some(etag.as_str());
            let mut response = if not_modified {
                StatusCode::NOT_MODIFIED.into_response()
            } else {
                (
                    [(
                        CONTENT_TYPE,
                        HeaderValue::from_str(&mime_type)
                            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
                    )],
                    bytes,
                )
                    .into_response()
            };
            response.headers_mut().insert(
                CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=86400, immutable"),
            );
            if let Ok(value) = HeaderValue::from_str(&etag) {
                response.headers_mut().insert(ETAG, value);
            }
            response
        }
        Err(error) => (StatusCode::NOT_FOUND, [(CACHE_CONTROL, HeaderValue::from_static("no-store"))], error).into_response(),
    }
}
