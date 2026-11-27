use once_cell_shim::OnceCell;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

mod once_cell_shim {
    pub use std::sync::OnceLock as OnceCell;
}

const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

fn shared_http_client() -> Result<Client, String> {
    static CELL: OnceCell<Client> = OnceCell::new();
    if let Some(c) = CELL.get() {
        return Ok(c.clone());
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(18))
        .user_agent(CHROME_UA)
        .cookie_store(true)
        .build()
        .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))?;
    let _ = CELL.set(client.clone());
    Ok(client)
}

fn current_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

fn prefix_utf8(s: &str, max_bytes: usize) -> &str {
    let mut end = s.len().min(max_bytes);
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn decode_html_entities_basic(input: &str) -> String {
    let mut out = input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    // Numeric decimal entities &#12345;
    let mut result = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(pos) = rest.find("&#") {
        result.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        if let Some(semi) = after.find(';') {
            let digits = &after[..semi];
            let parsed = if let Some(hex) = digits.strip_prefix('x').or_else(|| digits.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                digits.parse::<u32>().ok()
            };
            if semi <= 8 {
                if let Some(ch) = parsed.and_then(char::from_u32) {
                    result.push(ch);
                    rest = &after[semi + 1..];
                    continue;
                }
            }
        }
        result.push_str("&#");
        rest = after;
    }
    result.push_str(rest);
    out = result;
    out
}

pub fn strip_html_tags_clean(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    decode_html_entities_basic(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    for q in ['"', '\''] {
        let needle = format!("{attr}={q}");
        if let Some((_, rest)) = tag.split_once(&needle) {
            if let Some((val, _)) = rest.split_once(q) {
                return Some(decode_html_entities_basic(val));
            }
        }
    }
    None
}

/// Standardize a raw JAV code into up to 3 deduplicated query candidates (avdanyu-viewer toStandardCode parity).
/// Strips maker digit prefixes like `200GANA-1535` -> `GANA-1535`, `h_086fuga00041` -> `FUGA-041`,
/// `118abp00769` -> `ABP-769`, while also keeping `FC2-PPV-...` and normalized primary number.
pub fn standard_jav_query_codes(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push_unique = |cand: String| {
        let clean = cand.trim().to_uppercase();
        if clean.is_empty() {
            return;
        }
        if !out.iter().any(|x| x.eq_ignore_ascii_case(&clean)) {
            out.push(clean);
        }
    };

    if let Some(norm) = crate::jav_scraper::normalize_jav_number(raw) {
        push_unique(norm);
    }

    let trimmed = raw
        .trim()
        .trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
    let upper = trimmed.to_uppercase();

    // 1. FC2 special format
    if let Some(digits) = upper
        .strip_prefix("FC2-PPV-")
        .or_else(|| upper.strip_prefix("FC2_PPV_"))
        .or_else(|| upper.strip_prefix("FC2PPV-"))
        .or_else(|| upper.strip_prefix("FC2PPV"))
        .or_else(|| upper.strip_prefix("FC2-"))
    {
        let num: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
        if num.len() >= 5 {
            push_unique(format!("FC2-PPV-{num}"));
        }
    }

    // 2. Strip 3-4 leading maker digits before hyphenated code (200GANA-1535 -> GANA-1535)
    let digit_prefix_len = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if (3..=4).contains(&digit_prefix_len) {
        let rest = &trimmed[digit_prefix_len..];
        if let Some((l, r)) = rest.split_once('-') {
            if !l.is_empty() && l.chars().all(|c| c.is_ascii_alphabetic()) && !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric()) {
                push_unique(format!("{l}-{r}"));
            }
        }
    }

    // 3. Unhyphenated FANZA/DMM CID forms: h_086fuga00041 -> FUGA-041 / 118abp00769 -> ABP-769
    if !trimmed.contains('-') {
        let mut s = trimmed.to_ascii_lowercase();
        if let Some(rest) = s.strip_prefix("h_") {
            let stripped = rest.trim_start_matches(|c: char| c.is_ascii_digit());
            if !stripped.is_empty() {
                s = stripped.to_string();
            }
        }
        let s2 = s.trim_start_matches(|c: char| c.is_ascii_digit());
        let alpha_end = s2.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(s2.len());
        let prefix = &s2[..alpha_end];
        let digits: String = s2[alpha_end..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if (2..=6).contains(&prefix.len()) && !digits.is_empty() {
            let trimmed_zero = digits.trim_start_matches('0');
            let num = if trimmed_zero.is_empty() {
                "001".to_string()
            } else if trimmed_zero.len() < 3 && digits.len() >= 3 {
                format!("{:0>3}", trimmed_zero)
            } else {
                trimmed_zero.to_string()
            };
            push_unique(format!("{}-{num}", prefix.to_uppercase()));
        }
    } else {
        push_unique(upper);
    }

    out.truncate(3);
    out
}

pub fn extract_btih_from_magnet(magnet: &str) -> String {
    let lower = magnet.to_ascii_lowercase();
    if let Some(pos) = lower.find("urn:btih:") {
        let after = &lower[pos + 9..];
        let end = after.find('&').unwrap_or(after.len());
        return after[..end].trim().to_string();
    }
    lower
}

pub fn is_valid_btih_magnet(uri: &str) -> bool {
    let trimmed = uri.trim();
    if !trimmed.to_ascii_lowercase().starts_with("magnet:?xt=") {
        return false;
    }
    let key = extract_btih_from_magnet(trimmed);
    (key.len() == 40 && key.chars().all(|c| c.is_ascii_hexdigit()))
        || (key.len() == 32 && key.chars().all(|c| matches!(c, 'a'..='z' | '2'..='7')))
}

// ==================== JavBus AJAX + BTDig Magnet Scrapers ====================

pub async fn fetch_javbus_magnets(code: &str) -> Result<Vec<crate::performers::MovieMagnetItem>, String> {
    let clean = code.trim();
    if clean.is_empty() {
        return Ok(Vec::new());
    }
    let client = shared_http_client()?;
    let encoded = percent_encoding::utf8_percent_encode(clean, percent_encoding::NON_ALPHANUMERIC).to_string();
    let page_url = format!("https://www.javbus.com/{encoded}");
    let resp = client
        .get(&page_url)
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8,ja;q=0.7")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .header("Sec-Fetch-User", "?1")
        .header("Cookie", "existmag=all; age=verified; dv=1")
        .send()
        .await
        .map_err(|e| format!("请求 JavBus 失败：{e}"))?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let html = resp.text().await.unwrap_or_default();
    if html.contains("Age Verification") && !html.contains("var gid =") {
        return Err("JavBus 返回年龄验证页".to_string());
    }
    let gid = extract_js_var_num(&html, "gid");
    let Some(gid) = gid else {
        return Ok(Vec::new());
    };
    let uc = extract_js_var_num(&html, "uc").unwrap_or_else(|| "0".to_string());
    let img = extract_js_var_str(&html, "img").unwrap_or_default();
    let lang = extract_js_var_str(&html, "lang").unwrap_or_else(|| "zh".to_string());
    let floor = (current_ts() * 10).rem_euclid(1000);

    let mut ajax_url = reqwest::Url::parse("https://www.javbus.com/ajax/uncledatoolsbyajax.php")
        .map_err(|e| e.to_string())?;
    ajax_url
        .query_pairs_mut()
        .append_pair("gid", &gid)
        .append_pair("lang", &lang)
        .append_pair("img", &img)
        .append_pair("uc", &uc)
        .append_pair("floor", &floor.to_string());

    let frag_resp = client
        .get(ajax_url)
        .header("Referer", &page_url)
        .header("X-Requested-With", "XMLHttpRequest")
        .header("Accept", "*/*")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
        .header("Cookie", "existmag=all; age=verified; dv=1")
        .send()
        .await
        .map_err(|e| format!("请求 JavBus 磁力列表失败：{e}"))?;
    if !frag_resp.status().is_success() {
        return Ok(Vec::new());
    }
    let frag = frag_resp.text().await.unwrap_or_default();
    Ok(parse_javbus_ajax_magnets(&frag))
}

fn extract_js_var_num(html: &str, name: &str) -> Option<String> {
    let needle = format!("var {name} = ");
    let (_, rest) = html.split_once(&needle)?;
    let end = rest.find(';')?;
    let val: String = rest[..end].trim().chars().filter(|c| c.is_ascii_digit()).collect();
    if val.is_empty() { None } else { Some(val) }
}

fn extract_js_var_str(html: &str, name: &str) -> Option<String> {
    for q in ['\'', '"'] {
        let needle = format!("var {name} = {q}");
        if let Some((_, rest)) = html.split_once(&needle) {
            if let Some((val, _)) = rest.split_once(q) {
                return Some(val.trim().to_string());
            }
        }
    }
    None
}

pub fn parse_javbus_ajax_magnets(frag: &str) -> Vec<crate::performers::MovieMagnetItem> {
    let mut items = Vec::new();
    for seg in frag.split("<tr").skip(1) {
        let row = seg.split("</tr>").next().unwrap_or(seg);
        let Some(mag_pos) = row.find("href=\"magnet:?xt=") else { continue; };
        let after = &row[mag_pos + 6..];
        let Some(mag_end) = after.find('"') else { continue; };
        let magnet = decode_html_entities_basic(&after[..mag_end]);
        if !is_valid_btih_magnet(&magnet) {
            continue;
        }
        let mut tags = Vec::new();
        if row.contains("包含字幕") || row.contains("btn-warning") {
            tags.push("字幕".to_string());
        }
        if row.contains("包含高清") || row.contains("btn-primary") {
            tags.push("高清".to_string());
        }
        // Strip badge `<a class="btn...</a>` elements before reading text links
        let mut cleaned = String::with_capacity(row.len());
        let mut rest = row;
        while let Some(pos) = rest.find("<a class=\"btn") {
            cleaned.push_str(&rest[..pos]);
            let after_btn = &rest[pos..];
            if let Some(end_a) = after_btn.find("</a>") {
                rest = &after_btn[end_a + 4..];
            } else {
                rest = after_btn;
                break;
            }
        }
        cleaned.push_str(rest);

        let mut texts: Vec<String> = Vec::new();
        for a_seg in cleaned.split("<a ").skip(1) {
            let Some((_, inside)) = a_seg.split_once('>') else { continue; };
            let text_raw = inside.split("</a>").next().unwrap_or("");
            let t = strip_html_tags_clean(text_raw);
            if !t.is_empty() {
                texts.push(t);
            }
        }
        let title = texts
            .first()
            .cloned()
            .filter(|s| !s.is_empty())
            .or_else(|| extract_dn_from_magnet(&magnet))
            .unwrap_or_else(|| "JavBus 磁力".to_string());
        let size = texts.get(1).cloned().filter(|s| !s.is_empty());
        let date = texts.get(2).cloned().filter(|s| !s.is_empty());

        items.push(crate::performers::MovieMagnetItem {
            title,
            magnet,
            size,
            date,
            tags,
            seeders: None,
            source: "JavBus".to_string(),
        });
        if items.len() >= 30 {
            break;
        }
    }
    items
}

fn extract_dn_from_magnet(magnet: &str) -> Option<String> {
    let (_, q) = magnet.split_once('?')?;
    for pair in q.split('&') {
        if let Some(val) = pair.strip_prefix("dn=") {
            let decoded = percent_encoding::percent_decode_str(&val.replace('+', " "))
                .decode_utf8_lossy()
                .trim()
                .to_string();
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    None
}

pub async fn fetch_btdig_magnets(code: &str) -> Result<Vec<crate::performers::MovieMagnetItem>, String> {
    let clean = code.trim();
    if clean.is_empty() {
        return Ok(Vec::new());
    }
    let client = shared_http_client()?;
    let encoded = percent_encoding::utf8_percent_encode(clean, percent_encoding::NON_ALPHANUMERIC).to_string();
    let mut last_err = String::new();
    for host in ["btdig.com", "btdig.co"] {
        let url = format!("https://{host}/search?q={encoded}");
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let html = resp.text().await.unwrap_or_default();
                let parsed = parse_btdig_magnets(&html);
                if !parsed.is_empty() {
                    return Ok(parsed);
                }
            }
            Ok(resp) => {
                last_err = format!("{host}: HTTP {}", resp.status().as_u16());
            }
            Err(e) => {
                last_err = format!("{host}: {e}");
            }
        }
    }
    if last_err.is_empty() {
        Ok(Vec::new())
    } else {
        Err(last_err)
    }
}

pub fn parse_btdig_magnets(html: &str) -> Vec<crate::performers::MovieMagnetItem> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for seg in html.split("href=\"magnet:?xt=").skip(1) {
        let Some((raw_mag, after_quote)) = seg.split_once('"') else { continue; };
        let magnet = format!("magnet:?xt={}", decode_html_entities_basic(raw_mag));
        if !is_valid_btih_magnet(&magnet) {
            continue;
        }
        let hash = extract_btih_from_magnet(&magnet);
        if !seen.insert(hash) {
            continue;
        }
        let anchor_title = after_quote
            .split_once('>')
            .and_then(|(_, rest)| rest.split_once("</a>"))
            .map(|(inner, _)| strip_html_tags_clean(inner))
            .unwrap_or_default();
        let title = if !anchor_title.is_empty() && !anchor_title.to_ascii_lowercase().starts_with("magnet") {
            anchor_title
        } else {
            extract_dn_from_magnet(&magnet).unwrap_or_else(|| "BTDig 磁力".to_string())
        };
        let has_cn = title.contains("中文") || title.contains("中字");
        let mut tags = Vec::new();
        if has_cn {
            tags.push("中字".to_string());
        }
        items.push(crate::performers::MovieMagnetItem {
            title,
            magnet,
            size: None,
            date: None,
            tags,
            seeders: None,
            source: "BTDig".to_string(),
        });
        if items.len() >= 25 {
            break;
        }
    }
    items
}

// ==================== FANZA Trailer (Direct Multi-Bitrate MP4) ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrailerQualityItem {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieTrailerResult {
    pub number: String,
    pub cid: String,
    pub cached: bool,
    pub items: Vec<TrailerQualityItem>,
    pub fallback_url: String,
}

static TRAILER_CACHE: OnceLock<Mutex<HashMap<String, (i64, MovieTrailerResult)>>> = OnceLock::new();
const TRAILER_CACHE_TTL_SECS: i64 = 3600;

fn extract_cid_from_cover(cover_url: &str) -> Option<String> {
    let lower = cover_url.to_ascii_lowercase();
    if !lower.contains("dmm.co.jp") {
        return None;
    }
    for marker in ["/video/", "/digital/video/", "/mono/movie/adult/"] {
        if let Some((_, rest)) = lower.split_once(marker) {
            if let Some((cid_seg, _)) = rest.split_once('/') {
                let clean: String = cid_seg.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if (4..=36).contains(&clean.len()) {
                    return Some(clean);
                }
            }
        }
    }
    None
}

fn candidate_fanza_cids(number: &str, cover_url: Option<&str>) -> Vec<String> {
    let mut cids = Vec::new();
    let mut push = |c: String| {
        let clean = c.trim().to_ascii_lowercase();
        if clean.len() >= 4 && !cids.contains(&clean) {
            cids.push(clean);
        }
    };
    if let Some(cov) = cover_url {
        if let Some(cid) = extract_cid_from_cover(cov) {
            push(cid);
        }
    }
    let norm = crate::jav_scraper::normalize_jav_number(number).unwrap_or_else(|| number.trim().to_uppercase());
    if let Some((prefix, num)) = norm.split_once('-') {
        let p = prefix.to_ascii_lowercase();
        let d: String = num.chars().filter(|c| c.is_ascii_digit()).collect();
        if !p.is_empty() && !d.is_empty() {
            if let Ok(n) = d.parse::<u32>() {
                push(format!("{p}{n:05}"));
                push(format!("{p}{n:03}"));
                push(format!("1{p}{n:05}"));
            }
            push(format!("{p}{d}"));
        }
    } else {
        let raw: String = number.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        push(raw);
    }
    cids.truncate(4);
    cids
}

pub async fn fetch_fanza_trailer(
    number: &str,
    cover_url: Option<&str>,
    force_refresh: bool,
) -> Result<MovieTrailerResult, String> {
    let clean_num = crate::jav_scraper::normalize_jav_number(number)
        .unwrap_or_else(|| number.trim().to_uppercase());
    if clean_num.is_empty() {
        return Err("缺少番号参数".to_string());
    }
    let now = current_ts();
    let cache = TRAILER_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if !force_refresh {
        if let Ok(guard) = cache.lock() {
            if let Some((ts, cached)) = guard.get(&clean_num) {
                if now - *ts < TRAILER_CACHE_TTL_SECS {
                    let mut out = cached.clone();
                    out.cached = true;
                    return Ok(out);
                }
            }
        }
    }

    let client = shared_http_client()?;
    let cids = candidate_fanza_cids(&clean_num, cover_url);
    let fallback_url = format!(
        "https://javtrailers.com/search/{}",
        percent_encoding::utf8_percent_encode(&clean_num, percent_encoding::NON_ALPHANUMERIC)
    );
    let mut resolved_cid = cids.first().cloned().unwrap_or_default();
    let mut found_items = Vec::new();
    let mut last_err = String::new();

    for cid in &cids {
        let api_url = format!(
            "https://www.dmm.co.jp/service/digitalapi/-/html5_player/=/cid={cid}/mtype=AhRVShI_/service=litevideo/mode=/width=560/height=360/"
        );
        match client
            .get(&api_url)
            .header("Cookie", "age_check_done=1")
            .header("Referer", "https://www.dmm.co.jp/")
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let page = resp.text().await.unwrap_or_default();
                let parsed = parse_fanza_bitrates(&page);
                if !parsed.is_empty() {
                    resolved_cid = cid.clone();
                    found_items = parsed;
                    break;
                }
            }
            Ok(resp) => {
                last_err = format!("FANZA HTTP {}", resp.status().as_u16());
            }
            Err(e) => {
                last_err = format!("请求 FANZA 失败：{e}");
            }
        }
    }

    if found_items.is_empty() && !last_err.is_empty() {
        // Still return structured result so client can offer fallback link instead of opaque error
        return Ok(MovieTrailerResult {
            number: clean_num,
            cid: resolved_cid,
            cached: false,
            items: Vec::new(),
            fallback_url,
        });
    }

    let result = MovieTrailerResult {
        number: clean_num.clone(),
        cid: resolved_cid,
        cached: false,
        items: found_items,
        fallback_url,
    };
    if let Ok(mut guard) = cache.lock() {
        guard.insert(clean_num, (now, result.clone()));
    }
    Ok(result)
}

pub fn parse_fanza_bitrates(page: &str) -> Vec<TrailerQualityItem> {
    let mut items = Vec::new();
    let Some((_, after_key)) = page.split_once("\"bitrates\":") else {
        return items;
    };
    let trimmed = after_key.trim_start();
    if !trimmed.starts_with('[') {
        return items;
    }
    let Some(end_bracket) = trimmed.find(']') else {
        return items;
    };
    let slice = &trimmed[..=end_bracket];
    if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(slice) {
        for obj in arr {
            let label = obj.get("bitrate").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let mut src = obj.get("src").and_then(|v| v.as_str()).unwrap_or("").trim().replace("\\/", "/");
            if src.starts_with("//") {
                src = format!("https:{src}");
            }
            if !label.is_empty() && src.starts_with("https://") {
                items.push(TrailerQualityItem { label, url: src });
            }
        }
    }
    items
}

// ==================== Online HLS Preview (javday + 123av Fallback Proxy) ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineStreamSource {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OnlineStreamResult {
    pub code: String,
    pub via: String,
    pub page_url: String,
    pub search_url: String,
    pub poster: Option<String>,
    pub title: Option<String>,
    pub proxied: bool,
    pub cached: bool,
    pub sources: Vec<OnlineStreamSource>,
}

static ONLINE_STREAM_CACHE: OnceLock<Mutex<HashMap<String, (i64, OnlineStreamResult)>>> = OnceLock::new();
static ONLINE_PROXY_REGISTRY: OnceLock<Mutex<HashMap<String, (i64, String)>>> = OnceLock::new();
const ONLINE_CACHE_TTL_SECS: i64 = 1800;
const ONLINE_PROXY_TTL_SECS: i64 = 7200;

fn register_online_proxy_url(url: &str, referer: &str) {
    if !url.starts_with("https://") {
        return;
    }
    let now = current_ts();
    let reg = ONLINE_PROXY_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut guard) = reg.lock() {
        if guard.len() > 4096 {
            guard.retain(|_, (ts, _)| now - *ts < ONLINE_PROXY_TTL_SECS);
        }
        guard.insert(url.to_string(), (now, referer.to_string()));
    }
}

fn lookup_online_proxy_referer(url: &str) -> Option<String> {
    let now = current_ts();
    let reg = ONLINE_PROXY_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let guard = reg.lock().ok()?;
    let (ts, referer) = guard.get(url)?;
    if now - *ts < ONLINE_PROXY_TTL_SECS {
        Some(referer.clone())
    } else {
        None
    }
}

fn is_javday_m3u8_url(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else { return false; };
    if parsed.scheme() != "https" {
        return false;
    }
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    (host == "javday.homes" || host.ends_with(".javday.homes"))
        && parsed.path().to_ascii_lowercase().ends_with(".m3u8")
}

fn rewrite_javday_host(raw: &str) -> String {
    if let Ok(mut parsed) = reqwest::Url::parse(raw) {
        if let Some(host) = parsed.host_str().map(str::to_ascii_lowercase) {
            if host.ends_with(".javday.homes") {
                let _ = parsed.set_host(Some("javday.homes"));
                return parsed.to_string();
            }
        }
    }
    raw.to_string()
}

fn js_unescape_basic(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let next = chars[i + 1];
        if next == 'u' && i + 5 < chars.len() {
            let hex: String = chars[i + 2..i + 6].iter().collect();
            if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                if let Some(ch) = char::from_u32(cp) {
                    out.push(ch);
                    i += 6;
                    continue;
                }
            }
        }
        let mapped = match next {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '\\' => '\\',
            '\'' => '\'',
            '"' => '"',
            '/' => '/',
            other => other,
        };
        out.push(mapped);
        i += 2;
    }
    out
}

pub async fn fetch_online_movie_stream(number: &str, force_refresh: bool) -> Result<OnlineStreamResult, String> {
    let candidates = standard_jav_query_codes(number);
    let primary = candidates.first().cloned().unwrap_or_else(|| number.trim().to_uppercase());
    if primary.is_empty() {
        return Err("缺少番号参数".to_string());
    }
    let cache = ONLINE_STREAM_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let now = current_ts();
    if !force_refresh {
        if let Ok(guard) = cache.lock() {
            if let Some((ts, cached)) = guard.get(&primary) {
                if now - *ts < ONLINE_CACHE_TTL_SECS {
                    let mut out = cached.clone();
                    out.cached = true;
                    return Ok(out);
                }
            }
        }
    }

    let client = shared_http_client()?;
    for cand in &candidates {
        let code_lower = cand.to_ascii_lowercase();
        if let Some(res) = resolve_javday_stream(&client, &code_lower).await {
            if let Ok(mut guard) = cache.lock() {
                guard.insert(primary, (now, res.clone()));
            }
            return Ok(res);
        }
        if let Some(res) = resolve_123av_fallback(&client, &code_lower).await {
            if let Ok(mut guard) = cache.lock() {
                guard.insert(primary, (now, res.clone()));
            }
            return Ok(res);
        }
    }
    Err("暂未从在线播放源解析到可用视频流".to_string())
}

async fn resolve_javday_stream(client: &Client, code: &str) -> Option<OnlineStreamResult> {
    let encoded = percent_encoding::utf8_percent_encode(code, percent_encoding::NON_ALPHANUMERIC).to_string();
    let search_url = format!("https://javday.app/search/wd/{encoded}/");
    let resp = client
        .get(&search_url)
        .header("Referer", "https://javday.app/")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let search_html = resp.text().await.ok()?;
    let mut video_path: Option<String> = None;
    let mut video_title: Option<String> = None;
    for seg in search_html.split("href=\"/videos/").skip(1) {
        let Some((rest_path, after_quote)) = seg.split_once('"') else { continue; };
        let header = after_quote.split('>').next().unwrap_or("");
        if !seg.contains("videoBox") && !header.contains("videoBox") {
            // Check surrounding tag for videoBox just in case attribute order varies
            let chunk = prefix_utf8(after_quote, 600);
            if !chunk.contains("class=\"title\"") {
                continue;
            }
        }
        let path_clean = format!("/videos/{rest_path}");
        let t = after_quote
            .split_once("<span class=\"title\">")
            .and_then(|(_, r)| r.split_once('<'))
            .map(|(v, _)| strip_html_tags_clean(v));
        video_path = Some(path_clean);
        video_title = t.filter(|s| !s.is_empty());
        break;
    }
    let link = video_path?;
    let page_url = format!("https://javday.app{link}");
    let page_resp = client
        .get(&page_url)
        .header("Referer", "https://javday.app/")
        .send()
        .await
        .ok()?;
    if !page_resp.status().is_success() {
        return None;
    }
    let page = page_resp.text().await.ok()?;

    let mut m3u8s = Vec::new();
    for seg in page.split("https://").skip(1) {
        let end = seg
            .find(|c: char| matches!(c, '"' | '\'' | '\\' | '<' | '>' | ' ' | '\r' | '\n'))
            .unwrap_or(seg.len());
        let cand = format!("https://{}", &seg[..end]);
        if is_javday_m3u8_url(&cand) && !m3u8s.contains(&cand) {
            m3u8s.push(cand);
        }
    }
    if m3u8s.is_empty() {
        return None;
    }
    let poster = page
        .split("https://img.javday.app/upload/")
        .nth(1)
        .and_then(|rest| {
            let end = rest.find(|c: char| matches!(c, '"' | '\'' | '\\' | ')' | ' ')).unwrap_or(rest.len());
            Some(format!("https://img.javday.app/upload/{}", &rest[..end]))
        });
    let sources = m3u8s
        .into_iter()
        .enumerate()
        .map(|(idx, url)| OnlineStreamSource {
            name: format!("线路{}", idx + 1),
            url,
        })
        .collect();

    Some(OnlineStreamResult {
        code: code.to_uppercase(),
        via: "javday".to_string(),
        page_url,
        search_url,
        poster,
        title: video_title,
        proxied: false,
        cached: false,
        sources,
    })
}

async fn resolve_123av_fallback(client: &Client, code: &str) -> Option<OnlineStreamResult> {
    let encoded = percent_encoding::utf8_percent_encode(code, percent_encoding::NON_ALPHANUMERIC).to_string();
    let page_url = format!("https://123av.com/en/v/{encoded}");
    let search_url = format!("https://123av.com/en/search?keyword={encoded}");
    let resp = client
        .get(&page_url)
        .header("Referer", "https://123av.com/")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let page = resp.text().await.ok()?;
    let (_, after) = page.split_once("JSON.parse('[")?;
    let (raw_json_inner, _) = after.split_once("]')")?;
    let json_text = format!("[{}]", js_unescape_basic(raw_json_inner));
    let list: Vec<serde_json::Value> = serde_json::from_str(&json_text).ok()?;
    let mut episodes = Vec::new();
    for it in list {
        let mut u = it.get("url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if u.starts_with("//") {
            u = format!("https:{u}");
        }
        if u.starts_with("https://") {
            episodes.push(u);
        }
    }
    if episodes.is_empty() {
        return None;
    }

    let mut sources = Vec::new();
    let total_eps = episodes.len();
    for (idx, ep) in episodes.into_iter().enumerate() {
        let Ok(parsed) = reqwest::Url::parse(&ep) else { continue; };
        let Some(host) = parsed.host_str() else { continue; };
        let origin = format!("https://{host}");
        let Some((_, hid_rest)) = parsed.path().split_once("/e/") else { continue; };
        let hid: String = hid_rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        if hid.is_empty() {
            continue;
        }
        let stream_api = format!("{origin}/stream?id={hid}");
        let sresp = client
            .get(&stream_api)
            .header("Referer", format!("{origin}/"))
            .header("Origin", &origin)
            .send()
            .await
            .ok();
        let Some(sresp) = sresp else { continue; };
        if !sresp.status().is_success() {
            continue;
        }
        let Ok(val) = sresp.json::<serde_json::Value>().await else { continue; };
        let m3u8 = val.pointer("/media/stream").and_then(|v| v.as_str()).unwrap_or("").trim();
        if !m3u8.starts_with("https://") {
            continue;
        }
        register_online_proxy_url(m3u8, &format!("{origin}/"));
        sources.push(OnlineStreamSource {
            name: if total_eps == 1 {
                format!("备用线路{}", idx + 1)
            } else {
                format!("第{}段", idx + 1)
            },
            url: m3u8.to_string(),
        });
    }
    if sources.is_empty() {
        return None;
    }

    Some(OnlineStreamResult {
        code: code.to_uppercase(),
        via: "123av".to_string(),
        page_url,
        search_url,
        poster: None,
        title: None,
        proxied: true,
        cached: false,
        sources,
    })
}

pub async fn proxy_online_m3u8(raw_url: &str) -> Result<String, String> {
    let url = raw_url.trim();
    let client = shared_http_client()?;
    if is_javday_m3u8_url(url) {
        let resp = client
            .get(url)
            .header("Referer", "https://javday.app/")
            .send()
            .await
            .map_err(|e| format!("抓取播放列表失败：{e}"))?;
        if !resp.status().is_success() {
            return Err(format!("播放列表返回 HTTP {}", resp.status().as_u16()));
        }
        let text = resp.text().await.map_err(|e| format!("读取播放列表失败：{e}"))?;
        if !text.trim_start_matches('\u{feff}').starts_with("#EXTM3U") {
            return Err("返回内容不是有效的 M3U8 列表".to_string());
        }
        let base_rewritten = reqwest::Url::parse(&rewrite_javday_host(url)).ok();
        let mut out = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                out.push(line.to_string());
            } else if trimmed.starts_with("https://") {
                out.push(rewrite_javday_host(trimmed));
            } else if let Some(ref base) = base_rewritten {
                if let Ok(abs) = base.join(trimmed) {
                    out.push(abs.to_string());
                } else {
                    out.push(trimmed.to_string());
                }
            } else {
                out.push(trimmed.to_string());
            }
        }
        return Ok(format!("{}\n", out.join("\n")));
    }

    let referer = lookup_online_proxy_referer(url)
        .ok_or_else(|| "该播放列表地址未在代理白名单中".to_string())?;
    let resp = client
        .get(url)
        .header("Referer", &referer)
        .send()
        .await
        .map_err(|e| format!("抓取播放列表失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("播放列表返回 HTTP {}", resp.status().as_u16()));
    }
    let text = resp.text().await.map_err(|e| format!("读取播放列表失败：{e}"))?;
    if !text.trim_start_matches('\u{feff}').starts_with("#EXTM3U") {
        return Err("返回内容不是有效的 M3U8 列表".to_string());
    }
    let base = reqwest::Url::parse(url).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push(line.to_string());
            continue;
        }
        let Ok(abs_url) = base.join(trimmed) else {
            out.push(line.to_string());
            continue;
        };
        let abs = abs_url.to_string();
        register_online_proxy_url(&abs, &referer);
        let q = percent_encoding::utf8_percent_encode(&abs, percent_encoding::NON_ALPHANUMERIC).to_string();
        if abs_url.path().to_ascii_lowercase().ends_with(".m3u8") {
            out.push(format!("/api/online-preview/m3u8?u={q}"));
        } else {
            out.push(format!("/api/online-preview/ts?u={q}"));
        }
    }
    Ok(format!("{}\n", out.join("\n")))
}

pub async fn proxy_online_ts(raw_url: &str) -> Result<Vec<u8>, String> {
    let url = raw_url.trim();
    let referer = lookup_online_proxy_referer(url)
        .ok_or_else(|| "该视频分片未在代理白名单中".to_string())?;
    let client = shared_http_client()?;
    let resp = client
        .get(url)
        .header("Referer", &referer)
        .send()
        .await
        .map_err(|e| format!("抓取视频分片失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("分片请求返回 HTTP {}", resp.status().as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("读取分片数据失败：{e}"))?;
    Ok(bytes.to_vec())
}

// ==================== JavDB Comments (Lazy-Loaded Detail Drawer) ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieCommentItem {
    pub author: String,
    pub score: Option<String>,
    pub date: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieCommentsResult {
    pub number: String,
    pub video_url: String,
    pub total: usize,
    pub items: Vec<MovieCommentItem>,
    pub not_found: bool,
    pub geo_blocked: bool,
    pub cached: bool,
    pub error: Option<String>,
}

static JAVDB_COMMENTS_CACHE: OnceLock<Mutex<HashMap<String, (i64, MovieCommentsResult)>>> = OnceLock::new();
const JAVDB_COMMENTS_TTL_SECS: i64 = 1800;

fn javdb_hosts() -> Vec<String> {
    if let Ok(raw) = std::env::var("PML_JAVDB_HOSTS") {
        let list: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !list.is_empty() {
            return list;
        }
    }
    vec!["javdb.com".to_string(), "javdb580.com".to_string()]
}

fn is_javdb_blocked_page(html: &str) -> bool {
    let head = prefix_utf8(html, 6000);
    head.contains("版權限制")
        || head.contains("版权限制")
        || head.contains("_cf_chl_opt")
        || head.contains("Just a moment")
        || head.contains("challenge-form")
}

pub fn parse_javdb_review_items(frag: &str) -> Vec<MovieCommentItem> {
    let mut items = Vec::new();
    let chunks: Vec<&str> = if frag.contains("review-item") {
        frag.split("class=\"review-item").skip(1).collect()
    } else if frag.contains("class=\"box") {
        frag.split("class=\"box").skip(1).collect()
    } else {
        Vec::new()
    };

    for chunk in chunks {
        let block = prefix_utf8(chunk, 4000);
        let author = block
            .split("<strong>")
            .nth(1)
            .and_then(|s| s.split_once("</strong>"))
            .map(|(a, _)| strip_html_tags_clean(a))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "匿名观众".to_string());

        let score = block
            .split("score-stars")
            .nth(1)
            .map(|s| {
                let sslice = prefix_utf8(s, 800);
                let filled = sslice.matches("icon-star").count();
                if filled > 0 && filled <= 5 {
                    format!("{filled}")
                } else {
                    String::new()
                }
            })
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let s = strip_html_tags_clean(block);
                let pos = s.find('★')?;
                let after = &s[pos + '★'.len_utf8()..];
                let num: String = after.trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
                if num.is_empty() { None } else { Some(num) }
            });

        let date = block
            .split("time")
            .nth(1)
            .and_then(|s| s.split_once('>'))
            .and_then(|(_, r)| r.split_once('<'))
            .map(|(d, _)| strip_html_tags_clean(d))
            .filter(|s| !s.is_empty());

        let mut body_text = String::new();
        for p in block.split("<p").skip(1) {
            let Some((_, inside)) = p.split_once('>') else { continue; };
            let raw_p = inside.split("</p>").next().unwrap_or(inside);
            let t = strip_html_tags_clean(raw_p);
            if t.is_empty() || t.contains("登錄") || t.contains("登录") {
                continue;
            }
            if !body_text.is_empty() {
                body_text.push('\n');
            }
            body_text.push_str(&t);
        }
        if body_text.is_empty() {
            if let Some((_, c)) = block.split_once("class=\"content") {
                if let Some((_, inside)) = c.split_once('>') {
                    let raw_c = inside.split("</div>").next().unwrap_or(inside);
                    let t = strip_html_tags_clean(raw_c);
                    if !t.is_empty() {
                        body_text = t;
                    }
                }
            }
        }
        if body_text.is_empty() {
            continue;
        }
        items.push(MovieCommentItem {
            author,
            score,
            date,
            text: body_text,
        });
        if items.len() >= 30 {
            break;
        }
    }
    items
}

pub async fn fetch_javdb_comments(number: &str, force_refresh: bool) -> Result<MovieCommentsResult, String> {
    let candidates = standard_jav_query_codes(number);
    let clean_num = candidates.first().cloned().unwrap_or_else(|| number.trim().to_uppercase());
    if clean_num.is_empty() {
        return Err("缺少番号参数".to_string());
    }
    let cache = JAVDB_COMMENTS_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let now = current_ts();
    if !force_refresh {
        if let Ok(guard) = cache.lock() {
            if let Some((ts, cached)) = guard.get(&clean_num) {
                if now - *ts < JAVDB_COMMENTS_TTL_SECS {
                    let mut out = cached.clone();
                    out.cached = true;
                    return Ok(out);
                }
            }
        }
    }

    let client = shared_http_client()?;
    let hosts = javdb_hosts();
    let mut active_host: Option<String> = None;
    let mut video_href: Option<String> = None;
    let mut saw_geo_block = false;
    let mut last_err = String::new();

    'host_loop: for host in &hosts {
        for cand in &candidates {
            let encoded = percent_encoding::utf8_percent_encode(cand, percent_encoding::NON_ALPHANUMERIC).to_string();
            let search_url = format!("https://{host}/search?q={encoded}&f=all");
            match client
                .get(&search_url)
                .header("Cookie", "over18=1; locale=zh_CN")
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    let html = resp.text().await.unwrap_or_default();
                    if is_javdb_blocked_page(&html) {
                        saw_geo_block = true;
                        continue 'host_loop;
                    }
                    active_host = Some(host.clone());
                    for seg in html.split("href=\"/v/").skip(1) {
                        let Some((id_part, after)) = seg.split_once('"') else { continue; };
                        let snippet = prefix_utf8(after, 600);
                        let snippet_upper = snippet.to_ascii_uppercase();
                        let cand_upper = cand.to_ascii_uppercase();
                        if snippet_upper.contains(&cand_upper)
                            || snippet_upper.contains(&cand_upper.replace('-', ""))
                        {
                            video_href = Some(format!("/v/{id_part}"));
                            break 'host_loop;
                        }
                    }
                }
                Ok(resp) => {
                    last_err = format!("{host}: HTTP {}", resp.status().as_u16());
                }
                Err(e) => {
                    last_err = format!("{host}: {e}");
                }
            }
        }
    }

    if active_host.is_none() {
        return Ok(MovieCommentsResult {
            number: clean_num,
            video_url: String::new(),
            total: 0,
            items: Vec::new(),
            not_found: false,
            geo_blocked: saw_geo_block,
            cached: false,
            error: Some(if saw_geo_block {
                "当前出口 IP 被 JavDB 限制访问（如日本/韩国节点或人机验证），可配置非日韩分流代理后重试".to_string()
            } else if !last_err.is_empty() {
                last_err
            } else {
                "无法连接 JavDB".to_string()
            }),
        });
    }

    let Some(href) = video_href else {
        let res = MovieCommentsResult {
            number: clean_num.clone(),
            video_url: String::new(),
            total: 0,
            items: Vec::new(),
            not_found: true,
            geo_blocked: false,
            cached: false,
            error: None,
        };
        if let Ok(mut guard) = cache.lock() {
            guard.insert(clean_num, (now, res.clone()));
        }
        return Ok(res);
    };

    let host = active_host.unwrap_or_else(|| "javdb.com".to_string());
    let video_url = format!("https://{host}{href}");
    let reviews_url = format!("{video_url}/reviews/lastest");

    let mut items = Vec::new();
    let mut total: Option<usize> = None;

    if let Ok(resp) = client
        .get(&reviews_url)
        .header("Cookie", "over18=1; locale=zh_CN")
        .header("Referer", &video_url)
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .send()
        .await
    {
        if resp.status().is_success() {
            let frag = resp.text().await.unwrap_or_default();
            if !is_javdb_blocked_page(&frag) {
                items = parse_javdb_review_items(&frag);
            }
        }
    }

    if let Ok(vresp) = client
        .get(&video_url)
        .header("Cookie", "over18=1; locale=zh_CN")
        .header("Referer", &format!("https://{host}/"))
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .send()
        .await
    {
        if vresp.status().is_success() {
            let vhtml = vresp.text().await.unwrap_or_default();
            if !is_javdb_blocked_page(&vhtml) {
                for marker in ["評論 (", "评论 (", "Reviews (", "評論（", "评论（"] {
                    if let Some((_, rest)) = vhtml.split_once(marker) {
                        let num_str: String = rest.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(n) = num_str.parse::<usize>() {
                            total = Some(n);
                            break;
                        }
                    }
                }
                if items.is_empty() {
                    items = parse_javdb_review_items(&vhtml);
                }
            }
        }
    }

    let result = MovieCommentsResult {
        number: clean_num.clone(),
        video_url,
        total: total.unwrap_or(items.len()),
        items,
        not_found: false,
        geo_blocked: false,
        cached: false,
        error: None,
    };
    if let Ok(mut guard) = cache.lock() {
        guard.insert(clean_num, (now, result.clone()));
    }
    Ok(result)
}

// ==================== Japanese -> Chinese Title Translation ====================

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MovieTranslateConfig {
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
}

const TRANSLATE_CFG_KEY: &str = "movie_translate_config";

pub fn get_movie_translate_config(app: &crate::AppHandle) -> Result<MovieTranslateConfig, String> {
    let conn = crate::v2_config::open(app)?;
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [TRANSLATE_CFG_KEY],
            |r| r.get(0),
        )
        .ok();
    if let Some(json) = raw {
        if let Ok(cfg) = serde_json::from_str::<MovieTranslateConfig>(&json) {
            return Ok(cfg);
        }
    }
    Ok(MovieTranslateConfig::default())
}

pub fn save_movie_translate_config(app: &crate::AppHandle, cfg: &MovieTranslateConfig) -> Result<(), String> {
    let conn = crate::v2_config::open(app)?;
    let cleaned = MovieTranslateConfig {
        endpoint: cfg.endpoint.trim().to_string(),
        api_key: cfg.api_key.trim().to_string(),
        model: cfg.model.trim().to_string(),
    };
    let json = serde_json::to_string(&cleaned).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)",
        rusqlite::params![TRANSLATE_CFG_KEY, json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn translate_batch_via_llm(cfg: &MovieTranslateConfig, texts: &[String]) -> Result<Vec<Option<String>>, String> {
    let key = cfg.api_key.trim();
    if key.is_empty() {
        return Err("未配置大模型 API Key".to_string());
    }
    let endpoint_raw = cfg.endpoint.trim();
    let endpoint = if endpoint_raw.is_empty() {
        "https://open.bigmodel.cn/api/paas/v4/chat/completions".to_string()
    } else if endpoint_raw.ends_with("/chat/completions") {
        endpoint_raw.to_string()
    } else {
        format!("{}/chat/completions", endpoint_raw.trim_end_matches('/'))
    };
    let model = if cfg.model.trim().is_empty() {
        "glm-4-flash".to_string()
    } else {
        cfg.model.trim().to_string()
    };

    let numbered_input: Vec<serde_json::Value> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| serde_json::json!({ "i": i, "t": t }))
        .collect();
    let prompt = format!(
        "将下列日文影片标题翻译为自然流畅的简体中文，保留人名、品牌与关键意象，勿输出解释，仅返回严格 JSON 数组，每项形如 {{\"i\":0,\"t\":\"译文\"}}：\n{}",
        serde_json::to_string(&numbered_input).unwrap_or_default()
    );

    let client = shared_http_client()?;
    let body = serde_json::json!({
        "model": model,
        "temperature": 0.2,
        "messages": [
            { "role": "system", "content": "你是精准的日中标题翻译助手，仅输出 JSON 数组。" },
            { "role": "user", "content": prompt }
        ]
    });

    let resp = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求翻译模型失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("翻译模型返回 HTTP {}", resp.status().as_u16()));
    }
    let json_val: serde_json::Value = resp.json().await.map_err(|e| format!("解析模型响应失败：{e}"))?;
    let content = json_val
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    let json_slice = match (content.find('['), content.rfind(']')) {
        (Some(s), Some(e)) if e >= s => &content[s..=e],
        _ => return Err("模型未返回有效的 JSON 译文数组".to_string()),
    };
    let parsed: Vec<serde_json::Value> = serde_json::from_str(json_slice)
        .map_err(|e| format!("解析译文数组失败：{e}"))?;

    let mut out = vec![None; texts.len()];
    for item in parsed {
        if let Some(idx) = item.get("i").and_then(|v| v.as_u64()).map(|n| n as usize) {
            if idx < out.len() {
                if let Some(tr) = item.get("t").and_then(|v| v.as_str()) {
                    let clean = tr.trim();
                    if !clean.is_empty() {
                        out[idx] = Some(clean.to_string());
                    }
                }
            }
        }
    }
    Ok(out)
}

async fn translate_one_via_google(client: &Client, text: &str) -> Option<String> {
    let mut url = reqwest::Url::parse("https://translate.googleapis.com/translate_a/single").ok()?;
    url.query_pairs_mut()
        .append_pair("client", "gtx")
        .append_pair("sl", "auto")
        .append_pair("tl", "zh-CN")
        .append_pair("dt", "t")
        .append_pair("q", text);
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let val: serde_json::Value = resp.json().await.ok()?;
    let chunks = val.get(0)?.as_array()?;
    let mut merged = String::new();
    for seg in chunks {
        if let Some(part) = seg.get(0).and_then(|v| v.as_str()) {
            merged.push_str(part);
        }
    }
    let clean = merged.trim().to_string();
    if clean.is_empty() { None } else { Some(clean) }
}

async fn translate_one_via_mymemory(client: &Client, text: &str) -> Option<String> {
    let mut url = reqwest::Url::parse("https://api.mymemory.translated.net/get").ok()?;
    url.query_pairs_mut()
        .append_pair("q", text)
        .append_pair("langpair", "ja|zh-CN");
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let val: serde_json::Value = resp.json().await.ok()?;
    let tr = val.pointer("/responseData/translatedText").and_then(|v| v.as_str())?.trim();
    if tr.is_empty() { None } else { Some(tr.to_string()) }
}

pub async fn translate_titles_with_fallback(
    cfg: &MovieTranslateConfig,
    texts: &[String],
) -> Result<(Vec<String>, String), String> {
    if texts.is_empty() {
        return Ok((Vec::new(), "none".to_string()));
    }
    let mut results: Vec<Option<String>> = vec![None; texts.len()];
    let mut engines: Vec<&'static str> = Vec::new();

    if !cfg.api_key.trim().is_empty() {
        if let Ok(llm_out) = translate_batch_via_llm(cfg, texts).await {
            if llm_out.iter().any(|x| x.is_some()) {
                engines.push("LLM");
            }
            for (i, v) in llm_out.into_iter().enumerate() {
                if v.is_some() {
                    results[i] = v;
                }
            }
        }
    }

    let missing_indices: Vec<usize> = results
        .iter()
        .enumerate()
        .filter_map(|(i, v)| if v.is_none() { Some(i) } else { None })
        .collect();

    if !missing_indices.is_empty() {
        let client = shared_http_client()?;
        let mut used_google = false;
        let mut used_mymemory = false;
        let futs = missing_indices.iter().map(|&idx| {
            let text = texts[idx].clone();
            let c = client.clone();
            async move {
                if let Some(g) = translate_one_via_google(&c, &text).await {
                    return (idx, Some(g), "google");
                }
                if let Some(m) = translate_one_via_mymemory(&c, &text).await {
                    return (idx, Some(m), "mymemory");
                }
                (idx, None, "")
            }
        });
        for (idx, tr, eng) in futures_util::future::join_all(futs).await {
            if let Some(t) = tr {
                results[idx] = Some(t);
                if eng == "google" {
                    used_google = true;
                } else if eng == "mymemory" {
                    used_mymemory = true;
                }
            }
        }
        if used_google {
            engines.push("Google");
        }
        if used_mymemory {
            engines.push("MyMemory");
        }
    }

    let final_vec: Vec<String> = results
        .into_iter()
        .enumerate()
        .map(|(i, opt)| opt.unwrap_or_else(|| texts[i].clone()))
        .collect();
    if engines.is_empty() {
        return Err("所有翻译引擎均不可用，请检查网络或配置 API Key".to_string());
    }
    Ok((final_vec, engines.join(" + ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_jav_codes_strip_maker_and_dmm_prefixes() {
        assert_eq!(standard_jav_query_codes("200GANA-1535"), vec!["GANA-1535"]);
        let abp = standard_jav_query_codes("118abp00769");
        assert!(abp.contains(&"ABP-769".to_string()));
        let fuga = standard_jav_query_codes("h_086fuga00041");
        assert!(fuga.contains(&"FUGA-041".to_string()));
    }

    #[test]
    fn parses_fanza_bitrates_payload() {
        let html = r#"var params = {"bitrates":[{"bitrate":"3000kbps","src":"\/\/cc3001.dmm.co.jp\/litevideo\/freepv\/a\/abp\/abp00769\/abp00769hhb.mp4"}]};"#;
        let items = parse_fanza_bitrates(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "3000kbps");
        assert_eq!(
            items[0].url,
            "https://cc3001.dmm.co.jp/litevideo/freepv/a/abp/abp00769/abp00769hhb.mp4"
        );
    }

    #[test]
    fn parses_javbus_ajax_badges() {
        let frag = r#"<tr><td><a class="btn btn-mini btn-warning" title="包含字幕">字幕</a> <a style="color:#333" href="magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567">ABP-769-C</a></td><td><a>4.21GB</a></td><td><a>2024-01-01</a></td></tr>"#;
        let items = parse_javbus_ajax_magnets(frag);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "ABP-769-C");
        assert_eq!(items[0].tags, vec!["字幕"]);
        assert_eq!(items[0].size.as_deref(), Some("4.21GB"));
    }
}
