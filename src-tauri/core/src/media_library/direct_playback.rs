//! Strict playback resolver: control-plane JSON only, never registers or opens a media stream.
use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectPlaybackSource {
    url: String,
}

fn mapped_path(remote: &str, source: &str, target: &str) -> Result<String, String> {
    let source = source.trim().trim_end_matches('/');
    let target = target.trim().trim_end_matches('/');
    if source.is_empty() || !(remote == source || remote.starts_with(&format!("{source}/"))) {
        return Err("直链路径不在配置的 CD2 前缀下，请检查 AList 路径映射".into());
    }
    Ok(format!("{target}{}", &remote[source.len()..]))
}

fn validated_raw_url(body: &serde_json::Value, provider: &Url) -> Result<String, String> {
    if body.get("code").and_then(|v| v.as_i64()) != Some(200) {
        // Do not surface provider response bodies: they may contain tokens or signed URLs.
        return Err("AList 直链解析失败，请检查存储登录状态和路径配置".into());
    }
    let raw = body.pointer("/data/raw_url").and_then(|v| v.as_str()).unwrap_or_default();
    let url = Url::parse(raw).map_err(|_| "AList 未返回有效的原始直链".to_string())?;
    let path = percent_decode_str(url.path()).decode_utf8_lossy().to_ascii_lowercase();
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none()
        || !url.username().is_empty() || url.password().is_some()
        || url.origin() == provider.origin()
        || matches!(url.host_str(), Some("localhost" | "[::1]" | "::1" | "0.0.0.0"))
        || url.host_str().is_some_and(|host| host.starts_with("127."))
        || path.split('/').any(|part| matches!(part, "media" | "dav" | "d" | "p"))
        || path.starts_with("/api/")
    {
        return Err("上游只提供代理地址或含账号凭证的地址，严格直链播放已拒绝中转".into());
    }
    Ok(url.into())
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
        connection.query_row("SELECT remote_path FROM assets WHERE id = ?1 AND missing = 0", params![id], |row| row.get::<_, String>(0))
            .map_err(|_| "找不到待播放的视频，请刷新媒体库".to_string())?
    } else {
        remote_path.filter(|p| p.starts_with('/') && !p.split('/').any(|s| s == ".."))
            .ok_or_else(|| "缺少有效的视频路径".to_string())?
    };
    let config = get_alist_accel_config(app.clone())?;
    let (provider, token, path) = if config.enabled {
        let endpoint = Url::parse(config.api_url.trim()).map_err(|_| "AList 服务地址无效".to_string())?;
        let path = mapped_path(&path, &config.cd2_prefix, &config.alist_prefix)?;
        // Use an isolated, no-redirect client below so API credentials cannot follow redirects.
        let token = direct_login(&endpoint, &config.username, config.password.as_deref().unwrap_or_default()).await?;
        (endpoint, token, path)
    } else {
        let session = require_session(&state).await?;
        if session.provider != ProviderKind::Alist {
            return Err("严格直链播放需要配置并启用 AList 直链解析；不会回退服务器中转".into());
        }
        (session.endpoint, session.api_token, path)
    };
    let client = direct_client()?;
    let response = client.post(alist_api_url(&provider, "fs/get")?)
        .header(AUTHORIZATION, token)
        .json(&serde_json::json!({"path": path, "password": "", "refresh": true}))
        .send().await.map_err(|_| "AList 直链解析连接失败".to_string())?;
    if !response.status().is_success() {
        return Err("AList 直链解析请求失败，不会回退中转".into());
    }
    let body: serde_json::Value = response.json().await.map_err(|_| "AList 直链解析响应无效".to_string())?;
    Ok(DirectPlaybackSource { url: validated_raw_url(&body, &provider)? })
}

fn direct_client() -> Result<Client, String> {
    Client::builder().redirect(Policy::none()).timeout(Duration::from_secs(15))
        .build().map_err(|_| "无法初始化直链解析连接".to_string())
}

async fn direct_login(endpoint: &Url, username: &str, password: &str) -> Result<String, String> {
    if !matches!(endpoint.scheme(), "http" | "https") || !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err("AList 服务地址无效".into());
    }
    let response = direct_client()?.post(alist_api_url(endpoint, "auth/login")?)
        .json(&serde_json::json!({"username": username, "password": password}))
        .send().await.map_err(|_| "AList 直链解析登录失败".to_string())?;
    if !response.status().is_success() { return Err("AList 直链解析登录失败".into()); }
    let body: serde_json::Value = response.json().await.map_err(|_| "AList 登录响应无效".to_string())?;
    body.pointer("/data/token").and_then(|v| v.as_str()).filter(|v| !v.is_empty())
        .map(str::to_owned).ok_or_else(|| "AList 登录失败，请检查账号密码".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn direct_mapping_requires_segment_boundary() {
        assert_eq!(mapped_path("/115open/电影/a #1.mp4", "/115open/", "/115/").unwrap(), "/115/电影/a #1.mp4");
        assert!(mapped_path("/115open-other/a.mp4", "/115open", "/115").is_err());
        assert!(mapped_path("/a.mp4", "", "/115").is_err());
    }
    #[test]
    fn direct_resolver_uses_raw_url_never_signed_proxy_or_credentials() {
        let provider = Url::parse("https://alist.example:5244").unwrap();
        let body = |raw: &str| serde_json::json!({"code":200,"data":{"raw_url":raw,"url":"https://alist.example:5244/p/a.mp4"}});
        assert_eq!(validated_raw_url(&body("https://cdn.example/a.mp4?sign=abc"), &provider).unwrap(), "https://cdn.example/a.mp4?sign=abc");
        for raw in ["https://alist.example:5244/d/a.mp4", "https://user:secret@cdn.example/a.mp4", "file:///tmp/a.mp4", "https://pml.example/media/a.mp4", "https://pml.example/%6dedia/a.mp4", "https://pml.example/api/telegram_user_stream", ""] {
            assert!(validated_raw_url(&body(raw), &provider).is_err(), "accepted {raw}");
        }
        assert!(validated_raw_url(&serde_json::json!({"code":200,"data":{"url":"https://alist.example/d/a.mp4"}}), &provider).is_err());
        assert!(validated_raw_url(&serde_json::json!({"code":403,"data":{"raw_url":"https://cdn.example/a.mp4"}}), &provider).is_err());
    }
}
