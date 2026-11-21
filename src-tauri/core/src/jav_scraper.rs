use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieMetadata {
    pub number: String,
    pub asset_id: Option<String>,
    pub title: String,
    pub original_title: Option<String>,
    pub actors: Vec<String>,
    pub maker: Option<String>,
    pub label: Option<String>,
    pub series: Option<String>,
    pub director: Option<String>,
    pub release_date: Option<String>,
    pub duration_minutes: Option<i32>,
    pub cover_url: Option<String>,
    pub sample_images: Vec<String>,
    pub tags: Vec<String>,
    pub source: String,
    pub updated_at: i64,
}

/// Ensure database schema for movie metadata and scraper whitelist.
pub fn ensure_scraper_schema(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS movie_metadata (
            number TEXT PRIMARY KEY,
            asset_id TEXT,
            title TEXT NOT NULL,
            original_title TEXT,
            actors_json TEXT NOT NULL DEFAULT '[]',
            maker TEXT,
            label TEXT,
            series TEXT,
            director TEXT,
            release_date TEXT,
            duration_minutes INTEGER,
            cover_url TEXT,
            sample_images_json TEXT NOT NULL DEFAULT '[]',
            tags_json TEXT NOT NULL DEFAULT '[]',
            source TEXT NOT NULL DEFAULT 'javdb',
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_movie_metadata_maker ON movie_metadata(maker);
        CREATE INDEX IF NOT EXISTS idx_movie_metadata_series ON movie_metadata(series);
        CREATE INDEX IF NOT EXISTS idx_movie_metadata_date ON movie_metadata(release_date);

        CREATE TABLE IF NOT EXISTS movie_asset_bindings (
            asset_id TEXT PRIMARY KEY,
            number TEXT NOT NULL REFERENCES movie_metadata(number) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_movie_asset_bindings_number ON movie_asset_bindings(number);
        CREATE TABLE IF NOT EXISTS scrape_whitelist (
            path TEXT PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );"
    )?;
    let has_version = {
        let mut stmt = conn.prepare("PRAGMA table_info(movie_metadata)")?;
        let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
        let mut found = false;
        for name in names { if name? == "verification_version" { found = true; } }
        found
    };
    if !has_version {
        // Preserve legacy metadata, but quarantine unverified search-first results.
        conn.execute("ALTER TABLE movie_metadata ADD COLUMN verification_version INTEGER NOT NULL DEFAULT 0", [])?;
    }
    Ok(())
}

/// Bind an existing movie to each real file; duplicated editions/parts do not
/// overwrite each other. Asset IDs are opaque and case-sensitive.
pub fn bind_movie_asset(conn: &Connection, number: &str, asset_id: &str) -> Result<(), rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    conn.execute("INSERT INTO movie_asset_bindings(asset_id, number) VALUES (?1, ?2)
        ON CONFLICT(asset_id) DO UPDATE SET number = excluded.number", params![asset_id, number])?;
    conn.execute("UPDATE movie_metadata SET asset_id = NULL WHERE asset_id = ?2 AND number != ?1", params![number, asset_id])?;
    conn.execute("UPDATE movie_metadata SET asset_id = COALESCE(asset_id, ?2) WHERE number = ?1", params![number, asset_id])?;
    Ok(())
}

/// Extract an identity from a filename, never from a movie title or parent directory.
/// Date identities retain their delimiter: 1pondo uses `_`, Caribbeancom uses `-`.
pub fn extract_jav_number(raw_name: &str) -> Option<String> {
    let name = raw_name.rsplit(['/', '\\']).next()?.trim();
    let name = match name.rsplit_once('.') {
        Some((stem, ext)) if ["mp4", "mkv", "avi", "wmv", "mov", "m4v", "ts", "webm", "iso"].contains(&ext.to_ascii_lowercase().as_str()) => stem,
        _ => name,
    };
    let upper = name.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let mut candidates = std::collections::BTreeSet::new();
    for start in 0..bytes.len() {
        if !upper.is_char_boundary(start) { continue; }
        if start > 0 && bytes[start - 1].is_ascii_alphanumeric() { continue; }
        if upper[start..].starts_with("FC2") {
            let mut i = start + 3;
            while i < bytes.len() && matches!(bytes[i], b'-' | b'_' | b' ') { i += 1; }
            if upper[i..].starts_with("PPV") { i += 3; }
            while i < bytes.len() && matches!(bytes[i], b'-' | b'_' | b' ') { i += 1; }
            let n = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
            if (5..=8).contains(&(i - n)) && (i == bytes.len() || !bytes[i].is_ascii_alphanumeric()) {
                candidates.insert(format!("FC2-{}", &upper[n..i]));
            }
            continue;
        }
        if bytes[start].is_ascii_digit() {
            let end = start + 10;
            if end <= bytes.len() && bytes[start..start + 6].iter().all(u8::is_ascii_digit)
                && matches!(bytes[start + 6], b'-' | b'_')
                && bytes[start + 7..end].iter().all(u8::is_ascii_digit)
                && (end == bytes.len() || !bytes[end].is_ascii_alphanumeric()) {
                candidates.insert(upper[start..end].to_string());
            }
            continue;
        }
        if !bytes[start].is_ascii_alphabetic() { continue; }
        let mut i = start;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() { i += 1; }
        let letters = &upper[start..i];
        if !(2..=7).contains(&letters.len()) || ["FHD", "HD", "UHD", "MP", "MKV", "CH", "H264", "H265", "X264", "X265", "PPV", "CARIB"].contains(&letters) { continue; }
        if i < bytes.len() && matches!(bytes[i], b'-' | b'_' | b' ') { i += 1; }
        let n = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
        if (2..=6).contains(&(i - n)) && (i == bytes.len() || !bytes[i].is_ascii_alphabetic() || upper[i..].starts_with("CH")) {
            candidates.insert(format!("{}-{}", letters, &upper[n..i]));
        }
    }
    // Multiple different identities are ambiguous; do not silently choose the first.
    if candidates.len() == 1 { candidates.into_iter().next() } else { None }
}

/// Strict API identity validation (unlike filename extraction, rejects extra text).
pub fn normalize_jav_number(raw: &str) -> Option<String> {
    let upper = raw.trim().to_ascii_uppercase();
    let number = extract_jav_number(&upper)?;
    let compact = upper.replace(['-', '_', ' '], "");
    let expected = number.replace(['-', '_'], "");
    if compact == expected || (number.starts_with("FC2-") && compact == expected.replacen("FC2", "FC2PPV", 1)) {
        Some(number)
    } else { None }
}

fn scraper_client() -> Result<reqwest::Client, String> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() { return Ok(client.clone()); }
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build().map_err(|e| format!("构建 HTTP 客户端失败：{e}"))?;
    let _ = CLIENT.set(client.clone());
    Ok(client)
}

// Existing lightweight parsers use byte windows. Round to UTF-8 boundaries to
// avoid panics when a window ends inside a Japanese/Chinese character.
fn html_window(html: &str, mut start: usize, mut end: usize) -> &str {
    start = start.min(html.len());
    end = end.min(html.len());
    while !html.is_char_boundary(start) { start += 1; }
    while !html.is_char_boundary(end) { end -= 1; }
    if start > end { "" } else { &html[start..end] }
}

fn plain_text(html: &str) -> String {
    let mut text = String::new();
    let mut tag = false;
    for c in html.chars() {
        match c { '<' => { tag = true; text.push(' '); }, '>' => tag = false,
            _ if !tag => text.push(c), _ => {} }
    }
    text.replace("&nbsp;", " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn javbus_actors(html: &str) -> Vec<String> {
    let mut actors = Vec::new();
    for seg in html.split("class=\"star-name\"").skip(1) {
        let block = seg.split("</div>").next().unwrap_or("");
        for link in block.split("<a ").skip(1) {
            if let Some((_, body)) = link.split_once('>') {
                if let Some((body, _)) = body.split_once("</a>") {
                    let name = plain_text(body);
                    if !name.is_empty() && name.chars().count() <= 50 && !actors.contains(&name) { actors.push(name); }
                }
            }
        }
    }
    actors
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{name}={quote}");
        if let Some((_, rest)) = tag.split_once(&needle) {
            return rest.split_once(quote).map(|(value, _)| value.replace("&amp;", "&"));
        }
    }
    None
}

/// Only a card's dedicated <strong> identifier may establish its identity.
/// Titles mentioning another number must never qualify a search result.
fn javdb_exact_card<'a>(html: &'a str, number: &str) -> Option<(String, &'a str)> {
    for part in html.split("<a ").skip(1) {
        let (tag, body) = part.split_once('>')?;
        let Some(path) = attribute(tag, "href") else { continue; };
        if !path.starts_with("/v/") || path.contains(['?', '#']) { continue; }
        let Some((body, _)) = body.split_once("</a>") else { continue; };
        let Some((_, strong)) = body.split_once("<strong") else { continue; };
        let Some((_, strong)) = strong.split_once('>') else { continue; };
        let Some((identity, _)) = strong.split_once("</strong>") else { continue; };
        if normalize_jav_number(&plain_text(identity)).as_deref() == Some(number) {
            return Some((path, body));
        }
    }
    None
}

fn detail_identity(html: &str, labels: &[&str]) -> Option<String> {
    // Identity fields are bounded to their own paragraph/panel block.
    for label in labels {
        if let Some((_, rest)) = html.split_once(label) {
            let end = rest.find("</p>").or_else(|| rest.find("</div>")).unwrap_or(rest.len());
            let text = plain_text(&rest[..end]);
            if let Some(number) = text.split_whitespace().find_map(normalize_jav_number) { return Some(number); }
        }
    }
    None
}

/// Fetch metadata from JavBus by JAV number (Direct, fast, highly reliable for commercial JAV).
pub async fn scrape_from_javbus(number: &str) -> Result<MovieMetadata, String> {
    let client = scraper_client()?;

    let url = format!("https://www.javbus.com/{}", number);
    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,ja;q=0.8,en;q=0.7")
        .header("Cookie", "existmag=mag;")
        .send()
        .await
        .map_err(|e| format!("请求 JavBus 失败：{e}"))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("JavBus 未收录番号【{number}】"));
    }
    if !resp.status().is_success() {
        return Err(format!("JavBus 响应状态异常：{}", resp.status()));
    }

    let html = resp.text().await.map_err(|e| format!("读取 JavBus 响应失败：{e}"))?;
    if html.contains("404 Page Not Found") || html.contains("404 Not Found") {
        return Err(format!("JavBus 未找到番号【{number}】的作品"));
    }

    if detail_identity(&html, &["識別碼:</span>", "识别码:</span>"]).as_deref() != Some(number) {
        return Err(format!("JavBus 详情番号不匹配【{number}】"));
    }

    let html = html.split("id=\"related-waterfall\"").next().unwrap_or(&html).to_string();
    // 1. Title
    let mut title = number.to_string();
    if let Some(start) = html.find("<h3>") {
        let rest = &html[start + 4..];
        if let Some(end) = rest.find("</h3>") {
            let t = rest[..end].trim();
            if !t.is_empty() {
                title = t.to_string();
            }
        }
    }

    // 2. Big Cover
    let mut cover_url = None;
    if let Some(start) = html.find("<a class=\"bigImage\"") {
        let rest = html_window(&html, start, start + 250);
        if let Some(href_idx) = rest.find("href=\"") {
            let href_rest = &rest[href_idx + 6..];
            if let Some(href_end) = href_rest.find('"') {
                let c = href_rest[..href_end].trim().to_string();
                if !c.is_empty() {
                    cover_url = valid_image_url("https://www.javbus.com", &c);
                }
            }
        }
    }

    // 3. Release Date
    let mut release_date = None;
    if let Some(start) = html.find("發行日期:</span>") {
        let rest = html_window(&html, start + "發行日期:</span>".len(), start + 100);
        let date_chars: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
        let trimmed_date = date_chars.trim();
        if !trimmed_date.is_empty() && trimmed_date.contains('-') {
            release_date = Some(trimmed_date.to_string());
        }
    }

    // 4. Maker (制作商)
    let mut maker = None;
    if let Some(start) = html.find("製作商:</span>") {
        let rest = html_window(&html, start, start + 200);
        if let Some(a_start) = rest.find("<a ") {
            if let Some(txt_start) = rest[a_start..].find('>') {
                let txt_rest = &rest[a_start + txt_start + 1..];
                if let Some(txt_end) = txt_rest.find("</a>") {
                    let m = txt_rest[..txt_end].trim();
                    if !m.is_empty() {
                        maker = Some(m.to_string());
                    }
                }
            }
        }
    }

    // 5. Label (发行商)
    let mut label = None;
    if let Some(start) = html.find("發行商:</span>") {
        let rest = html_window(&html, start, start + 200);
        if let Some(a_start) = rest.find("<a ") {
            if let Some(txt_start) = rest[a_start..].find('>') {
                let txt_rest = &rest[a_start + txt_start + 1..];
                if let Some(txt_end) = txt_rest.find("</a>") {
                    let l = txt_rest[..txt_end].trim();
                    if !l.is_empty() {
                        label = Some(l.to_string());
                    }
                }
            }
        }
    }

    // 6. Series (系列)
    let mut series = None;
    if let Some(start) = html.find("系列:</span>") {
        let rest = html_window(&html, start, start + 200);
        if let Some(a_start) = rest.find("<a ") {
            if let Some(txt_start) = rest[a_start..].find('>') {
                let txt_rest = &rest[a_start + txt_start + 1..];
                if let Some(txt_end) = txt_rest.find("</a>") {
                    let s = txt_rest[..txt_end].trim();
                    if !s.is_empty() {
                        series = Some(s.to_string());
                    }
                }
            }
        }
    }

    let actors = javbus_actors(&html);

    // 8. Sample Images (剧照)
    let mut sample_images = Vec::new();
    for seg in html.split("<a class=\"sample-box\"").skip(1) {
        if let Some(href_idx) = seg.find("href=\"") {
            let rest = &seg[href_idx + 6..];
            if let Some(href_end) = rest.find('"') {
                let mut u = rest[..href_end].trim().to_string();
                if !u.is_empty() {
                    if !u.starts_with("http") {
                        u = format!("https://www.javbus.com{}", u);
                    }
                    if !sample_images.contains(&u) {
                        sample_images.push(u);
                    }
                }
            }
        }
    }

    // 9. Tags
    let mut tags = Vec::new();
    for seg in html.split("href=\"https://www.javbus.com/genre/") {
        if let Some(gt) = seg.find('>') {
            let inner = &seg[gt + 1..];
            if let Some(lt) = inner.find("</a>") {
                let t = inner[..lt].trim();
                if !t.is_empty() && t.len() <= 20 && !tags.contains(&t.to_string()) {
                    tags.push(t.to_string());
                }
            }
        }
    }

    let now = chrono::Utc::now().timestamp();
    if html.contains("driver-verify") || cover_url.is_none() {
        return Err(format!("JavBus 未找到封面或遭遇安全验证【{number}】"));
    }
    Ok(MovieMetadata {
        number: number.to_string(),
        asset_id: None,
        title,
        original_title: None,
        actors,
        maker,
        label,
        series,
        director: None,
        release_date,
        duration_minutes: None,
        cover_url,
        sample_images,
        tags,
        source: "javbus".to_string(),
        updated_at: now,
    })
}

/// Fetch metadata from JavDB by JAV number.
pub async fn scrape_from_javdb(number: &str) -> Result<MovieMetadata, String> {
    let client = scraper_client()?;

    let search_term = if number.starts_with("FC2-") {
        number.trim_start_matches("FC2-").to_string()
    } else {
        number.to_string()
    };

    let search_url = format!("https://javdb.com/search?q={}&f=all", search_term);
    let search_resp = client
        .get(&search_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,ja;q=0.8,en;q=0.7")
        .header("Cookie", "over18=1; locale=zh;")
        .send()
        .await
        .map_err(|e| format!("请求 JavDB 搜索失败：{e}"))?;

    if !search_resp.status().is_success() {
        return Err(format!("JavDB 搜索状态码异常：{}", search_resp.status()));
    }

    let search_html = search_resp.text().await.map_err(|e| format!("读取搜索响应失败：{e}"))?;

    let (detail_path, card) = javdb_exact_card(&search_html, number)
        .ok_or_else(|| format!("JavDB 未找到精确番号【{number}】"))?;
    let search_title = card.split("class=\"video-title\"").nth(1)
        .and_then(|s| s.split_once('>')).and_then(|(_, s)| s.split_once("</div>"))
        .map(|(s, _)| plain_text(s));
    let search_cover = card.split("<img ").skip(1).find_map(|s| {
        let tag = s.split('>').next()?;
        attribute(tag, "src").or_else(|| attribute(tag, "data-src"))
    });
    let search_date = None::<String>;

    let detail_url = format!("https://javdb.com{}", detail_path);
    let detail_resp = client
        .get(&detail_url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36")
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,ja;q=0.8,en;q=0.7")
        .header("Cookie", "over18=1; locale=zh;")
        .send()
        .await;

    let html = match detail_resp {
        Ok(resp) if resp.status().is_success() => resp.text().await.unwrap_or_default(),
        _ => String::new(),
    };

    let is_captcha = html.contains("auth-captcha-image") || html.contains("driver-verify") || html.is_empty();
    if !is_captcha && detail_identity(&html, &["<strong>番號:</strong>", "<strong>番号:</strong>"]).as_deref() != Some(number) {
        return Err(format!("JavDB 详情番号不匹配【{number}】"));
    }
    let html = if is_captcha { String::new() } else { html };

    // Parse Title
    let mut title = number.to_string();
    if let Some(start) = html.find("<h2 class=\"title is-4\">") {
        let rest = &html[start + "<h2 class=\"title is-4\">".len()..];
        if let Some(end) = rest.find("</h2>") {
            let raw_title = &rest[..end];
            let clean: String = raw_title
                .replace("<strong>", "")
                .replace("</strong>", "")
                .replace("<strong class=\"current-title\">", "")
                .trim()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !clean.is_empty() {
                title = clean;
            }
        }
    }

    // Parse Cover URL
    let mut cover_url = None;
    for part in html.split("<img ").skip(1) {
        let tag = part.split('>').next().unwrap_or("");
        if attribute(tag, "class").is_some_and(|classes| classes.split_whitespace().any(|c| c == "video-cover")) {
            cover_url = attribute(tag, "src").or_else(|| attribute(tag, "data-src"));
            break;
        }
    }

    // Parse Date
    let mut release_date = None;
    if let Some(start) = html.find("<strong>日期:</strong>") {
        let rest = html_window(&html, start, start + 200);
        if let Some(val_start) = rest.find("<span class=\"value\">") {
            let val_rest = &rest[val_start + "<span class=\"value\">".len()..];
            if let Some(val_end) = val_rest.find("</span>") {
                release_date = Some(val_rest[..val_end].trim().to_string());
            }
        }
    }

    // Parse Maker (片商)
    let mut maker = None;
    if let Some(start) = html.find("<strong>片商:</strong>") {
        let rest = html_window(&html, start, start + 300);
        if let Some(a_start) = rest.find("<a ") {
            if let Some(txt_start) = rest[a_start..].find('>') {
                let txt_rest = &rest[a_start + txt_start + 1..];
                if let Some(txt_end) = txt_rest.find("</a>") {
                    maker = Some(txt_rest[..txt_end].trim().to_string());
                }
            }
        }
    }

    // Parse Series (系列)
    let mut series = None;
    if let Some(start) = html.find("<strong>系列:</strong>") {
        let rest = html_window(&html, start, start + 300);
        if let Some(a_start) = rest.find("<a ") {
            if let Some(txt_start) = rest[a_start..].find('>') {
                let txt_rest = &rest[a_start + txt_start + 1..];
                if let Some(txt_end) = txt_rest.find("</a>") {
                    series = Some(txt_rest[..txt_end].trim().to_string());
                }
            }
        }
    }

    // Parse Director (导演)
    let mut director = None;
    if let Some(start) = html.find("<strong>導演:</strong>") {
        let rest = html_window(&html, start, start + 300);
        if let Some(a_start) = rest.find("<a ") {
            if let Some(txt_start) = rest[a_start..].find('>') {
                let txt_rest = &rest[a_start + txt_start + 1..];
                if let Some(txt_end) = txt_rest.find("</a>") {
                    director = Some(txt_rest[..txt_end].trim().to_string());
                }
            }
        }
    }

    // Parse Actresses
    let mut actors = Vec::new();
    if let Some(start) = html.find("<strong>演員:</strong>") {
        let rest = html_window(&html, start, start + 800);
        if let Some(span_end) = rest.find("</span>") {
            let actor_block = &rest[..span_end];
            for segment in actor_block.split("<a ") {
                if segment.contains("href=\"/actors/") {
                    if let Some(gt) = segment.find('>') {
                        let inner = &segment[gt + 1..];
                        if let Some(lt) = inner.find("</a>") {
                            let name = plain_text(&inner[..lt]);
                            if !name.is_empty() && name.chars().count() <= 50 && !actors.contains(&name) {
                                actors.push(name);
                            }
                        }
                    }
                }
            }
        }
    }

    // Parse Tags
    let mut tags = Vec::new();
    for segment in html.split("<a href=\"/tags") {
        if let Some(gt) = segment.find('>') {
            let inner = &segment[gt + 1..];
            if let Some(lt) = inner.find("</a>") {
                let tag = inner[..lt].trim();
                if !tag.is_empty() && tag.len() <= 20 && !tags.contains(&tag.to_string()) {
                    tags.push(tag.to_string());
                }
            }
        }
    }

    let mut sample_images = Vec::new();
    for anchor in html.split("<a ").skip(1) {
        let tag = anchor.split('>').next().unwrap_or("");
        if attribute(tag, "class").is_some_and(|classes| classes.split_whitespace().any(|c| c == "tile-item")) {
            if let Some(url) = attribute(tag, "href").and_then(|u| valid_image_url("https://javdb.com", &u)) {
                if !sample_images.contains(&url) { sample_images.push(url); }
            }
        }
    }

    // Fallback from search card if detail was blocked or missing elements
    if cover_url.is_none() {
        cover_url = search_cover;
    }
    if title.trim() == number.trim() {
        if let Some(st) = search_title {
            title = st;
        }
    }
    if release_date.is_none() {
        release_date = search_date;
    }
    if maker.is_none() && number.starts_with("FC2-") {
        maker = Some("FC2".to_string());
    }

    let cover_url = cover_url.and_then(|url| valid_image_url("https://javdb.com", &url));
    let final_cover = cover_url.ok_or_else(|| format!("未在 JavDB 找到作品有效封面【{}】", number))?;
    if title.trim() == number.trim() && !number.starts_with("FC2-") {
        return Err(format!("未在 JavDB 找到作品有效标题【{}】", number));
    }

    let now = chrono::Utc::now().timestamp();

    Ok(MovieMetadata {
        number: number.to_string(),
        asset_id: None,
        title,
        original_title: None,
        actors,
        maker,
        label: None,
        series,
        director,
        release_date,
        duration_minutes: None,
        cover_url: Some(final_cover),
        sample_images,
        tags,
        source: "javdb".to_string(),
        updated_at: now,
    })
}

/// Fetch metadata from Caribbeancom official website (unblocked, direct for date-based numbers MMDDYY-NNN)
pub async fn scrape_from_caribbeancom(number: &str) -> Result<MovieMetadata, String> {
    let client = scraper_client()?;

    let clean_num = number.replace('_', "-");
    let url = format!("https://www.caribbeancom.com/moviepages/{}/index.html", clean_num);
    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|e| format!("请求 Caribbeancom 失败：{e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Caribbeancom 状态码异常：{}", resp.status()));
    }

    let bytes = resp.bytes().await.map_err(|e| format!("读取 Caribbeancom 失败：{e}"))?;
    let (cow, _, _) = encoding_rs::EUC_JP.decode(&bytes);
    let mut html = cow.to_string();
    if !html.contains("<h1") && !html.contains("<title") {
        html = String::from_utf8_lossy(&bytes).to_string();
    }

    let expected_path = format!("/moviepages/{clean_num}/");
    let canonical_match = html.split('<').filter(|tag| tag.starts_with("link ") || tag.starts_with("meta "))
        .any(|tag| (attribute(tag, "rel").as_deref() == Some("canonical")
            || attribute(tag, "property").as_deref() == Some("og:url"))
            && attribute(tag, "href").or_else(|| attribute(tag, "content"))
                .is_some_and(|url| url.contains(&expected_path)));
    if !canonical_match { return Err(format!("Caribbeancom 无法核验详情番号【{number}】")); }
    // Title
    let mut title = clean_num.clone();
    if let Some(start) = html.find("<h1 itemprop=\"name\">") {
        let rest = &html[start + "<h1 itemprop=\"name\">".len()..];
        if let Some(end) = rest.find("</h1>") {
            let t = rest[..end].trim();
            if !t.is_empty() {
                title = t.to_string();
            }
        }
    } else if let Some(start) = html.find("<title>") {
        let rest = &html[start + 7..];
        if let Some(end) = rest.find("</title>") {
            let t = rest[..end].trim();
            if !t.is_empty() {
                title = t.to_string();
            }
        }
    }

    // Cover
    let cover_url = Some(format!("https://www.caribbeancom.com/moviepages/{}/images/l_l.jpg", clean_num));

    // Release Date (MMDDYY-NNN -> 20YY-MM-DD)
    let parts: Vec<&str> = clean_num.split('-').collect();
    let release_date = if parts.len() == 2 && parts[0].len() == 6 {
        let mm = &parts[0][0..2];
        let dd = &parts[0][2..4];
        let yy = &parts[0][4..6];
        Some(format!("20{}-{}-{}", yy, mm, dd))
    } else {
        None
    };

    // Do not harvest all itemprop=name spans: most are navigation/recommendations.
    // Until a dedicated performer field is present, unknown is safer than wrong.
    let actors = Vec::new();

    let now = chrono::Utc::now().timestamp();
    Ok(MovieMetadata {
        number: clean_num,
        asset_id: None,
        title,
        original_title: None,
        actors,
        maker: Some("Caribbeancom 加勒比".to_string()),
        label: Some("Caribbeancom".to_string()),
        series: None,
        director: None,
        release_date,
        duration_minutes: None,
        cover_url,
        sample_images: Vec::new(),
        tags: vec!["无码".to_string(), "加勒比".to_string()],
        source: "caribbeancom".to_string(),
        updated_at: now,
    })
}

/// Fetch metadata from 1pondo (一本道) official API (JSON based, 100% reliable)
pub async fn scrape_from_1pondo(number: &str) -> Result<MovieMetadata, String> {
    let client = scraper_client()?;

    let clean_num = number.replace('-', "_");
    let url = format!("https://www.1pondo.tv/dyn/phpauto/movie_details/movie_id/{}.json", clean_num);
    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|e| format!("请求 1pondo 失败：{e}"))?;

    if !resp.status().is_success() {
        return Err(format!("1pondo 状态码异常：{}", resp.status()));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| format!("解析 1pondo JSON 失败：{e}"))?;

    let returned_id = json.get("MovieID").and_then(|v| v.as_str());
    if returned_id != Some(clean_num.as_str()) {
        return Err(format!("1pondo JSON 番号不匹配【{number}】"));
    }
    let title = json.get("Title")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| number.to_string());

    let release_date = json.get("Release")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut actors = Vec::new();
    if let Some(arr) = json.get("ActressesJa").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() && !actors.contains(&trimmed.to_string()) {
                    actors.push(trimmed.to_string());
                }
            }
        }
    }
    if actors.is_empty() {
        if let Some(s) = json.get("Actor").and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                actors.push(trimmed.to_string());
            }
        }
    }

    let cover_url = json.get("ThumbHigh")
        .and_then(|v| v.as_str())
        .or_else(|| json.get("ThumbMed").and_then(|v| v.as_str()))
        .or_else(|| json.get("ThumbLow").and_then(|v| v.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if cover_url.is_none() {
        return Err(format!("1pondo 未找到封面【{}】", number));
    }

    let now = chrono::Utc::now().timestamp();
    Ok(MovieMetadata {
        number: number.to_string(),
        asset_id: None,
        title,
        original_title: None,
        actors,
        maker: Some("一本道 1pondo".to_string()),
        label: Some("1pondo".to_string()),
        series: json.get("Series").and_then(|v| v.as_str()).map(|s| s.to_string()),
        director: None,
        release_date,
        duration_minutes: None,
        cover_url,
        sample_images: Vec::new(),
        tags: vec!["无码".to_string(), "一本道".to_string()],
        source: "1pondo".to_string(),
        updated_at: now,
    })
}

fn valid_image_url(base: &str, value: &str) -> Option<String> {
    let url = reqwest::Url::parse(base).ok()?.join(value.trim()).ok()?;
    if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() { Some(url.to_string()) } else { None }
}

/// Shared successes avoid repeated network requests. Failures are never negative-cached:
/// a timeout/challenge is not proof that an identity does not exist.
pub async fn scrape_movie_metadata(raw: &str) -> Result<MovieMetadata, String> {
    let number = normalize_jav_number(raw).ok_or_else(|| "无效或有歧义的番号".to_string())?;
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, MovieMetadata>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Ok(items) = cache.lock() {
        if let Some(meta) = items.get(&number).filter(|m| chrono::Utc::now().timestamp() - m.updated_at < 3600) {
            return Ok(meta.clone());
        }
    }
    // Bound concurrent source requests across single and batch callers.
    static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    let _permit = GATE.get_or_init(|| tokio::sync::Semaphore::new(3)).acquire().await.map_err(|e| e.to_string())?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(22), async {
        let is_date = number.as_bytes().first().is_some_and(u8::is_ascii_digit);
        if is_date {
            // The same date + sequence can identify unrelated movies at different studios.
            // Never cross-fallback between these providers or JavDB's ambiguous date search.
            if number.contains('_') { scrape_from_1pondo(&number).await }
            else { scrape_from_caribbeancom(&number).await }
        } else {
            // Direct detail source is faster. Start the second source after a short hedge,
            // returning the first *validated* success, not the first HTTP 200.
            let bus = scrape_from_javbus(&number);
            let db = async {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                scrape_from_javdb(&number).await
            };
            tokio::pin!(bus, db);
            tokio::select! {
                first = &mut bus => match first {
                    Ok(meta) => Ok(meta),
                    Err(e) => db.await.map_err(|other| format!("JavBus: {e}; JavDB: {other}")),
                },
                first = &mut db => match first {
                    Ok(meta) => Ok(meta),
                    Err(e) => bus.await.map_err(|other| format!("JavDB: {e}; JavBus: {other}")),
                }
            }
        }
    }).await.map_err(|_| format!("刮削超时【{number}】"))?;
    if let Ok(meta) = &result {
        if let Ok(mut items) = cache.lock() {
            items.retain(|_, item| chrono::Utc::now().timestamp() - item.updated_at < 3600);
            if items.len() >= 256 { items.clear(); }
            items.insert(number, meta.clone());
        }
    }
    result
}

/// Save movie metadata into SQLite
pub fn save_movie_metadata(conn: &Connection, meta: &MovieMetadata) -> Result<(), rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let actors_json = serde_json::to_string(&meta.actors).unwrap_or_else(|_| "[]".to_string());
    let samples_json = serde_json::to_string(&meta.sample_images).unwrap_or_else(|_| "[]".to_string());
    let tags_json = serde_json::to_string(&meta.tags).unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        "INSERT INTO movie_metadata (
            number, asset_id, title, original_title, actors_json, maker, label, series,
            director, release_date, duration_minutes, cover_url, sample_images_json,
            tags_json, source, updated_at, verification_version
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 1)
        ON CONFLICT(number) DO UPDATE SET
            asset_id = COALESCE(excluded.asset_id, movie_metadata.asset_id),
            title = excluded.title,
            original_title = excluded.original_title,
            actors_json = excluded.actors_json,
            maker = excluded.maker,
            label = excluded.label,
            series = excluded.series,
            director = excluded.director,
            release_date = excluded.release_date,
            duration_minutes = excluded.duration_minutes,
            cover_url = excluded.cover_url,
            sample_images_json = excluded.sample_images_json,
            tags_json = excluded.tags_json,
            source = excluded.source,
            updated_at = excluded.updated_at,
            verification_version = excluded.verification_version",
        params![
            meta.number,
            meta.asset_id,
            meta.title,
            meta.original_title,
            actors_json,
            meta.maker,
            meta.label,
            meta.series,
            meta.director,
            meta.release_date,
            meta.duration_minutes,
            meta.cover_url,
            samples_json,
            tags_json,
            meta.source,
            meta.updated_at
        ],
    )?;

    if let Some(asset_id) = &meta.asset_id { bind_movie_asset(conn, &meta.number, asset_id)?; }
    Ok(())
}

/// Read movie metadata by number or asset_id
pub fn get_movie_metadata(conn: &Connection, identifier: &str) -> Result<Option<MovieMetadata>, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let upper = normalize_jav_number(identifier).unwrap_or_else(|| identifier.to_string());
    let mut stmt = conn.prepare(
        "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                director, release_date, duration_minutes, cover_url, sample_images_json,
                tags_json, source, updated_at
         FROM movie_metadata
         WHERE (number = ?1 OR asset_id = ?2 OR number IN (SELECT number FROM movie_asset_bindings WHERE asset_id = ?2))
           AND verification_version = 1
           AND source NOT IN ('javdb_not_found', 'not_found', '')
           AND cover_url IS NOT NULL AND cover_url != ''
         LIMIT 1"
    )?;

    let res = stmt.query_row(params![upper, identifier], |row| {
        let actors_json: String = row.get(4)?;
        let sample_json: String = row.get(12)?;
        let tags_json: String = row.get(13)?;

        Ok(MovieMetadata {
            number: row.get(0)?,
            asset_id: row.get(1)?,
            title: row.get(2)?,
            original_title: row.get(3)?,
            actors: serde_json::from_str(&actors_json).unwrap_or_default(),
            maker: row.get(5)?,
            label: row.get(6)?,
            series: row.get(7)?,
            director: row.get(8)?,
            release_date: row.get(9)?,
            duration_minutes: row.get(10)?,
            cover_url: row.get(11)?,
            sample_images: serde_json::from_str(&sample_json).unwrap_or_default(),
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            source: row.get(14)?,
            updated_at: row.get(15)?,
        })
    }).optional()?;

    // When addressed by asset ID, return that exact file, not the movie's first edition.
    Ok(res.map(|mut meta| {
        if normalize_jav_number(identifier).is_none() { meta.asset_id = Some(identifier.to_string()); }
        meta
    }))
}

/// List recently scraped movie metadata
pub fn list_movie_metadata(conn: &Connection, limit: u32, offset: u32) -> Result<Vec<MovieMetadata>, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let mut stmt = conn.prepare(
        "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                director, release_date, duration_minutes, cover_url, sample_images_json,
                tags_json, source, updated_at
         FROM movie_metadata
         WHERE verification_version = 1 AND source NOT IN ('javdb_not_found', 'not_found', '')
           AND cover_url IS NOT NULL AND cover_url != ''
         ORDER BY updated_at DESC, number ASC
         LIMIT ?1 OFFSET ?2"
    )?;

    let rows = stmt.query_map(params![limit, offset], |row| {
        let actors_json: String = row.get(4)?;
        let sample_json: String = row.get(12)?;
        let tags_json: String = row.get(13)?;

        Ok(MovieMetadata {
            number: row.get(0)?,
            asset_id: row.get(1)?,
            title: row.get(2)?,
            original_title: row.get(3)?,
            actors: serde_json::from_str(&actors_json).unwrap_or_default(),
            maker: row.get(5)?,
            label: row.get(6)?,
            series: row.get(7)?,
            director: row.get(8)?,
            release_date: row.get(9)?,
            duration_minutes: row.get(10)?,
            cover_url: row.get(11)?,
            sample_images: serde_json::from_str(&sample_json).unwrap_or_default(),
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            source: row.get(14)?,
            updated_at: row.get(15)?,
        })
    })?;

    rows.collect::<Result<Vec<_>, _>>()
}

/// Get related movies from the library (same actress, same maker, or same series)
pub fn get_related_movies(conn: &Connection, number: &str, limit: u32) -> Result<Vec<MovieMetadata>, rusqlite::Error> {
    let current = match get_movie_metadata(conn, number)? {
        Some(m) => m,
        None => return Ok(Vec::new()),
    };

    let mut related = Vec::new();
    let mut seen_numbers = std::collections::HashSet::new();
    seen_numbers.insert(current.number.clone());

    // 1. Same series
    if let Some(ref series) = current.series {
        if !series.is_empty() {
            let mut stmt = conn.prepare(
                "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                        director, release_date, duration_minutes, cover_url, sample_images_json,
                        tags_json, source, updated_at
                 FROM movie_metadata
                 WHERE verification_version = 1 AND series = ?1 AND number != ?2
                 LIMIT ?3"
            )?;
            let rows = stmt.query_map(params![series, current.number, limit], |row| {
                let actors_json: String = row.get(4)?;
                let sample_json: String = row.get(12)?;
                let tags_json: String = row.get(13)?;
                Ok(MovieMetadata {
                    number: row.get(0)?,
                    asset_id: row.get(1)?,
                    title: row.get(2)?,
                    original_title: row.get(3)?,
                    actors: serde_json::from_str(&actors_json).unwrap_or_default(),
                    maker: row.get(5)?,
                    label: row.get(6)?,
                    series: row.get(7)?,
                    director: row.get(8)?,
                    release_date: row.get(9)?,
                    duration_minutes: row.get(10)?,
                    cover_url: row.get(11)?,
                    sample_images: serde_json::from_str(&sample_json).unwrap_or_default(),
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    source: row.get(14)?,
                    updated_at: row.get(15)?,
                })
            })?;
            for r in rows.flatten() {
                if seen_numbers.insert(r.number.clone()) {
                    related.push(r);
                }
            }
        }
    }

    // 2. Same actresses
    for actor in &current.actors {
        if related.len() >= limit as usize {
            break;
        }
        let pattern = format!("%\"{}\"%", actor);
        let mut stmt = conn.prepare(
            "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                    director, release_date, duration_minutes, cover_url, sample_images_json,
                    tags_json, source, updated_at
             FROM movie_metadata
             WHERE verification_version = 1 AND actors_json LIKE ?1 AND number != ?2
             LIMIT ?3"
        )?;
        let rows = stmt.query_map(params![pattern, current.number, limit], |row| {
            let actors_json: String = row.get(4)?;
            let sample_json: String = row.get(12)?;
            let tags_json: String = row.get(13)?;
            Ok(MovieMetadata {
                number: row.get(0)?,
                asset_id: row.get(1)?,
                title: row.get(2)?,
                original_title: row.get(3)?,
                actors: serde_json::from_str(&actors_json).unwrap_or_default(),
                maker: row.get(5)?,
                label: row.get(6)?,
                series: row.get(7)?,
                director: row.get(8)?,
                release_date: row.get(9)?,
                duration_minutes: row.get(10)?,
                cover_url: row.get(11)?,
                sample_images: serde_json::from_str(&sample_json).unwrap_or_default(),
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                source: row.get(14)?,
                updated_at: row.get(15)?,
            })
        })?;
        for r in rows.flatten() {
            if seen_numbers.insert(r.number.clone()) {
                related.push(r);
                if related.len() >= limit as usize {
                    break;
                }
            }
        }
    }

    // 3. Same maker if needed
    if related.len() < limit as usize {
        if let Some(ref maker) = current.maker {
            let mut stmt = conn.prepare(
                "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                        director, release_date, duration_minutes, cover_url, sample_images_json,
                        tags_json, source, updated_at
                 FROM movie_metadata
                 WHERE verification_version = 1 AND maker = ?1 AND number != ?2
                 LIMIT ?3"
            )?;
            let rows = stmt.query_map(params![maker, current.number, (limit as usize - related.len()) as u32], |row| {
                let actors_json: String = row.get(4)?;
                let sample_json: String = row.get(12)?;
                let tags_json: String = row.get(13)?;
                Ok(MovieMetadata {
                    number: row.get(0)?,
                    asset_id: row.get(1)?,
                    title: row.get(2)?,
                    original_title: row.get(3)?,
                    actors: serde_json::from_str(&actors_json).unwrap_or_default(),
                    maker: row.get(5)?,
                    label: row.get(6)?,
                    series: row.get(7)?,
                    director: row.get(8)?,
                    release_date: row.get(9)?,
                    duration_minutes: row.get(10)?,
                    cover_url: row.get(11)?,
                    sample_images: serde_json::from_str(&sample_json).unwrap_or_default(),
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    source: row.get(14)?,
                    updated_at: row.get(15)?,
                })
            })?;
            for r in rows.flatten() {
                if seen_numbers.insert(r.number.clone()) {
                    related.push(r);
                }
            }
        }
    }

    Ok(related)
}

/// Whitelist management
pub fn get_scrape_whitelist(conn: &Connection) -> Result<Vec<String>, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let mut stmt = conn.prepare("SELECT path FROM scrape_whitelist WHERE enabled = 1 ORDER BY path ASC")?;
    let rows = stmt.query_map([], |row| row.get(0))?;
    let mut list = Vec::new();
    for r in rows.flatten() {
        list.push(r);
    }
    Ok(list)
}

pub fn set_scrape_whitelist(conn: &mut Connection, paths: Vec<String>) -> Result<(), rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM scrape_whitelist", [])?;
    let now = chrono::Utc::now().timestamp();
    for p in paths {
        let clean = p.trim();
        if !clean.is_empty() {
            tx.execute(
                "INSERT INTO scrape_whitelist (path, enabled, created_at) VALUES (?1, 1, ?2)",
                params![clean, now],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn path_under(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    prefix.is_empty() || path == prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
}

pub fn is_path_whitelisted(conn: &Connection, path: &str) -> bool {
    get_scrape_whitelist(conn).map(|paths| paths.iter().any(|p| path_under(path, p))).unwrap_or(false)
}

fn whitelisted_number_assets(conn: &Connection) -> Result<(usize, std::collections::BTreeMap<String, Vec<String>>), rusqlite::Error> {
    let whitelist = get_scrape_whitelist(conn)?;
    let mut stmt = conn.prepare("SELECT id, file_name, remote_path FROM assets WHERE missing = 0 AND media_type = 'video' ORDER BY id")?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))?;
    let mut total = 0;
    let mut numbers = std::collections::BTreeMap::<String, Vec<String>>::new();
    for row in rows {
        let (id, name, path) = row?;
        if !whitelist.iter().any(|p| path_under(&path, p)) { continue; }
        total += 1;
        if let Some(number) = extract_jav_number(&name) { numbers.entry(number).or_default().push(id); }
    }
    Ok((total, numbers))
}

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchScrapeProgress {
    pub running: bool,
    pub paused: bool,
    pub total: usize,
    pub completed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub current_number: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchScrapeStats {
    pub total_whitelisted_videos: usize,
    pub distinct_numbers: usize,
    pub already_scraped: usize,
    pub remaining_to_scrape: usize,
}

fn batch_progress_cell() -> &'static Mutex<BatchScrapeProgress> {
    static CELL: OnceLock<Mutex<BatchScrapeProgress>> = OnceLock::new();
    CELL.get_or_init(|| {
        Mutex::new(BatchScrapeProgress {
            running: false,
            paused: false,
            total: 0,
            completed: 0,
            succeeded: 0,
            failed: 0,
            current_number: None,
            last_error: None,
        })
    })
}

fn stop_batch_flag() -> &'static AtomicBool {
    static CELL: OnceLock<AtomicBool> = OnceLock::new();
    CELL.get_or_init(|| AtomicBool::new(false))
}

fn pause_batch_flag() -> &'static AtomicBool {
    static CELL: OnceLock<AtomicBool> = OnceLock::new();
    CELL.get_or_init(|| AtomicBool::new(false))
}

pub fn get_batch_scrape_progress() -> BatchScrapeProgress {
    batch_progress_cell().lock().map(|g| g.clone()).unwrap_or(BatchScrapeProgress {
        running: false,
        paused: false,
        total: 0,
        completed: 0,
        succeeded: 0,
        failed: 0,
        current_number: None,
        last_error: None,
    })
}

pub fn pause_batch_scrape() {
    pause_batch_flag().store(true, Ordering::SeqCst);
    if let Ok(mut g) = batch_progress_cell().lock() {
        g.paused = true;
    }
}

pub fn resume_batch_scrape() {
    pause_batch_flag().store(false, Ordering::SeqCst);
    if let Ok(mut g) = batch_progress_cell().lock() {
        g.paused = false;
    }
}

pub fn stop_batch_scrape() {
    stop_batch_flag().store(true, Ordering::SeqCst);
    pause_batch_flag().store(false, Ordering::SeqCst);
    if let Ok(mut g) = batch_progress_cell().lock() {
        g.paused = false;
    }
}

pub fn get_batch_scrape_stats(conn: &Connection) -> Result<BatchScrapeStats, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let (total_videos, number_assets) = whitelisted_number_assets(conn)?;
    let distinct_numbers: std::collections::HashSet<_> = number_assets.keys().cloned().collect();

    let mut already_scraped = 0usize;
    let mut check_stmt = conn.prepare("SELECT 1 FROM movie_metadata WHERE number = ?1 AND verification_version = 1 AND source NOT IN ('javdb_not_found', 'not_found', '') AND cover_url IS NOT NULL AND cover_url != '' LIMIT 1")?;
    for num in &distinct_numbers {
        if check_stmt.exists(params![num])? {
            already_scraped += 1;
        }
    }

    let remaining = distinct_numbers.len().saturating_sub(already_scraped);

    Ok(BatchScrapeStats {
        total_whitelisted_videos: total_videos,
        distinct_numbers: distinct_numbers.len(),
        already_scraped,
        remaining_to_scrape: remaining,
    })
}

pub fn start_batch_scrape_engine(db_path: String) -> Result<(), String> {
    {
        let mut g = batch_progress_cell().lock().map_err(|e| e.to_string())?;
        if g.running {
            return Err("后台批量刮削已在运行中".to_string());
        }
        g.running = true;
        g.paused = false;
        g.total = 0;
        g.completed = 0;
        g.succeeded = 0;
        g.failed = 0;
        g.current_number = None;
        g.last_error = None;
    }
    stop_batch_flag().store(false, Ordering::SeqCst);
    pause_batch_flag().store(false, Ordering::SeqCst);

    tokio::spawn(async move {
        let conn = match Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => {
                if let Ok(mut g) = batch_progress_cell().lock() {
                    g.running = false;
                    g.last_error = Some(format!("打开数据库失败：{e}"));
                }
                return;
            }
        };

        if let Err(e) = ensure_scraper_schema(&conn) {
            if let Ok(mut g) = batch_progress_cell().lock() {
                g.running = false;
                g.last_error = Some(format!("初始化数据表失败：{e}"));
            }
            return;
        }

        let whitelist = match get_scrape_whitelist(&conn) {
            Ok(w) => w,
            Err(e) => {
                if let Ok(mut g) = batch_progress_cell().lock() {
                    g.running = false;
                    g.last_error = Some(format!("获取白名单失败：{e}"));
                }
                return;
            }
        };

        if whitelist.is_empty() {
            if let Ok(mut g) = batch_progress_cell().lock() {
                g.running = false;
                g.last_error = Some("当前未配置任何白名单目录，已取消批量刮削".to_string());
            }
            return;
        }

        let (_, number_assets) = match whitelisted_number_assets(&conn) {
            Ok(items) => items,
            Err(e) => {
                if let Ok(mut g) = batch_progress_cell().lock() {
                    g.running = false;
                    g.last_error = Some(format!("读取资产失败：{e}"));
                }
                return;
            }
        };
        let mut queue = Vec::new();
        for (number, assets) in &number_assets {
            match get_movie_metadata(&conn, number) {
                Ok(Some(_)) => {
                    // Repair cached entries too: no network is necessary to bind each duplicate file.
                    for asset in assets {
                        if let Err(e) = bind_movie_asset(&conn, number, asset) {
                            if let Ok(mut g) = batch_progress_cell().lock() { g.last_error = Some(e.to_string()); }
                        }
                    }
                }
                Ok(None) => queue.push(number.clone()),
                Err(e) => {
                    if let Ok(mut g) = batch_progress_cell().lock() {
                        g.running = false;
                        g.last_error = Some(format!("读取缓存失败：{e}"));
                    }
                    return;
                }
            }
        }

        queue.sort();

        if let Ok(mut g) = batch_progress_cell().lock() {
            g.total = queue.len();
        }

        for number in queue {
            if stop_batch_flag().load(Ordering::SeqCst) {
                break;
            }

            while pause_batch_flag().load(Ordering::SeqCst) {
                if stop_batch_flag().load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }

            if stop_batch_flag().load(Ordering::SeqCst) {
                break;
            }

            if let Ok(mut g) = batch_progress_cell().lock() {
                g.current_number = Some(number.clone());
            }

            let result = match scrape_movie_metadata(&number).await {
                Ok(mut meta) => {
                    meta.asset_id = number_assets.get(&number).and_then(|ids| ids.first()).cloned();
                    save_movie_metadata(&conn, &meta).and_then(|_| {
                        if let Some(ids) = number_assets.get(&number) {
                            for id in ids { bind_movie_asset(&conn, &number, id)?; }
                        }
                        Ok(())
                    }).map_err(|e| format!("保存元数据失败：{e}"))
                }
                Err(e) => Err(e),
            };
            if let Ok(mut g) = batch_progress_cell().lock() {
                g.completed += 1;
                match result {
                    Ok(()) => g.succeeded += 1,
                    Err(e) => { g.failed += 1; g.last_error = Some(format!("{number}: {e}")); }
                }
            }

            // A modest inter-item delay; source requests have a shared concurrency bound.
            let jitter = 300 + (number.len() * 73) % 200;
            tokio::time::sleep(std::time::Duration::from_millis(jitter as u64)).await;
        }

        if let Ok(mut g) = batch_progress_cell().lock() {
            g.running = false;
            g.paused = false;
            g.current_number = None;
        }
    });

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(number: &str, asset: Option<&str>) -> MovieMetadata {
        MovieMetadata {
            number: number.into(), asset_id: asset.map(str::to_string), title: "offline fixture".into(),
            original_title: None, actors: vec![], maker: None, label: None, series: None,
            director: None, release_date: None, duration_minutes: None,
            cover_url: Some("https://example.com/cover.jpg".into()), sample_images: vec![],
            tags: vec![], source: "javbus".into(), updated_at: 123,
        }
    }

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE assets(id TEXT PRIMARY KEY, file_name TEXT, remote_path TEXT, missing INTEGER, media_type TEXT);").unwrap();
        ensure_scraper_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn filename_identities_are_precise_and_provider_scoped() {
        for (input, expected) in [
            ("[中文字幕] SSIS-123_4K.mp4", "SSIS-123"),
            ("midv-045.mkv", "MIDV-045"), ("ABW001", "ABW-001"),
            ("FC2-PPV-2839102_1080p.mp4", "FC2-2839102"),
            ("1pondo-010123_001.mp4", "010123_001"),
            ("CARIB-010123-001.mkv", "010123-001"),
            ("D:\\parent.ABP-444\\MIDV-045.mkv", "MIDV-045"),
            ("SSIS-123_x264.mkv", "SSIS-123"),
        ] { assert_eq!(extract_jav_number(input).as_deref(), Some(expected), "{input}"); }
        for input in ["SSIS-123 MIDV-045.mp4", "FC2-123456789.mp4", "holiday.mp4", "1080p.H264.mp4"] {
            assert_eq!(extract_jav_number(input), None, "{input}");
        }
        assert_eq!(normalize_jav_number("fc2 ppv 2839102").as_deref(), Some("FC2-2839102"));
        assert_eq!(normalize_jav_number("SSIS-123 title"), None);
        assert_ne!(normalize_jav_number("010123_001"), normalize_jav_number("010123-001"));
    }

    #[test]
    fn search_uses_exact_dedicated_number_not_first_result_or_title() {
        let html = r#"<a href="/v/wrong"><div class="video-title"><strong>SSIS-1234</strong> SSIS-123 in title</div><img src="wrong.jpg"></a>
        <a href="/v/correct"><div class="video-title"><strong>SSIS-123</strong> Correct</div><img src="correct.jpg"></a>"#;
        let (path, card) = javdb_exact_card(html, "SSIS-123").unwrap();
        assert_eq!(path, "/v/correct");
        assert!(card.contains("correct.jpg"));
        assert!(!card.contains("wrong.jpg"));
        assert!(javdb_exact_card(html, "SSIS-12").is_none());
        assert!(javdb_exact_card(r#"<a href="/v/title"><div>SSIS-123</div></a>"#, "SSIS-123").is_none());
        assert!(javdb_exact_card(r#"<a href="/v/a"><strong>FC2-PPV-1234567</strong></a>"#, "FC2-1234567").is_some());
    }

    #[test]
    fn detail_identity_does_not_use_related_movies() {
        let html = "<p><span>識別碼:</span><span>SSIS-124</span></p><div>SSIS-123</div>";
        assert_eq!(detail_identity(html, &["識別碼:</span>"]).as_deref(), Some("SSIS-124"));
        assert_eq!(detail_identity("<title>SSIS-123</title>", &["識別碼:</span>"]), None);
        let japanese = "あいうえおかきくけこ";
        assert!(std::panic::catch_unwind(|| html_window(japanese, 1, 20)).is_ok());
    }

    #[test]
    fn asset_binding_preserves_case_and_duplicate_files() {
        let conn = database();
        save_movie_metadata(&conn, &metadata("SSIS-123", Some("AbC-hash"))).unwrap();
        bind_movie_asset(&conn, "SSIS-123", "Other-part").unwrap();
        for id in ["AbC-hash", "Other-part"] {
            let meta = get_movie_metadata(&conn, id).unwrap().unwrap();
            assert_eq!(meta.number, "SSIS-123");
            assert_eq!(meta.asset_id.as_deref(), Some(id));
        }
        assert!(get_movie_metadata(&conn, "ABC-HASH").unwrap().is_none());
        assert_eq!(list_movie_metadata(&conn, 10, 0).unwrap().len(), 1);
    }

    #[test]
    fn quarantine_preserves_legacy_rows_and_pagination_is_stable() {
        let conn = database();
        for number in ["SSIS-003", "SSIS-001", "SSIS-002"] {
            save_movie_metadata(&conn, &metadata(number, None)).unwrap();
        }
        conn.execute("UPDATE movie_metadata SET verification_version=0 WHERE number='SSIS-003'", []).unwrap();
        assert!(get_movie_metadata(&conn, "SSIS-003").unwrap().is_none());
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM movie_metadata", [], |row| row.get::<_, i64>(0)).unwrap(), 3);
        assert_eq!(list_movie_metadata(&conn, 1, 0).unwrap()[0].number, "SSIS-001");
        assert_eq!(list_movie_metadata(&conn, 1, 1).unwrap()[0].number, "SSIS-002");
        assert!(list_movie_metadata(&conn, 1, 2).unwrap().is_empty());
    }

    #[test]
    fn overlapping_whitelists_and_wildcards_do_not_duplicate_or_escape() {
        let mut conn = database();
        set_scrape_whitelist(&mut conn, vec!["/media/a_1".into(), "/media/a_1/sub".into()]).unwrap();
        for (id, name, path) in [
            ("part1", "SSIS-123.mp4", "/media/a_1/sub/SSIS-123.mp4"),
            ("part2", "SSIS-123-CD2.mkv", "/media/a_1/SSIS-123-CD2.mkv"),
            ("outside", "SSIS-124.mp4", "/media/ax1/SSIS-124.mp4"),
            ("sibling", "SSIS-125.mp4", "/media/a_12/SSIS-125.mp4"),
        ] { conn.execute("INSERT INTO assets VALUES (?1,?2,?3,0,'video')", params![id, name, path]).unwrap(); }
        let (total, numbers) = whitelisted_number_assets(&conn).unwrap();
        assert_eq!(total, 2);
        assert_eq!(numbers.len(), 1);
        assert_eq!(numbers["SSIS-123"].len(), 2);
        let stats = get_batch_scrape_stats(&conn).unwrap();
        assert_eq!(stats.total_whitelisted_videos, 2);
        assert_eq!(stats.distinct_numbers, 1);
    }

    #[test]
    fn images_reject_script_and_resolve_protocol_relative_urls() {
        assert_eq!(valid_image_url("https://javdb.com", "//img.example.com/a.jpg").as_deref(), Some("https://img.example.com/a.jpg"));
        assert!(valid_image_url("https://javdb.com", "javascript:alert(1)").is_none());
        assert_eq!(plain_text("<img src='logo.jpg'>"), "");
        let polluted = r#"<div class="star-name"><a href="/star/a">演员甲</a></div><div class="star-name"></div><a href="/"><img src="logo.jpg"></a>"#;
        assert_eq!(javbus_actors(polluted), vec!["演员甲"]);
    }
}
