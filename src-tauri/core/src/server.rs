use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

static COMIC_PAGE_CACHE: LazyLock<Mutex<HashMap<String, (String, Vec<u8>)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

use axum::{
    body::Body,
    extract::{Path, Query, State as AxumState},
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
use base64::{engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}, Engine};
use futures_util::StreamExt;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(feature = "tauri")]
use tauri::Manager;
use crate::four_domains;
use crate::telegram_public;
use crate::AppHandle;
use tower_http::services::ServeDir;

use crate::media_library::{self, MediaLibraryState};

fn validate_http_url(value: &str) -> bool {
    matches!(url::Url::parse(value), Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some())
}

fn strict_string_array(value: &Value, field: &str) -> Result<Vec<String>, String> {
    let array = value.as_array().ok_or_else(|| format!("{field}必须是字符串数组"))?;
    array.iter().map(|item| item.as_str().map(str::to_owned).ok_or_else(|| format!("{field}必须是字符串数组"))).collect()
}

fn optional_string(payload: &Value, key: &str) -> Result<Option<String>, String> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.to_owned())),
        Some(_) => Err(format!("{key}必须是字符串或null")),
    }
}

#[cfg(test)]
mod metadata_validation_tests {
    use super::*;
    #[test]
    fn validates_urls_and_arrays() {
        assert!(validate_http_url("https://example.com/a.jpg"));
        assert!(!validate_http_url("file:///tmp/a.jpg"));
        assert!(strict_string_array(&serde_json::json!(["a"]), "actors").is_ok());
        assert!(strict_string_array(&serde_json::json!([1]), "actors").is_err());
        assert!(optional_string(&serde_json::json!({"maker": null}), "maker").unwrap().is_none());
        assert!(optional_string(&serde_json::json!({"maker": 1}), "maker").is_err());
    }
}

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
    let download_dir = std::env::var("PML_DOWNLOAD_DIR")
        .unwrap_or_else(|_| "/data/downloads".to_owned());
    let _ = std::fs::create_dir_all(&download_dir);
    Router::new()
        .nest_service("/downloads", ServeDir::new(download_dir))
        .route("/api/health", get(health))
        .route("/api/covers/{asset_id}/{version}", get(cached_cover))
        .route("/api/comic-pages/{asset_id}/{page_index}", get(cached_comic_page))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/login", post(auth_login))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/telegram-proxy-media", get(proxy_telegram_media).head(proxy_telegram_media))
        .route("/api/telegram-user-stream/{chat_id}/{message_id}", get(proxy_telegram_user_stream).head(proxy_telegram_user_stream))
        .route("/api/telegram-user-thumb/{chat_id}/{message_id}", get(proxy_telegram_user_thumb).head(proxy_telegram_user_thumb))
        .route("/api/scraper/proxy-image", get(proxy_scraper_image).head(proxy_scraper_image))
        .route("/api/movie-subtitles/{id}", get(serve_movie_subtitle).head(serve_movie_subtitle))
        // A 2 MiB avatar expands to ~2.7 MiB base64; keep a bounded JSON envelope.
        .route("/api/{command}", post(dispatch_command).layer(axum::extract::DefaultBodyLimit::max(3 * 1024 * 1024)))
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
    let cache_policy = if path.starts_with("/media/") || path.starts_with("/compatible/") || path.starts_with("/thumbnail/") || path.starts_with("/api/telegram-proxy-media") || path.starts_with("/api/telegram-user-stream/") || path.starts_with("/api/scraper/proxy-image") {
        "private, max-age=300"
    } else if path.starts_with("/api/telegram-user-thumb/") {
        "public, max-age=86400"
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
    if path == "/api/health"
        || path.starts_with("/api/auth/")
        || path == "/api/telegram-proxy-media"
        || path.starts_with("/api/telegram-user-stream/")
        || path.starts_with("/api/telegram-user-thumb/")
        || path.starts_with("/api/scraper/proxy-image")
        || path.starts_with("/api/movie-subtitles/")
    {
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

/// Reuse the HTTP command implementation for trusted native IPC without
/// requiring a running HTTP listener or a second copy of scraper business logic.
pub async fn dispatch_native(app: AppHandle, command: &str, payload: Value) -> Result<Value, String> {
    let response = dispatch_command(
        AxumState(ServerState { app, auth: AuthConfig::default() }),
        Path(command.to_owned()),
        Json(payload),
    ).await;
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 32 * 1024 * 1024)
        .await.map_err(|e| format!("读取命令响应失败：{e}"))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("解析命令响应失败：{e}"))?;
    if !status.is_success() {
        return Err(value.get("error").and_then(Value::as_str)
            .unwrap_or("命令执行失败").to_owned());
    }
    Ok(value)
}

async fn dispatch_command(
    AxumState(context): AxumState<ServerState>,
    Path(command): Path<String>,
    Json(payload): Json<Value>,
) -> Response {
    let app = &context.app;
    match command.as_str() {
        "four_domain_summaries" => {
            let app = app.clone();
            blocking_result_response(move || four_domains::summaries(&app)).await
        }
        "refresh_domain_path" => {
            let domain = payload.get("domain").and_then(Value::as_str).unwrap_or("").to_owned();
            let path = payload.get("path").and_then(Value::as_str).unwrap_or("").to_owned();
            result_response(four_domains::refresh_domain_path(app.clone(), app.state::<MediaLibraryState>(), &domain, &path).await)
        }
        "scan_domain" => {
            let domain = payload.get("domain").and_then(Value::as_str).unwrap_or("").to_owned();
            let force = payload.get("force").and_then(Value::as_bool).unwrap_or(false);
            if force {
                result_response(four_domains::scan_domain_full(app.clone(), app.state::<MediaLibraryState>(), &domain).await)
            } else {
                result_response(four_domains::scan_domain(app.clone(), app.state::<MediaLibraryState>(), &domain).await)
            }
        }
        "clean_domain" => {
            let domain = payload.get("domain").and_then(Value::as_str).unwrap_or("").to_owned();
            let dry_run = payload.get("dryRun").and_then(Value::as_bool).unwrap_or(true);
            let app = app.clone();
            blocking_result_response(move || four_domains::clean(&app, &domain, dry_run)).await
        }
        "list_domain_children" => {
            let domain = payload.get("domain").and_then(Value::as_str).unwrap_or("").to_owned();
            let path = payload.get("path").and_then(Value::as_str).map(str::to_owned);
            let app = app.clone();
            blocking_result_response(move || four_domains::children(&app, &domain, path.as_deref())).await
        }
        "list_domain_assets" => {
            let domain = payload.get("domain").and_then(Value::as_str).unwrap_or("").to_owned();
            let media_type = payload.get("mediaType").and_then(Value::as_str).map(str::to_owned);
            let query = payload.get("query").and_then(Value::as_str).map(str::to_owned);
            let limit = payload.get("limit").and_then(Value::as_i64).unwrap_or(100);
            let offset = payload.get("offset").and_then(Value::as_i64).unwrap_or(0);
            let app = app.clone();
            blocking_result_response(move || four_domains::list_assets(&app, &domain, media_type.as_deref(), query.as_deref(), limit, offset)).await
        }
        "list_pending_projects" => {
            let app = app.clone();
            blocking_result_response(move || four_domains::projects(&app)).await
        }
        "pending_project_items" => {
            let project_id = payload.get("projectId").and_then(Value::as_str).unwrap_or("").to_owned();
            let app = app.clone();
            blocking_result_response(move || four_domains::project_items(&app, &project_id)).await
        }
        "assign_pending_item" => {
            let asset_id = payload.get("assetId").and_then(Value::as_str).unwrap_or("").to_owned();
            let category_id = payload.get("categoryId").and_then(Value::as_str).map(str::to_owned);
            let author_id = payload.get("authorId").and_then(Value::as_str).map(str::to_owned);
            let app = app.clone();
            blocking_result_response(move || four_domains::assign_pending(&app, &asset_id, category_id, author_id)).await
        }
        "move_pending_item" => {
            let asset_id = payload.get("assetId").and_then(Value::as_str).unwrap_or("").to_owned();
            result_response(four_domains::move_pending_item(app.clone(), app.state::<MediaLibraryState>(), asset_id).await)
        }
        "scrape_asset" => {
            let asset_id = payload.get("assetId").and_then(Value::as_str).unwrap_or("").to_owned();
            let file_name = payload.get("fileName").and_then(Value::as_str).unwrap_or("").to_owned();
            let _remote_path = payload.get("remotePath").and_then(Value::as_str).unwrap_or("").to_owned();
            let force = payload.get("force").and_then(Value::as_bool).unwrap_or(false);
            let explicit_number = payload.get("number").and_then(Value::as_str).unwrap_or("").trim().to_owned();
            let app = app.clone();
            let res: Result<crate::jav_scraper::MovieMetadata, String> = async move {
                let conn = four_domains::open_v2(&app)?;
                // Titles are display data, not stable scraper identities. New clients
                // pass number explicitly; old directory clients can still use fileName.
                let number = if !explicit_number.is_empty() {
                    crate::jav_scraper::normalize_jav_number(&explicit_number)
                } else {
                    crate::jav_scraper::extract_jav_number(&file_name)
                }
                    .ok_or_else(|| format!("无法识别有效番号，请提供明确番号或原始文件名【{}】", file_name))?;

                if !force {
                    if let Ok(Some(mut existing)) = crate::jav_scraper::get_movie_metadata(&conn, &number) {
                        if !asset_id.is_empty() {
                            crate::jav_scraper::bind_movie_asset(&conn, &number, &asset_id)
                                .map_err(|e| format!("绑定视频资产失败：{e}"))?;
                            existing.asset_id = Some(asset_id.clone());
                        }
                        return Ok(existing);
                    }
                }
                if force { crate::jav_scraper::invalidate_metadata_cache(&number); }
                let mut meta = match crate::jav_scraper::scrape_movie_metadata(&number).await {
                    Ok(meta) => meta,
                    Err(error) => {
                        conn.execute("INSERT INTO scrape_failures(number,error,updated_at) VALUES(?1,?2,?3) ON CONFLICT(number) DO UPDATE SET error=excluded.error,updated_at=excluded.updated_at", rusqlite::params![number, error, chrono::Utc::now().timestamp()]).map_err(|e| format!("保存失败原因失败：{e}"))?;
                        return Err(error);
                    }
                };
                meta.asset_id = if asset_id.is_empty() { None } else { Some(asset_id) };
                crate::jav_scraper::save_movie_metadata(&conn, &meta)
                    .map_err(|e| format!("保存元数据到数据库失败：{e}"))?;
                if let Some(id) = meta.asset_id.as_deref() {
                    crate::jav_scraper::bind_movie_asset(&conn, &number, id)
                        .map_err(|e| format!("绑定视频资产失败：{e}"))?;
                }
                Ok(meta)
            }.await;
            result_response(res)
        }
        "get_batch_scrape_stats" => {
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::get_batch_scrape_stats(&conn)
                    .map_err(|e| format!("获取待刮削统计失败：{e}"))
            }).await
        }
        "get_batch_scrape_progress" => {
            result_response(Ok(crate::jav_scraper::get_batch_scrape_progress()))
        }
        "start_batch_scrape" => {
            let app = app.clone();
            blocking_result_response(move || {
                let p = four_domains::v2_path(&app)?;
                crate::jav_scraper::start_batch_scrape_engine(p.to_string_lossy().to_string())
            }).await
        }
        "start_batch_scrape_failed" => {
            let app = app.clone();
            blocking_result_response(move || {
                let p = four_domains::v2_path(&app)?;
                crate::jav_scraper::start_failed_batch_scrape_engine(p.to_string_lossy().to_string())
            }).await
        }
        "pause_batch_scrape" => {
            crate::jav_scraper::pause_batch_scrape();
            result_response(Ok(crate::jav_scraper::get_batch_scrape_progress()))
        }
        "resume_batch_scrape" => {
            crate::jav_scraper::resume_batch_scrape();
            result_response(Ok(crate::jav_scraper::get_batch_scrape_progress()))
        }
        "stop_batch_scrape" => {
            crate::jav_scraper::stop_batch_scrape();
            result_response(Ok(crate::jav_scraper::get_batch_scrape_progress()))
        }
        "update_movie_metadata" => {
            let number = payload.get("number").and_then(Value::as_str).unwrap_or("").trim().to_owned();
            let title = payload.get("title").and_then(Value::as_str).unwrap_or("").trim().to_owned();
            let actors_value = payload.get("actors").ok_or_else(|| "actors必须是字符串数组".to_owned());
            let actors = match actors_value.and_then(|value| strict_string_array(value, "actors")) {
                Ok(value) => value,
                Err(message) => return error_response(&message),
            };
            let maker = match optional_string(&payload, "maker") { Ok(value) => value, Err(message) => return error_response(&message) };
            let release_date = match optional_string(&payload, "releaseDate") { Ok(value) => value, Err(message) => return error_response(&message) };
            let cover_url = match optional_string(&payload, "coverUrl") { Ok(value) => value, Err(message) => return error_response(&message) };
            let samples_value = payload.get("sampleImages").ok_or_else(|| "sampleImages必须是字符串数组".to_owned());
            let samples = match samples_value.and_then(|value| strict_string_array(value, "sampleImages")) {
                Ok(value) => value,
                Err(message) => return error_response(&message),
            };
            if number.is_empty() || title.is_empty() { return error_response("缺少番号或标题"); }
            if cover_url.as_deref().map(|url| !validate_http_url(url)).unwrap_or(true) {
                return error_response("coverUrl必须是有效的http/https URL且不可为空");
            }
            if samples.iter().any(|url| !validate_http_url(url)) {
                return error_response("sampleImages必须全部为有效的http/https URL");
            }
            let actors_json = serde_json::to_string(&actors).unwrap_or_else(|_| "[]".to_owned());
            let samples_json = serde_json::to_string(&samples).unwrap_or_else(|_| "[]".to_owned());
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                let changed = conn.execute("UPDATE movie_metadata SET title=?1,actors_json=?2,maker=?3,release_date=?4,cover_url=?5,sample_images_json=?6,verification_version=1,updated_at=?7 WHERE number=?8 AND verification_version=1", rusqlite::params![title, actors_json, maker, release_date, cover_url, samples_json, chrono::Utc::now().timestamp(), number]).map_err(|e| format!("保存元数据失败：{e}"))?;
                if changed == 0 { return Err("仅允许编辑已验证元数据记录".into()); }
                crate::jav_scraper::invalidate_metadata_cache(&number);
                conn.execute("DELETE FROM scrape_failures WHERE number = ?1", rusqlite::params![number]).map_err(|e| format!("清除失败记录失败：{e}"))?;
                crate::jav_scraper::get_movie_metadata(&conn, &number).map_err(|e| format!("读取更新结果失败：{e}"))?.ok_or_else(|| "元数据不存在".into())
            }).await
        }
        "get_movie_metadata" => {
            let number = payload.get("number").and_then(Value::as_str).unwrap_or("").to_owned();
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::get_movie_metadata(&conn, &number)
                    .map_err(|e| format!("查询元数据失败：{e}"))
            }).await
        }
        "query_movie_library" | "list_scrape_issues" | "list_unrecognized_videos" | "list_movie_performers" => {
            let operation = command.to_owned();
            let query = payload.get("query").and_then(Value::as_str).unwrap_or("").to_owned();
            let maker = payload.get("maker").and_then(Value::as_str).unwrap_or("").to_owned();
            let actor = payload.get("actor").and_then(Value::as_str).unwrap_or("").to_owned();
            let sort = payload.get("sort").and_then(Value::as_str).unwrap_or("date").to_owned();
            let category = payload.get("category").and_then(Value::as_str).unwrap_or("").to_owned();
            let limit = payload.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 100) as u32;
            let offset = payload.get("offset").and_then(Value::as_u64).unwrap_or(0).min(u32::MAX as u64) as u32;
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                match operation.as_str() {
                    "query_movie_library" => serde_json::to_value(crate::jav_scraper::query_movie_library(&conn, &query, &maker, &actor, &sort, limit, offset).map_err(|e| e.to_string())?).map_err(|e| e.to_string()),
                    "list_scrape_issues" => serde_json::to_value(crate::jav_scraper::list_scrape_issues(&conn, &category, &query, limit, offset).map_err(|e| e.to_string())?).map_err(|e| e.to_string()),
                    "list_unrecognized_videos" => serde_json::to_value(crate::jav_scraper::list_unrecognized_videos(&conn, limit, offset).map_err(|e| e.to_string())?).map_err(|e| e.to_string()),
                    _ => serde_json::to_value(crate::performers::list_performers(&conn, &query, limit, offset).map_err(|e| e.to_string())?).map_err(|e| e.to_string()),
                }
            }).await
        }
        "preview_movie_performer_profile" => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
            let profile_url = payload.get("profileUrl").and_then(Value::as_str).unwrap_or("").to_owned();
            let app = app.clone();
            let identity = tokio::task::spawn_blocking(move || -> Result<(String, Vec<String>), String> {
                let conn = four_domains::open_v2(&app)?;
                let detail = crate::performers::get_performer(&conn, &id).map_err(|e| e.to_string())?.ok_or("人物不存在")?;
                let mut names = detail.person.aliases;
                names.push(detail.person.name);
                let mut stmt = conn.prepare("SELECT DISTINCT raw_name FROM movie_cast WHERE person_id=?1").map_err(|e| e.to_string())?;
                let rows = stmt.query_map([&id], |r| r.get::<_,String>(0)).map_err(|e| e.to_string())?;
                for row in rows { names.push(row.map_err(|e| e.to_string())?); }
                Ok((detail.person.source, names))
            }).await;
            match identity {
                Ok(Ok((source, names))) => result_response(crate::performers::fetch_profile_candidate(&source, &profile_url, &names).await),
                Ok(Err(e)) => error_response(&e),
                Err(e) => error_response(&e.to_string()),
            }
        }
        "enrich_movie_performer" => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
            let force = payload.get("force").and_then(Value::as_bool).unwrap_or(false);
            let app_read = app.clone();
            let id_read = id.clone();
            let target = tokio::task::spawn_blocking(move || -> Result<(String, Option<String>), String> {
                let conn = four_domains::open_v2(&app_read)?;
                let detail = crate::performers::get_performer(&conn, &id_read)
                    .map_err(|e| e.to_string())?
                    .ok_or("人物不存在")?;
                Ok((detail.person.name, detail.person.profile_url))
            }).await;
            match target {
                Ok(Ok((name, existing_url))) => {
                    match crate::performers::fetch_performer_enrichment(&name, existing_url.as_deref()).await {
                        Ok(info) => {
                            let app_write = app.clone();
                            blocking_result_response(move || {
                                let conn = four_domains::open_v2(&app_write)?;
                                crate::performers::apply_performer_enrichment(&conn, &id, &info, force)
                                    .map_err(|e| e.to_string())
                            }).await
                        }
                        Err(e) => error_response(&e),
                    }
                }
                Ok(Err(e)) => error_response(&e),
                Err(e) => error_response(&e.to_string()),
            }
        }
        "batch_enrich_movie_performers" => {
            let limit = payload.get("limit").and_then(Value::as_u64).unwrap_or(15).min(50) as u32;
            let app_list = app.clone();
            let pending = tokio::task::spawn_blocking(move || -> Result<(Vec<(String, String, Option<String>)>, u64), String> {
                let conn = four_domains::open_v2(&app_list)?;
                crate::performers::list_performers_missing_profile(&conn, limit).map_err(|e| e.to_string())
            }).await;
            match pending {
                Ok(Ok((items, total_missing))) => {
                    let mut enriched: u64 = 0;
                    let mut failed: u64 = 0;
                    for (id, name, existing_url) in items {
                        match crate::performers::fetch_performer_enrichment(&name, existing_url.as_deref()).await {
                            Ok(info) => {
                                let app_save = app.clone();
                                let saved = tokio::task::spawn_blocking(move || -> Result<(), String> {
                                    let conn = four_domains::open_v2(&app_save)?;
                                    crate::performers::apply_performer_enrichment(&conn, &id, &info, false)
                                        .map_err(|e| e.to_string())?;
                                    Ok(())
                                }).await;
                                if matches!(saved, Ok(Ok(()))) { enriched += 1; } else { failed += 1; }
                            }
                            Err(_) => { failed += 1; }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    }
                    result_response(Ok(serde_json::json!({
                        "enriched": enriched,
                        "failed": failed,
                        "remaining": total_missing.saturating_sub(enriched),
                    })))
                }
                Ok(Err(e)) => error_response(&e),
                Err(e) => error_response(&e.to_string()),
            }
        }
        "get_movie_performer" | "update_movie_performer" | "set_movie_performer_status" => {
            let operation = command.to_owned();
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                if operation == "get_movie_performer" {
                    return serde_json::to_value(crate::performers::get_performer(&conn, &id).map_err(|e| e.to_string())?).map_err(|e| e.to_string());
                }
                let person = if operation == "set_movie_performer_status" {
                    let status = payload.get("status").and_then(Value::as_str).ok_or("缺少确认状态")?;
                    crate::performers::set_performer_status(&conn, &id, status).map_err(|e| e.to_string())?
                } else {
                    let name = payload.get("name").and_then(Value::as_str).ok_or("缺少姓名")?;
                    let aliases = strict_string_array(payload.get("aliases").ok_or("缺少别名数组")?, "aliases")?;
                    let avatar = optional_string(&payload, "avatarUrl")?;
                    let profile = optional_string(&payload, "profileUrl")?;
                    crate::performers::update_performer(&conn, &id, name, &aliases, avatar.as_deref(), profile.as_deref()).map_err(|e| e.to_string())?
                };
                serde_json::to_value(person).map_err(|e| e.to_string())
            }).await
        }
        "list_movie_assets" => {
            let number = payload.get("number").and_then(Value::as_str).unwrap_or("").to_owned();
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::list_movie_assets(&conn, &number)
                    .map_err(|e| format!("获取影片分片失败：{e}"))
            }).await
        }
        "search_movie_subtitles" => {
            let number = payload.get("number").and_then(Value::as_str).unwrap_or("").to_owned();
            result_response(crate::jav_scraper::search_movie_subtitles(&number).await)
        }
        "download_movie_subtitle" => {
            let number = payload.get("number").and_then(Value::as_str).unwrap_or("").to_owned();
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
            let lang = payload.get("lang").and_then(Value::as_str).unwrap_or("zh-CN").to_owned();
            let download_url = payload.get("downloadUrl").and_then(Value::as_str).unwrap_or("").to_owned();
            let downloaded = match crate::jav_scraper::download_and_save_subtitle(&number, &name, &lang, &download_url).await {
                Ok(v) => v,
                Err(e) => return error_response(&e),
            };
            let app = app.clone();
            blocking_result_response(move || {
                let (id, clean_num, clean_name, clean_lang, content) = downloaded;
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::save_movie_subtitle_row(
                    &conn,
                    &id,
                    &clean_num,
                    &clean_name,
                    &clean_lang,
                    "subtitlecat",
                    &content,
                )
                .map_err(|e| format!("保存字幕失败：{e}"))
            })
            .await
        }
        "list_movie_subtitles" => {
            let number = payload.get("number").and_then(Value::as_str).unwrap_or("").to_owned();
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::list_movie_subtitles(&conn, &number)
                    .map_err(|e| format!("读取已保存字幕失败：{e}"))
            })
            .await
        }
        "delete_movie_subtitle" => {
            let id = payload.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::delete_movie_subtitle(&conn, &id)
                    .map_err(|e| format!("删除字幕失败：{e}"))
            })
            .await
        }
        "get_related_movies_grouped" => {
            let number = payload.get("number").and_then(Value::as_str).unwrap_or("").to_owned();
            let limit = payload.get("limitPerGroup").and_then(Value::as_u64).unwrap_or(8) as u32;
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::get_related_movies_grouped(&conn, &number, limit)
                    .map_err(|e| format!("获取分区关联推荐失败：{e}"))
            }).await
        }
        "get_performer_enrich_stats" => {
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::performers::get_performer_enrich_stats(&conn)
                    .map_err(|e| format!("获取演员资料统计失败：{e}"))
            }).await
        }
        "get_performer_enrich_progress" => {
            result_response(Ok(crate::performers::get_performer_enrich_progress()))
        }
        "start_performer_enrich" => {
            let force_all = payload.get("forceAll").and_then(Value::as_bool).unwrap_or(false);
            let app_clone = app.clone();
            result_response(crate::performers::start_performer_enrich_task(
                move || four_domains::open_v2(&app_clone),
                force_all,
            ))
        }
        "pause_performer_enrich" => {
            result_response(Ok(crate::performers::pause_performer_enrich()))
        }
        "resume_performer_enrich" => {
            result_response(Ok(crate::performers::resume_performer_enrich()))
        }
        "stop_performer_enrich" => {
            result_response(Ok(crate::performers::stop_performer_enrich()))
        }
        "get_random_featured_movies" => {
            let limit = payload.get("limit").and_then(Value::as_u64).unwrap_or(20) as u32;
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::get_random_featured_movies(&conn, limit)
                    .map_err(|e| format!("获取随机推荐影片失败：{e}"))
            }).await
        }
        "ignore_unrecognized_video" => {
            let asset_id = payload.get("assetId").and_then(Value::as_str).unwrap_or("").to_owned();
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::ignore_unrecognized_video(&conn, &asset_id)
                    .map_err(|e| format!("忽略视频失败：{e}"))
            }).await
        }
        "sync_remote_and_reconcile" => {
            let auto_scrape = payload.get("autoScrape").and_then(Value::as_bool).unwrap_or(false);
            let mode = payload.get("mode").and_then(Value::as_str).unwrap_or("fast").to_ascii_lowercase();
            let app_wl = app.clone();
            let mode_for_paths = mode.clone();
            let paths_res = tokio::task::spawn_blocking(move || -> Result<Vec<String>, String> {
                let conn = four_domains::open_v2(&app_wl)?;
                if mode_for_paths == "issues" {
                    crate::jav_scraper::issue_and_unrecognized_parent_dirs(&conn).map_err(|e| e.to_string())
                } else {
                    crate::jav_scraper::get_scrape_whitelist(&conn).map_err(|e| e.to_string())
                }
            }).await;
            let paths = match paths_res {
                Ok(Ok(p)) => p,
                Ok(Err(e)) => return error_response(&e),
                Err(e) => return error_response(&e.to_string()),
            };
            let (recursive, incremental) = match mode.as_str() {
                "issues" => (false, false),
                "deep" => (true, false),
                _ => (true, true),
            };
            let (scanned_dirs, added, updated, removed) = match four_domains::sync_whitelisted_paths(
                app.clone(),
                app.state::<MediaLibraryState>(),
                &paths,
                recursive,
                incremental,
            ).await {
                Ok(counts) => counts,
                Err(e) => return error_response(&e),
            };
            let app_rec = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app_rec)?;
                let report = crate::jav_scraper::reconcile_movie_library_changes(
                    &conn,
                    scanned_dirs,
                    added,
                    updated,
                    removed,
                ).map_err(|e| format!("对账清理失败：{e}"))?;
                if auto_scrape && report.pending_new_count > 0 {
                    if let Ok(db_path) = four_domains::v2_path(&app_rec) {
                        let _ = crate::jav_scraper::start_batch_scrape_engine(db_path.to_string_lossy().to_string());
                    }
                }
                Ok(report)
            }).await
        }
        "list_movie_metadata" => {
            let limit = payload.get("limit").and_then(Value::as_u64).unwrap_or(50) as u32;
            let offset = payload.get("offset").and_then(Value::as_u64).unwrap_or(0) as u32;
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::list_movie_metadata(&conn, limit, offset)
                    .map_err(|e| format!("列出元数据失败：{e}"))
            }).await
        }
        "get_related_movies" => {
            let number = payload.get("number").and_then(Value::as_str).unwrap_or("").to_owned();
            let limit = payload.get("limit").and_then(Value::as_u64).unwrap_or(12) as u32;
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::get_related_movies(&conn, &number, limit)
                    .map_err(|e| format!("获取关联作品失败：{e}"))
            }).await
        }
        "get_scrape_whitelist" => {
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::get_scrape_whitelist(&conn)
                    .map_err(|e| format!("获取白名单失败：{e}"))
            }).await
        }
        "set_scrape_whitelist" => {
            let paths = payload.get("paths")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>())
                .unwrap_or_default();
            let app = app.clone();
            blocking_result_response(move || {
                let mut conn = four_domains::open_v2(&app)?;
                crate::jav_scraper::set_scrape_whitelist(&mut conn, paths)
                    .map_err(|e| format!("设置白名单失败：{e}"))
            }).await
        }
        "get_album_details" => {
            let dir_path = payload.get("dirPath").and_then(Value::as_str).unwrap_or("").to_owned();
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::album_bundler::get_album_details(&conn, &dir_path)
                    .map_err(|e| format!("获取画册详情失败：{e}"))
            }).await
        }
        "scan_cleanup_items" => {
            let root_path = payload.get("rootPath").and_then(Value::as_str).map(str::to_owned);
            let app = app.clone();
            blocking_result_response(move || {
                let conn = four_domains::open_v2(&app)?;
                crate::cleaner::scan_cleanup_items(&conn, root_path.as_deref())
                    .map_err(|e| format!("扫描垃圾文件失败：{e}"))
            }).await
        }
        "delete_cleanup_items" => {
            let asset_ids = payload.get("assetIds")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>())
                .unwrap_or_default();
            let app = app.clone();
            blocking_result_response(move || {
                let mut conn = four_domains::open_v2(&app)?;
                crate::cleaner::delete_cleanup_items(&mut conn, &asset_ids)
                    .map_err(|e| format!("删除垃圾文件失败：{e}"))
            }).await
        }
        "list_domain_meta" => {
            let app = app.clone();
            blocking_result_response(move || four_domains::metadata(&app)).await
        }
        "create_pending_project" => {
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
            let asset_ids = payload.get("assetIds").and_then(Value::as_array).map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
            result_response(four_domains::create_pending_project(app.clone(), app.state::<MediaLibraryState>(), name, asset_ids).await)
        }
        "list_move_conflicts" => {
            let app = app.clone();
            blocking_result_response(move || four_domains::conflicts(&app)).await
        }
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
            let v2 = four_domains::compat_remote_children(
                app.clone(),
                app.state::<MediaLibraryState>(),
                &args.root_id,
                &args.path,
                args.refresh.unwrap_or(false),
                args.sort.as_deref().unwrap_or("name"),
            )
            .await;
            // A failed provider refresh must remain visible to the caller; returning
            // V1 data here makes an unsuccessful refresh look like success.
            result_response(v2)
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
        "list_playback_history_ids" => {
            let app = app.clone();
            blocking_result_response(move || crate::v2_state::list_playback_history_ids(app)).await
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
        "start_curate_move_job" => {
            let args: media_library::StartCurateMoveJobRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(media_library::start_curate_move_job(app.clone(), args).await)
        }
        "list_curate_move_jobs" => {
            result_response(media_library::list_curate_move_jobs())
        }
        "cancel_curate_move_job" => {
            let job_id: String = match payload.get("jobId").and_then(Value::as_str) {
                Some(value) if !value.is_empty() => value.to_owned(),
                _ => return error_response("缺少 jobId 参数"),
            };
            result_response(media_library::cancel_curate_move_job(job_id))
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
            if let Some(sub_id) = remote_path.strip_prefix("pml-sub://") {
                return result_response(Ok::<String, String>(format!("/api/movie-subtitles/{sub_id}")));
            }
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
        "resolve_direct_playback" => {
            result_response(media_library::resolve_direct_playback(
                app.clone(),
                app.state::<MediaLibraryState>(),
                payload.get("assetId").and_then(Value::as_str).map(str::to_owned),
                payload.get("remotePath").and_then(Value::as_str).map(str::to_owned),
            ).await)
        }
        "open_media_source" => {
            let args: AssetIdRequest = match parse_args(&payload) {
                Ok(value) => value,
                Err(error) => return error,
            };
            result_response(rewrite_media_url(
                four_domains::compat_media_source(
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
        "telegram_list_channels" => {
            let app = app.clone();
            blocking_result_response(move || telegram_public::list_channels(&app)).await
        }
        "telegram_add_channel" => {
            let username = payload
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            result_response(telegram_public::add_channel(app.clone(), username).await)
        }
        "telegram_remove_channel" => {
            let username = payload
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let app = app.clone();
            blocking_result_response(move || telegram_public::remove_channel(&app, &username)).await
        }
        "telegram_list_posts" => {
            let username = payload
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let before = payload.get("before").and_then(Value::as_i64);
            let refresh = payload.get("refresh").and_then(Value::as_bool);
            result_response(telegram_public::list_posts(app.clone(), username, before, refresh).await)
        }
        "telegram_resolve_media" => {
            let username = payload
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
            let post_id = match payload.get("postId").and_then(Value::as_i64) {
                Some(value) if value > 0 => value,
                _ => return error_response("缺少 postId 参数"),
            };
            result_response(telegram_public::resolve_media(app.clone(), username, post_id).await)
        }
        "telegram_user_status" => {
            forward_to_tg_streamer(reqwest::Method::GET, "/auth/status", None).await
        }
        "telegram_user_qr_start" => {
            forward_to_tg_streamer(reqwest::Method::POST, "/auth/qr/start", Some(payload)).await
        }
        "telegram_user_qr_check" => {
            forward_to_tg_streamer(reqwest::Method::GET, "/auth/qr/check", None).await
        }
        "telegram_user_qr_password" => {
            forward_to_tg_streamer(reqwest::Method::POST, "/auth/qr/password", Some(payload)).await
        }
        "telegram_user_send_code" => {
            forward_to_tg_streamer(reqwest::Method::POST, "/auth/send-code", Some(payload)).await
        }
        "telegram_user_sign_in" => {
            forward_to_tg_streamer(reqwest::Method::POST, "/auth/sign-in", Some(payload)).await
        }
        "telegram_user_logout" => {
            forward_to_tg_streamer(reqwest::Method::POST, "/auth/logout", None).await
        }
        "telegram_user_list_dialogs" => {
            forward_to_tg_streamer(reqwest::Method::GET, "/user/dialogs", None).await
        }
        "telegram_user_subscriptions" => {
            forward_to_tg_streamer(reqwest::Method::GET, "/user/subscriptions", None).await
        }
        "telegram_user_toggle_subscription" => {
            forward_to_tg_streamer(reqwest::Method::POST, "/user/subscriptions/toggle", Some(payload)).await
        }
        "telegram_user_update_subscription" => {
            forward_to_tg_streamer(reqwest::Method::POST, "/user/subscriptions/update", Some(payload)).await
        }
        "telegram_user_list_messages" => {
            let chat_id = payload.get("chat_id").and_then(Value::as_i64).or_else(|| {
                payload.get("chat_id").and_then(Value::as_str).and_then(|s| s.parse().ok())
            }).unwrap_or(0);
            let offset_id = payload.get("offset_id").and_then(Value::as_i64).unwrap_or(0);
            let limit = payload.get("limit").and_then(Value::as_i64).unwrap_or(30);
            let filter_ads = payload.get("filter_ads").and_then(Value::as_bool).unwrap_or(true);
            let filter_duplicates = payload.get("filter_duplicates").and_then(Value::as_bool).unwrap_or(true);
            let mut path = format!("/user/messages?chat_id={chat_id}&offset_id={offset_id}&limit={limit}&filter_ads={filter_ads}&filter_duplicates={filter_duplicates}");
            if let Some(sv) = payload.get("show_videos").and_then(Value::as_bool) {
                path.push_str(&format!("&show_videos={sv}"));
            }
            if let Some(sp) = payload.get("show_photos").and_then(Value::as_bool) {
                path.push_str(&format!("&show_photos={sp}"));
            }
            if let Some(sl) = payload.get("show_links").and_then(Value::as_bool) {
                path.push_str(&format!("&show_links={sl}"));
            }
            if let Some(pc) = payload.get("penetrate_comments").and_then(Value::as_bool) {
                path.push_str(&format!("&penetrate_comments={pc}"));
            }
            forward_to_tg_streamer(reqwest::Method::GET, &path, None).await
        }
        "telegram_user_list_comments" => {
            let chat_id = payload.get("chat_id").and_then(Value::as_i64).or_else(|| {
                payload.get("chat_id").and_then(Value::as_str).and_then(|s| s.parse().ok())
            }).unwrap_or(0);
            let message_id = payload.get("message_id").and_then(Value::as_i64).unwrap_or(0);
            let offset_id = payload.get("offset_id").and_then(Value::as_i64).unwrap_or(0);
            let limit = payload.get("limit").and_then(Value::as_i64).unwrap_or(50);
            let path = format!("/user/comments?chat_id={chat_id}&message_id={message_id}&offset_id={offset_id}&limit={limit}");
            forward_to_tg_streamer(reqwest::Method::GET, &path, None).await
        }
        "telegram_user_read_acknowledge" => {
            forward_to_tg_streamer(reqwest::Method::POST, "/user/read_acknowledge", Some(payload)).await
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

#[derive(Debug, Deserialize)]
struct TelegramProxyQuery {
    url: String,
}

async fn proxy_telegram_media(
    AxumState(_context): AxumState<ServerState>,
    Query(query): Query<TelegramProxyQuery>,
    headers: HeaderMap,
) -> Response {
    let raw_url = query.url.trim();
    if !raw_url.starts_with("https://") && !raw_url.starts_with("http://") {
        return (StatusCode::BAD_REQUEST, "Invalid URL").into_response();
    }
    let Ok(parsed) = url::Url::parse(raw_url) else {
        return (StatusCode::BAD_REQUEST, "Malformed URL").into_response();
    };
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    if !host.contains("telesco.pe")
        && !host.contains("telegram")
        && !host.contains("t.me")
        && !host.ends_with(".cdn-telegram.org")
    {
        return (StatusCode::FORBIDDEN, "Domain not allowed").into_response();
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(4))
        .build()
    {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let mut req = client.get(raw_url)
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36")
        .header(reqwest::header::REFERER, "https://t.me/");

    if let Some(range) = headers.get(RANGE) {
        req = req.header(reqwest::header::RANGE, range);
    }

    let upstream = match req.send().await {
        Ok(resp) => resp,
        Err(err) => return (StatusCode::BAD_GATEWAY, format!("Upstream request failed: {err}")).into_response(),
    };

    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
    let mut builder = Response::builder().status(status);

    for (k, v) in upstream.headers() {
        if matches!(
            k.as_str(),
            "content-type" | "content-length" | "content-range" | "accept-ranges" | "cache-control" | "etag" | "last-modified"
        ) {
            builder = builder.header(k, v);
        }
    }
    builder = builder.header("access-control-allow-origin", "*");
    builder = builder.header("access-control-allow-headers", "*");

    let body = Body::from_stream(upstream.bytes_stream());
    builder.body(body).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn forward_to_tg_streamer(
    method: reqwest::Method,
    path: &str,
    json_body: Option<Value>,
) -> Response {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:18082{}", path);
    let mut req = client.request(method, &url);
    if let Some(body) = json_body {
        req = req.json(&body);
    }
    match req.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::OK);
            let val = resp.json::<Value>().await.unwrap_or_else(|_| json!({"error": "Invalid response"}));
            (status, Json(val)).into_response()
        }
        Err(err) => {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok": false, "error": format!("Telegram Streamer 离线或未启动: {err}")})),
            )
                .into_response()
        }
    }
}

async fn proxy_telegram_user_stream(
    AxumState(_context): AxumState<ServerState>,
    Path((chat_id, message_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3600))
        .build()
    {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let url = format!("http://127.0.0.1:18082/stream/{chat_id}/{message_id}");
    let mut req = client.get(&url);
    if let Some(range) = headers.get(RANGE) {
        req = req.header(reqwest::header::RANGE, range);
    }

    let upstream = match req.send().await {
        Ok(resp) => resp,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("Telegram Streamer request failed: {err}"),
            )
                .into_response()
        }
    };

    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
    let mut builder = Response::builder().status(status);

    for (k, v) in upstream.headers() {
        if matches!(
            k.as_str(),
            "content-type"
                | "content-length"
                | "content-range"
                | "accept-ranges"
                | "cache-control"
                | "etag"
                | "last-modified"
                | "content-disposition"
        ) {
            builder = builder.header(k, v);
        }
    }
    builder = builder.header("access-control-allow-origin", "*");
    builder = builder.header("access-control-allow-headers", "*");

    let body = Body::from_stream(upstream.bytes_stream());
    builder.body(body).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn proxy_telegram_user_thumb(
    AxumState(_context): AxumState<ServerState>,
    Path((chat_id, message_id)): Path<(String, String)>,
) -> Response {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:18082/thumb/{chat_id}/{message_id}");
    let upstream = match client.get(&url).send().await {
        Ok(resp) => resp,
        Err(err) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("Thumb request failed: {err}"),
            )
                .into_response()
        }
    };
    let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::OK);
    let mut builder = Response::builder().status(status);
    for (k, v) in upstream.headers() {
        if matches!(k.as_str(), "content-type" | "content-length" | "cache-control") {
            builder = builder.header(k, v);
        }
    }
    let body = Body::from_stream(upstream.bytes_stream());
    builder.body(body).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[derive(Deserialize)]
struct ScraperProxyQuery {
    source: Option<String>,
    url: Option<String>,
}

const SCRAPER_IMAGE_MAX_SOURCE_LEN: usize = 16384;

#[derive(Deserialize, Default)]
struct MovieSubtitleQuery {
    format: Option<String>,
}

async fn serve_movie_subtitle(
    AxumState(context): AxumState<ServerState>,
    Path(id): Path<String>,
    Query(query): Query<MovieSubtitleQuery>,
) -> Response {
    let app = context.app.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<(String, String)>, String> {
        let conn = four_domains::open_v2(&app)?;
        crate::jav_scraper::get_movie_subtitle_content(&conn, &id)
            .map_err(|e| format!("读取字幕失败：{e}"))
    })
    .await;
    match result {
        Ok(Ok(Some((_name, content)))) => {
            let as_vtt = query.format.as_deref().map(|f| f.eq_ignore_ascii_case("vtt")).unwrap_or(false);
            let (body, mime) = if as_vtt {
                (crate::jav_scraper::srt_to_webvtt(&content), "text/vtt; charset=utf-8")
            } else {
                (content, "text/plain; charset=utf-8")
            };
            let mut resp = Response::new(Body::from(body));
            resp.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(mime));
            resp.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600"));
            resp
        }
        Ok(Ok(None)) => (StatusCode::NOT_FOUND, "Subtitle not found").into_response(),
        Ok(Err(err)) => (StatusCode::INTERNAL_SERVER_ERROR, err).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Subtitle task failed").into_response(),
    }
}

fn decode_scraper_image_source(query: ScraperProxyQuery) -> Result<String, &'static str> {
    // A present source always wins; malformed input must not fall back to url.
    if let Some(source) = query.source {
        if source.is_empty() || source.len() > SCRAPER_IMAGE_MAX_SOURCE_LEN {
            return Err("Invalid image source length");
        }
        let bytes = URL_SAFE_NO_PAD.decode(source).map_err(|_| "Invalid image source encoding")?;
        return String::from_utf8(bytes).map_err(|_| "Invalid image source UTF-8");
    }
    query.url.filter(|url| !url.trim().is_empty()).ok_or("Missing image source")
}

async fn proxy_scraper_image(
    AxumState(context): AxumState<ServerState>,
    query: Result<Query<ScraperProxyQuery>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let query = match query {
        Ok(Query(query)) => query,
        Err(_) => return scraper_image_error(StatusCode::BAD_REQUEST, "Invalid image query"),
    };
    let source = match decode_scraper_image_source(query) {
        Ok(source) => source,
        Err(message) => return scraper_image_error(StatusCode::BAD_REQUEST, message),
    };
    let url = match scraper_image_url(source.trim()) {
        Ok(url) => url,
        Err(_) => return scraper_image_error(StatusCode::BAD_REQUEST, "Invalid public image URL"),
    };
    let key = format!("{:x}", Sha256::digest(url.as_str().as_bytes()));
    let cache_path = context.app.path().app_cache_dir().ok()
        .map(|root| root.join("scraper-images").join(&key));
    if let Some(bytes) = cache_path.as_deref().and_then(read_scraper_image_cache) {
        return scraper_image_response(bytes);
    }
    // Shared gate bounds simultaneous downloads and memory use. Recheck after
    // acquiring it: a previous request may already have populated the disk cache.
    static GATE: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| tokio::sync::Semaphore::new(5));
    let _permit = match tokio::time::timeout(std::time::Duration::from_secs(90), GATE.acquire()).await {
        Ok(Ok(permit)) => permit,
        _ => return scraper_image_error(StatusCode::SERVICE_UNAVAILABLE, "Image queue busy"),
    };
    if let Some(bytes) = cache_path.as_deref().and_then(read_scraper_image_cache) {
        return scraper_image_response(bytes);
    }
    let client = match &*SCRAPER_IMAGE_CLIENT {
        Ok(client) => client,
        Err(_) => return scraper_image_error(StatusCode::INTERNAL_SERVER_ERROR, "Image client unavailable"),
    };
    // Two attempts total, not two additional retries. Never persist failures.
    for attempt in 0..2 {
        let result = tokio::time::timeout(std::time::Duration::from_secs(12), fetch_scraper_image(client, &url)).await;
        if let Ok(Ok(bytes)) = result {
            if let Some(path) = &cache_path {
                if let Err(error) = write_scraper_image_cache(path, &bytes) {
                    eprintln!("scraper image cache write failed: {error}");
                }
            }
            return scraper_image_response(bytes);
        }
        if attempt == 0 { tokio::time::sleep(std::time::Duration::from_millis(200)).await; }
    }
    scraper_image_error(StatusCode::BAD_GATEWAY, "Image source unavailable")
}

const SCRAPER_IMAGE_MAX_BYTES: usize = 20 * 1024 * 1024;
const SCRAPER_IMAGE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);

fn scraper_public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private() && !ip.is_loopback() && !ip.is_link_local()
                && !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast()
                && a != 0 && a < 224 && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (18..=19).contains(&b)) && !ip.is_documentation()
        }
        std::net::IpAddr::V6(ip) => {
            // IPv4-mapped, link/site-local and transition ranges must not bypass
            // the IPv4 checks. Ordinary public IPv6 currently lives in 2000::/3.
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000 && !(s[0] == 0x2001 && s[1] == 0x0db8)
                && s[0] != 0x2002 && !(s[0] == 0x2001 && s[1] == 0)
        }
    }
}

fn scraper_image_url(value: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(value).map_err(|_| "invalid URL")?;
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return Err("invalid scheme or credentials".into());
    }
    let host = url.host_str().ok_or("missing host")?;
    if host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") || host.ends_with(".local") {
        return Err("local hostname".into());
    }
    if let Some(host) = url.host() {
        let ip = match host {
            url::Host::Ipv4(ip) => Some(std::net::IpAddr::V4(ip)),
            url::Host::Ipv6(ip) => Some(std::net::IpAddr::V6(ip)),
            _ => None,
        };
        if ip.is_some_and(|ip| !scraper_public_ip(ip)) { return Err("non-public address".into()); }
    }
    url.set_fragment(None);
    Ok(url)
}

#[derive(Debug)]
struct ScraperPublicDns;
impl reqwest::dns::Resolve for ScraperPublicDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let addresses: Vec<_> = tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            // Reject mixed public/private answers too; never let a DNS rebinding
            // target reach a local address. Resolver is used on every new connection.
            if addresses.is_empty() || addresses.iter().any(|address| !scraper_public_ip(address.ip())) {
                return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "non-public DNS address").into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

static SCRAPER_IMAGE_CLIENT: LazyLock<Result<reqwest::Client, reqwest::Error>> = LazyLock::new(|| {
    reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(std::sync::Arc::new(ScraperPublicDns))
        .connect_timeout(std::time::Duration::from_secs(6))
        .timeout(std::time::Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 4 { attempt.error("too many image redirects") }
            else if scraper_image_url(attempt.url().as_str()).is_err() { attempt.error("non-public image redirect") }
            else { attempt.follow() }
        }))
        .build()
});

fn scraper_image_referer(url: &reqwest::Url) -> &'static str {
    let host = url.host_str().unwrap_or("");
    let domain = |suffix: &str| host == suffix || host.ends_with(&format!(".{suffix}"));
    if domain("fc2.com") { "https://adult.contents.fc2.com/" }
    else if domain("jdbstatic.com") || domain("javdb.com") { "https://javdb.com/" }
    else if domain("javbus.com") { "https://www.javbus.com/" }
    else if domain("caribbeancom.com") { "https://www.caribbeancom.com/" }
    else if domain("1pondo.tv") { "https://www.1pondo.tv/" }
    else if domain("dmm.co.jp") || domain("dmm.com") { "https://www.dmm.co.jp/" }
    else { "https://google.com/" }
}

fn scraper_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() > SCRAPER_IMAGE_MAX_BYTES { return None; }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) && bytes.len() >= 12 { Some("image/jpeg") }
    else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 && &bytes[12..16] == b"IHDR" { Some("image/png") }
    else if (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) && bytes.len() >= 13 { Some("image/gif") }
    else if bytes.len() >= 16 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" { Some("image/webp") }
    else if bytes.len() >= 16 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"avif" | b"avis") { Some("image/avif") }
    else { None }
}

async fn fetch_scraper_image(client: &reqwest::Client, url: &reqwest::Url) -> Result<Vec<u8>, String> {
    let response = client.get(url.clone())
        .header(reqwest::header::USER_AGENT, "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header(reqwest::header::REFERER, scraper_image_referer(url))
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > SCRAPER_IMAGE_MAX_BYTES as u64) {
        return Err("unsuccessful or oversized image response".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if chunk.len() > SCRAPER_IMAGE_MAX_BYTES.saturating_sub(bytes.len()) { return Err("image too large".into()); }
        bytes.extend_from_slice(&chunk);
    }
    scraper_image_mime(&bytes).ok_or("response is not an image")?;
    Ok(bytes)
}

fn read_scraper_image_cache(path: &std::path::Path) -> Option<Vec<u8>> {
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > SCRAPER_IMAGE_MAX_BYTES as u64 { return None; }
    if let Ok(modified) = metadata.modified() {
        if modified.elapsed().ok()?.gt(&SCRAPER_IMAGE_CACHE_TTL) { return None; }
    }
    let bytes = std::fs::read(path).ok()?;
    scraper_image_mime(&bytes)?;
    Some(bytes)
}

fn write_scraper_image_cache(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if scraper_image_mime(bytes).is_none() { return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not an image")); }
    std::fs::create_dir_all(path.parent().ok_or_else(|| std::io::Error::other("missing cache parent"))?)?;
    let temporary = path.with_extension(format!("{}.tmp", rand::random::<u64>()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        match std::fs::rename(&temporary, path) {
            Ok(()) => Ok(()),
            Err(_) if read_scraper_image_cache(path).is_some() => Ok(()),
            Err(error) => Err(error),
        }
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

fn scraper_image_error(status: StatusCode, message: &'static str) -> Response {
    (status, [(CACHE_CONTROL, "no-store")], message).into_response()
}

fn scraper_image_response(bytes: Vec<u8>) -> Response {
    let Some(mime) = scraper_image_mime(&bytes) else {
        return scraper_image_error(StatusCode::BAD_GATEWAY, "Invalid cached image");
    };
    // These are our decoded bytes: never forward upstream content-length or encoding.
    ([(CONTENT_TYPE, mime), (CACHE_CONTROL, "public, max-age=86400"),
      (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff")], bytes).into_response()
}

#[cfg(test)]
mod scraper_image_tests {
    use super::*;
    #[test]
    fn source_roundtrips_unicode_and_query_strings() {
        for url in [
            "https://www.javbus.com/封面/映画😀.jpg?title=中文&token=a+b/c==&next=%2Fpath%3Fx%3D1#preview",
            "https://example.com/a.jpg?x=1&x=2&empty=&escaped=%26%3D%25",
        ] {
            let source = URL_SAFE_NO_PAD.encode(url.as_bytes());
            assert!(!source.contains(['+', '/', '=']));
            let uri = format!("/api/scraper/proxy-image?source={source}").parse().unwrap();
            let Query(query) = Query::<ScraperProxyQuery>::try_from_uri(&uri).unwrap();
            assert_eq!(decode_scraper_image_source(query).unwrap(), url);
        }
    }
    #[test]
    fn source_rejects_invalid_base64_and_utf8() {
        for source in ["", "a", "aGVsbG8=", "+w", "/w", "a Gk", "_w", "Zh"] {
            assert!(decode_scraper_image_source(ScraperProxyQuery {
                source: Some(source.into()), url: Some("https://example.com/legacy.jpg".into()),
            }).is_err(), "{source}");
        }
    }
    #[test]
    fn source_enforces_encoded_length_limit() {
        let source = URL_SAFE_NO_PAD.encode("a".repeat(SCRAPER_IMAGE_MAX_SOURCE_LEN / 4 * 3));
        assert_eq!(source.len(), SCRAPER_IMAGE_MAX_SOURCE_LEN);
        assert!(decode_scraper_image_source(ScraperProxyQuery {
            source: Some(source.clone()), url: None,
        }).is_ok());
        assert_eq!(decode_scraper_image_source(ScraperProxyQuery {
            source: Some(format!("{source}A")), url: None,
        }), Err("Invalid image source length"));
    }
    #[test]
    fn source_takes_precedence_and_legacy_url_remains_supported() {
        let url = "https://example.com/封面.jpg?x=a+b&next=%2Fpath";
        let query_string: String = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("url", url).finish();
        let uri = format!("/api/scraper/proxy-image?{query_string}").parse().unwrap();
        let Query(query) = Query::<ScraperProxyQuery>::try_from_uri(&uri).unwrap();
        assert_eq!(decode_scraper_image_source(query).unwrap(), url);
        assert_eq!(decode_scraper_image_source(ScraperProxyQuery {
            source: Some(URL_SAFE_NO_PAD.encode(url.as_bytes())),
            url: Some("https://example.com/ignored.jpg".into()),
        }).unwrap(), url);
    }
    #[test]
    fn source_rejects_missing_or_blank_legacy_url() {
        let uri = "/api/scraper/proxy-image".parse().unwrap();
        let Query(query) = Query::<ScraperProxyQuery>::try_from_uri(&uri).unwrap();
        assert!(decode_scraper_image_source(query).is_err());
        for url in ["", " \t "] {
            assert!(decode_scraper_image_source(ScraperProxyQuery {
                source: None, url: Some(url.into()),
            }).is_err());
        }
        let response = scraper_image_error(StatusCode::BAD_REQUEST, "Invalid image source");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
    }
    #[test]
    fn rejects_private_urls_and_host_spoofing() {
        for url in ["file:///tmp/a.jpg", "http://127.0.0.1/a", "http://2130706433/a", "http://10.0.0.1/a", "http://169.254.169.254/a", "http://[::1]/a", "http://[::ffff:127.0.0.1]/a", "http://localhost/a", "https://user:pass@example.com/a"] {
            assert!(scraper_image_url(url).is_err(), "{url}");
        }
        let url = scraper_image_url("https://example.com/a?javbus.com").unwrap();
        assert_eq!(scraper_image_referer(&url), "https://google.com/");
        assert_eq!(scraper_image_referer(&scraper_image_url("https://contents-thumbnail2.fc2.com/a.jpg").unwrap()), "https://adult.contents.fc2.com/");
        assert!(scraper_public_ip("8.8.8.8".parse().unwrap()));
        assert!(!scraper_public_ip("100.64.0.1".parse().unwrap()));
    }
    #[test]
    fn validates_magic_size_and_response_headers() {
        assert!(scraper_image_mime(b"<html>blocked</html>").is_none());
        assert!(scraper_image_mime(b"{\"error\":\"blocked\"}").is_none());
        let mut bytes = vec![0xff, 0xd8, 0xff]; bytes.resize(32, 0);
        assert_eq!(scraper_image_mime(&bytes), Some("image/jpeg"));
        let response = scraper_image_response(bytes);
        assert_eq!(response.headers()[CONTENT_TYPE], "image/jpeg");
        assert!(!response.headers().contains_key("content-encoding"));
        assert_eq!(scraper_image_error(StatusCode::BAD_GATEWAY, "failed").headers()[CACHE_CONTROL], "no-store");
        assert!(scraper_image_mime(&vec![0; SCRAPER_IMAGE_MAX_BYTES + 1]).is_none());
    }
    #[test]
    fn disk_cache_only_persists_valid_images_atomically() {
        let directory = std::env::temp_dir().join(format!("pml-scraper-image-test-{}", rand::random::<u64>()));
        let path = directory.join("test-key");
        assert!(write_scraper_image_cache(&path, b"<html>no</html>").is_err());
        assert!(!path.exists());
        let mut bytes = vec![0xff, 0xd8, 0xff]; bytes.resize(32, 0);
        write_scraper_image_cache(&path, &bytes).unwrap();
        assert_eq!(read_scraper_image_cache(&path), Some(bytes));
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::write(&path, b"not an image").unwrap();
        assert!(read_scraper_image_cache(&path).is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
