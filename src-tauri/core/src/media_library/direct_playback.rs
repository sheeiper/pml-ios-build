//! Strict playback resolver: control-plane JSON only, never registers or opens a media stream.
use super::*;
use std::{collections::HashMap, sync::{Mutex as StdMutex, OnceLock}};
use std::time::{Duration, Instant};

static DIRECT_TOKEN_CACHE: OnceLock<StdMutex<HashMap<String, (String, Instant)>>> = OnceLock::new();
static DIRECT_URL_CACHE: OnceLock<StdMutex<HashMap<String, (String, Instant)>>> = OnceLock::new();
static DIRECT_CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_else(|_| Client::new())
});
const DIRECT_TOKEN_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const DIRECT_URL_TTL: Duration = Duration::from_secs(60);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectPlaybackSource {
    url: String,
}

fn mapped_path(remote: &str, source: &str, target: &str) -> Result<String, String> {
    let source = source.trim().trim_end_matches('/');
    let target = target.trim().trim_end_matches('/');
    if source.is_empty() || !(remote == source || remote.starts_with(&format!("{source}/"))) {
        return Err("Direct path is outside the configured CD2 prefix; check the AList path mapping".into());
    }
    Ok(format!("{target}{}", &remote[source.len()..]))
}

/// Build the stable AList download route instead of returning fs/get.data.raw_url.
/// raw_url is commonly a short-lived provider signature; /d/<path> lets AList
/// issue a fresh redirect at playback time.
fn validated_download_url(body: &serde_json::Value, provider: &Url, path: &str) -> Result<String, String> {
    if body.get("code").and_then(|v| v.as_i64()) != Some(200) {
        return Err("AList direct-link resolution failed; check storage login and path configuration".into());
    }
    if path.is_empty() || !path.starts_with('/') || path.split('/').any(|part| part == "..") {
        return Err("Invalid AList direct-link path".into());
    }
    let route = alist_route_root_url(provider, "d")?;
    let mut url = route;
    let route_path = format!("{}/{}", url.path().trim_end_matches('/'), path.trim_start_matches('/'));
    url.set_path(&route_path);
    url.set_query(None);
    url.set_fragment(None);

    // Some AList builds protect /d with a short-lived `sign` returned next to
    // the route. Keep that signature on the direct URL; dropping it makes the
    // otherwise-correct anonymous AVPlayer request fail with 401/403. Only
    // carry the scalar sign value, never arbitrary query data from the API.
    if let Some(sign) = body
        .pointer("/data/sign")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        url.query_pairs_mut().append_pair("sign", sign);
    }
    Ok(url.to_string())
}

pub async fn resolve_direct_playback(
    app: AppHandle,
    state: State<'_, MediaLibraryState>,
    asset_id: Option<String>,
    remote_path: Option<String>,
) -> Result<DirectPlaybackSource, String> {
    // Asset ID takes precedence over a client-supplied path; movie sheets only have an ID.
    let path = if let Some(id) = asset_id.filter(|id| !id.is_empty()) {
        let connection = open_media_database(&app)?;
        connection
            .query_row(
                "SELECT remote_path FROM assets WHERE id = ?1 AND missing = 0",
                params![id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| "Video not found; refresh the media library".to_string())?
    } else {
        remote_path
            .filter(|p| p.starts_with('/') && !p.split('/').any(|s| s == ".."))
            .ok_or_else(|| "A valid video path is required".to_string())?
    };
    let config = get_alist_accel_config(app.clone())?;
    let (provider, token, path) = if config.enabled {
        let endpoint = Url::parse(config.api_url.trim()).map_err(|_| "Invalid AList service URL".to_string())?;
        let path = mapped_path(&path, &config.cd2_prefix, &config.alist_prefix)?;
        // Use an isolated, no-redirect client below so API credentials cannot follow redirects.
        // Reuse the short-lived control-plane token: opening a file should not log in to AList
        // again, especially when the user quickly scrubs through a playlist.
        let token = direct_login(&endpoint, &config.username, config.password.as_deref().unwrap_or_default()).await?;
        (endpoint, token, path)
    } else {
        let session = require_session(&state).await?;
        if session.provider != ProviderKind::Alist {
            return Err("Strict direct playback requires an enabled AList direct-link configuration; relay fallback is disabled".into());
        }
        (session.endpoint, session.api_token, path)
    };
    let cache_key = format!("{}|{}", provider, path);
    if let Some(cached) = DIRECT_URL_CACHE
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock().ok()
        .and_then(|mut guard| guard.get(&cache_key).cloned().and_then(|(url, at)| {
            if at.elapsed() < DIRECT_URL_TTL { Some(url) } else { guard.remove(&cache_key); None }
        }))
    {
        return Ok(DirectPlaybackSource { url: cached });
    }
    let response = DIRECT_CLIENT
        .post(alist_api_url(&provider, "fs/get")?)
        .header(AUTHORIZATION, token)
        .json(&serde_json::json!({"path": path, "password": "", "refresh": false}))
        .send()
        .await
        .map_err(|_| "AList direct-link resolution connection failed".to_string())?;
    if !response.status().is_success() {
        return Err("AList direct-link resolution request failed; relay fallback is disabled".into());
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Invalid AList direct-link resolution response".to_string())?;
    let url = validated_download_url(&body, &provider, &path)?;
    // AList may attach a short-lived sign; never retain that credential beyond
    // this response. Unsigned /d paths are stable and safe to reuse briefly.
    if !url.contains("?sign=") && !url.contains("&sign=") {
        if let Ok(mut guard) = DIRECT_URL_CACHE.get_or_init(|| StdMutex::new(HashMap::new())).lock() {
            guard.insert(cache_key, (url.clone(), Instant::now()));
        }
    }
    Ok(DirectPlaybackSource { url })
}

fn direct_client() -> &'static Client {
    &DIRECT_CLIENT
}

async fn direct_login(endpoint: &Url, username: &str, password: &str) -> Result<String, String> {
    if !matches!(endpoint.scheme(), "http" | "https") || !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err("Invalid AList service URL".into());
    }
    // Include a one-way password fingerprint so changing credentials cannot
    // reuse a token issued for the previous password without logging secrets.
    let password_fingerprint = format!("{:x}", Sha256::digest(password.as_bytes()));
    let cache_key = format!("{}|{}|{}", endpoint, username, password_fingerprint);
    if let Ok(mut guard) = DIRECT_TOKEN_CACHE.get_or_init(|| StdMutex::new(HashMap::new())).lock() {
        if let Some((token, created)) = guard.get(&cache_key).cloned() {
            if created.elapsed() < DIRECT_TOKEN_TTL {
                return Ok(token);
            }
            guard.remove(&cache_key);
        }
    }
    let response = direct_client()
        .post(alist_api_url(endpoint, "auth/login")?)
        .json(&serde_json::json!({"username": username, "password": password}))
        .send()
        .await
        .map_err(|_| "AList direct-link login failed".to_string())?;
    if !response.status().is_success() {
        return Err("AList direct-link login failed".into());
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Invalid AList login response".to_string())?;
    let token = body.pointer("/data/token")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "AList login failed; check username and password".to_string())?;
    if let Ok(mut guard) = DIRECT_TOKEN_CACHE.get_or_init(|| StdMutex::new(HashMap::new())).lock() {
        guard.insert(cache_key, (token.clone(), Instant::now()));
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_mapping_requires_segment_boundary() {
        assert_eq!(mapped_path("/115open/movie/a #1.mp4", "/115open/", "/115/").unwrap(), "/115/movie/a #1.mp4");
        assert!(mapped_path("/115open-other/a.mp4", "/115open", "/115").is_err());
        assert!(mapped_path("/a.mp4", "", "/115").is_err());
    }

    #[test]
    fn direct_resolver_uses_stable_download_route_not_signed_raw_url() {
        let provider = Url::parse("https://alist.example:5244/alist").unwrap();
        let body = serde_json::json!({"code":200,"data":{"raw_url":"https://115cdn.example/a.mp4?sign=expired"}});
        assert_eq!(
            validated_download_url(&body, &provider, "/115/movie/a #1.mp4").unwrap(),
            "https://alist.example:5244/alist/d/115/movie/a%20%231.mp4"
        );
        assert!(validated_download_url(&serde_json::json!({"code":403}), &provider, "/115/a.mp4").is_err());
        for path in ["", "a.mp4", "/115/../secret.mp4"] {
            assert!(validated_download_url(&body, &provider, path).is_err(), "accepted {path}");
        }
    }
}
