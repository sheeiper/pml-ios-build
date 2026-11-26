use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::{comic_library, four_domains, media_library, AppHandle, State};
#[cfg(feature = "tauri")]
use tauri::Manager;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Minimal zero-dependency SHA-1 (FIPS 180-4) for E-Hentai cover image hash lookup (`f_shash`).
pub fn sha1_hex(data: &[u8]) -> String {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    format!("{h0:08x}{h1:08x}{h2:08x}{h3:08x}{h4:08x}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicExternalCandidate {
    pub source: String,
    pub external_id: String,
    pub external_url: Option<String>,
    pub title: String,
    pub authors: Vec<String>,
    pub circle: Option<String>,
    pub parody: Option<String>,
    pub tags: Vec<String>,
    pub rating: Option<f64>,
    #[serde(default)]
    pub published_at: Option<String>,
    pub summary: Option<String>,
    pub cover_url: Option<String>,
    pub match_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicBatchResultItem {
    pub work_id: String,
    pub title: String,
    pub ok: bool,
    pub source: Option<String>,
    pub tags_added: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ComicBatchProgress {
    pub running: bool,
    pub total: usize,
    pub completed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub current_title: Option<String>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub recent_items: Vec<ComicBatchResultItem>,
}

fn comic_batch_cell() -> &'static Mutex<ComicBatchProgress> {
    static CELL: OnceLock<Mutex<ComicBatchProgress>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(ComicBatchProgress::default()))
}

fn stop_comic_batch_flag() -> &'static AtomicBool {
    static FLAG: OnceLock<AtomicBool> = OnceLock::new();
    FLAG.get_or_init(|| AtomicBool::new(false))
}

pub fn get_comic_batch_scrape_progress() -> ComicBatchProgress {
    comic_batch_cell()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn stop_comic_batch_scrape() -> ComicBatchProgress {
    stop_comic_batch_flag().store(true, Ordering::SeqCst);
    get_comic_batch_scrape_progress()
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(16))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()
        .map_err(|e| e.to_string())
}

/// Curated E-Hentai tag -> Chinese tag dictionary for clean browsing & filtering.
fn translate_eh_tag(ns: &str, raw: &str) -> Option<String> {
    if ns == "language" {
        return match raw {
            "chinese" => Some("中文".to_string()),
            "japanese" => Some("日文".to_string()),
            "english" => Some("英文".to_string()),
            "translated" => None,
            _ => None,
        };
    }
    let zh = match raw {
        "full color" => "全彩",
        "uncensored" | "mosaic censorship" if raw == "uncensored" => "无修正",
        "big breasts" | "huge breasts" => "巨乳",
        "small breasts" => "贫乳",
        "sole female" => "单女主",
        "sole male" => "1v1",
        "schoolgirl uniform" => "JK制服",
        "maid" => "女仆",
        "stockings" | "pantyhose" => "丝袜",
        "glasses" => "眼镜",
        "milf" => "人妻/熟女",
        "elf" => "精灵",
        "kemonomimi" | "catgirl" | "fox girl" | "bunny girl" => "兽耳/兔女郎",
        "swimsuit" | "bikini" | "school swimsuit" => "泳装",
        "kimono" | "yukata" => "和服",
        "nurse" => "护士",
        "teacher" => "老师",
        "sister" | "incest" => "姐妹/骨科",
        "twintails" => "双马尾",
        "ponytail" => "马尾",
        "nakadashi" => "中出",
        "paizuri" => "乳交",
        "blowjob" => "口交",
        "ahegao" => "阿黑颜",
        "mind control" | "hypnosis" => "催眠",
        "netorare" => "NTR",
        "vanilla" => "纯爱",
        "harem" | "ffm threesome" => "后宫",
        "yuri" | "females only" => "百合",
        "lingerie" => "情趣内衣",
        "bondage" | "shibari" => "拘束",
        "tentacles" => "触手",
        "femdom" => "女王/调教",
        "gyaru" => "辣妹",
        "dark skin" => "黑皮",
        "demon girl" | "succubus" => "魅魔",
        "vtuber" => "虚拟主播",
        "cosplaying" => "Cosplay",
        "story arc" => "剧情向",
        "multi-work series" => "系列作",
        "anthology" => "合集",
        "tankoubon" => "单行本",
        "webtoon" => "条漫",
        "digital" => "DL版",
        "footjob" => "足交",
        "anal" => "后庭",
        "lactation" => "母乳",
        "pregnant" => "怀孕",
        "tsundere" => "傲娇",
        "yandere" => "病娇",
        "office lady" => "OL",
        "cheerleader" => "啦啦队",
        "miko" => "巫女",
        "nun" => "修女",
        "magical girl" => "魔法少女",
        "time stop" => "时间停止",
        "exhibitionism" => "露出",
        "cheating" => "出轨",
        "defloration" => "破处",
        "rimjob" => "舔肛",
        "x-ray" => "透视",
        "cross-section" => "断面图",
        _ => return None,
    };
    Some(zh.to_string())
}

fn extract_eh_gallery_links(html: &str, max_items: usize) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    let needle = "e-hentai.org/g/";
    let mut pos = 0;
    while let Some(idx) = html[pos..].find(needle) {
        let start = pos + idx + needle.len();
        let rest = &html[start..];
        if let Some(slash1) = rest.find('/') {
            let gid_str = &rest[..slash1];
            if let Ok(gid) = gid_str.parse::<u64>() {
                let after = &rest[slash1 + 1..];
                if let Some(slash2) = after.find('/') {
                    let token = &after[..slash2];
                    if token.len() == 10 && token.chars().all(|c| c.is_ascii_hexdigit()) {
                        let pair = (gid, token.to_string());
                        if !out.contains(&pair) {
                            out.push(pair);
                            if out.len() >= max_items {
                                break;
                            }
                        }
                    }
                }
            }
        }
        pos = start;
    }
    out
}

async fn fetch_eh_gdata(
    client: &reqwest::Client,
    gids: &[(u64, String)],
    match_reason: &str,
) -> Result<Vec<ComicExternalCandidate>, String> {
    if gids.is_empty() {
        return Ok(Vec::new());
    }
    let gidlist: Vec<serde_json::Value> = gids
        .iter()
        .map(|(g, t)| serde_json::json!([g, t]))
        .collect();
    let body = serde_json::json!({
        "method": "gdata",
        "gidlist": gidlist,
        "namespace": 1
    });

    let resp = client
        .post("https://api.e-hentai.org/api.php")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求 E-Hentai API 失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("E-Hentai API 状态码异常：{}", resp.status()));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 E-Hentai API 响应失败：{e}"))?;

    let mut out = Vec::new();
    if let Some(arr) = json.get("gmetadata").and_then(|v| v.as_array()) {
        for g in arr {
            if g.get("error").is_some() {
                continue;
            }
            let gid = g.get("gid").and_then(|v| v.as_u64()).unwrap_or(0);
            let token = g.get("token").and_then(|v| v.as_str()).unwrap_or("");
            let title_jpn = g.get("title_jpn").and_then(|v| v.as_str()).unwrap_or("").trim();
            let title_en = g.get("title").and_then(|v| v.as_str()).unwrap_or("").trim();
            let title = if !title_jpn.is_empty() { title_jpn } else { title_en };
            if title.is_empty() {
                continue;
            }
            let rating = g
                .get("rating")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<f64>().ok());
            let published_at = g
                .get("posted")
                .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                .map(|dt| dt.format("%Y-%m-%d").to_string());
            let cover_url = g.get("thumb").and_then(|v| v.as_str()).map(|s| s.to_string());

            let mut authors = Vec::new();
            let mut circle: Option<String> = None;
            let mut parody: Option<String> = None;
            let mut tags = Vec::new();

            if let Some(raw_tags) = g.get("tags").and_then(|v| v.as_array()) {
                for t in raw_tags {
                    if let Some(ts) = t.as_str() {
                        if let Some((ns, val)) = ts.split_once(':') {
                            match ns {
                                "artist" => {
                                    if !authors.contains(&val.to_string()) {
                                        authors.push(val.to_string());
                                    }
                                }
                                "group" => {
                                    if circle.is_none() {
                                        circle = Some(val.to_string());
                                    }
                                }
                                "parody" => {
                                    if parody.is_none() {
                                        parody = Some(if val == "original" {
                                            "原创".to_string()
                                        } else {
                                            val.to_string()
                                        });
                                    }
                                }
                                _ => {
                                    if let Some(zh) = translate_eh_tag(ns, val) {
                                        if !tags.contains(&zh) {
                                            tags.push(zh);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            out.push(ComicExternalCandidate {
                source: "ehentai".to_string(),
                external_id: format!("{gid}/{token}"),
                external_url: Some(format!("https://e-hentai.org/g/{gid}/{token}/")),
                title: title.to_string(),
                authors,
                circle,
                parody,
                tags,
                rating,
                published_at,
                summary: None,
                cover_url,
                match_reason: match_reason.to_string(),
            });
        }
    }
    Ok(out)
}

async fn search_ehentai_by_hash(
    client: &reqwest::Client,
    sha1: &str,
) -> Result<Vec<ComicExternalCandidate>, String> {
    let url = format!(
        "https://e-hentai.org/?f_shash={sha1}&fs_similar=on&fs_covers=on"
    );
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("E-Hentai 封面哈希搜索失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("E-Hentai 哈希搜索返回 {}", resp.status()));
    }
    let html = resp.text().await.map_err(|e| e.to_string())?;
    let links = extract_eh_gallery_links(&html, 4);
    fetch_eh_gdata(client, &links, "封面 SHA-1 精确命中").await
}

async fn search_ehentai_by_text(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<ComicExternalCandidate>, String> {
    let encoded = percent_encoding::utf8_percent_encode(query.trim(), percent_encoding::NON_ALPHANUMERIC).to_string();
    let url = format!("https://e-hentai.org/?f_search={encoded}");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("E-Hentai 文本搜索失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!("E-Hentai 文本搜索返回 {}", resp.status()));
    }
    let html = resp.text().await.map_err(|e| e.to_string())?;
    let links = extract_eh_gallery_links(&html, 5);
    fetch_eh_gdata(client, &links, "E-Hentai 文本匹配").await
}

async fn search_bangumi_manga(
    client: &reqwest::Client,
    query: &str,
) -> Result<Vec<ComicExternalCandidate>, String> {
    let encoded = percent_encoding::utf8_percent_encode(query.trim(), percent_encoding::NON_ALPHANUMERIC).to_string();
    let url = format!("https://api.bgm.tv/search/subject/{encoded}?type=1&responseGroup=large&max_results=5");
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Bangumi 搜索失败：{e}"))?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let json: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    let mut out = Vec::new();
    if let Some(list) = json.get("list").and_then(|v| v.as_array()) {
        for item in list {
            let id = item.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            let name_cn = item.get("name_cn").and_then(|v| v.as_str()).unwrap_or("").trim();
            let name_jp = item.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
            let title = if !name_cn.is_empty() { name_cn } else { name_jp };
            if title.is_empty() || id == 0 {
                continue;
            }
            let summary = item
                .get("summary")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let rating = item
                .get("rating")
                .and_then(|r| r.get("score"))
                .and_then(|v| v.as_f64())
                .filter(|s| *s > 0.0);
            let published_at = item
                .get("air_date")
                .or_else(|| item.get("date"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let cover_url = item
                .get("images")
                .and_then(|i| i.get("large").or_else(|| i.get("common")))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let mut tags = Vec::new();
            if let Some(arr) = item.get("tags").and_then(|v| v.as_array()) {
                for tg in arr.iter().take(10) {
                    if let Some(n) = tg.get("name").and_then(|v| v.as_str()) {
                        let clean = n.trim();
                        if !clean.is_empty() && !tags.contains(&clean.to_string()) {
                            tags.push(clean.to_string());
                        }
                    }
                }
            }

            out.push(ComicExternalCandidate {
                source: "bangumi".to_string(),
                external_id: id.to_string(),
                external_url: Some(format!("https://bgm.tv/subject/{id}")),
                title: title.to_string(),
                authors: Vec::new(),
                circle: None,
                parody: None,
                tags,
                rating,
                published_at,
                summary,
                cover_url,
                match_reason: "Bangumi 番组计划".to_string(),
            });
        }
    }
    Ok(out)
}

async fn compute_work_cover_sha1(
    app: &AppHandle,
    state: &State<'_, media_library::MediaLibraryState>,
    cover_asset_id: &str,
    cover_is_archive: bool,
) -> Option<String> {
    if !cover_is_archive {
        return None;
    }
    // Fetch original page 0 bytes via Range (within 12s timeout) for exact SHA-1
    let fut = media_library::read_comic_archive_page_binary(app, state, cover_asset_id, 0);
    if let Ok(Ok((_, bytes))) = tokio::time::timeout(Duration::from_secs(12), fut).await {
        if !bytes.is_empty() {
            return Some(sha1_hex(&bytes));
        }
    }
    None
}

/// Multi-source search for a single comic work (manual picker in Detail sheet/dialog).
pub async fn search_comic_online_sources(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    work_id: Option<String>,
    query: Option<String>,
    use_cover_hash: bool,
) -> Result<Vec<ComicExternalCandidate>, String> {
    let client = http_client()?;
    let mut results = Vec::new();

    let (search_text, cover_info) = if let Some(ref wid) = work_id {
        let detail = comic_library::get_comic_work_detail(&app, wid)?;
        let q = query
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if let Some(a) = detail.work.authors.first() {
                    format!("{} {}", detail.work.title, a)
                } else {
                    detail.work.title.clone()
                }
            });
        (q, detail.work.cover_asset_id.map(|id| (id, detail.work.cover_is_archive)))
    } else {
        (query.unwrap_or_default().trim().to_string(), None)
    };

    if use_cover_hash {
        if let Some((aid, is_arc)) = cover_info {
            if let Some(hash) = compute_work_cover_sha1(&app, &state, &aid, is_arc).await {
                if let Ok(mut hash_hits) = search_ehentai_by_hash(&client, &hash).await {
                    results.append(&mut hash_hits);
                }
            }
        }
    }

    if !search_text.is_empty() {
        let (eh_res, bgm_res) = tokio::join!(
            search_ehentai_by_text(&client, &search_text),
            search_bangumi_manga(&client, &search_text)
        );
        if let Ok(eh_hits) = eh_res {
            for item in eh_hits {
                if !results.iter().any(|r| r.source == item.source && r.external_id == item.external_id) {
                    results.push(item);
                }
            }
        }
        if let Ok(bgm_hits) = bgm_res {
            for item in bgm_hits {
                results.push(item);
            }
        }
    }

    Ok(results)
}

pub fn apply_comic_external_source_with_conn(
    conn: &Connection,
    work_id: &str,
    cand: &ComicExternalCandidate,
    cover_sha1: Option<&str>,
) -> Result<(), String> {
    comic_library::ensure_comic_library_schema(conn)?;
    let ts = now();

    // Load existing work to merge tags cleanly without erasing local attributes
    let existing: Option<(String, String, Option<String>, Option<String>, String)> = conn
        .query_row(
            "SELECT authors_json, tags_json, circle, parody, title FROM comic_works WHERE id = ?1",
            params![work_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .ok();

    let (old_authors_raw, old_tags_raw, old_circle, old_parody, old_title) =
        existing.ok_or_else(|| format!("未找到漫画作品：{work_id}"))?;

    let mut authors: Vec<String> = serde_json::from_str(&old_authors_raw).unwrap_or_default();
    if authors.is_empty() && !cand.authors.is_empty() {
        authors = cand.authors.clone();
    }
    let mut tags: Vec<String> = serde_json::from_str(&old_tags_raw).unwrap_or_default();
    for t in &cand.tags {
        if !tags.contains(t) {
            tags.push(t.clone());
        }
    }
    let circle = old_circle.or_else(|| cand.circle.clone());
    let parody = old_parody.or_else(|| cand.parody.clone());
    // Prefer Chinese Bangumi title if available, otherwise keep clean local base title
    let final_title = if cand.source == "bangumi" && !cand.title.trim().is_empty() {
        cand.title.trim().to_string()
    } else {
        old_title
    };

    let authors_json = serde_json::to_string(&authors).unwrap_or_else(|_| "[]".into());
    let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into());

    conn.execute(
        "INSERT INTO comic_external_meta (
            work_id, source, external_id, external_url, title, authors_json, circle, parody,
            tags_json, rating, published_at, summary, cover_sha1, updated_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
         ON CONFLICT(work_id) DO UPDATE SET
            source = excluded.source,
            external_id = excluded.external_id,
            external_url = excluded.external_url,
            title = excluded.title,
            authors_json = excluded.authors_json,
            circle = excluded.circle,
            parody = excluded.parody,
            tags_json = excluded.tags_json,
            rating = excluded.rating,
            published_at = COALESCE(excluded.published_at, comic_external_meta.published_at),
            summary = excluded.summary,
            cover_sha1 = COALESCE(excluded.cover_sha1, comic_external_meta.cover_sha1),
            updated_at = excluded.updated_at",
        params![
            work_id,
            &cand.source,
            &cand.external_id,
            &cand.external_url,
            &final_title,
            &authors_json,
            &circle,
            &parody,
            &tags_json,
            cand.rating,
            &cand.published_at,
            &cand.summary,
            cover_sha1,
            ts,
        ],
    )
    .map_err(|e| e.to_string())?;

    let is_uncensored = tags.iter().any(|t| t == "无修正");
    let is_full_color = tags.iter().any(|t| t == "全彩");

    conn.execute(
        "UPDATE comic_works SET
            title = ?2,
            authors_json = ?3,
            circle = ?4,
            parody = ?5,
            tags_json = ?6,
            is_uncensored = MAX(is_uncensored, ?7),
            is_full_color = MAX(is_full_color, ?8),
            source = ?9,
            rating = COALESCE(?10, rating),
            published_at = COALESCE(?11, published_at),
            summary = COALESCE(?12, summary),
            updated_at = ?13
         WHERE id = ?1",
        params![
            work_id,
            &final_title,
            &authors_json,
            &circle,
            &parody,
            &tags_json,
            if is_uncensored { 1 } else { 0 },
            if is_full_color { 1 } else { 0 },
            &cand.source,
            cand.rating,
            &cand.published_at,
            &cand.summary,
            ts,
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

pub fn apply_comic_external_source(
    app: &AppHandle,
    work_id: &str,
    cand: ComicExternalCandidate,
) -> Result<comic_library::ComicWorkDetail, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    apply_comic_external_source_with_conn(&conn, work_id, &cand, None)?;
    comic_library::get_comic_work_detail(app, work_id)
}

/// Start polite single-threaded background batch enrichment for all un-enriched (`source = 'local'`) works.
pub fn start_comic_batch_scrape(
    app: AppHandle,
    use_cover_hash: bool,
) -> Result<ComicBatchProgress, String> {
    {
        let mut g = comic_batch_cell().lock().map_err(|e| e.to_string())?;
        if g.running {
            return Ok(g.clone());
        }
        *g = ComicBatchProgress {
            running: true,
            total: 0,
            completed: 0,
            succeeded: 0,
            failed: 0,
            current_title: Some("准备待刮削队列…".to_string()),
            last_error: None,
            recent_items: Vec::new(),
        };
    }
    stop_comic_batch_flag().store(false, Ordering::SeqCst);

    tokio::spawn(async move {
        let queue: Vec<(String, String, Option<String>, Option<String>, bool)> = {
            let conn = match four_domains::open_v2(&app) {
                Ok(c) => c,
                Err(e) => {
                    if let Ok(mut g) = comic_batch_cell().lock() {
                        g.running = false;
                        g.last_error = Some(e);
                    }
                    return;
                }
            };
            let _ = comic_library::ensure_comic_library_schema(&conn);
            let mut list = Vec::new();
            if let Ok(mut stmt) = conn.prepare(
                "SELECT id, title, authors_json, cover_asset_id, cover_is_archive
                 FROM comic_works
                 WHERE source = 'local'
                 ORDER BY volume_count DESC, title ASC",
            ) {
                if let Ok(rows) = stmt.query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, i64>(4)? != 0,
                    ))
                }) {
                    for (id, title, authors_raw, cover_id, is_arc) in rows.flatten() {
                        let first_author = serde_json::from_str::<Vec<String>>(&authors_raw)
                            .ok()
                            .and_then(|v| v.into_iter().next());
                        list.push((id, title, first_author, cover_id, is_arc));
                    }
                }
            }
            list
        };

        if let Ok(mut g) = comic_batch_cell().lock() {
            g.total = queue.len();
        }

        let client = match http_client() {
            Ok(c) => c,
            Err(e) => {
                if let Ok(mut g) = comic_batch_cell().lock() {
                    g.running = false;
                    g.last_error = Some(e);
                }
                return;
            }
        };

        let state = app.state::<media_library::MediaLibraryState>();

        for (work_id, title, first_author, cover_id, is_arc) in queue {
            if stop_comic_batch_flag().load(Ordering::SeqCst) {
                break;
            }
            if let Ok(mut g) = comic_batch_cell().lock() {
                g.current_title = Some(title.clone());
            }

            let mut hit: Option<ComicExternalCandidate> = None;
            let mut computed_sha1: Option<String> = None;

            // 1. Cover SHA-1 exact lookup on E-Hentai (highest accuracy)
            if use_cover_hash {
                if let Some(ref cid) = cover_id {
                    if let Some(hash) = compute_work_cover_sha1(&app, &state, cid, is_arc).await {
                        computed_sha1 = Some(hash.clone());
                        if let Ok(list) = search_ehentai_by_hash(&client, &hash).await {
                            hit = list.into_iter().next();
                        }
                    }
                }
            }

            // 2. E-Hentai text search fallback
            if hit.is_none() {
                let q = match first_author.as_deref() {
                    Some(a) if !a.is_empty() => format!("{title} {a}"),
                    _ => title.clone(),
                };
                if let Ok(list) = search_ehentai_by_text(&client, &q).await {
                    hit = list.into_iter().next();
                }
            }

            // 3. Bangumi fallback (great for serialized manga)
            if hit.is_none() {
                if let Ok(list) = search_bangumi_manga(&client, &title).await {
                    hit = list.into_iter().next();
                }
            }

            let receipt = if let Some(cand) = hit {
                let tags_added = cand.tags.len();
                let src = cand.source.clone();
                let save_res = four_domains::open_v2(&app).and_then(|conn| {
                    apply_comic_external_source_with_conn(&conn, &work_id, &cand, computed_sha1.as_deref())
                });
                match save_res {
                    Ok(()) => ComicBatchResultItem {
                        work_id,
                        title,
                        ok: true,
                        source: Some(src),
                        tags_added,
                        error: None,
                    },
                    Err(e) => ComicBatchResultItem {
                        work_id,
                        title,
                        ok: false,
                        source: None,
                        tags_added: 0,
                        error: Some(e),
                    },
                }
            } else {
                ComicBatchResultItem {
                    work_id,
                    title,
                    ok: false,
                    source: None,
                    tags_added: 0,
                    error: Some("E-Hentai 与 Bangumi 均未命中".to_string()),
                }
            };

            if let Ok(mut g) = comic_batch_cell().lock() {
                g.completed += 1;
                if receipt.ok {
                    g.succeeded += 1;
                } else {
                    g.failed += 1;
                }
                g.recent_items.insert(0, receipt);
                if g.recent_items.len() > 40 {
                    g.recent_items.truncate(40);
                }
            }

            // Polite 2.5s delay to stay well within E-Hentai rate limits
            tokio::time::sleep(Duration::from_millis(2500)).await;
        }

        if let Ok(mut g) = comic_batch_cell().lock() {
            g.running = false;
            g.current_title = None;
        }
    });

    Ok(get_comic_batch_scrape_progress())
}
