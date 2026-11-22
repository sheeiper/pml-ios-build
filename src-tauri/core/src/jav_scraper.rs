use rusqlite::{params, Connection, OptionalExtension, ToSql};
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
        CREATE TABLE IF NOT EXISTS scrape_failures (
            number TEXT PRIMARY KEY,
            error TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS scrape_whitelist (
            path TEXT PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS scrape_ignored_assets (
            asset_id TEXT PRIMARY KEY,
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
    // Normalize FC2/legacy slash dates (e.g. 2026/07/15 -> 2026-07-15) so release_date
    // lexicographical sorting matches true chronological order across all providers.
    let _ = conn.execute(
        "UPDATE movie_metadata SET release_date = replace(release_date, '/', '-') WHERE release_date LIKE '%/%'",
        [],
    );
    Ok(())
}

/// Normalize release dates like `2026/7/5`, `2026/07/05`, `2026.07.05` into `YYYY-MM-DD`.
pub fn normalize_release_date(raw: Option<&str>) -> Option<String> {
    let text = raw?.trim();
    if text.is_empty() { return None; }
    let unified = text.replace('/', "-").replace('.', "-");
    let parts: Vec<&str> = unified.split('-').filter(|s| !s.is_empty()).collect();
    if parts.len() == 3
        && parts[0].len() == 4
        && parts[0].bytes().all(|b| b.is_ascii_digit())
        && parts[1].bytes().all(|b| b.is_ascii_digit())
        && parts[2].bytes().all(|b| b.is_ascii_digit())
    {
        if let (Ok(m), Ok(d)) = (parts[1].parse::<u32>(), parts[2].parse::<u32>()) {
            if (1..=12).contains(&m) && (1..=31).contains(&d) {
                return Some(format!("{}-{:02}-{:02}", parts[0], m, d));
            }
        }
    }
    Some(unified)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieAssetPart {
    pub asset_id: String,
    pub file_name: String,
    pub remote_path: String,
    pub size_bytes: i64,
    pub duration_seconds: Option<f64>,
}

/// Return all non-missing video assets (parts / editions) bound to a movie number,
/// ordered naturally by file name so multi-part FC2/JAV releases can switch parts cleanly.
pub fn list_movie_assets(conn: &Connection, number: &str) -> Result<Vec<MovieAssetPart>, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let upper = normalize_jav_number(number).unwrap_or_else(|| number.to_string());
    let mut stmt = conn.prepare(
        "SELECT DISTINCT a.id, a.file_name, a.remote_path, COALESCE(a.size_bytes, 0), a.duration_seconds
         FROM assets a
         WHERE a.missing = 0 AND a.media_type = 'video'
           AND (
             a.id IN (SELECT asset_id FROM movie_asset_bindings WHERE number = ?1)
             OR a.id IN (SELECT asset_id FROM movie_metadata WHERE number = ?1 AND asset_id IS NOT NULL)
           )
         ORDER BY a.file_name COLLATE NOCASE ASC, a.id ASC"
    )?;
    let rows = stmt.query_map(params![upper], |r| {
        Ok(MovieAssetPart {
            asset_id: r.get(0)?,
            file_name: r.get(1)?,
            remote_path: r.get(2)?,
            size_bytes: r.get(3)?,
            duration_seconds: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// Mark a non-JAV / non-movie video asset as ignored so it no longer clutters the unrecognized list.
pub fn ignore_unrecognized_video(conn: &Connection, asset_id: &str) -> Result<(), rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let id = asset_id.trim();
    if id.is_empty() { return Ok(()); }
    conn.execute(
        "INSERT OR REPLACE INTO scrape_ignored_assets(asset_id, created_at) VALUES (?1, ?2)",
        params![id, chrono::Utc::now().timestamp()],
    )?;
    Ok(())
}

/// Extract a JAV/FC2 number from file_name first, falling back to up to 2 ancestor folder
/// names when the video file itself is named generically (e.g. `/FC2-PPV-4833053/HD/01.mp4`).
pub fn extract_jav_number_with_path(file_name: &str, remote_path: &str) -> Option<String> {
    if let Some(num) = extract_jav_number(file_name) {
        return Some(num);
    }
    let normalized = remote_path.replace('\\', "/");
    let segments: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    let generic_dir = |s: &str| {
        let lower = s.to_ascii_lowercase();
        matches!(
            lower.as_str(),
            "fc2" | "jav" | "videos" | "video" | "movies" | "movie" | "115" | "115open"
                | "云下载" | "暂存文件" | "未分类" | "{未分类}" | "文件" | "漫画" | "downloads"
                | "hd" | "fhd" | "4k" | "uhd" | "sd" | "正片" | "特典" | "无码" | "有码" | "中字" | "字幕"
        )
    };
    for depth in 2..=3 {
        if segments.len() >= depth {
            let folder = segments[segments.len() - depth];
            if !generic_dir(folder) {
                if let Some(num) = extract_jav_number(folder) {
                    return Some(num);
                }
            }
        }
    }
    None
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

fn strip_promo_domains(upper: &str) -> String {
    let mut out = upper.to_string();
    for tld in [".COM", ".NET", ".ORG", ".ME", ".TV", ".CC", ".XYZ", ".INFO", ".BIZ", ".LA", ".IO", ".PW", ".TOP", ".SITE", ".VIP"] {
        while let Some(pos) = out.find(tld) {
            let mut start = pos;
            let bytes = out.as_bytes();
            while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || matches!(bytes[start - 1], b'-' | b'.')) {
                start -= 1;
            }
            let mut end = pos + tld.len();
            while end < bytes.len() && matches!(bytes[end], b'@' | b'-' | b'_' | b' ' | b']' | b')') {
                end += 1;
            }
            out.replace_range(start..end, " ");
        }
    }
    out
}

fn valid_number_tail(upper: &str, pos: usize) -> bool {
    let bytes = upper.as_bytes();
    if pos >= bytes.len() || !bytes[pos].is_ascii_alphanumeric() {
        return true;
    }
    if bytes[pos].is_ascii_digit() {
        return false;
    }
    // Allow common directly-attached release suffixes (C, CH, UC, U, A, B, K, R, Z, X)
    for suffix in ["CH", "UC", "C", "U", "A", "B", "K", "R", "Z", "X"] {
        if upper[pos..].starts_with(suffix) {
            let after = pos + suffix.len();
            if after == bytes.len() || !bytes[after].is_ascii_alphanumeric() {
                return true;
            }
        }
    }
    false
}

/// Extract an identity from a filename, stripping promotional domain prefixes and common noise tags.
/// Date identities retain their delimiter: 1pondo uses `_`, Caribbeancom uses `-`.
pub fn extract_jav_number(raw_name: &str) -> Option<String> {
    let name = raw_name.rsplit(['/', '\\']).next()?.trim();
    let name = match name.rsplit_once('.') {
        Some((stem, ext)) if ["mp4", "mkv", "avi", "wmv", "mov", "m4v", "ts", "webm", "iso", "flv", "rmvb"].contains(&ext.to_ascii_lowercase().as_str()) => stem,
        _ => name,
    };
    let upper = strip_promo_domains(&name.to_ascii_uppercase());
    let bytes = upper.as_bytes();
    let mut candidates = std::collections::BTreeSet::new();
    const NOISE_PREFIXES: &[&str] = &[
        "FHD", "HD", "UHD", "SD", "MP", "MKV", "MP4", "AVI", "WMV", "MOV",
        "CH", "CHS", "CHT", "SUB", "H264", "H265", "X264", "X265", "HEVC", "AVC",
        "AAC", "DTS", "FLAC", "FPS", "BIT", "PPV", "CARIB", "PART", "CD", "DISC",
        "DISK", "VOL", "EP", "UNCENSORED", "LEAK", "LEAKED", "RAW", "FULL", "SAMPLE",
        "TRAILER", "BONUS", "VER", "EDITION", "CUT", "UNCUT", "VR", "HDR", "SDR",
        "WEB", "WEBRIP", "BDRIP", "DVDRIP", "BLURAY", "REMUX", "RIP", "ENC",
    ];

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
            if (5..=8).contains(&(i - n)) && valid_number_tail(&upper, i) {
                candidates.insert(format!("FC2-{}", &upper[n..i]));
            }
            continue;
        }
        if bytes[start].is_ascii_digit() {
            // 1) Date-style IDs (6 digits + '-'/'_' + 3 digits, e.g. 010126-123 / 010126_123)
            let end = start + 10;
            if end <= bytes.len() && bytes[start..start + 6].iter().all(u8::is_ascii_digit)
                && matches!(bytes[start + 6], b'-' | b'_')
                && bytes[start + 7..end].iter().all(u8::is_ascii_digit)
                && valid_number_tail(&upper, end) {
                candidates.insert(upper[start..end].to_string());
                continue;
            }
            // 2) Digit-prefixed studio IDs (e.g. 300MIUM-123, 259LUXU-1234, 200GANA-1234, 420STH-123)
            let mut d = start;
            while d < bytes.len() && bytes[d].is_ascii_digit() { d += 1; }
            let d_len = d - start;
            if (2..=3).contains(&d_len) && d < bytes.len() && bytes[d].is_ascii_alphabetic() {
                let mut l = d;
                while l < bytes.len() && bytes[l].is_ascii_alphabetic() { l += 1; }
                let l_len = l - d;
                if (2..=6).contains(&l_len) {
                    let prefix = &upper[start..l];
                    let letters_only = &upper[d..l];
                    if !NOISE_PREFIXES.contains(&letters_only) {
                        let mut i = l;
                        if i < bytes.len() && matches!(bytes[i], b'-' | b'_' | b' ') { i += 1; }
                        let n = i;
                        while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
                        if (2..=5).contains(&(i - n)) && valid_number_tail(&upper, i) {
                            candidates.insert(format!("{}-{}", prefix, &upper[n..i]));
                        }
                    }
                }
            }
            continue;
        }
        if !bytes[start].is_ascii_alphabetic() { continue; }
        let mut i = start;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() { i += 1; }
        let letters = &upper[start..i];
        // Support alphanumeric studio prefixes like T28-123, S2MBD-123, M2G-123
        if letters.len() == 1 && i < bytes.len() && bytes[i].is_ascii_digit() {
            let mut p = i;
            while p < bytes.len() && bytes[p].is_ascii_alphanumeric() { p += 1; }
            let prefix = &upper[start..p];
            if (2..=6).contains(&prefix.len()) && p < bytes.len() && matches!(bytes[p], b'-' | b'_') {
                let n = p + 1;
                let mut k = n;
                while k < bytes.len() && bytes[k].is_ascii_digit() { k += 1; }
                if (2..=5).contains(&(k - n)) && valid_number_tail(&upper, k) {
                    candidates.insert(format!("{}-{}", prefix, &upper[n..k]));
                    continue;
                }
            }
        }
        if !(2..=7).contains(&letters.len()) || NOISE_PREFIXES.contains(&letters) { continue; }
        if i < bytes.len() && matches!(bytes[i], b'-' | b'_' | b' ') { i += 1; }
        let n = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
        if (2..=6).contains(&(i - n)) && valid_number_tail(&upper, i) {
            candidates.insert(format!("{}-{}", letters, &upper[n..i]));
        }
    }
    if candidates.len() == 1 {
        return candidates.into_iter().next();
    }
    // If FC2-xxxx was detected alongside a secondary noise match, prefer the unambiguous FC2 ID.
    let fc2_hits: Vec<String> = candidates.iter().filter(|c| c.starts_with("FC2-")).cloned().collect();
    if fc2_hits.len() == 1 {
        return fc2_hits.into_iter().next();
    }
    None
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

fn caribbean_exact_identity(html: &str, number: &str) -> bool {
    let expected = number.replace('_', "-");
    let detail_prefix = format!("/moviepages/{expected}/");
    let gallery_prefix = format!("/moviepages/{expected}/images/l/");

    // Canonical/OG identity is authoritative. If a canonical tag exists but
    // points elsewhere, reject immediately; never fall back to recommendations.
    let mut canonical_seen = false;
    let mut canonical_exact = false;
    for seg in html.split('<').filter(|tag| tag.starts_with("link ") || tag.starts_with("meta ")) {
        let is_canonical = attribute(seg, "rel").is_some_and(|v| v.split_whitespace().any(|x| x.eq_ignore_ascii_case("canonical")));
        let is_og = attribute(seg, "property").is_some_and(|v| v.eq_ignore_ascii_case("og:url"));
        if !(is_canonical || is_og) { continue; }
        if is_canonical { canonical_seen = true; }
        let Some(raw) = attribute(seg, "href").or_else(|| attribute(seg, "content")) else { return false; };
        let Ok(url) = reqwest::Url::parse(&raw).or_else(|_| reqwest::Url::parse("https://www.caribbeancom.com/").and_then(|base| base.join(&raw))) else { return false; };
        let exact = url.host_str() == Some("www.caribbeancom.com") && url.path() == detail_prefix;
        if is_canonical && exact { canonical_exact = true; }
        if is_canonical && !exact { return false; }
        if is_og && !exact { return false; }
    }
    if canonical_seen { return canonical_exact; }

    // Archived pages without canonical: require a real main gallery anchor,
    // not an arbitrary h1, recommendation link, or nearby text. The live page
    // uses <a class="gallery-item fancy-gallery" href="/moviepages/<id>/images/l/001.jpg">.
    for (idx, _) in html.match_indices("<a ") {
        let part = &html[idx + 3..];
        let Some((tag, body)) = part.split_once('>') else { continue; };
        if !attribute(tag, "class").is_some_and(|classes| classes.split_whitespace().any(|c| c == "gallery-item")) { continue; }
        // Build a UTF-8-safe forward context window; reversing the chars would
        // make marker searches fail and slicing raw bytes could panic on CJK.
        let context = html[..idx].chars().rev().take(1200).collect::<String>().chars().rev().collect::<String>().to_ascii_lowercase();
        if !context.contains("movie-gallery") || context.contains("recommend") || context.contains("related") { continue; }
        let Some(raw) = attribute(tag, "href") else { continue; };
        let image_src = body.split('<').find_map(|piece| {
            let tag = piece.strip_prefix("img ").unwrap_or(piece);
            if tag == piece { None } else { attribute(tag, "src") }
        });
        let Some(image_src) = image_src else { continue; };
        let Ok(image_url) = reqwest::Url::parse(&image_src).or_else(|_| reqwest::Url::parse("https://www.caribbeancom.com/").and_then(|base| base.join(&image_src))) else { continue; };
        if image_url.host_str() != Some("www.caribbeancom.com") || !image_url.path().starts_with(&format!("/moviepages/{expected}/images/s/")) { continue; }
        let Ok(url) = reqwest::Url::parse(&raw).or_else(|_| reqwest::Url::parse("https://www.caribbeancom.com/").and_then(|base| base.join(&raw))) else { continue; };
        if url.host_str() != Some("www.caribbeancom.com") || !url.path().starts_with(&gallery_prefix) { continue; }
        let tail = &url.path()[gallery_prefix.len()..];
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit() || c == '.' || c.is_ascii_alphabetic() || c == '-') {
            return true;
        }
    }
    false
}

fn caribbean_sample_images(html: &str, number: &str) -> Vec<String> {
    let mut images = Vec::new();
    let expected = format!("/moviepages/{number}/images/l/");
    for seg in html.split("<a ").skip(1) {
        let tag = seg.split('>').next().unwrap_or("");
        let Some(raw) = attribute(tag, "href") else { continue };
        let Some(url) = valid_image_url("https://www.caribbeancom.com/", &raw) else { continue };
        let Ok(parsed) = reqwest::Url::parse(&url) else { continue };
        // Member-only links and recommendations must not become broken public stills.
        if parsed.host_str() == Some("www.caribbeancom.com") && parsed.path().starts_with(&expected)
            && !images.contains(&url) { images.push(url); }
    }
    images
}

fn javbus_sample_images(html: &str) -> Vec<String> {
    let mut images = Vec::new();
    for seg in html.split("<a ").skip(1) {
        let Some(tag) = seg.split('>').next() else { continue };
        if !attribute(tag, "class").is_some_and(|classes| classes.split_whitespace().any(|c| c == "sample-box")) { continue; }
        let Some(raw) = attribute(tag, "href") else { continue };
        if raw.trim().is_empty() { continue; }
        if let Some(url) = valid_image_url("https://www.javbus.com/", &raw) {
            if !images.contains(&url) { images.push(url); }
        }
    }
    images
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

#[derive(Debug, Clone)]
struct Fc2Parsed {
    title: String,
    cover: Option<String>,
    samples: Vec<String>,
    release_date: Option<String>,
}

fn exact_fc2_url(value: &str, id: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else { return false; };
    url.scheme() == "https" && url.host_str() == Some("adult.contents.fc2.com") && url.path() == format!("/article/{id}/")
}

fn fc2_title_has_id(title: &str, id: &str) -> bool {
    let upper = title.to_ascii_uppercase();
    [format!("FC2-PPV-{id}"), format!("FC2 PPV {id}")].iter().any(|needle| {
        let mut from = 0;
        while let Some(pos) = upper[from..].find(needle) {
            let start = from + pos;
            let end = start + needle.len();
            let before = upper[..start].chars().next_back();
            let after = upper[end..].chars().next();
            if !before.is_some_and(|c| c.is_ascii_alphanumeric()) && !after.is_some_and(|c| c.is_ascii_digit()) { return true; }
            from = end;
        }
        false
    })
}

fn parse_fc2_html(html: &str, id: &str, base: &str) -> Result<Fc2Parsed, String> {
    let canonical = html.split("<link ").skip(1).filter_map(|s| s.split('>').next()).find_map(|tag| {
        (attribute(tag, "rel").is_some_and(|v| v.split_whitespace().any(|x| x.eq_ignore_ascii_case("canonical"))))
            .then(|| attribute(tag, "href")).flatten()
    });
    let og_url = html.split("<meta ").skip(1).filter_map(|s| s.split('>').next()).find_map(|tag| {
        (attribute(tag, "property").is_some_and(|v| v.eq_ignore_ascii_case("og:url"))).then(|| attribute(tag, "content")).flatten()
    });
    if !canonical.as_deref().is_some_and(|v| exact_fc2_url(v, id)) && !og_url.as_deref().is_some_and(|v| exact_fc2_url(v, id)) {
        return Err(format!("FC2 官方页面身份未验证【FC2-{id}】"));
    }
    let mut product = None::<serde_json::Value>;
    for block in html.split("<script type=\"application/ld+json\">").skip(1) {
        let Some(json) = block.split("</script>").next().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()) else { continue };
        let matches = json.get("sku").and_then(|v| v.as_str()) == Some(id) || json.get("productID").and_then(|v| v.as_str()) == Some(id);
        if matches { product = Some(json); break; }
    }
    let product = product.ok_or_else(|| format!("FC2 官方页面缺少精确商品数据【FC2-{id}】"))?;
    // Official Product.name may omit the printed FC2 number. The exact canonical
    // URL and the same Product's exact sku/productID already establish identity.
    let title = product.get("name").and_then(|v| v.as_str()).map(plain_text).filter(|s| !s.trim().is_empty()).ok_or_else(|| format!("FC2 官方商品缺少标题【FC2-{id}】"))?;
    let cover = product.get("image").and_then(|v| v.get("url")).and_then(|v| v.as_str()).and_then(|v| valid_image_url(base, v));
    let mut samples = Vec::new();
    if let Some(seg) = html.split("data-feed=\"sample-images\"").nth(1).and_then(|s| s.split("</ul>").next()) {
        for a in seg.split("<a ").skip(1) {
            if let Some(v) = attribute(a.split('>').next().unwrap_or(""), "href").and_then(|v| valid_image_url(base, &v)) { if !samples.contains(&v) { samples.push(v); } }
        }
    }
    let release_date = html.split("販売日 :").nth(1).and_then(|s| s.split('<').next()).map(str::trim).filter(|s| s.len() >= 10).map(str::to_string);
    Ok(Fc2Parsed { title, cover, samples, release_date })
}

pub async fn scrape_from_fc2(number: &str) -> Result<MovieMetadata, String> {
    let id = number.strip_prefix("FC2-").ok_or_else(|| "FC2番号格式错误".to_string())?;
    if !id.chars().all(|c| c.is_ascii_digit()) || !(5..=8).contains(&id.len()) {
        return Err(format!("FC2番号格式错误【{number}】"));
    }
    let client = scraper_client()?;
    let url = format!("https://adult.contents.fc2.com/article/{id}/");
    let resp = client.get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/122 Safari/537.36")
        .header("Accept", "text/html,application/xhtml+xml")
        .send().await.map_err(|e| format!("请求 FC2 官方页面失败：{e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(format!("FC2 官方页面未找到作品【{number}】"));
    }
    if !resp.status().is_success() { return Err(format!("FC2 官方页面状态异常：{}", resp.status())); }
    let html = resp.text().await.map_err(|e| format!("读取 FC2 官方页面失败：{e}"))?;
    let lower = html.to_ascii_lowercase();
    if lower.contains("cf-chl-") || lower.contains("captcha") {
        return Err(format!("FC2 官方页面需要验证【{number}】"));
    }
    // FC2 sometimes returns HTTP 200 for a soft-404 page. Treat it as a
    // missing work, not as an identity or generic parse failure.
    if lower.contains("page not found") || lower.contains("ページが見つかりません")
        || lower.contains("お探しのページは見つかりません")
        || lower.contains("商品が見つかりません") {
        return Err(format!("FC2 官方页面未找到作品（软404）【{number}】"));
    }
    let parsed = parse_fc2_html(&html, id, &url)?;
    Ok(MovieMetadata { number: number.to_string(), asset_id: None, title: parsed.title.clone(), original_title: Some(parsed.title), actors: vec![], maker: Some("FC2".to_string()), label: None, series: None, director: None, release_date: parsed.release_date, duration_minutes: None, cover_url: parsed.cover, sample_images: parsed.samples, tags: vec!["FC2".to_string()], source: "fc2-official".to_string(), updated_at: chrono::Utc::now().timestamp() })
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
    let sample_images = javbus_sample_images(&html);

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
    if is_captcha {
        return Err(format!("JavDB 详情页需要验证或加载失败【{number}】"));
    }
    if detail_identity(&html, &["<strong>番號:</strong>", "<strong>番号:</strong>"]).as_deref() != Some(number) {
        return Err(format!("JavDB 详情番号不匹配【{number}】"));
    }

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

    // Parse Maker (片商 / FC2 卖家)
    let mut maker = None;
    for label in ["<strong>片商:</strong>", "<strong>賣家:</strong>", "<strong>卖家:</strong>"] {
        if let Some(start) = html.find(label) {
            let rest = html_window(&html, start, start + 300);
            if let Some(a_start) = rest.find("<a ") {
                if let Some(txt_start) = rest[a_start..].find('>') {
                    let txt_rest = &rest[a_start + txt_start + 1..];
                    if let Some(txt_end) = txt_rest.find("</a>") {
                        let m = txt_rest[..txt_end].trim().to_string();
                        if !m.is_empty() { maker = Some(m); break; }
                    }
                }
            }
        }
    }
    if maker.is_none() && number.starts_with("FC2-") {
        maker = Some("FC2".to_string());
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

    if !caribbean_exact_identity(&html, &clean_num) {
        return Err(format!("Caribbeancom 无法核验详情番号【{number}】"));
    }
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

    let sample_images = caribbean_sample_images(&html, &clean_num);

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
        sample_images,
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

    // The official JSON exposes video samples, not still-image URLs. Do not
    // reinterpret SampleFiles or invent gallery paths when HasGallery=false.
    let sample_images = Vec::new();

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
        sample_images,
        tags: vec!["无码".to_string(), "一本道".to_string()],
        source: "1pondo".to_string(),
        updated_at: now,
    })
}

fn valid_image_url(base: &str, value: &str) -> Option<String> {
    let url = reqwest::Url::parse(base).ok()?.join(value.trim()).ok()?;
    if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() { Some(url.to_string()) } else { None }
}

fn metadata_cache() -> &'static Mutex<std::collections::HashMap<String, MovieMetadata>> {
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, MovieMetadata>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub fn invalidate_metadata_cache(number: &str) {
    if let Ok(mut items) = metadata_cache().lock() { items.remove(number); }
}

/// Shared successes avoid repeated network requests. Failures are never negative-cached:
/// a timeout/challenge is not proof that an identity does not exist.
pub async fn scrape_movie_metadata(raw: &str) -> Result<MovieMetadata, String> {
    let number = normalize_jav_number(raw).ok_or_else(|| "无效或有歧义的番号".to_string())?;
    let cache = metadata_cache();
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
        } else if number.starts_with("FC2-") {
            // Prefer FC2 official page; automatically fall back to JavDB when the seller
            // delisted the item on FC2 (404 / soft-404) so archived FC2 titles still scrape.
            match scrape_from_fc2(&number).await {
                Ok(meta) => Ok(meta),
                Err(fc2_err) => scrape_from_javdb(&number)
                    .await
                    .map_err(|db_err| format!("{fc2_err}; JavDB: {db_err}")),
            }
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

    let normalized_date = normalize_release_date(meta.release_date.as_deref());
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
            normalized_date,
            meta.duration_minutes,
            meta.cover_url,
            samples_json,
            tags_json,
            meta.source,
            meta.updated_at
        ],
    )?;

    if let Some(asset_id) = &meta.asset_id { bind_movie_asset(conn, &meta.number, asset_id)?; }
    conn.execute("DELETE FROM scrape_failures WHERE number=?1", params![meta.number])?;
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieLibraryPage {
    pub items: Vec<MovieMetadata>,
    pub total: usize,
    pub makers: Vec<String>,
    pub actors: Vec<String>,
}

fn metadata_from_row(row: &rusqlite::Row<'_>) -> Result<MovieMetadata, rusqlite::Error> {
    let actors_json: String = row.get(4)?;
    let sample_json: String = row.get(12)?;
    let tags_json: String = row.get(13)?;
    Ok(MovieMetadata {
        number: row.get(0)?, asset_id: row.get(1)?, title: row.get(2)?, original_title: row.get(3)?,
        actors: serde_json::from_str(&actors_json).unwrap_or_default(), maker: row.get(5)?, label: row.get(6)?,
        series: row.get(7)?, director: row.get(8)?, release_date: row.get(9)?, duration_minutes: row.get(10)?,
        cover_url: row.get(11)?, sample_images: serde_json::from_str(&sample_json).unwrap_or_default(),
        tags: serde_json::from_str(&tags_json).unwrap_or_default(), source: row.get(14)?, updated_at: row.get(15)?,
    })
}

/// Search the verified movie library. All user values are bound SQL parameters;
/// sort is selected from a fixed allow-list and always has number as a tie-breaker.
fn escape_like_pattern(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') { escaped.push('\\'); }
        escaped.push(ch);
    }
    escaped
}

pub fn query_movie_library(conn: &Connection, query: &str, maker: &str, actor: &str, sort: &str, limit: u32, offset: u32) -> Result<MovieLibraryPage, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    // Keep performer candidates synchronized before any read. Errors are wrapped
    // as rusqlite conversion failures so the public API remains DB-compatible.
    crate::performers::sync_performers(conn).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))))?;
    let verified_where = " WHERE verification_version = 1 AND source NOT IN ('javdb_not_found','not_found','') AND cover_url IS NOT NULL AND cover_url != ''";
    let mut where_sql = verified_where.to_string();
    let q = query.trim();
    let m = maker.trim();
    let a = actor.trim();
    let mut args: Vec<Box<dyn ToSql>> = Vec::new();
    if !q.is_empty() {
        where_sql.push_str(" AND (lower(number) LIKE lower(?) ESCAPE '\\' OR lower(title) LIKE lower(?) ESCAPE '\\' OR lower(COALESCE(original_title,'')) LIKE lower(?) ESCAPE '\\' OR lower(COALESCE(maker,'')) LIKE lower(?) ESCAPE '\\' OR lower(COALESCE(label,'')) LIKE lower(?) ESCAPE '\\' OR lower(COALESCE(series,'')) LIKE lower(?) ESCAPE '\\' OR lower(COALESCE(director,'')) LIKE lower(?) ESCAPE '\\' OR lower(actors_json) LIKE lower(?) ESCAPE '\\' OR lower(tags_json) LIKE lower(?) ESCAPE '\\' OR EXISTS (SELECT 1 FROM movie_cast qc JOIN performers qp ON qp.id=qc.person_id WHERE qc.number=movie_metadata.number AND (lower(qp.name) LIKE lower(?) ESCAPE '\\' OR EXISTS (SELECT 1 FROM json_each(qp.aliases_json) qa WHERE lower(CAST(qa.value AS TEXT)) LIKE lower(?) ESCAPE '\\'))))");
        let pat = format!("%{}%", escape_like_pattern(q));
        for _ in 0..11 { args.push(Box::new(pat.clone())); }
    }
    if !m.is_empty() { where_sql.push_str(" AND maker = ?"); args.push(Box::new(m.to_string())); }
    if !a.is_empty() {
        // Actor filter accepts a stable performer id, the original raw credit,
        // or an exact current name/alias. Every comparison is parameterized.
        where_sql.push_str(" AND (EXISTS (SELECT 1 FROM json_each(actors_json) actor_value WHERE lower(CAST(actor_value.value AS TEXT)) = lower(?)) OR EXISTS (SELECT 1 FROM movie_cast ac WHERE ac.number=movie_metadata.number AND (lower(ac.person_id)=lower(?) OR lower(ac.raw_name)=lower(?) OR EXISTS (SELECT 1 FROM performers ap WHERE ap.id=ac.person_id AND (lower(ap.name)=lower(?) OR EXISTS (SELECT 1 FROM json_each(ap.aliases_json) aa WHERE lower(CAST(aa.value AS TEXT))=lower(?)))))))");
        for _ in 0..5 { args.push(Box::new(a.to_string())); }
    }
    let total_sql = format!("SELECT COUNT(*) FROM movie_metadata{where_sql}");
    let total: usize = conn.query_row(&total_sql, rusqlite::params_from_iter(args.iter().map(|v| v.as_ref())), |r| r.get::<_, i64>(0))? as usize;

    // Facets intentionally come from the full verified library, not the active
    // maker/actor/query filter, so changing one dropdown does not erase options.
    let mut makers = std::collections::BTreeSet::new();
    let mut maker_stmt = conn.prepare("SELECT DISTINCT maker FROM movie_metadata WHERE verification_version = 1 AND source NOT IN ('javdb_not_found','not_found','') AND cover_url IS NOT NULL AND cover_url != '' AND maker IS NOT NULL AND maker != '' ORDER BY maker")?;
    let maker_rows = maker_stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in maker_rows { makers.insert(row?); }
    let mut actors_set = std::collections::BTreeSet::new();
    let mut actor_stmt = conn.prepare("SELECT actors_json FROM movie_metadata WHERE verification_version = 1 AND source NOT IN ('javdb_not_found','not_found','') AND cover_url IS NOT NULL AND cover_url != ''")?;
    let actor_rows = actor_stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in actor_rows {
        if let Ok(values) = serde_json::from_str::<Vec<String>>(&row?) { for value in values { if !value.trim().is_empty() { actors_set.insert(value); } } }
    }

    let order_sql = match sort.trim().to_ascii_lowercase().as_str() {
        "number" | "number_asc" => "number ASC, updated_at DESC",
        "title" | "title_asc" => "title COLLATE NOCASE ASC, number ASC",
        "date" | "release_date" | "release_date_desc" => {
            "CASE WHEN release_date IS NULL OR trim(release_date) = '' THEN 1 ELSE 0 END ASC, replace(release_date, '/', '-') DESC, updated_at DESC, number ASC"
        }
        "release_date_asc" => {
            "CASE WHEN release_date IS NULL OR trim(release_date) = '' THEN 1 ELSE 0 END ASC, replace(release_date, '/', '-') ASC, updated_at DESC, number ASC"
        }
        "updated_at" | "updated_at_desc" | "added" | "added_desc" => "updated_at DESC, number ASC",
        "updated_at_asc" | "added_asc" => "updated_at ASC, number ASC",
        _ => "updated_at DESC, number ASC",
    };
    let sql = format!("SELECT number, asset_id, title, original_title, actors_json, maker, label, series, director, release_date, duration_minutes, cover_url, sample_images_json, tags_json, source, updated_at FROM movie_metadata{where_sql} ORDER BY {order_sql} LIMIT ? OFFSET ?");
    let mut stmt = conn.prepare(&sql)?;
    let mut page_args: Vec<Box<dyn ToSql>> = args;
    page_args.push(Box::new(limit.min(100)));
    page_args.push(Box::new(offset));
    let rows = stmt.query_map(rusqlite::params_from_iter(page_args.iter().map(|v| v.as_ref())), metadata_from_row)?;
    let items = rows.collect::<Result<Vec<_>, _>>()?;
    Ok(MovieLibraryPage { items, total, makers: makers.into_iter().collect(), actors: actors_set.into_iter().collect() })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeIssue {
    pub number: String,
    pub error: String,
    pub category: String,
    pub retryable: bool,
    pub updated_at: i64,
    #[serde(default)]
    pub files: Vec<UnrecognizedVideo>,
}

/// Return a random pool of verified movies across the entire library for the Hero recommendation banner.
pub fn get_random_featured_movies(conn: &Connection, limit: u32) -> Result<Vec<MovieMetadata>, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let cap = limit.clamp(1, 30);
    let mut stmt = conn.prepare(
        "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                director, release_date, duration_minutes, cover_url, sample_images_json,
                tags_json, source, updated_at
         FROM movie_metadata
         WHERE verification_version = 1
           AND source NOT IN ('javdb_not_found','not_found','')
           AND cover_url IS NOT NULL AND cover_url != ''
         ORDER BY CASE WHEN sample_images_json IS NOT NULL AND sample_images_json != '[]' THEN 0 ELSE 1 END ASC,
                  RANDOM()
         LIMIT ?1"
    )?;
    let rows = stmt.query_map(params![cap], metadata_from_row)?;
    rows.collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSyncReport {
    pub scanned_dirs: usize,
    pub added_files: i64,
    pub updated_files: i64,
    pub removed_files: i64,
    pub unbound_missing_assets: usize,
    pub rebound_existing_movies: usize,
    pub cleaned_stale_failures: usize,
    pub pending_new_count: usize,
    pub pending_new_numbers: Vec<String>,
}

/// Collect the distinct immediate parent directory paths of all current failure-related files
/// and unrecognized whitelisted videos, so targeted remote refresh finishes in ~1 second.
pub fn issue_and_unrecognized_parent_dirs(conn: &Connection) -> Result<Vec<String>, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let whitelist = get_scrape_whitelist(conn)?;
    let mut fail_numbers = std::collections::HashSet::<String>::new();
    {
        let mut stmt = conn.prepare("SELECT number FROM scrape_failures")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows.flatten() {
            fail_numbers.insert(r);
        }
    }
    let mut dirs = std::collections::BTreeSet::<String>::new();
    let mut stmt = conn.prepare("SELECT file_name, remote_path, parent_path FROM assets WHERE missing = 0 AND media_type = 'video'")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    for row in rows.flatten() {
        let (file_name, remote_path, parent_path) = row;
        if !whitelist.iter().any(|p| path_under(&remote_path, p)) { continue; }
        let matched = match extract_jav_number_with_path(&file_name, &remote_path) {
            None => true,
            Some(num) => fail_numbers.contains(&num),
        };
        if matched {
            let p = parent_path.trim().trim_end_matches('/');
            if !p.is_empty() {
                dirs.insert(p.to_string());
            }
        }
    }
    Ok(dirs.into_iter().collect())
}

/// Reconcile local scraper bindings and failure records after a remote provider directory scan:
/// Wrapped in a single SQLite transaction so hundreds/thousands of rebinds complete in milliseconds.
pub fn reconcile_movie_library_changes(
    conn: &Connection,
    scanned_dirs: usize,
    added_files: i64,
    updated_files: i64,
    removed_files: i64,
) -> Result<RemoteSyncReport, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let (_, number_assets) = whitelisted_number_assets(conn)?;

    let tx = conn.unchecked_transaction()?;
    let unbound_missing_assets = tx.execute(
        "DELETE FROM movie_asset_bindings
         WHERE NOT EXISTS (SELECT 1 FROM assets a WHERE a.id = movie_asset_bindings.asset_id AND a.missing = 0)",
        [],
    )?;
    tx.execute(
        "UPDATE movie_metadata
         SET asset_id = (
             SELECT b.asset_id FROM movie_asset_bindings b
             JOIN assets a ON a.id = b.asset_id AND a.missing = 0
             WHERE b.number = movie_metadata.number
             ORDER BY a.file_name COLLATE NOCASE ASC, a.id ASC
             LIMIT 1
         )
         WHERE asset_id IS NULL
            OR NOT EXISTS (SELECT 1 FROM assets a WHERE a.id = movie_metadata.asset_id AND a.missing = 0)",
        [],
    )?;

    let mut verified_numbers = std::collections::HashSet::<String>::new();
    {
        let mut stmt = tx.prepare(
            "SELECT number FROM movie_metadata WHERE verification_version = 1 AND source NOT IN ('javdb_not_found','not_found','') AND cover_url IS NOT NULL AND cover_url != ''"
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for r in rows.flatten() {
            verified_numbers.insert(r);
        }
    }

    let mut rebound_existing_movies = 0usize;
    let mut pending_new_numbers = Vec::new();
    for (number, assets) in &number_assets {
        if verified_numbers.contains(number) {
            for asset_id in assets {
                tx.execute(
                    "INSERT INTO movie_asset_bindings(asset_id, number) VALUES (?1, ?2)
                     ON CONFLICT(asset_id) DO UPDATE SET number = excluded.number",
                    params![asset_id, number],
                )?;
                tx.execute(
                    "UPDATE movie_metadata SET asset_id = COALESCE(asset_id, ?2) WHERE number = ?1",
                    params![number, asset_id],
                )?;
                rebound_existing_movies += 1;
            }
        } else {
            pending_new_numbers.push(number.clone());
        }
    }

    let fail_numbers: Vec<String> = {
        let mut fail_stmt = tx.prepare("SELECT number FROM scrape_failures")?;
        let rows = fail_stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut cleaned_stale_failures = 0usize;
    for num in fail_numbers {
        if !number_assets.contains_key(&num) {
            cleaned_stale_failures += tx.execute("DELETE FROM scrape_failures WHERE number = ?1", params![num])?;
        }
    }
    tx.commit()?;

    let pending_new_count = pending_new_numbers.len();
    pending_new_numbers.truncate(30);

    Ok(RemoteSyncReport {
        scanned_dirs,
        added_files,
        updated_files,
        removed_files,
        unbound_missing_assets,
        rebound_existing_movies,
        cleaned_stale_failures,
        pending_new_count,
        pending_new_numbers,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeIssueCount { pub category: String, pub count: usize }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeIssuePage {
    pub items: Vec<ScrapeIssue>,
    pub total: usize,
    pub counts: Vec<ScrapeIssueCount>,
}

fn looks_like_padding_candidate(number: &str) -> bool {
    let Some((prefix, digits)) = number.split_once('-') else { return false; };
    (2..=7).contains(&prefix.len()) && digits.len() == 5 && digits.starts_with("00")
}

fn classify_scrape_error(number: &str, error: &str) -> (&'static str, bool) {
    let lower = error.to_ascii_lowercase();
    // A conventional title with a zero-padded 5-digit suffix may be an
    // alternate presentation (e.g. IPX-00015), never an automatic binding.
    // Surface it for an explicit operator decision instead of retrying blindly.
    if looks_like_padding_candidate(number) && !number.starts_with("FC2-")
        && (lower.contains("404") || lower.contains("未找到") || lower.contains("not found") || lower.contains("不匹配") || lower.contains("身份")) {
        return ("padding_candidate", false);
    }
    if lower.contains("fc2") && (lower.contains("404") || lower.contains("未找到") || lower.contains("软404") || lower.contains("not found")) {
        // Legacy FC2-only failures (before JavDB fallback existed) are retryable so "仅重试失败项"
        // automatically rescues delisted FC2 titles via JavDB.
        let retryable = !lower.contains("javdb");
        ("fc2_missing", retryable)
    }
    else if lower.contains("身份") || lower.contains("不匹配") || lower.contains("核验") || lower.contains("番号格式") { ("identity", false) }
    else if lower.contains("404") || lower.contains("未找到") || lower.contains("not found") || lower.contains("not_found") { ("not_found", false) }
    else if lower.contains("验证") || lower.contains("captcha") || lower.contains("cloudflare") || lower.contains("driver-verify") || lower.contains("challenge") { ("blocked", true) }
    else if lower.contains("请求") || lower.contains("读取") || lower.contains("超时") || lower.contains("timeout") || lower.contains("状态异常") || lower.contains("连接") { ("network", true) }
    else { ("other", true) }
}

pub fn list_scrape_issues(conn: &Connection, kind: &str, query: &str, limit: u32, offset: u32) -> Result<ScrapeIssuePage, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let whitelist = get_scrape_whitelist(conn)?;
    let mut files_by_number = std::collections::HashMap::<String, Vec<UnrecognizedVideo>>::new();
    if let Ok(mut asset_stmt) = conn.prepare("SELECT id, file_name, remote_path FROM assets WHERE missing = 0 AND media_type = 'video' ORDER BY file_name COLLATE NOCASE ASC, id ASC") {
        if let Ok(asset_rows) = asset_stmt.query_map([], |r| Ok(UnrecognizedVideo { id: r.get(0)?, file_name: r.get(1)?, remote_path: r.get(2)? })) {
            for item in asset_rows.flatten() {
                if !whitelist.iter().any(|prefix| path_under(&item.remote_path, prefix)) { continue; }
                if let Some(num) = extract_jav_number_with_path(&item.file_name, &item.remote_path) {
                    files_by_number.entry(num).or_default().push(item);
                }
            }
        }
    }
    let mut stmt = conn.prepare("SELECT number, error, updated_at FROM scrape_failures ORDER BY updated_at DESC, number ASC")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
    let kind = kind.trim().to_ascii_lowercase();
    let query = query.trim().to_ascii_lowercase();
    let mut candidates = Vec::new();
    for row in rows {
        let (number, error, updated_at) = row?;
        let (category, retryable) = classify_scrape_error(&number, &error);
        let files = files_by_number.get(&number).cloned().unwrap_or_default();
        let file_haystack: String = files.iter().map(|f| format!("{} {}", f.file_name, f.remote_path)).collect::<Vec<_>>().join(" ");
        if !query.is_empty() && !format!("{} {} {} {}", number, error, category, file_haystack).to_ascii_lowercase().contains(&query) { continue; }
        candidates.push(ScrapeIssue { number, error, category: category.to_string(), retryable, updated_at, files });
    }
    // Counts represent every category matching the text query, independent of
    // the selected category chip; total/items honor the selected kind.
    let mut count_map = std::collections::BTreeMap::<String, usize>::new();
    for item in &candidates { *count_map.entry(item.category.clone()).or_default() += 1; }
    let all: Vec<ScrapeIssue> = candidates.into_iter().filter(|item| kind.is_empty() || kind == "all" || kind == item.category).collect();
    let total = all.len();
    let start = offset as usize;
    let end = start.saturating_add(limit.min(100) as usize).min(total);
    let items = if start >= total { Vec::new() } else { all[start..end].to_vec() };
    let counts = count_map.into_iter().map(|(category, count)| ScrapeIssueCount { category, count }).collect();
    Ok(ScrapeIssuePage { items, total, counts })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnrecognizedVideo { pub id: String, pub file_name: String, pub remote_path: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnrecognizedVideoPage { pub items: Vec<UnrecognizedVideo>, pub total: usize }

/// Return only current, non-missing video assets under the configured whitelist
/// whose filename extractor cannot establish one unambiguous identity.
pub fn list_unrecognized_videos(conn: &Connection, limit: u32, offset: u32) -> Result<UnrecognizedVideoPage, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let whitelist = get_scrape_whitelist(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, file_name, remote_path FROM assets
         WHERE missing = 0 AND media_type = 'video'
           AND id NOT IN (SELECT asset_id FROM scrape_ignored_assets)
         ORDER BY id ASC"
    )?;
    let rows = stmt.query_map([], |r| Ok(UnrecognizedVideo { id: r.get(0)?, file_name: r.get(1)?, remote_path: r.get(2)? }))?;
    let mut all = Vec::new();
    for row in rows {
        let item = row?;
        if whitelist.iter().any(|prefix| path_under(&item.remote_path, prefix)) && extract_jav_number_with_path(&item.file_name, &item.remote_path).is_none() { all.push(item); }
    }
    let total = all.len();
    let start = offset as usize;
    let end = start.saturating_add(limit.min(100) as usize).min(total);
    let items = if start >= total { Vec::new() } else { all[start..end].to_vec() };
    Ok(UnrecognizedVideoPage { items, total })
}

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedMoviesGrouped {
    pub actor_names: Vec<String>,
    pub by_actors: Vec<MovieMetadata>,
    pub maker_name: Option<String>,
    pub by_maker: Vec<MovieMetadata>,
}

/// Partitioned related recommendations: separates same-actor works from same-maker works.
/// Movies without concrete actors (e.g. FC2) naturally return an empty `by_actors` and
/// populate `by_maker` cleanly.
pub fn get_related_movies_grouped(conn: &Connection, number: &str, limit_per_group: u32) -> Result<RelatedMoviesGrouped, rusqlite::Error> {
    let current = match get_movie_metadata(conn, number)? {
        Some(m) => m,
        None => return Ok(RelatedMoviesGrouped::default()),
    };
    let cap = limit_per_group.clamp(1, 20) as usize;
    let mut seen_numbers = std::collections::HashSet::new();
    seen_numbers.insert(current.number.clone());

    let mut by_actors = Vec::new();
    let actor_names: Vec<String> = current.actors.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    if !actor_names.is_empty() {
        for actor in &actor_names {
            if by_actors.len() >= cap { break; }
            let pattern = format!("%\"{}\"%", escape_like_pattern(actor));
            let mut stmt = conn.prepare(
                "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                        director, release_date, duration_minutes, cover_url, sample_images_json,
                        tags_json, source, updated_at
                 FROM movie_metadata
                 WHERE verification_version = 1
                   AND source NOT IN ('javdb_not_found','not_found','')
                   AND cover_url IS NOT NULL AND cover_url != ''
                   AND actors_json LIKE ?1 ESCAPE '\\' AND number != ?2
                 ORDER BY CASE WHEN release_date IS NULL OR trim(release_date) = '' THEN 1 ELSE 0 END ASC,
                          replace(release_date, '/', '-') DESC, updated_at DESC, number ASC
                 LIMIT ?3"
            )?;
            let rows = stmt.query_map(params![pattern, current.number, cap as u32], metadata_from_row)?;
            for r in rows.flatten() {
                if seen_numbers.insert(r.number.clone()) {
                    by_actors.push(r);
                    if by_actors.len() >= cap { break; }
                }
            }
        }
    }

    let mut by_maker = Vec::new();
    let maker_name = current.maker.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(ref maker) = maker_name {
        let mut stmt = conn.prepare(
            "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                    director, release_date, duration_minutes, cover_url, sample_images_json,
                    tags_json, source, updated_at
             FROM movie_metadata
             WHERE verification_version = 1
               AND source NOT IN ('javdb_not_found','not_found','')
               AND cover_url IS NOT NULL AND cover_url != ''
               AND maker = ?1 AND number != ?2
             ORDER BY CASE WHEN release_date IS NULL OR trim(release_date) = '' THEN 1 ELSE 0 END ASC,
                      replace(release_date, '/', '-') DESC, updated_at DESC, number ASC
             LIMIT ?3"
        )?;
        let rows = stmt.query_map(params![maker, current.number, (cap + by_actors.len()) as u32], metadata_from_row)?;
        for r in rows.flatten() {
            if seen_numbers.insert(r.number.clone()) {
                by_maker.push(r);
                if by_maker.len() >= cap { break; }
            }
        }
    }

    Ok(RelatedMoviesGrouped {
        actor_names,
        by_actors,
        maker_name,
        by_maker,
    })
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
        let clean = normalize_scrape_path(&p);
        if !clean.is_empty() {
            tx.execute(
                "INSERT OR IGNORE INTO scrape_whitelist (path, enabled, created_at) VALUES (?1, 1, ?2)",
                params![clean, now],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

fn normalize_scrape_path(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let mut result = String::new();
    for ch in path.chars() {
        if ch != '/' || !result.ends_with('/') { result.push(ch); }
    }
    result
}

fn path_under(path: &str, prefix: &str) -> bool {
    let path = normalize_scrape_path(path);
    let prefix = normalize_scrape_path(prefix);
    let prefix = prefix.trim_end_matches('/');
    prefix.is_empty() || path == prefix || path.strip_prefix(prefix).is_some_and(|rest| rest.starts_with('/'))
}

pub fn is_path_whitelisted(conn: &Connection, path: &str) -> bool {
    get_scrape_whitelist(conn).map(|paths| paths.iter().any(|p| path_under(path, p))).unwrap_or(false)
}

fn whitelisted_number_assets(conn: &Connection) -> Result<(usize, std::collections::BTreeMap<String, Vec<String>>), rusqlite::Error> {
    let whitelist = get_scrape_whitelist(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, file_name, remote_path FROM assets
         WHERE missing = 0 AND media_type = 'video'
           AND id NOT IN (SELECT asset_id FROM scrape_ignored_assets)
         ORDER BY id"
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))?;
    let mut total = 0;
    let mut numbers = std::collections::BTreeMap::<String, Vec<String>>::new();
    for row in rows {
        let (id, name, path) = row?;
        if !whitelist.iter().any(|p| path_under(&path, p)) { continue; }
        total += 1;
        if let Some(number) = extract_jav_number_with_path(&name, &path) { numbers.entry(number).or_default().push(id); }
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
    pub failed_count: usize,
    pub pending_revalidation: usize,
    pub unrecognized_videos: usize,
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
        failed_count: number_assets.keys().filter(|number| conn.query_row("SELECT EXISTS(SELECT 1 FROM scrape_failures WHERE number=?1)", params![number], |row| row.get::<_, bool>(0)).unwrap_or(false)).count(),
        pending_revalidation: conn.query_row("SELECT count(*) FROM movie_metadata WHERE verification_version=0", [], |row| row.get(0))?,
        unrecognized_videos: total_videos.saturating_sub(number_assets.values().map(Vec::len).sum::<usize>()),
    })
}

pub fn start_batch_scrape_engine(db_path: String) -> Result<(), String> {
    start_batch_scrape_with_mode(db_path, false)
}

pub fn start_failed_batch_scrape_engine(db_path: String) -> Result<(), String> {
    start_batch_scrape_with_mode(db_path, true)
}

fn start_batch_scrape_with_mode(db_path: String, failed_only: bool) -> Result<(), String> {
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
            if failed_only {
                match conn.query_row("SELECT EXISTS(SELECT 1 FROM scrape_failures WHERE number=?1)", params![number], |row| row.get::<_, bool>(0)) {
                    Ok(true) => {},
                    Ok(false) => continue,
                    Err(e) => {
                        if let Ok(mut g) = batch_progress_cell().lock() { g.running = false; g.last_error = Some(e.to_string()); }
                        return;
                    }
                }
            }
            match get_movie_metadata(&conn, number) {
                Ok(Some(_)) => {
                    // Repair cached entries too: no network is necessary to bind each duplicate file.
                    let _ = conn.execute("DELETE FROM scrape_failures WHERE number=?1", params![number]);
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
            let persistence = match &result {
                Ok(()) => conn.execute("DELETE FROM scrape_failures WHERE number=?1", params![number]),
                Err(error) => conn.execute("INSERT INTO scrape_failures(number,error,updated_at) VALUES (?1,?2,?3) ON CONFLICT(number) DO UPDATE SET error=excluded.error,updated_at=excluded.updated_at", params![number, error, chrono::Utc::now().timestamp()]),
            };
            if let Err(e) = persistence {
                if let Ok(mut g) = batch_progress_cell().lock() { g.last_error = Some(format!("记录重试状态失败：{e}")); }
            }
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
    fn fc2_parser_rejects_related_and_near_id_pages() {
        let base = "https://adult.contents.fc2.com/article/4939195/";
        let related = r#"<link rel="canonical" href="https://adult.contents.fc2.com/article/4939195/"><script type="application/ld+json">{"sku":"49391950","name":"FC2-PPV-49391950"}</script>"#;
        assert!(parse_fc2_html(related, "4939195", base).is_err());
        let wrong = r#"<link rel="canonical" href="https://adult.contents.fc2.com/article/49391950/"><script type="application/ld+json">{"sku":"49391950","name":"FC2-PPV-49391950"}</script>"#;
        assert!(parse_fc2_html(wrong, "4939195", base).is_err());
        let no_data = r#"<link rel="canonical" href="https://adult.contents.fc2.com/article/4939195/"><meta property="og:title" content="FC2-PPV-4939195">"#;
        assert!(parse_fc2_html(no_data, "4939195", base).is_err());
    }

    #[test]
    fn fc2_parser_accepts_actual_html_fixture() {
        let html = r#"<link rel="canonical" href="https://adult.contents.fc2.com/article/4939195/"><meta property="og:url" content="https://adult.contents.fc2.com/article/4939195/"><script type="application/ld+json">{"sku":"4939195","productID":"4939195","name":"FC2-PPV-4939195 Sample","image":{"url":"//storage200000.contents.fc2.com/cover.png"}}</script><div data-feed="sample-images"><ul><a href="//contents-thumbnail2.fc2.com/w1280/sample.png"></a></ul></div><p>販売日 : 2026/07/15</p>"#;
        let parsed = parse_fc2_html(html, "4939195", "https://adult.contents.fc2.com/article/4939195/").unwrap();
        assert_eq!(parsed.title, "FC2-PPV-4939195 Sample");
        let unnumbered = html.replace("FC2-PPV-4939195 Sample", "Official product title without printed number");
        assert!(parse_fc2_html(&unnumbered, "4939195", "https://adult.contents.fc2.com/article/4939195/").is_ok());
        assert_eq!(parsed.samples.len(), 1);
        assert_eq!(parsed.release_date.as_deref(), Some("2026/07/15"));
    }

    #[test]
    fn fc2_filename_normalization_does_not_guess_nonstandard_ids() {
        assert_eq!(extract_jav_number("FC2PPV-4939195.mp4").as_deref(), Some("FC2-4939195"));
        assert_eq!(extract_jav_number("fc3162920_4K.mp4"), None);
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
    fn caribbean_identity_compatibility_requires_main_work_evidence() {
        let canonical = r#"<link rel='canonical' href='https://www.caribbeancom.com/moviepages/011015-780/'><div class='recommend'><h1>011015-780</h1></div>"#;
        assert!(caribbean_exact_identity(canonical, "011015-780"));
        let archived = r#"<main><div class='movie-gallery section' id='sampleexclude'><div class='gallery-item fancy-gallery'><a class='gallery-item fancy-gallery' href='/moviepages/011015-780/images/l/001.jpg'><img src='/moviepages/011015-780/images/s/001.jpg'></a></div></div><h1>日本語タイトル</h1></main><div class='recommend'><h1>011015-999</h1></div>"#;
        assert!(caribbean_exact_identity(archived, "011015-780"));
        let related_only = r#"<div class='recommend'><h1>011015-780</h1><a class='gallery-item fancy-gallery' href='/moviepages/011015-7800/images/l/001.jpg'></a></div>"#;
        assert!(!caribbean_exact_identity(related_only, "011015-780"));
    }

    #[test]
    fn caribbean_identity_rejects_contradictory_canonical_and_near_id() {
        let contradictory = r#"<link rel='canonical' href='https://www.caribbeancom.com/moviepages/011015-781/'><a class='gallery-item fancy-gallery' href='/moviepages/011015-780/images/l/001.jpg'></a>"#;
        assert!(!caribbean_exact_identity(contradictory, "011015-780"));
        let near = r#"<a class='gallery-item fancy-gallery' href='/moviepages/011015-7800/images/l/001.jpg'></a>"#;
        assert!(!caribbean_exact_identity(near, "011015-780"));
        let unicode = "<main>日本語タイトル<div class='movie-gallery'><a class='gallery-item fancy-gallery' href='/moviepages/011015-780/images/l/001.jpg'><img src='/moviepages/011015-780/images/s/001.jpg'></a></div></main>";
        assert!(caribbean_exact_identity(unicode, "011015-780"));
    }

    #[test]
    fn caribbean_gallery_excludes_members_and_related_movies() {
        let html = r#"<a href='/moviepages/011015-780/images/l/001.jpg'><img src='/moviepages/011015-780/images/s/001.jpg'></a>
            <a href='/member/moviepages/011015-780/images/l/006.jpg'>Member</a>
            <a href='/moviepages/other/images/l/001.jpg'>Related</a>"#;
        assert_eq!(caribbean_sample_images(html, "011015-780"), vec!["https://www.caribbeancom.com/moviepages/011015-780/images/l/001.jpg"]);
        assert!(path_under(r"/library\\movies//one", "/library/movies/"));
        assert!(!path_under("/library/movies-other", "/library/movies"));
    }

    #[test]
    fn javbus_gallery_handles_class_order_quotes_and_relative_links() {
        let html = r#"<a href='//cdn.example.com/a.jpg?x=1&amp;y=2' class='extra sample-box'><img></a>
            <a id="b" class="sample-box extra" href="images/b.jpg"></a>
            <a class="sample-box" href="images/b.jpg"></a>
            <a class="not-sample-box" href="wrong.jpg"></a>"#;
        assert_eq!(javbus_sample_images(html), vec![
            "https://cdn.example.com/a.jpg?x=1&y=2", "https://www.javbus.com/images/b.jpg"
        ]);
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
    fn library_query_filters_verified_rows_and_paginates_stably() {
        let conn = database();
        let mut first = metadata("SSIS-002", None); first.actors = vec!["演员甲".into()]; first.maker = Some("MakerA".into()); first.updated_at = 2;
        let mut second = metadata("SSIS-001", None); second.actors = vec!["演员乙".into()]; second.maker = Some("MakerA".into()); second.updated_at = 1;
        save_movie_metadata(&conn, &first).unwrap(); save_movie_metadata(&conn, &second).unwrap();
        conn.execute("INSERT INTO scrape_failures(number,error,updated_at) VALUES ('BAD-001','FC2 官方页面未找到作品（软404）',9)", []).unwrap();
        let page = query_movie_library(&conn, "", "MakerA", "演员甲", "number", 10, 0).unwrap();
        assert_eq!(page.total, 1); assert_eq!(page.items[0].number, "SSIS-002");
        assert_eq!(page.makers, vec!["MakerA"]); assert!(page.actors.iter().any(|value| value == "演员甲"));
        let performer_id: String = conn.query_row("SELECT id FROM performers WHERE name='演员甲'", [], |r| r.get(0)).unwrap();
        conn.execute("UPDATE performers SET aliases_json='[\"Alias A\"]' WHERE id=?1", params![performer_id]).unwrap();
        assert_eq!(query_movie_library(&conn, "Alias A", "", "", "number", 10, 0).unwrap().total, 1);
        assert_eq!(query_movie_library(&conn, "", "", &performer_id, "number", 10, 0).unwrap().total, 1);
        let all = query_movie_library(&conn, "", "", "", "number", 1, 1).unwrap();
        assert_eq!(all.items[0].number, "SSIS-002");
        assert_eq!(query_movie_library(&conn, "%", "", "", "number", 10, 0).unwrap().total, 0);
        let date = query_movie_library(&conn, "", "", "", "date", 10, 0).unwrap();
        assert_eq!(date.total, 2);
        assert!(date.makers.iter().any(|value| value == "MakerA"));
        assert!(date.actors.iter().any(|value| value == "演员甲") && date.actors.iter().any(|value| value == "演员乙"));
    }

    #[test]
    fn issue_categories_are_filtered_and_counts_exclude_successes() {
        let conn = database();
        for (number, error) in [("FC2-1", "FC2 官方页面未找到作品（软404）"), ("SSIS-1", "JavBus 请求超时"), ("SSIS-2", "JavDB 详情番号不匹配")] {
            conn.execute("INSERT INTO scrape_failures(number,error,updated_at) VALUES (?1,?2,1)", params![number, error]).unwrap();
        }
        conn.execute("INSERT INTO scrape_failures(number,error,updated_at) VALUES ('IPX-00015','JavBus 未找到番号',1)", []).unwrap();
        let page = list_scrape_issues(&conn, "network", "", 10, 0).unwrap();
        assert_eq!(page.total, 1); assert_eq!(page.items[0].category, "network"); assert!(page.items[0].retryable);
        let padding = list_scrape_issues(&conn, "padding_candidate", "", 10, 0).unwrap();
        assert_eq!(padding.total, 1); assert!(!padding.items[0].retryable);
        let all = list_scrape_issues(&conn, "all", "", 10, 0).unwrap();
        assert_eq!(all.total, 4); assert!(all.counts.iter().any(|c| c.category == "fc2_missing" && c.count == 1));
    }

    #[test]
    fn unrecognized_videos_honor_whitelist_and_current_video_boundary() {
        let mut conn = database();
        set_scrape_whitelist(&mut conn, vec!["/media".into()]).unwrap();
        for (id, name, path, missing, media) in [("a", "holiday.mp4", "/media/holiday.mp4", 0, "video"), ("b", "SSIS-123.mp4", "/media/ok.mp4", 0, "video"), ("c", "unknown.mp4", "/outside/unknown.mp4", 0, "video"), ("d", "gone.mp4", "/media/gone.mp4", 1, "video"), ("e", "doc.bin", "/media/doc.bin", 0, "image")] {
            conn.execute("INSERT INTO assets VALUES (?1,?2,?3,?4,?5)", params![id,name,path,missing,media]).unwrap();
        }
        let page = list_unrecognized_videos(&conn, 10, 0).unwrap();
        assert_eq!(page.total, 1); assert_eq!(page.items[0].id, "a");
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
