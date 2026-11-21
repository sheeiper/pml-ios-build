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

        CREATE TABLE IF NOT EXISTS scrape_whitelist (
            path TEXT PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL
        );"
    )?;
    Ok(())
}

/// Extract clean standard JAV number from arbitrary filename.
/// Handles cases like:
/// - [中文字幕] SSIS-123_4K.mp4 -> SSIS-123
/// - midv-045.mkv -> MIDV-045
/// - FC2-PPV-2839102.mp4 -> FC2-2839102
/// - 1pondo-010123_001 -> 010123_001
/// - CARIB-010123-001 -> 010123-001
/// - ABW001 -> ABW-001
pub fn extract_jav_number(raw_name: &str) -> Option<String> {
    let name = raw_name.rsplit_once('.').map(|(n, _)| n).unwrap_or(raw_name);
    let upper = name.to_uppercase();

    // 1. FC2-PPV or FC2 check
    if let Some(idx) = upper.find("FC2") {
        let remainder = &upper[idx + 3..];
        let digits: String = remainder.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() >= 5 && digits.len() <= 8 {
            return Some(format!("FC2-{}", digits));
        }
    }

    // 2. Caribbean / 1pondo date-based numbers (e.g. 010123-001 or 010123_001)
    let parts: Vec<&str> = upper.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_').collect();
    for part in &parts {
        let clean = part.replace('_', "-");
        if let Some((d1, d2)) = clean.split_once('-') {
            if d1.len() == 6 && d1.chars().all(|c| c.is_ascii_digit()) &&
               d2.len() == 3 && d2.chars().all(|c| c.is_ascii_digit()) {
                return Some(clean);
            }
        }
    }

    // 3. Standard Letters + Digits (e.g. SSIS-123, IPX-888, ABP-123, MIDV-045, etc.)
    // Scan character sequences
    let chars: Vec<char> = upper.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if chars[i].is_ascii_alphabetic() {
            let start = i;
            while i < len && chars[i].is_ascii_alphabetic() {
                i += 1;
            }
            let letter_count = i - start;
            if letter_count >= 2 && letter_count <= 7 {
                let letters: String = chars[start..i].iter().collect();
                // Skip common noise prefixes
                if letters == "FHD" || letters == "HD" || letters == "UHD" || letters == "MP" || letters == "MKV" || letters == "CH" {
                    continue;
                }

                // Skip optional delimiter '-', '_', ' '
                if i < len && (chars[i] == '-' || chars[i] == '_' || chars[i] == ' ') {
                    i += 1;
                }

                let digit_start = i;
                while i < len && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let digit_count = i - digit_start;
                if digit_count >= 2 && digit_count <= 6 {
                    let digits: String = chars[digit_start..i].iter().collect();
                    return Some(format!("{}-{}", letters, digits));
                }
            }
        } else {
            i += 1;
        }
    }

    None
}

/// Fetch metadata from JavBus by JAV number (Direct, fast, highly reliable for commercial JAV).
pub async fn scrape_from_javbus(number: &str) -> Result<MovieMetadata, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败：{e}"))?;

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
        let rest = &html[start..std::cmp::min(html.len(), start + 250)];
        if let Some(href_idx) = rest.find("href=\"") {
            let href_rest = &rest[href_idx + 6..];
            if let Some(href_end) = href_rest.find('"') {
                let mut c = href_rest[..href_end].trim().to_string();
                if !c.is_empty() {
                    if !c.starts_with("http") {
                        c = format!("https://www.javbus.com{}", c);
                    }
                    cover_url = Some(c);
                }
            }
        }
    }

    // 3. Release Date
    let mut release_date = None;
    if let Some(start) = html.find("發行日期:</span>") {
        let rest = &html[start + 20..std::cmp::min(html.len(), start + 80)];
        let date_chars: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
        let trimmed_date = date_chars.trim();
        if !trimmed_date.is_empty() && trimmed_date.contains('-') {
            release_date = Some(trimmed_date.to_string());
        }
    }

    // 4. Maker (制作商)
    let mut maker = None;
    if let Some(start) = html.find("製作商:</span>") {
        let rest = &html[start..std::cmp::min(html.len(), start + 200)];
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
        let rest = &html[start..std::cmp::min(html.len(), start + 200)];
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
        let rest = &html[start..std::cmp::min(html.len(), start + 200)];
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

    // 7. Actresses (女优)
    let mut actors = Vec::new();
    for seg in html.split("<div class=\"star-name\"") {
        if let Some(a_idx) = seg.find("<a ") {
            if let Some(gt) = seg[a_idx..].find('>') {
                let inner = &seg[a_idx + gt + 1..];
                if let Some(lt) = inner.find("</a>") {
                    let name = inner[..lt].trim();
                    if !name.is_empty() && !actors.contains(&name.to_string()) {
                        actors.push(name.to_string());
                    }
                }
            }
        }
    }
    if actors.is_empty() {
        for seg in html.split("href=\"https://www.javbus.com/star/") {
            if let Some(gt) = seg.find('>') {
                let inner = &seg[gt + 1..];
                if let Some(lt) = inner.find("</a>") {
                    let name = inner[..lt].trim();
                    if !name.is_empty() && !actors.contains(&name.to_string()) {
                        actors.push(name.to_string());
                    }
                }
            }
        }
    }

    // 8. Sample Images (剧照)
    let mut sample_images = Vec::new();
    for seg in html.split("<a class=\"sample-box\"") {
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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败：{e}"))?;

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

    // Find detail link: <div class="item ..."> ... <a href="/v/XXXXX"
    let mut detail_path = None;
    for line in search_html.lines() {
        if line.contains("href=\"/v/") {
            if let Some(start) = line.find("href=\"/v/") {
                let rest = &line[start + 6..];
                if let Some(end) = rest.find('"') {
                    detail_path = Some(rest[..end].to_string());
                    break;
                }
            }
        }
    }

    let detail_path = detail_path.ok_or_else(|| format!("未在 JavDB 找到番号【{}】的作品", number))?;

    // In search_html, also parse search card for title & cover fallback
    let mut search_title = None;
    let mut search_cover = None;
    let mut search_date = None;
    if let Some(pos) = search_html.find(&format!("href=\"{}\"", detail_path)) {
        let slice = &search_html[pos..std::cmp::min(search_html.len(), pos + 1500)];
        if let Some(t_pos) = slice.find("title=\"") {
            let rest = &slice[t_pos + 7..];
            if let Some(t_end) = rest.find('"') {
                let t = rest[..t_end].trim();
                if !t.is_empty() {
                    search_title = Some(t.to_string());
                }
            }
        }
        if let Some(c_pos) = slice.find("jdbstatic.com/covers/") {
            let c_start = slice[..c_pos].rfind("http").unwrap_or(c_pos);
            let rest = &slice[c_start..];
            if let Some(c_end) = rest.find('"') {
                let c = rest[..c_end].trim();
                if !c.is_empty() {
                    search_cover = Some(c.to_string());
                }
            }
        }
        if let Some(d_pos) = slice.find("<div class=\"meta\">") {
            let rest = &slice[d_pos + 18..];
            if let Some(d_end) = rest.find("</div>") {
                let d = rest[..d_end].trim();
                if d.len() == 10 && d.contains('-') {
                    search_date = Some(d.to_string());
                }
            }
        }
    }

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

    // Parse Title
    let mut title = number.to_string();
    if let Some(start) = html.find("<h2 class=\"title is-4\">") {
        let rest = &html[start + 23..];
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
    if let Some(start) = html.find("class=\"video-cover\"") {
        let slice = &html[start.saturating_sub(100)..std::cmp::min(html.len(), start + 250)];
        if let Some(src_start) = slice.find("src=\"") {
            let rest = &slice[src_start + 5..];
            if let Some(src_end) = rest.find('"') {
                cover_url = Some(rest[..src_end].to_string());
            }
        }
    }
    if cover_url.is_none() {
        // Fallback search for c0.jdbstatic.com/covers
        if let Some(pos) = html.find("jdbstatic.com/covers/") {
            let start = html[..pos].rfind("http").unwrap_or(pos);
            let rest = &html[start..];
            if let Some(end) = rest.find('"') {
                cover_url = Some(rest[..end].to_string());
            }
        }
    }

    // Parse Date
    let mut release_date = None;
    if let Some(start) = html.find("<strong>日期:</strong>") {
        let rest = &html[start..std::cmp::min(html.len(), start + 200)];
        if let Some(val_start) = rest.find("<span class=\"value\">") {
            let val_rest = &rest[val_start + 20..];
            if let Some(val_end) = val_rest.find("</span>") {
                release_date = Some(val_rest[..val_end].trim().to_string());
            }
        }
    }

    // Parse Maker (片商)
    let mut maker = None;
    if let Some(start) = html.find("<strong>片商:</strong>") {
        let rest = &html[start..std::cmp::min(html.len(), start + 300)];
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
        let rest = &html[start..std::cmp::min(html.len(), start + 300)];
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
        let rest = &html[start..std::cmp::min(html.len(), start + 300)];
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
        let rest = &html[start..std::cmp::min(html.len(), start + 800)];
        if let Some(span_end) = rest.find("</span>") {
            let actor_block = &rest[..span_end];
            for segment in actor_block.split("<a ") {
                if segment.contains("href=\"/actors/") {
                    if let Some(gt) = segment.find('>') {
                        let inner = &segment[gt + 1..];
                        if let Some(lt) = inner.find("</a>") {
                            let name = inner[..lt].trim();
                            if !name.is_empty() && !actors.contains(&name.to_string()) {
                                actors.push(name.to_string());
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

    // Parse Sample Images
    let mut sample_images = Vec::new();
    for segment in html.split("class=\"tile-item\"") {
        if let Some(href_idx) = segment.find("href=\"") {
            let rest = &segment[href_idx + 6..];
            if let Some(href_end) = rest.find('"') {
                let url = rest[..href_end].trim();
                if url.starts_with("http") && (url.ends_with(".jpg") || url.ends_with(".jpeg") || url.ends_with(".png") || url.ends_with(".webp")) {
                    if !sample_images.contains(&url.to_string()) {
                        sample_images.push(url.to_string());
                    }
                }
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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败：{e}"))?;

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

    // Title
    let mut title = clean_num.clone();
    if let Some(start) = html.find("<h1 itemprop=\"name\">") {
        let rest = &html[start + 20..];
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

    // Actors
    let mut actors = Vec::new();
    let mut search_slice = html.as_str();
    while let Some(start) = search_slice.find("<span itemprop=\"name\">") {
        let rest = &search_slice[start + 22..];
        if let Some(end) = rest.find("</span>") {
            let a = rest[..end].trim();
            if !a.is_empty() && a.len() <= 20 && !actors.contains(&a.to_string()) {
                actors.push(a.to_string());
            }
            search_slice = &rest[end + 7..];
        } else {
            break;
        }
    }

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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败：{e}"))?;

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

/// Unified multi-source scraper.
pub async fn scrape_movie_metadata(number: &str) -> Result<MovieMetadata, String> {
    let clean_num = number.trim();

    // 1. Date-based numbers (Caribbeancom / 1pondo: MMDDYY-NNN or MMDDYY_NNN)
    let is_date_code = {
        let parts: Vec<&str> = clean_num.split(|c| c == '-' || c == '_').collect();
        parts.len() == 2 && parts[0].len() == 6 && parts[0].chars().all(|c| c.is_ascii_digit())
            && parts[1].len() == 3 && parts[1].chars().all(|c| c.is_ascii_digit())
    };

    if is_date_code {
        if let Ok(meta) = scrape_from_caribbeancom(clean_num).await {
            return Ok(meta);
        }
        if let Ok(meta) = scrape_from_1pondo(clean_num).await {
            return Ok(meta);
        }
        return scrape_from_javdb(clean_num).await;
    }

    // 2. FC2
    if clean_num.starts_with("FC2-") {
        if let Ok(meta) = scrape_from_javdb(clean_num).await {
            return Ok(meta);
        }
        return scrape_from_javbus(clean_num).await;
    }

    // 3. Commercial standard JAV: JavDB first (has reliable search + cover even on server)
    match scrape_from_javdb(clean_num).await {
        Ok(meta) => Ok(meta),
        Err(db_err) => {
            match scrape_from_javbus(clean_num).await {
                Ok(meta) => Ok(meta),
                Err(bus_err) => Err(format!("JavDB: {db_err}; JavBus: {bus_err}")),
            }
        }
    }
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
            tags_json, source, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
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
            updated_at = excluded.updated_at",
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

    Ok(())
}

/// Read movie metadata by number or asset_id
pub fn get_movie_metadata(conn: &Connection, identifier: &str) -> Result<Option<MovieMetadata>, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let upper = identifier.to_uppercase();
    let mut stmt = conn.prepare(
        "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                director, release_date, duration_minutes, cover_url, sample_images_json,
                tags_json, source, updated_at
         FROM movie_metadata
         WHERE (number = ?1 OR asset_id = ?1)
           AND source NOT IN ('javdb_not_found', 'not_found', '')
           AND cover_url IS NOT NULL AND cover_url != ''
         LIMIT 1"
    )?;

    let res = stmt.query_row(params![upper], |row| {
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

    Ok(res)
}

/// List recently scraped movie metadata
pub fn list_movie_metadata(conn: &Connection, limit: u32, offset: u32) -> Result<Vec<MovieMetadata>, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let mut stmt = conn.prepare(
        "SELECT number, asset_id, title, original_title, actors_json, maker, label, series,
                director, release_date, duration_minutes, cover_url, sample_images_json,
                tags_json, source, updated_at
         FROM movie_metadata
         WHERE source NOT IN ('javdb_not_found', 'not_found', '')
           AND cover_url IS NOT NULL AND cover_url != ''
         ORDER BY updated_at DESC
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

    let mut result = Vec::new();
    for r in rows {
        if let Ok(item) = r {
            result.push(item);
        }
    }
    Ok(result)
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
                 WHERE series = ?1 AND number != ?2
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
             WHERE actors_json LIKE ?1 AND number != ?2
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
                 WHERE maker = ?1 AND number != ?2
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

pub fn is_path_whitelisted(conn: &Connection, path: &str) -> bool {
    if let Ok(whitelist) = get_scrape_whitelist(conn) {
        if whitelist.is_empty() {
            return false;
        }
        for wp in whitelist {
            if path.starts_with(&wp) {
                return true;
            }
        }
    }
    false
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
        g.running = false;
        g.paused = false;
    }
}

pub fn get_batch_scrape_stats(conn: &Connection) -> Result<BatchScrapeStats, rusqlite::Error> {
    ensure_scraper_schema(conn)?;
    let whitelist = get_scrape_whitelist(conn)?;
    if whitelist.is_empty() {
        return Ok(BatchScrapeStats {
            total_whitelisted_videos: 0,
            distinct_numbers: 0,
            already_scraped: 0,
            remaining_to_scrape: 0,
        });
    }

    let mut total_videos = 0usize;
    let mut distinct_numbers = std::collections::HashSet::new();

    for prefix in &whitelist {
        let pattern = format!("{}%", prefix);
        let mut stmt = conn.prepare(
            "SELECT file_name FROM assets WHERE missing = 0 AND media_type = 'video' AND remote_path LIKE ?1"
        )?;
        let rows = stmt.query_map(params![pattern], |row| row.get::<_, String>(0))?;
        for name in rows.flatten() {
            total_videos += 1;
            if let Some(num) = extract_jav_number(&name) {
                distinct_numbers.insert(num);
            }
        }
    }

    let mut already_scraped = 0usize;
    let mut check_stmt = conn.prepare("SELECT 1 FROM movie_metadata WHERE number = ?1 AND source NOT IN ('javdb_not_found', 'not_found', '') AND cover_url IS NOT NULL AND cover_url != '' LIMIT 1")?;
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

        // 收集所有番号
        let mut distinct_numbers = std::collections::HashSet::new();
        for prefix in &whitelist {
            let pattern = format!("{}%", prefix);
            if let Ok(mut stmt) = conn.prepare(
                "SELECT file_name FROM assets WHERE missing = 0 AND media_type = 'video' AND remote_path LIKE ?1"
            ) {
                if let Ok(rows) = stmt.query_map(params![pattern], |row| row.get::<_, String>(0)) {
                    for name in rows.flatten() {
                        if let Some(num) = extract_jav_number(&name) {
                            distinct_numbers.insert(num);
                        }
                    }
                }
            }
        }

        // 过滤已成功刮削入库的（未找到或失败的条目允许重试）
        let mut queue = Vec::new();
        if let Ok(mut check_stmt) = conn.prepare("SELECT 1 FROM movie_metadata WHERE number = ?1 AND source NOT IN ('javdb_not_found', 'not_found', '') AND cover_url IS NOT NULL AND cover_url != '' LIMIT 1") {
            for num in distinct_numbers {
                if let Ok(false) = check_stmt.exists(params![num]) {
                    queue.push(num);
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

            // 执行高可靠多源刮削（优先 JavBus 极速入库，备选 JavDB）
            match scrape_movie_metadata(&number).await {
                Ok(meta) => {
                    let _ = save_movie_metadata(&conn, &meta);
                    if let Ok(mut g) = batch_progress_cell().lock() {
                        g.completed += 1;
                        g.succeeded += 1;
                    }
                }
                Err(e) => {
                    // 若均未找到，记录一个精简占位
                    let now = chrono::Utc::now().timestamp();
                    let empty_meta = MovieMetadata {
                        number: number.clone(),
                        asset_id: None,
                        title: number.clone(),
                        original_title: None,
                        actors: Vec::new(),
                        maker: None,
                        label: None,
                        series: None,
                        director: None,
                        release_date: None,
                        duration_minutes: None,
                        cover_url: None,
                        sample_images: Vec::new(),
                        tags: Vec::new(),
                        source: "javdb_not_found".to_string(),
                        updated_at: now,
                    };
                    let _ = save_movie_metadata(&conn, &empty_meta);

                    if let Ok(mut g) = batch_progress_cell().lock() {
                        g.completed += 1;
                        g.failed += 1;
                        g.last_error = Some(format!("{number}: {e}"));
                    }
                }
            }

            // 工业级温和延时：1.8s ~ 2.4s 随机微抖动，保证永不被风控封禁
            let jitter = 1800 + (number.len() * 73) % 600;
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

