use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MagnetSearchRequest {
    pub query: String,
    pub category: Option<String>, // all | video | anime | software | audio_book | adult
    pub sort: Option<String>,     // seeders | size_desc | size_asc | date_desc
    pub sources: Option<Vec<String>>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalMagnetItem {
    pub title: String,
    pub magnet: String,
    pub info_hash: String,
    pub size_bytes: u64,
    pub size_text: String,
    pub seeders: i64,
    pub leechers: i64,
    pub category: String,
    pub source: String,
    pub tracker: String,
    pub pub_date: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MagnetSourceStat {
    pub source: String,
    pub label: String,
    pub count: usize,
    pub ok: bool,
    pub elapsed_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MagnetSearchResponse {
    pub query: String,
    pub total: usize,
    pub items: Vec<UniversalMagnetItem>,
    pub source_stats: Vec<MagnetSourceStat>,
}

fn urlencoding(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "未知".to_string();
    }
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut val = bytes as f64;
    let mut idx = 0usize;
    while val >= 1024.0 && idx + 1 < UNITS.len() {
        val /= 1024.0;
        idx += 1;
    }
    if idx <= 1 {
        format!("{val:.0} {}", UNITS[idx])
    } else {
        format!("{val:.2} {}", UNITS[idx])
    }
}

fn parse_size_text_to_bytes(text: &str) -> u64 {
    let trimmed = text.trim();
    let mut parts = trimmed.split_whitespace();
    let Some(num_str) = parts.next() else {
        return 0;
    };
    let num: f64 = num_str.replace(',', "").parse().unwrap_or(0.0);
    let unit = parts.next().unwrap_or("").to_lowercase();
    let mul: f64 = match unit.as_str() {
        "b" | "bytes" => 1.0,
        "kb" | "kib" => 1024.0,
        "mb" | "mib" => 1024.0 * 1024.0,
        "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "tb" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (num * mul).max(0.0) as u64
}

fn build_magnet(info_hash: &str, title: &str) -> String {
    format!(
        "magnet:?xt=urn:btih:{info_hash}&dn={}&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=http%3A%2F%2Fnyaa.tracker.wf%3A7777%2Fannounce",
        urlencoding(title)
    )
}

fn parse_rfc3339_or_rfc2822_ts(s: &str) -> (String, i64) {
    let t = s.trim();
    if t.is_empty() {
        return (String::new(), 0);
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
        return (dt.format("%Y-%m-%d").to_string(), dt.timestamp());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(t) {
        return (dt.format("%Y-%m-%d").to_string(), dt.timestamp());
    }
    if let Ok(ts) = t.parse::<i64>() {
        if let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) {
            return (dt.format("%Y-%m-%d").to_string(), ts);
        }
    }
    (t.chars().take(16).collect(), 0)
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(4))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| format!("初始化 HTTP 客户端失败：{e}"))
}

// 1. Knaben Meta-API (aggregates 1337x, TPB, RARBG, RuTracker, EZTV, Nyaa, AnimeTosho, etc.)
async fn query_knaben(client: &reqwest::Client, query: &str, category: &str) -> (Vec<UniversalMagnetItem>, MagnetSourceStat) {
    let t0 = Instant::now();
    let cats: Option<Vec<i64>> = match category {
        "video" => Some(vec![2000000, 3000000]),
        "anime" => Some(vec![6000000]),
        "software" => Some(vec![4000000, 7000000]),
        "audio_book" => Some(vec![1000000, 9000000]),
        "adult" => Some(vec![5000000]),
        _ => None,
    };
    let mut body = serde_json::json!({
        "query": query,
        "size": 50,
        "order_by": "seeders",
        "order_direction": "desc",
        "hide_unsafe": false,
        "hide_xxx": false,
    });
    if let Some(c) = cats {
        body["categories"] = serde_json::json!(c);
    }

    let resp = match client.post("https://api.knaben.org/v1").json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: "knaben".to_string(),
                    label: "Knaben 聚合".to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };
    if !resp.status().is_success() {
        let code = resp.status();
        return (
            Vec::new(),
            MagnetSourceStat {
                source: "knaben".to_string(),
                label: "Knaben 聚合".to_string(),
                count: 0,
                ok: false,
                elapsed_ms: t0.elapsed().as_millis() as u64,
                error: Some(format!("HTTP {code}")),
            },
        );
    }
    let json: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: "knaben".to_string(),
                    label: "Knaben 聚合".to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };

    let mut out = Vec::new();
    if let Some(hits) = json.get("hits").and_then(Value::as_array) {
        for h in hits {
            let title = h.get("title").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let hash = h.get("hash").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
            let magnet = h
                .get("magnetUrl")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| s.starts_with("magnet:"))
                .unwrap_or_else(|| if !hash.is_empty() { build_magnet(&hash, &title) } else { String::new() });
            if title.is_empty() || magnet.is_empty() || hash.is_empty() {
                continue;
            }
            let size_bytes = h.get("bytes").and_then(Value::as_u64).unwrap_or(0);
            let seeders = h.get("seeders").and_then(Value::as_i64).unwrap_or(0);
            let leechers = h.get("peers").and_then(Value::as_i64).unwrap_or(0);
            let cat = h.get("category").and_then(Value::as_str).unwrap_or("综合").to_string();
            let tracker = h
                .get("tracker")
                .and_then(Value::as_str)
                .or_else(|| h.get("cachedOrigin").and_then(Value::as_str))
                .unwrap_or("Knaben")
                .to_string();
            let (pub_date, timestamp) = parse_rfc3339_or_rfc2822_ts(h.get("date").and_then(Value::as_str).unwrap_or(""));
            out.push(UniversalMagnetItem {
                title,
                magnet,
                info_hash: hash,
                size_bytes,
                size_text: format_bytes(size_bytes),
                seeders,
                leechers,
                category: cat,
                source: "knaben".to_string(),
                tracker,
                pub_date,
                timestamp,
            });
        }
    }
    let count = out.len();
    (
        out,
        MagnetSourceStat {
            source: "knaben".to_string(),
            label: "Knaben 聚合".to_string(),
            count,
            ok: true,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            error: None,
        },
    )
}

// 2. The Pirate Bay Official JSON API (apibay.org)
async fn query_tpb(client: &reqwest::Client, query: &str, category: &str) -> (Vec<UniversalMagnetItem>, MagnetSourceStat) {
    let t0 = Instant::now();
    let cat_param = match category {
        "video" => "200",
        "software" => "300,400",
        "audio_book" => "100,601",
        "adult" => "500",
        _ => "0",
    };
    let url = format!("https://apibay.org/q.php?q={}&cat={cat_param}", urlencoding(query));
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: "tpb".to_string(),
                    label: "The Pirate Bay".to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };
    let arr: Vec<Value> = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: "tpb".to_string(),
                    label: "The Pirate Bay".to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };

    let mut out = Vec::new();
    for item in arr.into_iter().take(50) {
        let hash = item.get("info_hash").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
        if hash.is_empty() || hash == "0000000000000000000000000000000000000000" {
            continue;
        }
        let title = item.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if title.is_empty() {
            continue;
        }
        let size_bytes = item
            .get("size")
            .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0);
        let seeders = item
            .get("seeders")
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0);
        let leechers = item
            .get("leechers")
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .unwrap_or(0);
        let added = item.get("added").and_then(Value::as_str).unwrap_or("");
        let (pub_date, timestamp) = parse_rfc3339_or_rfc2822_ts(added);
        let cat_id = item.get("category").and_then(Value::as_str).unwrap_or("");
        let cat_label = match cat_id.chars().next() {
            Some('1') => "音乐/音频",
            Some('2') => "影视/视频",
            Some('3') => "软件/应用",
            Some('4') => "游戏",
            Some('5') => "成人",
            _ => "综合",
        };
        out.push(UniversalMagnetItem {
            magnet: build_magnet(&hash, &title),
            title,
            info_hash: hash,
            size_bytes,
            size_text: format_bytes(size_bytes),
            seeders,
            leechers,
            category: cat_label.to_string(),
            source: "tpb".to_string(),
            tracker: "The Pirate Bay".to_string(),
            pub_date,
            timestamp,
        });
    }
    let count = out.len();
    (
        out,
        MagnetSourceStat {
            source: "tpb".to_string(),
            label: "The Pirate Bay".to_string(),
            count,
            ok: true,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            error: None,
        },
    )
}

// 3 & 4. Nyaa & Sukebei Nyaa RSS
async fn query_nyaa_family(
    client: &reqwest::Client,
    host: &str,
    source_id: &str,
    label: &str,
    query: &str,
    cat_code: &str,
) -> (Vec<UniversalMagnetItem>, MagnetSourceStat) {
    let t0 = Instant::now();
    let url = format!("https://{host}/?page=rss&q={}&c={cat_code}&s=seeders&o=desc", urlencoding(query));
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: source_id.to_string(),
                    label: label.to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };
    if !resp.status().is_success() {
        let code = resp.status();
        return (
            Vec::new(),
            MagnetSourceStat {
                source: source_id.to_string(),
                label: label.to_string(),
                count: 0,
                ok: false,
                elapsed_ms: t0.elapsed().as_millis() as u64,
                error: Some(format!("HTTP {code}")),
            },
        );
    }
    let xml = match resp.text().await {
        Ok(s) => s,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: source_id.to_string(),
                    label: label.to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };
    let Ok(doc) = roxmltree::Document::parse(&xml) else {
        return (
            Vec::new(),
            MagnetSourceStat {
                source: source_id.to_string(),
                label: label.to_string(),
                count: 0,
                ok: false,
                elapsed_ms: t0.elapsed().as_millis() as u64,
                error: Some("RSS 解析失败".to_string()),
            },
        );
    };

    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("item")).take(50) {
        let mut title = String::new();
        let mut hash = String::new();
        let mut size_raw = String::new();
        let mut seeders: i64 = 0;
        let mut leechers: i64 = 0;
        let mut cat = String::new();
        let mut pub_raw = String::new();
        for child in node.children().filter(|c| c.is_element()) {
            let tag = child.tag_name().name();
            let text = child.text().unwrap_or("").trim();
            match tag {
                "title" => title = text.to_string(),
                "infoHash" => hash = text.to_lowercase(),
                "size" => size_raw = text.to_string(),
                "seeders" => seeders = text.parse().unwrap_or(0),
                "leechers" => leechers = text.parse().unwrap_or(0),
                "category" => cat = text.to_string(),
                "pubDate" => pub_raw = text.to_string(),
                _ => {}
            }
        }
        if title.is_empty() || hash.is_empty() {
            continue;
        }
        let size_bytes = parse_size_text_to_bytes(&size_raw);
        let (pub_date, timestamp) = parse_rfc3339_or_rfc2822_ts(&pub_raw);
        out.push(UniversalMagnetItem {
            magnet: build_magnet(&hash, &title),
            title,
            info_hash: hash,
            size_bytes,
            size_text: if size_raw.is_empty() { format_bytes(size_bytes) } else { size_raw },
            seeders,
            leechers,
            category: if cat.is_empty() { label.to_string() } else { cat },
            source: source_id.to_string(),
            tracker: label.to_string(),
            pub_date,
            timestamp,
        });
    }
    let count = out.len();
    (
        out,
        MagnetSourceStat {
            source: source_id.to_string(),
            label: label.to_string(),
            count,
            ok: true,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            error: None,
        },
    )
}

fn extract_info_hash_from_magnet(magnet: &str) -> String {
    let lower = magnet.to_lowercase();
    if let Some(pos) = lower.find("xt=urn:btih:") {
        let rest = &lower[pos + "xt=urn:btih:".len()..];
        let hash: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        return hash;
    }
    String::new()
}

fn is_cjk_char(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3040..=0x30FF | 0xAC00..=0xD7AF
    )
}

/// Strict relevance filter to eliminate upstream fallback junk (e.g. TPB/Knaben returning top English shows for Chinese queries)
fn is_relevant_title(query: &str, title: &str) -> bool {
    let q_lower = query.trim().to_lowercase();
    let t_lower = title.to_lowercase();
    if q_lower.is_empty() || t_lower.is_empty() {
        return false;
    }
    if t_lower.contains(&q_lower) {
        return true;
    }

    // Collect unique CJK characters from query
    let mut cjk_chars: Vec<char> = Vec::new();
    for ch in q_lower.chars() {
        if is_cjk_char(ch) && !cjk_chars.contains(&ch) {
            cjk_chars.push(ch);
        }
    }

    // Collect ASCII alphanumeric tokens (length >= 2)
    let ascii_tokens: Vec<String> = q_lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(str::to_string)
        .collect();

    if !cjk_chars.is_empty() {
        let matched_cjk = cjk_chars.iter().filter(|&&c| t_lower.contains(c)).count();
        let needed_cjk = match cjk_chars.len() {
            1 => 1,
            2 | 3 => 2,
            n => (n * 6 + 9) / 10, // >= 60% of CJK chars (handles Simplified/Traditional differences like 海贼王 vs 海賊王)
        };
        if matched_cjk < needed_cjk {
            return false;
        }
    }

    if !ascii_tokens.is_empty() {
        let matched_ascii = ascii_tokens.iter().filter(|tok| t_lower.contains(tok.as_str())).count();
        let needed_ascii = (ascii_tokens.len() + 1) / 2;
        if matched_ascii < needed_ascii {
            return false;
        }
    }

    !cjk_chars.is_empty() || !ascii_tokens.is_empty()
}

fn parse_size_from_description(desc: &str) -> (u64, String) {
    // BT4G RSS description often contains "... <br> 1.45 GB" or "... 850 MB"
    let cleaned = desc.replace("<br>", " ").replace("<br/>", " ").replace("<br />", " ");
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    for win in tokens.windows(2) {
        let u = win[1].trim_matches(|c: char| !c.is_ascii_alphabetic()).to_lowercase();
        if matches!(u.as_str(), "b" | "kb" | "kib" | "mb" | "mib" | "gb" | "gib" | "tb" | "tib") {
            let candidate = format!("{} {}", win[0].trim_matches(|c: char| !c.is_ascii_digit() && c != '.'), u);
            let bytes = parse_size_text_to_bytes(&candidate);
            if bytes > 0 {
                return (bytes, format_bytes(bytes));
            }
        }
    }
    (0, "未知".to_string())
}

// 5. BT4G Chinese-friendly Global DHT RSS (bt4gprx.com)
async fn query_bt4g(client: &reqwest::Client, query: &str, category: &str, sort: &str) -> (Vec<UniversalMagnetItem>, MagnetSourceStat) {
    let t0 = Instant::now();
    let orderby = match sort {
        "size_desc" => "size",
        "date_desc" => "time",
        _ => "seeders",
    };
    let cat_q = match category {
        "video" => "&category=movie",
        "audio_book" => "&category=audio",
        "software" => "&category=app",
        _ => "",
    };
    let url = format!(
        "https://bt4gprx.com/search?q={}&page=rss&orderby={orderby}{cat_q}",
        urlencoding(query)
    );
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: "bt4g".to_string(),
                    label: "BT4G (中文DHT)".to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };
    if !resp.status().is_success() {
        let code = resp.status();
        return (
            Vec::new(),
            MagnetSourceStat {
                source: "bt4g".to_string(),
                label: "BT4G (中文DHT)".to_string(),
                count: 0,
                ok: false,
                elapsed_ms: t0.elapsed().as_millis() as u64,
                error: Some(format!("HTTP {code}")),
            },
        );
    }
    let xml = match resp.text().await {
        Ok(s) => s,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: "bt4g".to_string(),
                    label: "BT4G (中文DHT)".to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };
    let Ok(doc) = roxmltree::Document::parse(&xml) else {
        return (
            Vec::new(),
            MagnetSourceStat {
                source: "bt4g".to_string(),
                label: "BT4G (中文DHT)".to_string(),
                count: 0,
                ok: false,
                elapsed_ms: t0.elapsed().as_millis() as u64,
                error: Some("RSS 解析失败".to_string()),
            },
        );
    };

    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("item")).take(50) {
        let mut title = String::new();
        let mut magnet = String::new();
        let mut desc = String::new();
        let mut pub_raw = String::new();
        for child in node.children().filter(|c| c.is_element()) {
            let tag = child.tag_name().name();
            let text = child.text().unwrap_or("").trim();
            match tag {
                "title" => title = text.to_string(),
                "link" => magnet = text.to_string(),
                "description" => desc = text.to_string(),
                "pubDate" => pub_raw = text.to_string(),
                _ => {}
            }
        }
        let hash = extract_info_hash_from_magnet(&magnet);
        if title.is_empty() || hash.is_empty() || !is_relevant_title(query, &title) {
            continue;
        }
        let (size_bytes, size_text) = parse_size_from_description(&desc);
        let (pub_date, timestamp) = parse_rfc3339_or_rfc2822_ts(&pub_raw);
        out.push(UniversalMagnetItem {
            title,
            magnet,
            info_hash: hash,
            size_bytes,
            size_text,
            seeders: 1, // DHT index doesn't report live tracker seeders
            leechers: 0,
            category: "DHT 资源".to_string(),
            source: "bt4g".to_string(),
            tracker: "BT4G".to_string(),
            pub_date,
            timestamp,
        });
    }
    let count = out.len();
    (
        out,
        MagnetSourceStat {
            source: "bt4g".to_string(),
            label: "BT4G (中文DHT)".to_string(),
            count,
            ok: true,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            error: None,
        },
    )
}

// 6. 动漫花园 DMHY RSS (share.dmhy.org)
async fn query_dmhy(client: &reqwest::Client, query: &str) -> (Vec<UniversalMagnetItem>, MagnetSourceStat) {
    let t0 = Instant::now();
    let url = format!("https://share.dmhy.org/topics/rss/rss.xml?keyword={}", urlencoding(query));
    let resp = match client.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: "dmhy".to_string(),
                    label: "动漫花园".to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };
    if !resp.status().is_success() {
        let code = resp.status();
        return (
            Vec::new(),
            MagnetSourceStat {
                source: "dmhy".to_string(),
                label: "动漫花园".to_string(),
                count: 0,
                ok: false,
                elapsed_ms: t0.elapsed().as_millis() as u64,
                error: Some(format!("HTTP {code}")),
            },
        );
    }
    let xml = match resp.text().await {
        Ok(s) => s,
        Err(e) => {
            return (
                Vec::new(),
                MagnetSourceStat {
                    source: "dmhy".to_string(),
                    label: "动漫花园".to_string(),
                    count: 0,
                    ok: false,
                    elapsed_ms: t0.elapsed().as_millis() as u64,
                    error: Some(e.to_string()),
                },
            );
        }
    };
    let Ok(doc) = roxmltree::Document::parse(&xml) else {
        return (
            Vec::new(),
            MagnetSourceStat {
                source: "dmhy".to_string(),
                label: "动漫花园".to_string(),
                count: 0,
                ok: false,
                elapsed_ms: t0.elapsed().as_millis() as u64,
                error: Some("RSS 解析失败".to_string()),
            },
        );
    };

    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("item")).take(50) {
        let mut title = String::new();
        let mut magnet = String::new();
        let mut cat = String::new();
        let mut pub_raw = String::new();
        for child in node.children().filter(|c| c.is_element()) {
            let tag = child.tag_name().name();
            match tag {
                "title" => title = child.text().unwrap_or("").trim().to_string(),
                "enclosure" => {
                    if let Some(u) = child.attribute("url") {
                        if u.starts_with("magnet:") {
                            magnet = u.to_string();
                        }
                    }
                }
                "category" => cat = child.text().unwrap_or("").trim().to_string(),
                "pubDate" => pub_raw = child.text().unwrap_or("").trim().to_string(),
                _ => {}
            }
        }
        let hash = extract_info_hash_from_magnet(&magnet);
        if title.is_empty() || hash.is_empty() || !is_relevant_title(query, &title) {
            continue;
        }
        let (pub_date, timestamp) = parse_rfc3339_or_rfc2822_ts(&pub_raw);
        out.push(UniversalMagnetItem {
            title,
            magnet,
            info_hash: hash,
            size_bytes: 0,
            size_text: "见磁力".to_string(),
            seeders: 1,
            leechers: 0,
            category: if cat.is_empty() { "动漫花园".to_string() } else { cat },
            source: "dmhy".to_string(),
            tracker: "动漫花园".to_string(),
            pub_date,
            timestamp,
        });
    }
    let count = out.len();
    (
        out,
        MagnetSourceStat {
            source: "dmhy".to_string(),
            label: "动漫花园".to_string(),
            count,
            ok: true,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            error: None,
        },
    )
}

pub fn search_all_magnets(req: MagnetSearchRequest) -> Result<MagnetSearchResponse, String> {
    let q = req.query.trim().to_string();
    if q.is_empty() {
        return Ok(MagnetSearchResponse {
            query: q,
            total: 0,
            items: Vec::new(),
            source_stats: Vec::new(),
        });
    }
    let category = req.category.as_deref().unwrap_or("all").trim().to_lowercase();
    let sort = req.sort.as_deref().unwrap_or("seeders").trim().to_lowercase();
    let limit = req.limit.unwrap_or(80).clamp(10, 150);

    let enabled_sources: Vec<String> = match req.sources {
        Some(list) if !list.is_empty() => list
            .into_iter()
            .map(|s| s.to_lowercase())
            .filter(|s| s != "tpb" && s != "sukebei")
            .collect(),
        _ => match category.as_str() {
            "anime" => vec!["dmhy".into(), "nyaa".into(), "bt4g".into(), "knaben".into()],
            "adult" => vec!["bt4g".into(), "knaben".into(), "nyaa".into()],
            "video" => vec!["bt4g".into(), "dmhy".into(), "knaben".into(), "nyaa".into()],
            "software" | "audio_book" => vec!["bt4g".into(), "knaben".into(), "nyaa".into()],
            _ => vec!["bt4g".into(), "dmhy".into(), "knaben".into(), "nyaa".into()],
        },
    };

    let want_bt4g = enabled_sources.iter().any(|s| s == "bt4g");
    let want_dmhy = enabled_sources.iter().any(|s| s == "dmhy");
    let want_knaben = enabled_sources.iter().any(|s| s == "knaben");
    let want_tpb = enabled_sources.iter().any(|s| s == "tpb");
    let want_nyaa = enabled_sources.iter().any(|s| s == "nyaa");
    let want_sukebei = enabled_sources.iter().any(|s| s == "sukebei");

    let q_clone = q.clone();
    let cat_clone = category.clone();
    let sort_clone = sort.clone();
    let fut = async move {
        let client = build_client()?;
        let nyaa_cat = match cat_clone.as_str() {
            "video" => "4_0",
            "audio_book" => "2_0",
            "software" => "6_0",
            _ => "0_0",
        };

        let f_bt4g = async {
            if want_bt4g { Some(query_bt4g(&client, &q_clone, &cat_clone, &sort_clone).await) } else { None }
        };
        let f_dmhy = async {
            if want_dmhy { Some(query_dmhy(&client, &q_clone).await) } else { None }
        };
        let f_knaben = async {
            if want_knaben { Some(query_knaben(&client, &q_clone, &cat_clone).await) } else { None }
        };
        let f_tpb = async {
            if want_tpb { Some(query_tpb(&client, &q_clone, &cat_clone).await) } else { None }
        };
        let f_nyaa = async {
            if want_nyaa {
                Some(query_nyaa_family(&client, "nyaa.si", "nyaa", "Nyaa", &q_clone, nyaa_cat).await)
            } else {
                None
            }
        };
        let f_sukebei = async {
            if want_sukebei {
                Some(query_nyaa_family(&client, "sukebei.nyaa.si", "sukebei", "Sukebei", &q_clone, "0_0").await)
            } else {
                None
            }
        };

        let (r_bt4g, r_dmhy, r_knaben, r_tpb, r_nyaa, r_sukebei) =
            tokio::join!(f_bt4g, f_dmhy, f_knaben, f_tpb, f_nyaa, f_sukebei);

        let mut source_stats = Vec::new();
        let mut by_hash: HashMap<String, UniversalMagnetItem> = HashMap::new();

        for res_opt in [r_bt4g, r_dmhy, r_knaben, r_tpb, r_nyaa, r_sukebei] {
            if let Some((items, mut stat)) = res_opt {
                // Apply strict relevance filter across ALL sources so upstream fallback junk is dropped cleanly
                let relevant: Vec<UniversalMagnetItem> = items
                    .into_iter()
                    .filter(|it| is_relevant_title(&q_clone, &it.title))
                    .collect();
                stat.count = relevant.len();
                source_stats.push(stat);
                for item in relevant {
                    let key = item.info_hash.clone();
                    match by_hash.get_mut(&key) {
                        Some(existing) => {
                            if item.seeders > existing.seeders {
                                existing.seeders = item.seeders;
                                existing.leechers = item.leechers;
                            }
                            if existing.size_bytes == 0 && item.size_bytes > 0 {
                                existing.size_bytes = item.size_bytes;
                                existing.size_text = item.size_text;
                            }
                        }
                        None => {
                            by_hash.insert(key, item);
                        }
                    }
                }
            }
        }

        let mut items: Vec<UniversalMagnetItem> = by_hash.into_values().collect();
        match sort.as_str() {
            "size_desc" => items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| b.seeders.cmp(&a.seeders))),
            "size_asc" => items.sort_by(|a, b| {
                let sa = if a.size_bytes == 0 { u64::MAX } else { a.size_bytes };
                let sb = if b.size_bytes == 0 { u64::MAX } else { b.size_bytes };
                sa.cmp(&sb).then_with(|| b.seeders.cmp(&a.seeders))
            }),
            "date_desc" => items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp).then_with(|| b.seeders.cmp(&a.seeders))),
            _ => items.sort_by(|a, b| b.seeders.cmp(&a.seeders).then_with(|| b.size_bytes.cmp(&a.size_bytes))),
        }

        let total = items.len();
        items.truncate(limit);

        Ok::<MagnetSearchResponse, String>(MagnetSearchResponse {
            query: q_clone,
            total,
            items,
            source_stats,
        })
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(fut))
    } else {
        tokio::runtime::Runtime::new()
            .map_err(|e| e.to_string())?
            .block_on(fut)
    }
}

// ============================================================================
// Torrent Metadata File-List Inspector (Zero-dep Bencode Parser + Public Cache)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MagnetFileEntry {
    pub path: String,
    pub size_bytes: u64,
    pub size_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MagnetFilesResponse {
    pub info_hash: String,
    pub name: String,
    pub total_size_bytes: u64,
    pub total_size_text: String,
    pub file_count: usize,
    pub files: Vec<MagnetFileEntry>,
    pub source: String,
}

#[derive(Debug)]
enum BVal {
    Int(i64),
    Bytes(Vec<u8>),
    List(Vec<BVal>),
    Dict(Vec<(Vec<u8>, BVal)>),
}

fn parse_bencode(data: &[u8], pos: &mut usize) -> Option<BVal> {
    if *pos >= data.len() {
        return None;
    }
    match data[*pos] {
        b'i' => {
            *pos += 1;
            let start = *pos;
            while *pos < data.len() && data[*pos] != b'e' {
                *pos += 1;
            }
            if *pos >= data.len() {
                return None;
            }
            let num_str = std::str::from_utf8(&data[start..*pos]).ok()?;
            let val = num_str.parse::<i64>().ok()?;
            *pos += 1;
            Some(BVal::Int(val))
        }
        b'l' => {
            *pos += 1;
            let mut list = Vec::new();
            while *pos < data.len() && data[*pos] != b'e' {
                list.push(parse_bencode(data, pos)?);
            }
            if *pos >= data.len() {
                return None;
            }
            *pos += 1;
            Some(BVal::List(list))
        }
        b'd' => {
            *pos += 1;
            let mut map = Vec::new();
            while *pos < data.len() && data[*pos] != b'e' {
                let BVal::Bytes(k) = parse_bencode(data, pos)? else {
                    return None;
                };
                let v = parse_bencode(data, pos)?;
                map.push((k, v));
            }
            if *pos >= data.len() {
                return None;
            }
            *pos += 1;
            Some(BVal::Dict(map))
        }
        b'0'..=b'9' => {
            let start = *pos;
            while *pos < data.len() && data[*pos].is_ascii_digit() {
                *pos += 1;
            }
            if *pos >= data.len() || data[*pos] != b':' {
                return None;
            }
            let len: usize = std::str::from_utf8(&data[start..*pos]).ok()?.parse().ok()?;
            *pos += 1;
            if *pos + len > data.len() {
                return None;
            }
            let bytes = data[*pos..*pos + len].to_vec();
            *pos += len;
            Some(BVal::Bytes(bytes))
        }
        _ => None,
    }
}

fn bdict_get<'a>(entries: &'a [(Vec<u8>, BVal)], key: &[u8]) -> Option<&'a BVal> {
    entries.iter().find(|(k, _)| k.as_slice() == key).map(|(_, v)| v)
}

fn parse_torrent_files(torrent_bytes: &[u8], info_hash: &str, source: &str) -> Option<MagnetFilesResponse> {
    let mut pos = 0;
    let BVal::Dict(root) = parse_bencode(torrent_bytes, &mut pos)? else {
        return None;
    };
    let BVal::Dict(info) = bdict_get(&root, b"info")? else {
        return None;
    };
    let name = match bdict_get(info, b"name.utf-8").or_else(|| bdict_get(info, b"name")) {
        Some(BVal::Bytes(b)) => String::from_utf8_lossy(b).to_string(),
        _ => String::new(),
    };

    let mut files = Vec::new();
    let mut total_size: u64 = 0;

    if let Some(BVal::List(file_list)) = bdict_get(info, b"files") {
        for item in file_list {
            if let BVal::Dict(fdict) = item {
                let sz = match bdict_get(fdict, b"length") {
                    Some(BVal::Int(n)) if *n >= 0 => *n as u64,
                    _ => 0,
                };
                let parts_val = bdict_get(fdict, b"path.utf-8").or_else(|| bdict_get(fdict, b"path"));
                if let Some(BVal::List(parts)) = parts_val {
                    let segs: Vec<String> = parts
                        .iter()
                        .filter_map(|p| match p {
                            BVal::Bytes(b) => Some(String::from_utf8_lossy(b).to_string()),
                            _ => None,
                        })
                        .collect();
                    if !segs.is_empty() {
                        let path = segs.join("/");
                        total_size = total_size.saturating_add(sz);
                        files.push(MagnetFileEntry {
                            path,
                            size_bytes: sz,
                            size_text: format_bytes(sz),
                        });
                    }
                }
            }
        }
    } else if let Some(BVal::Int(len)) = bdict_get(info, b"length") {
        let sz = (*len).max(0) as u64;
        total_size = sz;
        files.push(MagnetFileEntry {
            path: if name.is_empty() { "file".to_string() } else { name.clone() },
            size_bytes: sz,
            size_text: format_bytes(sz),
        });
    }

    if files.is_empty() {
        return None;
    }
    files.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes).then_with(|| a.path.cmp(&b.path)));
    let file_count = files.len();
    files.truncate(300);

    Some(MagnetFilesResponse {
        info_hash: info_hash.to_lowercase(),
        name,
        total_size_bytes: total_size,
        total_size_text: format_bytes(total_size),
        file_count,
        files,
        source: source.to_string(),
    })
}

pub fn inspect_magnet_files(info_hash: &str) -> Result<MagnetFilesResponse, String> {
    let clean = info_hash
        .trim()
        .trim_start_matches("magnet:?xt=urn:btih:")
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>();
    if clean.len() < 32 {
        return Err("无效的磁力 InfoHash".to_string());
    }
    let upper = clean.to_uppercase();
    let lower = clean.to_lowercase();

    let fut = async move {
        let client = build_client()?;
        let candidates = [
            ("iTorrents", format!("https://itorrents.org/torrent/{upper}.torrent")),
            ("TorCache", format!("https://torcache.net/torrent/{upper}.torrent")),
            ("BTCache", format!("https://btcache.me/torrent/{upper}")),
            ("iTorrents-low", format!("https://itorrents.org/torrent/{lower}.torrent")),
        ];

        for (label, url) in candidates {
            let Ok(resp) = client
                .get(&url)
                .header("Referer", "https://itorrents.org/")
                .send()
                .await
            else {
                continue;
            };
            if !resp.status().is_success() {
                continue;
            }
            let Ok(bytes) = resp.bytes().await else {
                continue;
            };
            if bytes.len() > 20 && bytes[0] == b'd' {
                if let Some(parsed) = parse_torrent_files(&bytes, &lower, label) {
                    return Ok::<MagnetFilesResponse, String>(parsed);
                }
            }
        }
        Err("暂未在公开种子缓存中收录该磁力的文件清单（冷门 DHT 种子可直接复制磁力到 115 离线解析）".to_string())
    };

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(fut))
    } else {
        tokio::runtime::Runtime::new()
            .map_err(|e| e.to_string())?
            .block_on(fut)
    }
}
