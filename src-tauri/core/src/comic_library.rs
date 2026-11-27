use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use rand::seq::SliceRandom;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{four_domains, AppHandle};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn stable_hash(key: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicWorkItem {
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub circle: Option<String>,
    pub parody: Option<String>,
    pub event_tag: Option<String>,
    pub translators: Vec<String>,
    pub tags: Vec<String>,
    pub language: String,
    pub is_uncensored: bool,
    pub is_full_color: bool,
    pub is_dl: bool,
    pub is_ongoing: bool,
    pub cover_asset_id: Option<String>,
    pub cover_is_archive: bool,
    #[serde(default)]
    pub cover_page_index: i64,
    pub volume_count: i64,
    #[serde(default)]
    pub total_pages: i64,
    pub total_bytes: i64,
    pub source: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub rating: Option<f64>,
    pub summary: Option<String>,
    pub updated_at: i64,
    #[serde(default)]
    pub last_read_volume_id: Option<String>,
    #[serde(default)]
    pub last_read_volume_label: Option<String>,
    #[serde(default)]
    pub read_page: i64,
    #[serde(default)]
    pub read_total_pages: i64,
    #[serde(default)]
    pub read_percent: i64,
    #[serde(default)]
    pub read_status: String,
    #[serde(default)]
    pub last_read_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicVolumeItem {
    pub id: String,
    pub work_id: String,
    pub volume_label: String,
    pub sort_index: f64,
    pub is_archive: bool,
    pub asset_id: String,
    pub folder_path: Option<String>,
    pub file_name: String,
    pub remote_path: String,
    pub page_count: i64,
    pub size_bytes: i64,
    #[serde(default)]
    pub read_page: i64,
    #[serde(default)]
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicWorkDetail {
    pub work: ComicWorkItem,
    pub volumes: Vec<ComicVolumeItem>,
    pub related_by_author: Vec<ComicWorkItem>,
    pub related_by_parody: Vec<ComicWorkItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicLibraryPage {
    pub items: Vec<ComicWorkItem>,
    pub total: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicAuthorItem {
    pub name: String,
    pub circle: Option<String>,
    pub work_count: i64,
    pub volume_count: i64,
    pub cover_asset_id: Option<String>,
    pub cover_is_archive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicFilterFacet {
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicFilterOptions {
    pub categories: Vec<ComicFilterFacet>,
    pub genres: Vec<ComicFilterFacet>,
    pub years: Vec<ComicFilterFacet>,
    pub authors: Vec<ComicFilterFacet>,
    pub parodies: Vec<ComicFilterFacet>,
    pub tags: Vec<ComicFilterFacet>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComicLibraryStats {
    pub total_works: i64,
    pub total_volumes: i64,
    pub total_series: i64,
    pub total_authors: i64,
    pub zh_count: i64,
    pub raw_count: i64,
    pub uncensored_count: i64,
    pub archive_volumes: i64,
    pub folder_volumes: i64,
    #[serde(default)]
    pub enriched_count: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct QueryComicLibraryRequest {
    pub query: Option<String>,
    pub category: Option<String>,
    pub year: Option<String>,
    pub author: Option<String>,
    pub parody: Option<String>,
    pub tag: Option<String>,
    pub language: Option<String>,
    pub read_status: Option<String>,
    pub only_series: Option<bool>,
    pub only_uncensored: Option<bool>,
    pub only_full_color: Option<bool>,
    pub only_dl: Option<bool>,
    pub only_favorites: Option<bool>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveComicProgressRequest {
    pub work_id: String,
    pub volume_id: String,
    pub asset_id: String,
    pub current_page: i64,
    pub total_pages: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetComicCoverRequest {
    pub work_id: String,
    pub asset_id: Option<String>,
    pub page_index: i64,
}

pub fn ensure_comic_library_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS comic_works (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            raw_title TEXT NOT NULL,
            authors_json TEXT NOT NULL DEFAULT '[]',
            circle TEXT,
            parody TEXT,
            event_tag TEXT,
            translators_json TEXT NOT NULL DEFAULT '[]',
            tags_json TEXT NOT NULL DEFAULT '[]',
            language TEXT NOT NULL DEFAULT 'zh',
            is_uncensored INTEGER NOT NULL DEFAULT 0,
            is_full_color INTEGER NOT NULL DEFAULT 0,
            is_dl INTEGER NOT NULL DEFAULT 0,
            is_ongoing INTEGER NOT NULL DEFAULT 0,
            cover_asset_id TEXT,
            cover_is_archive INTEGER NOT NULL DEFAULT 0,
            volume_count INTEGER NOT NULL DEFAULT 1,
            total_bytes INTEGER NOT NULL DEFAULT 0,
            source TEXT NOT NULL DEFAULT 'local',
            summary TEXT,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS comic_volumes (
            id TEXT PRIMARY KEY,
            work_id TEXT NOT NULL REFERENCES comic_works(id) ON DELETE CASCADE,
            volume_label TEXT NOT NULL,
            sort_index REAL NOT NULL DEFAULT 0,
            is_archive INTEGER NOT NULL DEFAULT 1,
            asset_id TEXT NOT NULL,
            folder_path TEXT,
            file_name TEXT NOT NULL,
            remote_path TEXT NOT NULL,
            page_count INTEGER NOT NULL DEFAULT 0,
            size_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_comic_volumes_work_sort ON comic_volumes(work_id, sort_index);
        CREATE INDEX IF NOT EXISTS idx_comic_works_updated ON comic_works(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_comic_works_parody ON comic_works(parody);

        CREATE TABLE IF NOT EXISTS comic_reading_progress (
            work_id TEXT PRIMARY KEY,
            volume_id TEXT NOT NULL,
            volume_label TEXT NOT NULL DEFAULT '',
            asset_id TEXT NOT NULL,
            current_page INTEGER NOT NULL DEFAULT 0,
            total_pages INTEGER NOT NULL DEFAULT 0,
            completed INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS comic_volume_progress (
            volume_id TEXT PRIMARY KEY,
            work_id TEXT NOT NULL,
            current_page INTEGER NOT NULL DEFAULT 0,
            total_pages INTEGER NOT NULL DEFAULT 0,
            completed INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_comic_reading_updated ON comic_reading_progress(updated_at DESC);

        CREATE TABLE IF NOT EXISTS comic_external_meta (
            work_id TEXT PRIMARY KEY,
            source TEXT NOT NULL,
            external_id TEXT NOT NULL,
            external_url TEXT,
            title TEXT,
            authors_json TEXT NOT NULL DEFAULT '[]',
            circle TEXT,
            parody TEXT,
            tags_json TEXT NOT NULL DEFAULT '[]',
            rating REAL,
            published_at TEXT,
            summary TEXT,
            cover_sha1 TEXT,
            updated_at INTEGER NOT NULL
        );
        "#,
    )
    .map_err(|e| format!("初始化漫画库表结构失败：{e}"))?;

    // Additive column migrations on comic_works & comic_external_meta
    let ensure_col = |table: &str, col: &str, def: &str| {
        let mut found = false;
        if let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) {
                for c in rows.flatten() {
                    if c == col {
                        found = true;
                    }
                }
            }
        }
        if !found {
            let _ = conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"), []);
        }
    };
    ensure_col("comic_works", "rating", "REAL");
    ensure_col("comic_works", "category", "TEXT NOT NULL DEFAULT ''");
    ensure_col("comic_works", "published_at", "TEXT");
    ensure_col("comic_works", "cover_page_index", "INTEGER NOT NULL DEFAULT 0");
    ensure_col("comic_external_meta", "published_at", "TEXT");
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS comic_cover_overrides (
            work_id TEXT PRIMARY KEY,
            cover_asset_id TEXT NOT NULL,
            cover_page_index INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );
        "#,
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ============================================================================
// Folder-independent Filename Parser & Series Clustering
// ============================================================================

#[derive(Debug, Clone)]
struct ParsedComicMeta {
    base_title: String,
    volume_label: String,
    sort_index: f64,
    authors: Vec<String>,
    circle: Option<String>,
    parody: Option<String>,
    event_tag: Option<String>,
    translators: Vec<String>,
    tags: Vec<String>,
    language: String,
    is_uncensored: bool,
    is_full_color: bool,
    is_dl: bool,
    is_ongoing: bool,
    is_magazine_chapter: bool,
    is_cg_or_fanbox: bool,
}

fn strip_archive_ext(name: &str) -> String {
    let lower = name.to_lowercase();
    for ext in [".zip", ".cbz", ".rar", ".cbr", ".7z", ".pdf"] {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].trim().to_string();
        }
    }
    name.trim().to_string()
}

fn kanji_numeral_to_f64(s: &str) -> Option<f64> {
    match s {
        "一" => Some(1.0),
        "二" => Some(2.0),
        "三" => Some(3.0),
        "四" => Some(4.0),
        "五" => Some(5.0),
        "六" => Some(6.0),
        "七" => Some(7.0),
        "八" => Some(8.0),
        "九" => Some(9.0),
        "十" => Some(10.0),
        "十一" => Some(11.0),
        "十二" => Some(12.0),
        _ => None,
    }
}

fn is_translator_or_lang_token(token: &str) -> (bool, bool, bool) {
    // returns (is_lang_only, is_translator_group, is_chinese)
    let lower = token.to_lowercase();
    let is_zh = token.contains("中国")
        || token.contains("中文")
        || token.contains("漢化")
        || token.contains("汉化")
        || token.contains("翻訳")
        || token.contains("翻译")
        || token.contains("繁體")
        || token.contains("繁体")
        || token.contains("简体")
        || lower.contains("chinese")
        || lower == "chi"
        || lower == "chs"
        || lower == "cht";

    let is_lang_only = matches!(
        token.trim(),
        "中国翻訳" | "中国翻译" | "中文" | "汉化" | "漢化" | "Chinese" | "chinese" | "中文翻译" | "中文翻譯" | "日本語" | "English"
    );

    let is_translator = !is_lang_only
        && (token.contains("汉化")
            || token.contains("漢化")
            || token.contains("翻译")
            || token.contains("翻譯")
            || token.contains("翻訳")
            || token.contains("扫图")
            || token.contains("掃圖")
            || token.contains("重嵌")
            || token.contains("修图")
            || token.contains("嵌字")
            || token.contains("禁漫")
            || token.contains("空気系")
            || token.contains("个人")
            || token.contains("個人")
            || token.contains("字幕")
            || token.contains("社") && is_zh);

    (is_lang_only, is_translator, is_zh)
}

fn parse_leading_number(s: &str) -> Option<(f64, usize)> {
    let mut end = 0;
    let mut seen_dot = false;
    for (i, ch) in s.char_indices() {
        if ch.is_ascii_digit() {
            end = i + ch.len_utf8();
        } else if ch == '.' && !seen_dot && end > 0 {
            seen_dot = true;
            end = i + 1;
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let slice = s[..end].trim_end_matches('.');
    let val = slice.parse::<f64>().ok()?;
    Some((val, slice.len()))
}

fn is_chapter_folder_name(name: &str) -> Option<(String, f64)> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Pure number like "01", "100", "12.5"
    if let Ok(v) = trimmed.parse::<f64>() {
        return Some((trimmed.to_string(), v));
    }
    // 第100話 / 第2卷
    if let Some(rest) = trimmed.strip_prefix('第') {
        let rest = rest.trim_start();
        if let Some((v, len)) = parse_leading_number(rest) {
            let tail = rest[len..].trim();
            if matches!(tail, "話" | "话" | "巻" | "卷" | "章" | "集" | "回" | "部") {
                return Some((trimmed.to_string(), v));
            }
        }
    }
    // Ch 10 / Vol.2 / Ep 3 / Part 1
    let lower = trimmed.to_lowercase();
    for pref in ["ch.", "ch", "vol.", "vol", "ep.", "ep", "part.", "part"] {
        if let Some(rest) = lower.strip_prefix(pref) {
            let rest = rest.trim_start();
            if let Some((v, len)) = parse_leading_number(rest) {
                if rest[len..].trim().is_empty() {
                    return Some((trimmed.to_string(), v));
                }
            }
        }
    }
    None
}

fn strip_volume_suffix(raw_title: &str) -> (String, Option<(String, f64)>) {
    let title = raw_title.trim().to_string();
    if title.is_empty() {
        return (title, None);
    }

    // 1. Trailing 第1-69話 / 第100話 / 第2巻 / 第二楽章
    if let Some(pos) = title.rfind('第') {
        let tail = &title[pos..];
        let after_di = tail["第".len()..].trim_start();
        if let Some((num, nlen)) = parse_leading_number(after_di) {
            let rem = after_di[nlen..].trim_start();
            // Optional range like "-69" or "~69"
            let rem2 = if let Some(r) = rem.strip_prefix('-').or_else(|| rem.strip_prefix('~')).or_else(|| rem.strip_prefix('～')) {
                let r = r.trim_start();
                if let Some((_, len2)) = parse_leading_number(r) {
                    r[len2..].trim()
                } else {
                    rem.trim()
                }
            } else {
                rem.trim()
            };
            if matches!(rem2, "話" | "话" | "巻" | "卷" | "章" | "集" | "部" | "弾" | "弹") {
                let head = title[..pos].trim().to_string();
                if !head.is_empty() {
                    return (head, Some((tail.trim().to_string(), num)));
                }
            }
        } else {
            for (k, val) in [
                ("十二", 12.0), ("十一", 11.0), ("十", 10.0), ("九", 9.0), ("八", 8.0),
                ("七", 7.0), ("六", 6.0), ("五", 5.0), ("四", 4.0), ("三", 3.0), ("二", 2.0), ("一", 1.0),
            ] {
                if let Some(rem) = after_di.strip_prefix(k) {
                    let rem = rem.trim();
                    if matches!(rem, "楽章" | "乐章" | "章" | "話" | "话" | "巻" | "卷" | "部" | "幕") {
                        let head = title[..pos].trim().to_string();
                        if !head.is_empty() {
                            return (head, Some((tail.trim().to_string(), val)));
                        }
                    }
                }
            }
        }
    }

    // 2. Trailing Vol.2 / Part 3 / Ch.11-14 / #2
    let lower = title.to_lowercase();
    for kw in ["vol.", "vol ", "part.", "part ", "ch.", "ch ", "ep.", "ep ", "#"] {
        if let Some(pos) = lower.rfind(kw) {
            let after = title[pos + kw.len()..].trim_start();
            if let Some((num, nlen)) = parse_leading_number(after) {
                let rem = after[nlen..].trim();
                let valid_tail = rem.is_empty()
                    || rem
                        .strip_prefix('-')
                        .or_else(|| rem.strip_prefix('~'))
                        .or_else(|| rem.strip_prefix('～'))
                        .and_then(|r| parse_leading_number(r.trim_start()))
                        .map(|(_, l2)| rem[1..].trim_start()[l2..].trim().is_empty())
                        .unwrap_or(false);
                if valid_tail {
                    let head = title[..pos].trim().to_string();
                    if head.chars().count() >= 2 {
                        return (head, Some((title[pos..].trim().to_string(), num)));
                    }
                }
            }
        }
    }

    // 3. Trailing 前編/後編/上/中/下/総集編 (both bare and inside 【...】)
    for (suf, idx) in [
        ("【完結編】", 4.0), ("【完结篇】", 4.0), ("【総集編】", 99.0), ("【总集篇】", 99.0),
        ("【番外編】", 90.0), ("【前編】", 1.0), ("【前篇】", 1.0), ("【中編】", 2.0), ("【中篇】", 2.0),
        ("【後編】", 3.0), ("【后篇】", 3.0), ("【上巻】", 1.0), ("【中巻】", 2.0), ("【下巻】", 3.0),
        ("【上】", 1.0), ("【中】", 2.0), ("【下】", 3.0),
        ("完結編", 4.0), ("完结篇", 4.0), ("総集編", 99.0), ("总集篇", 99.0),
        ("番外編", 90.0), ("外伝", 90.0), ("前編", 1.0), ("前篇", 1.0),
        ("中編", 2.0), ("中篇", 2.0), ("後編", 3.0), ("后篇", 3.0),
        ("上巻", 1.0), ("上卷", 1.0), ("中巻", 2.0), ("中卷", 2.0), ("下巻", 3.0), ("下卷", 3.0),
        ("上", 1.0), ("中", 2.0), ("下", 3.0),
    ] {
        if let Some(rest) = title.strip_suffix(suf) {
            let head = rest.trim().to_string();
            if head.chars().count() >= 3 {
                let clean_label = suf.trim_matches(&['【', '】'][..]).to_string();
                return (head, Some((clean_label, idx)));
            }
        }
    }

    // 3b. Classical kanji volume markers like `-壱- ...`, `-弐- ...`, `-参- ...`
    for (marker, label, idx) in [
        ("-壱-", "壱", 1.0), ("-弐-", "弐", 2.0), ("-参-", "参", 3.0), ("-肆-", "肆", 4.0), ("-伍-", "伍", 5.0),
        ("－壱－", "壱", 1.0), ("－弐－", "弐", 2.0), ("－参－", "参", 3.0),
    ] {
        if let Some(pos) = title.find(marker) {
            let head = title[..pos].trim().to_string();
            if head.chars().count() >= 2 {
                let tail = title[pos..].trim().to_string();
                let vlabel = if tail.chars().count() <= 24 { tail } else { format!("第 {label} 卷") };
                return (head, Some((vlabel, idx)));
            }
        }
    }

    // 3c. Trailing circled numerals ①..⑩
    for (ch, idx) in [
        ('①', 1.0), ('②', 2.0), ('③', 3.0), ('④', 4.0), ('⑤', 5.0),
        ('⑥', 6.0), ('⑦', 7.0), ('⑧', 8.0), ('⑨', 9.0), ('⑩', 10.0),
    ] {
        if let Some(rest) = title.strip_suffix(ch) {
            let head = rest.trim().to_string();
            if head.chars().count() >= 2 {
                return (head, Some((format!("第 {} 卷", idx as i64), idx)));
            }
        }
    }

    // 3d. Trailing Roman numerals (full-width Ⅰ..Ⅹ and ASCII II/III/IV/V/VI/VII/VIII/IX/X)
    for (rom, idx) in [
        ("Ⅹ", 10.0), ("Ⅸ", 9.0), ("Ⅷ", 8.0), ("Ⅶ", 7.0), ("Ⅵ", 6.0),
        ("Ⅴ", 5.0), ("Ⅳ", 4.0), ("Ⅲ", 3.0), ("Ⅱ", 2.0), ("Ⅰ", 1.0),
    ] {
        if let Some(rest) = title.strip_suffix(rom) {
            let head = rest.trim().to_string();
            if head.chars().count() >= 2 {
                return (head, Some((rom.to_string(), idx)));
            }
        }
    }
    for (rom, idx, need_space) in [
        ("VIII", 8.0, false), ("VII", 7.0, false), ("III", 3.0, false),
        ("VI", 6.0, false), ("IV", 4.0, false), ("IX", 9.0, false), ("II", 2.0, false),
        ("X", 10.0, true), ("V", 5.0, true), ("I", 1.0, true),
    ] {
        if let Some(rest) = title.strip_suffix(rom) {
            let prev_char = rest.chars().last();
            let boundary_ok = match prev_char {
                Some(c) if c.is_whitespace() || c == '-' || c == '_' => true,
                Some(c) if !need_space && !c.is_ascii_alphabetic() => true,
                _ => false,
            };
            if boundary_ok {
                let head = rest.trim().trim_end_matches(&['-', '_'][..]).trim().to_string();
                if head.chars().count() >= 2 {
                    return (head, Some((rom.to_string(), idx)));
                }
            }
        }
    }

    // 4. Trailing standalone small volume number (1..40), e.g. `ぱい☆パニック ～挟まれたデカぱい～4`
    let mut digit_start = title.len();
    for (i, ch) in title.char_indices().rev() {
        if ch.is_ascii_digit() {
            digit_start = i;
        } else {
            break;
        }
    }
    if digit_start < title.len() {
        let num_str = &title[digit_start..];
        if num_str.len() <= 2 {
            if let Ok(n) = num_str.parse::<i64>() {
                if (1..=40).contains(&n) {
                    let head = title[..digit_start]
                        .trim()
                        .trim_end_matches(&['-', '_', ' '][..])
                        .trim()
                        .to_string();
                    if head.chars().count() >= 4 {
                        return (head, Some((format!("第 {n} 卷"), n as f64)));
                    }
                }
            }
        }
    }

    (title, None)
}

fn split_authors(raw: &str) -> Vec<String> {
    raw.split(|c| c == ',' || c == '、' || c == '＆' || c == '&' || c == '／')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.chars().count() <= 40)
        .collect()
}

fn extract_bracket_spans(s: &str, open: char, close: char) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, ch) in s.char_indices() {
        if ch == open && start.is_none() {
            start = Some(i);
        } else if ch == close {
            if let Some(st) = start.take() {
                let inner_start = st + open.len_utf8();
                out.push((st, i + close.len_utf8(), s[inner_start..i].trim().to_string()));
            }
        }
    }
    out
}

fn split_circle_and_author(inner: &str) -> (Option<String>, Vec<String>) {
    let trimmed = inner.trim();
    for (open, close) in [('(', ')'), ('（', '）')] {
        if trimmed.ends_with(close) {
            if let Some(pos) = trimmed.rfind(open) {
                let c_str = trimmed[..pos].trim();
                let a_str = trimmed[pos + open.len_utf8()..trimmed.len() - close.len_utf8()].trim();
                if !c_str.is_empty() && !a_str.is_empty() {
                    return (Some(c_str.to_string()), split_authors(a_str));
                }
            }
        }
    }
    (None, split_authors(trimmed))
}

fn is_event_token(s: &str) -> bool {
    let up = s.trim().replace(' ', "").to_uppercase();
    for pref in ["COMITIA", "SC", "C", "例大祭", "紅楼夢", "こみトレ", "サンクリ"] {
        if let Some(rest) = up.strip_prefix(pref) {
            if !rest.is_empty() && rest.chars().take(2).all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

fn is_magazine_paren(inner: &str) -> bool {
    let lower = inner.to_lowercase();
    lower.starts_with("comic ")
        || lower.starts_with("comic") && (inner.contains("月号") || inner.contains("快楽天") || inner.contains("失楽天") || inner.contains("アンスリウム") || lower.contains("bavel") || lower.contains("x-eros"))
        || inner.contains("快楽天")
        || inner.contains("失楽天")
        || inner.contains("月号")
        || inner.contains("マガジン")
        || lower.contains("shitsurakuten")
        || lower.contains("kairakuten")
        || lower.contains("anthurium")
        || lower.contains("bavel")
        || lower.contains("x-eros")
}

fn is_cg_or_fanbox_token(inner: &str) -> bool {
    let lower = inner.to_lowercase();
    lower.contains("fanbox")
        || lower.contains("fantia")
        || lower.contains("patreon")
        || lower.contains("artist cg")
        || lower == "cg"
        || inner.contains("CG集")
        || inner.contains("画集")
        || inner.contains("イラスト集")
}

fn is_valid_parody_token(inner: &str) -> bool {
    let t = inner.trim();
    if t.chars().count() <= 1 {
        return false;
    }
    if t.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == ' ') {
        return false;
    }
    let (is_lang, is_trans, is_zh) = is_translator_or_lang_token(t);
    if is_lang || is_trans || is_zh {
        return false;
    }
    if is_event_token(t) || is_magazine_paren(t) || is_cg_or_fanbox_token(t) {
        return false;
    }
    let lower = t.to_lowercase();
    if matches!(
        lower.as_str(),
        "scbk9" | "chinese" | "english" | "japanese" | "digital" | "dl" | "dl版" | "raw" | "uncensored" | "decensored"
    ) {
        return false;
    }
    for bad in [
        "同人誌", "同人志", "成年コミック", "一般コミック", "単行本", "单行本",
        "無修正", "无修正", "修正", "催眠で", "仮", "完整版", "合集", "高画质",
        "高清", "重嵌", "扫图", "掃圖", "梓喵打酱油", "个人汉化", "個人漢化",
    ] {
        if t == bad || t.contains(bad) {
            return false;
        }
    }
    true
}

fn is_technical_tag(tag: &str) -> bool {
    let t = tag.trim();
    matches!(
        t,
        "DL版" | "无修正" | "無修正" | "全彩" | "连载中" | "单行本" | "同人志" | "中文" | "汉化" | "生肉" | "CG/画集"
    ) || is_event_token(t)
}

fn infer_comic_genres(title: &str, raw_title: &str) -> Vec<String> {
    let hay = format!("{title} {raw_title}");
    let lower = hay.to_lowercase();
    let rules: &[(&str, &[&str])] = &[
        ("姐妹/家庭", &["妹", "義妹", "姉", "義姉", "お姉ちゃん", "従妹", "姉妹", "実妹", "家族"]),
        ("校园", &["学園", "学园", "学校", "高校", "女子校", "部活", "放課後", "教師", "先生", "同級生", "先輩", "後輩", "クラス", "制服", "生徒", "jk"]),
        ("纯爱", &["純愛", "纯爱", "イチャイチャ", "恋愛", "恋爱", "求愛", "恋人", "彼女", "ラブラブ", "甘い", "両想い", "初恋", "微熱"]),
        ("人妻/熟女", &["人妻", "妻", "未亡人", "奥さん", "熟女", "ママ", "義母", "おばさん", "団地妻"]),
        ("青梅竹马", &["幼馴染", "幼なじみ", "青梅竹马", "青梅竹馬"]),
        ("辣妹/Gal", &["ギャル", "辣妹", "ビッチ", "黒ギャル"]),
        ("奇幻/异世界", &["ダンジョン", "聖女", "魔王", "エルフ", "異世界", "异世界", "魔法", "冒険", "勇者", "ファンタジー", "サキュバス", "魔物", "騎士", "精霊"]),
        ("催眠/时停", &["催眠", "時間停止", "时间停止", "洗脳", "洗脑", "常識改変", "常识改变"]),
        ("职场/OL", &["ol", "上司", "部下", "会社", "秘書", "バイト", "店長", "女教師", "ナース", "看護師", "メイド", "泡姫"]),
        ("NTR/出轨", &["ntr", "寝取", "浮気", "不倫", "間男", "裏切"]),
        ("百合", &["百合", "レズ", "ガールズラブ"]),
        ("调教/凌辱", &["拷問", "調教", "调教", "触手", "拘束", "奴隷", "奴隶", "凌辱", "犯す", "犯さ", "監禁", "监禁", "痴漢", "堕ち", "快楽堕ち", "性欲処理"]),
        ("后宫/多P", &["ハーレム", "后宫", "後宮", "3p", "乱交", "シェアハウス"]),
    ];
    let mut out = Vec::new();
    for (genre, keywords) in rules {
        if keywords.iter().any(|kw| {
            if kw.is_ascii() {
                lower.contains(kw)
            } else {
                hay.contains(kw)
            }
        }) {
            out.push((*genre).to_string());
        }
    }
    out
}

fn parse_comic_filename(raw_name: &str, remote_path: &str, explicit_volume: Option<(String, f64)>) -> ParsedComicMeta {
    let mut work = strip_archive_ext(raw_name);
    let mut event_tag: Option<String> = None;
    let mut circle: Option<String> = None;
    let mut authors: Vec<String> = Vec::new();
    let mut parody: Option<String> = None;
    let mut translators: Vec<String> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    let mut language = if remote_path.contains("未汉化") { "ja".to_string() } else { "unknown".to_string() };
    let mut is_uncensored = false;
    let mut is_full_color = false;
    let mut is_dl = false;
    let mut is_ongoing = false;
    let mut is_magazine_chapter = false;
    let mut is_cg_or_fanbox = is_cg_or_fanbox_token(&work) || is_cg_or_fanbox_token(remote_path);
    let mut bracket_volume: Option<(String, f64)> = explicit_volume;

    // 1. Leading (...) tag like (C107) / (COMITIA145) / (同人誌) / (成年コミック)
    loop {
        let trimmed_start = work.trim_start().to_string();
        let mut stripped = false;
        for (open, close) in [('(', ')'), ('（', '）')] {
            if let Some(after_open) = trimmed_start.strip_prefix(open) {
                if let Some(end_idx) = after_open.find(close) {
                    let inner = after_open[..end_idx].trim();
                    if is_event_token(inner) {
                        event_tag = Some(inner.replace(' ', "").to_uppercase());
                        work = after_open[end_idx + close.len_utf8()..].trim().to_string();
                        stripped = true;
                        break;
                    } else if matches!(inner, "同人誌" | "同人志" | "成年コミック" | "一般コミック" | "商业志" | "商業誌") {
                        work = after_open[end_idx + close.len_utf8()..].trim().to_string();
                        stripped = true;
                        break;
                    }
                }
            }
        }
        if !stripped {
            break;
        }
    }

    // 2. Extract all [...] blocks
    {
        let spans = extract_bracket_spans(&work, '[', ']');
        let mut to_remove: Vec<(usize, usize)> = Vec::new();
        for (start, end, inner) in spans {
            if inner.is_empty() {
                to_remove.push((start, end));
                continue;
            }
            let lower = inner.to_lowercase();
            let (is_lang_only, is_translator, is_zh) = is_translator_or_lang_token(&inner);
            if is_zh {
                language = "zh".to_string();
            }
            if lower.contains("無修正") || lower.contains("无修正") || lower.contains("无修") || lower.contains("decensored") || lower.contains("uncensored") {
                is_uncensored = true;
                to_remove.push((start, end));
                continue;
            }
            if lower == "dl版" || lower == "digital" || lower.contains("dl版") {
                is_dl = true;
                to_remove.push((start, end));
                continue;
            }
            if inner.contains("進行中") || inner.contains("进行中") || inner.contains("连载中") || lower == "ongoing" {
                is_ongoing = true;
                to_remove.push((start, end));
                continue;
            }
            if inner.contains("フルカラー") || inner.contains("全彩") || lower.contains("full color") {
                is_full_color = true;
                to_remove.push((start, end));
                continue;
            }
            if is_lang_only {
                to_remove.push((start, end));
                continue;
            }
            if is_translator {
                if !translators.contains(&inner) {
                    translators.push(inner);
                }
                to_remove.push((start, end));
                continue;
            }
            if lower.starts_with("ch")
                || lower.starts_with("vol")
                || inner.contains("完结")
                || inner.contains("完結")
                || inner.contains("分p")
            {
                if bracket_volume.is_none() {
                    let num_part = inner
                        .trim_start_matches(|c: char| !c.is_ascii_digit());
                    let idx = parse_leading_number(num_part).map(|(v, _)| v).unwrap_or(0.0);
                    bracket_volume = Some((inner, idx));
                }
                to_remove.push((start, end));
                continue;
            }

            if authors.is_empty() && circle.is_none() {
                let (c, a) = split_circle_and_author(&inner);
                circle = c;
                authors = a;
                to_remove.push((start, end));
            }
        }
        to_remove.sort_by(|a, b| b.0.cmp(&a.0));
        for (s, e) in to_remove {
            work.replace_range(s..e, " ");
        }
    }

    // 3. Extract (...) and （...） blocks for parody / magazine chapter / CG / full-color / DL
    for (open, close) in [('(', ')'), ('（', '）')] {
        let spans = extract_bracket_spans(&work, open, close);
        let mut to_remove: Vec<(usize, usize)> = Vec::new();
        for (start, end, inner) in spans {
            if inner.is_empty() {
                to_remove.push((start, end));
                continue;
            }
            let lower = inner.to_lowercase();
            if inner.contains("フルカラー") || inner.contains("全彩") || lower.contains("full color") {
                is_full_color = true;
                to_remove.push((start, end));
            } else if lower == "dl版" || lower == "digital" {
                is_dl = true;
                to_remove.push((start, end));
            } else if is_magazine_paren(&inner) {
                is_magazine_chapter = true;
                to_remove.push((start, end));
            } else if is_cg_or_fanbox_token(&inner) {
                is_cg_or_fanbox = true;
                to_remove.push((start, end));
            } else if start > 2 && parody.is_none() {
                if inner == "オリジナル" || lower == "original" {
                    parody = Some("原创".to_string());
                    to_remove.push((start, end));
                } else if is_valid_parody_token(&inner) {
                    parody = Some(inner);
                    to_remove.push((start, end));
                }
            }
        }
        to_remove.sort_by(|a, b| b.0.cmp(&a.0));
        for (s, e) in to_remove {
            work.replace_range(s..e, " ");
        }
    }

    // Clean whitespace
    let cleaned_title = work
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();

    let (base_title, suffix_vol) = strip_volume_suffix(&cleaned_title);
    let final_title = if base_title.is_empty() {
        strip_archive_ext(raw_name)
    } else {
        base_title
    };

    let (volume_label, sort_index) = match (bracket_volume, suffix_vol) {
        (Some(v), _) => v,
        (None, Some(v)) => v,
        (None, None) => ("单本".to_string(), 0.0),
    };

    if language == "unknown" {
        if !translators.is_empty() || remote_path.contains("汉化") || remote_path.contains("中文") {
            language = "zh".to_string();
        } else {
            language = "ja".to_string();
        }
    }

    if is_uncensored {
        tags.push("无修正".to_string());
    }
    if is_full_color {
        tags.push("全彩".to_string());
    }
    if is_dl {
        tags.push("DL版".to_string());
    }
    if is_ongoing {
        tags.push("连载中".to_string());
    }
    if let Some(ref ev) = event_tag {
        tags.push(ev.clone());
    }

    if is_cg_or_fanbox {
        tags.push("CG/画集".to_string());
    }
    for g in infer_comic_genres(&final_title, raw_name) {
        if !tags.contains(&g) {
            tags.push(g);
        }
    }

    ParsedComicMeta {
        base_title: final_title,
        volume_label,
        sort_index,
        authors,
        circle,
        parody,
        event_tag,
        translators,
        tags,
        language,
        is_uncensored,
        is_full_color,
        is_dl,
        is_ongoing,
        is_magazine_chapter,
        is_cg_or_fanbox,
    }
}

fn infer_published_at(event_tag: Option<&str>, title: &str, raw_title: &str) -> Option<String> {
    // 1. Comiket event number -> accurate year-month (Winter Dec / Summer Aug)
    if let Some(ev) = event_tag {
        let up = ev.to_uppercase();
        if let Some(num_str) = up.strip_prefix('C') {
            if let Ok(c) = num_str.parse::<i32>() {
                let ym = match c {
                    70..=97 => {
                        // C70=2006-08, C71=2006-12, ..., C97=2019-12
                        let year = 2006 + (c - 70) / 2;
                        let month = if c % 2 == 0 { "08" } else { "12" };
                        Some(format!("{year}-{month}"))
                    }
                    98 => Some("2020-05".to_string()),
                    99 => Some("2021-12".to_string()),
                    100 => Some("2022-08".to_string()),
                    101 => Some("2022-12".to_string()),
                    102 => Some("2023-08".to_string()),
                    103 => Some("2023-12".to_string()),
                    104 => Some("2024-08".to_string()),
                    105 => Some("2024-12".to_string()),
                    106 => Some("2025-08".to_string()),
                    107 => Some("2025-12".to_string()),
                    108 => Some("2026-08".to_string()),
                    109 => Some("2026-12".to_string()),
                    _ => None,
                };
                if ym.is_some() {
                    return ym;
                }
            }
        }
    }

    // 2. Explicit YYYY-MM or YYYY年M月 or standalone 2005..2026 in title/filename
    for hay in [raw_title, title] {
        let bytes = hay.as_bytes();
        for i in 0..bytes.len().saturating_sub(3) {
            if bytes[i] == b'2' && bytes[i + 1] == b'0' && bytes[i + 2].is_ascii_digit() && bytes[i + 3].is_ascii_digit() {
                if let Ok(year) = hay[i..i + 4].parse::<i32>() {
                    if (2005..=2026).contains(&year) {
                        let after = &hay[i + 4..];
                        if let Some(rest) = after.strip_prefix('-').or_else(|| after.strip_prefix('.')).or_else(|| after.strip_prefix('年')) {
                            if let Some((m, _)) = parse_leading_number(rest.trim_start()) {
                                let mi = m as i32;
                                if (1..=12).contains(&mi) {
                                    return Some(format!("{year}-{mi:02}"));
                                }
                            }
                        }
                        return Some(format!("{year}"));
                    }
                }
            }
        }
    }
    None
}

fn infer_comic_category(
    title: &str,
    raw_title: &str,
    raw_path: &str,
    event_tag: Option<&str>,
    circle: Option<&str>,
    parody: Option<&str>,
    is_ongoing: bool,
    is_full_color: bool,
    is_magazine_chapter: bool,
    is_cg_or_fanbox: bool,
    volume_count: i64,
    tags: &[String],
) -> String {
    let lower = title.to_lowercase();
    if !is_magazine_chapter
        && (lower.starts_with("comic ")
            || lower.contains("shitsurakuten")
            || lower.contains("kairakuten")
            || lower.contains("x-eros")
            || lower.contains("bavel")
            || lower.contains("anthurium")
            || title.contains("快楽天")
            || title.contains("失楽天")
            || title.contains("月号")
            || title.contains("マガジン"))
    {
        return "杂志合集".to_string();
    }
    if is_full_color || is_cg_or_fanbox {
        return "全彩 / CG".to_string();
    }
    if is_ongoing || volume_count >= 5 || raw_path.contains("长篇") || raw_path.contains("日漫") {
        return "连载长篇".to_string();
    }
    if raw_path.contains("短篇") || is_magazine_chapter {
        return "短篇".to_string();
    }
    if volume_count >= 2 || raw_path.contains("系列合集") {
        return "系列合集".to_string();
    }
    if event_tag.is_some()
        || circle.is_some()
        || parody.map(|p| p != "原创" && !p.is_empty()).unwrap_or(false)
        || raw_title.contains("同人誌")
        || raw_title.contains("同人志")
    {
        return "同人志".to_string();
    }
    if tags.iter().any(|t| t == "单行本") {
        return "单行本".to_string();
    }
    "单行本".to_string()
}

fn normalize_cluster_text(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && !"～~ー-—_・☆★!！?？:：;；,.，。/／()（）[]【】「」『』".contains(*c))
        .collect()
}

fn extract_series_folder_title(remote_path: &str) -> Option<(String, Option<String>, Vec<String>)> {
    if !remote_path.contains("系列合集") {
        return None;
    }
    let parent_dir = remote_path.rsplit_once('/').map(|(p, _)| p)?;
    let folder_name = parent_dir.rsplit('/').next()?.trim();
    if folder_name.is_empty() || folder_name.contains("系列合集") {
        return None;
    }
    // Skip pure year folders (e.g. "2007", "2019") and magazine folders
    if folder_name.len() == 4 && folder_name.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if is_magazine_paren(folder_name) || folder_name.to_lowercase().starts_with("comic ") {
        return None;
    }
    // Unwrap outer full-width parentheses like `（[大田優一] ねことげぼく）` or `（百濁の塔）`
    let unwrapped = if folder_name.starts_with('（') && folder_name.ends_with('）') {
        folder_name
            .strip_prefix('（')
            .and_then(|s| s.strip_suffix('）'))
            .unwrap_or(folder_name)
            .trim()
    } else {
        folder_name
    };
    // Skip pure author/circle folders like `【おかゆさん】`, `[70年式悠久機関 (おはぎさん)]`, `【じゃじゅじょ (じょぶじゅん。)】`
    if (unwrapped.starts_with('【') && unwrapped.ends_with('】'))
        || (unwrapped.starts_with('[') && unwrapped.ends_with(']'))
    {
        return None;
    }
    let parsed = parse_comic_filename(unwrapped, remote_path, None);
    let base = parsed.base_title.trim();
    if base.chars().count() < 2 {
        return None;
    }
    Some((base.to_string(), parsed.circle, parsed.authors))
}

fn is_ad_page_entry(entry_name: &str) -> bool {
    let lower = entry_name.to_lowercase();
    for kw in [
        "credit", "advert", "promo", "t.me", "telegram", "notice", "disclaimer",
        "discord", "patreon", "qq群", "招募", "宣传", "宣傳", "广告", "廣告",
        "汉化组", "漢化組", "说明", "說明", "声明", "聲明",
    ] {
        if lower.contains(kw) {
            return true;
        }
    }
    false
}

fn pick_smart_cover_page(pages: &[(i64, String, i64)]) -> i64 {
    if pages.is_empty() {
        return 0;
    }
    let max_later_size = pages
        .iter()
        .skip(1)
        .take(4)
        .map(|(_, _, sz)| *sz)
        .max()
        .unwrap_or(0);

    for (idx, name, sz) in pages.iter().take(5) {
        if is_ad_page_entry(name) {
            continue;
        }
        // Tiny text/ad banner page (< 45 KB) when a subsequent page is >= 80 KB and >= 3x larger
        if *sz > 0 && *sz < 45_000 && max_later_size >= 80_000 && max_later_size >= sz.saturating_mul(3) {
            continue;
        }
        return *idx;
    }
    0
}

// ============================================================================
// Rebuild Comic Library (Folder-Independent Clustering)
// ============================================================================

fn configured_comic_paths(conn: &Connection) -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT remote_path, COALESCE(extra_paths_json, '[]') FROM library_modules WHERE kind = 'comics' AND enabled = 1",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
            for row in rows.flatten() {
                let p0 = row.0.trim().trim_end_matches('/').to_string();
                if !p0.is_empty() && !paths.contains(&p0) {
                    paths.push(p0);
                }
                if let Ok(extras) = serde_json::from_str::<Vec<String>>(&row.1) {
                    for ex in extras {
                        let clean = ex.trim().trim_end_matches('/').to_string();
                        if !clean.is_empty() && !paths.contains(&clean) {
                            paths.push(clean);
                        }
                    }
                }
            }
        }
    }
    if paths.is_empty() {
        if let Ok(Some(p)) = conn
            .query_row("SELECT remote_path FROM domain_roots WHERE domain = 'comics'", [], |r| r.get::<_, String>(0))
            .optional()
        {
            let clean = p.trim().trim_end_matches('/').to_string();
            if !clean.is_empty() {
                paths.push(clean);
            }
        }
    }
    paths
}

fn path_matches_any(remote_path: &str, roots: &[String]) -> bool {
    roots.iter().any(|r| {
        remote_path == r || remote_path.starts_with(&format!("{r}/"))
    })
}

struct WorkAccumulator {
    id: String,
    title: String,
    raw_title: String,
    authors: BTreeSet<String>,
    circle: Option<String>,
    parody: Option<String>,
    event_tag: Option<String>,
    translators: BTreeSet<String>,
    tags: BTreeSet<String>,
    language: String,
    is_uncensored: bool,
    is_full_color: bool,
    is_dl: bool,
    is_ongoing: bool,
    is_magazine_chapter: bool,
    is_cg_or_fanbox: bool,
    cover_asset_id: Option<String>,
    cover_is_archive: bool,
    total_bytes: i64,
    volumes: Vec<ComicVolumeItem>,
}

pub fn rebuild_comic_library(app: &AppHandle) -> Result<ComicLibraryStats, String> {
    four_domains::ensure_v2(app)?;
    let mut conn = four_domains::open_v2(app)?;
    rebuild_comic_library_with_conn(&mut conn)
}

pub fn rebuild_comic_library_with_conn(conn: &mut Connection) -> Result<ComicLibraryStats, String> {
    ensure_comic_library_schema(conn)?;
    let roots = configured_comic_paths(conn);
    if roots.is_empty() {
        return get_comic_library_stats_with_conn(conn);
    }

    // Preload archive page counts from comic_archives and head pages from comic_pages (for smart ad-skip cover)
    let mut archive_pages: HashMap<String, i64> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT asset_id, page_count FROM comic_archives") {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
            for (aid, cnt) in rows.flatten() {
                archive_pages.insert(aid, cnt);
            }
        }
    }
    let mut archive_head_pages: HashMap<String, Vec<(i64, String, i64)>> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT archive_asset_id, page_index, entry_name, COALESCE(uncompressed_size, 0)
         FROM comic_pages
         WHERE page_index < 6
         ORDER BY archive_asset_id, page_index ASC",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        }) {
            for (aid, pidx, ename, sz) in rows.flatten() {
                archive_head_pages.entry(aid).or_default().push((pidx, ename, sz));
            }
        }
    }

    // Query all candidate assets (archives + loose pages/images)
    let mut archives: Vec<(String, String, String, i64)> = Vec::new();
    let mut folder_pages: BTreeMap<String, Vec<(String, String, String, i64)>> = BTreeMap::new();

    {
        let mut stmt = conn
            .prepare(
                "SELECT id, file_name, remote_path, parent_path, media_type, COALESCE(size_bytes, 0)
                 FROM assets
                 WHERE missing = 0 AND media_type IN ('comicArchive', 'comicPage', 'image')",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| e.to_string())?;

        for row in rows.flatten() {
            let (id, fname, rpath, ppath, mtype, sz) = row;
            if !path_matches_any(&rpath, &roots) {
                continue;
            }
            if mtype == "comicArchive" {
                archives.push((id, fname, rpath, sz));
            } else {
                folder_pages.entry(ppath).or_default().push((id, fname, rpath, sz));
            }
        }
    }

    // Deduplicate identical archives by remote_path
    archives.sort_by(|a, b| a.2.cmp(&b.2));
    archives.dedup_by(|a, b| a.2 == b.2);

    let mut works: HashMap<String, WorkAccumulator> = HashMap::new();
    let mut title_to_work_id: HashMap<String, String> = HashMap::new();

    let mut insert_volume = |mut parsed: ParsedComicMeta, vol_id: String, is_archive: bool, asset_id: String, folder_path: Option<String>, file_name: String, remote_path: String, page_count: i64, size_bytes: i64| {
        // Track B: Series-folder assisted clustering for `系列合集` subfolders with an explicit series title
        if let Some((folder_title, folder_circle, folder_authors)) = extract_series_folder_title(&remote_path) {
            let file_title = parsed.base_title.clone();
            if parsed.volume_label == "单本" && normalize_cluster_text(&file_title) != normalize_cluster_text(&folder_title) {
                parsed.volume_label = file_title;
            }
            parsed.base_title = folder_title;
            if parsed.circle.is_none() {
                parsed.circle = folder_circle;
            }
            if parsed.authors.is_empty() {
                parsed.authors = folder_authors;
            }
        }

        let norm_title = normalize_cluster_text(&parsed.base_title);
        let primary_author = parsed
            .authors
            .first()
            .map(|a| normalize_cluster_text(a))
            .unwrap_or_default();

        // Cluster by (author + base_title), or fall back to existing work with same normalized base_title
        let cluster_key = if !primary_author.is_empty() {
            format!("{primary_author}::{norm_title}")
        } else if let Some(existing_id) = title_to_work_id.get(&norm_title) {
            existing_id.clone()
        } else {
            format!("title::{norm_title}")
        };

        let work_id = if works.contains_key(&cluster_key) {
            cluster_key.clone()
        } else if cluster_key.starts_with("cw-") {
            cluster_key.clone()
        } else {
            format!("cw-{}", stable_hash(&cluster_key))
        };

        if !norm_title.is_empty() {
            title_to_work_id.entry(norm_title).or_insert_with(|| work_id.clone());
        }

        let acc = works.entry(work_id.clone()).or_insert_with(|| WorkAccumulator {
            id: work_id.clone(),
            title: parsed.base_title.clone(),
            raw_title: file_name.clone(),
            authors: BTreeSet::new(),
            circle: parsed.circle.clone(),
            parody: parsed.parody.clone(),
            event_tag: parsed.event_tag.clone(),
            translators: BTreeSet::new(),
            tags: BTreeSet::new(),
            language: parsed.language.clone(),
            is_uncensored: parsed.is_uncensored,
            is_full_color: parsed.is_full_color,
            is_dl: parsed.is_dl,
            is_ongoing: parsed.is_ongoing,
            is_magazine_chapter: parsed.is_magazine_chapter,
            is_cg_or_fanbox: parsed.is_cg_or_fanbox,
            cover_asset_id: Some(asset_id.clone()),
            cover_is_archive: is_archive,
            total_bytes: 0,
            volumes: Vec::new(),
        });

        for a in parsed.authors {
            acc.authors.insert(a);
        }
        if acc.circle.is_none() {
            acc.circle = parsed.circle;
        }
        if acc.parody.is_none() {
            acc.parody = parsed.parody;
        }
        if acc.event_tag.is_none() {
            acc.event_tag = parsed.event_tag;
        }
        for t in parsed.translators {
            acc.translators.insert(t);
        }
        for tg in parsed.tags {
            acc.tags.insert(tg);
        }
        if parsed.language == "zh" {
            acc.language = "zh".to_string();
        }
        acc.is_uncensored |= parsed.is_uncensored;
        acc.is_full_color |= parsed.is_full_color;
        acc.is_dl |= parsed.is_dl;
        acc.is_ongoing |= parsed.is_ongoing;
        acc.is_magazine_chapter |= parsed.is_magazine_chapter;
        acc.is_cg_or_fanbox |= parsed.is_cg_or_fanbox;
        acc.total_bytes += size_bytes;

        acc.volumes.push(ComicVolumeItem {
            id: vol_id,
            work_id,
            volume_label: parsed.volume_label,
            sort_index: parsed.sort_index,
            is_archive,
            asset_id,
            folder_path,
            file_name,
            remote_path,
            page_count,
            size_bytes,
            read_page: 0,
            completed: false,
        });
    };

    // 1. Process all archives
    for (id, fname, rpath, sz) in archives {
        let parsed = parse_comic_filename(&fname, &rpath, None);
        let pcount = archive_pages.get(&id).copied().unwrap_or(0);
        insert_volume(parsed, id.clone(), true, id, None, fname, rpath, pcount, sz);
    }

    // 2. Process loose image folders (>= 3 images)
    for (ppath, mut pages) in folder_pages {
        if pages.len() < 3 {
            continue;
        }
        pages.sort_by(|a, b| a.1.cmp(&b.1));
        let first = &pages[0];
        let total_sz: i64 = pages.iter().map(|p| p.3).sum();
        let page_cnt = pages.len() as i64;
        let leaf_name = ppath.rsplit('/').next().unwrap_or(&ppath).to_string();

        let (parse_target, explicit_vol) = if let Some((ch_label, ch_idx)) = is_chapter_folder_name(&leaf_name) {
            let grand_parent = ppath.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            let parent_name = grand_parent.rsplit('/').next().unwrap_or(&leaf_name).to_string();
            (parent_name, Some((ch_label, ch_idx)))
        } else {
            (leaf_name.clone(), None)
        };

        let parsed = parse_comic_filename(&parse_target, &ppath, explicit_vol);
        let vol_id = format!("folder-{}", stable_hash(&ppath));
        insert_volume(
            parsed,
            vol_id,
            false,
            first.0.clone(),
            Some(ppath.clone()),
            leaf_name,
            ppath,
            page_cnt,
            total_sz,
        );
    }

    let ts = now();
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM comic_volumes", []).map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM comic_works", []).map_err(|e| e.to_string())?;

    for (_, mut acc) in works {
        acc.volumes.sort_by(|a, b| {
            a.sort_index
                .partial_cmp(&b.sort_index)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.file_name.cmp(&b.file_name))
        });
        // Pick first sorted volume as cover + smart ad-skip page index
        let mut cover_page_index: i64 = 0;
        if let Some(first_vol) = acc.volumes.first() {
            acc.cover_asset_id = Some(first_vol.asset_id.clone());
            acc.cover_is_archive = first_vol.is_archive;
            if first_vol.is_archive {
                if let Some(head_pages) = archive_head_pages.get(&first_vol.asset_id) {
                    cover_page_index = pick_smart_cover_page(head_pages);
                }
            }
        }

        let authors_vec: Vec<String> = acc.authors.into_iter().collect();
        let translators_vec: Vec<String> = acc.translators.into_iter().collect();
        let tags_vec: Vec<String> = acc.tags.into_iter().collect();
        let vol_count = acc.volumes.len() as i64;
        let sample_path = acc.volumes.first().map(|v| v.remote_path.as_str()).unwrap_or("");
        let published_at = infer_published_at(acc.event_tag.as_deref(), &acc.title, &acc.raw_title);
        let category = infer_comic_category(
            &acc.title,
            &acc.raw_title,
            sample_path,
            acc.event_tag.as_deref(),
            acc.circle.as_deref(),
            acc.parody.as_deref(),
            acc.is_ongoing,
            acc.is_full_color,
            acc.is_magazine_chapter,
            acc.is_cg_or_fanbox,
            vol_count,
            &tags_vec,
        );

        tx.execute(
            "INSERT INTO comic_works (
                id, title, raw_title, authors_json, circle, parody, event_tag,
                translators_json, tags_json, language, is_uncensored, is_full_color,
                is_dl, is_ongoing, cover_asset_id, cover_is_archive, volume_count,
                total_bytes, source, summary, updated_at, category, published_at, cover_page_index
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,'local',NULL,?19,?20,?21,?22)",
            params![
                acc.id,
                acc.title,
                acc.raw_title,
                serde_json::to_string(&authors_vec).unwrap_or_else(|_| "[]".into()),
                acc.circle,
                acc.parody,
                acc.event_tag,
                serde_json::to_string(&translators_vec).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&tags_vec).unwrap_or_else(|_| "[]".into()),
                acc.language,
                if acc.is_uncensored { 1 } else { 0 },
                if acc.is_full_color { 1 } else { 0 },
                if acc.is_dl { 1 } else { 0 },
                if acc.is_ongoing { 1 } else { 0 },
                acc.cover_asset_id,
                if acc.cover_is_archive { 1 } else { 0 },
                vol_count,
                acc.total_bytes,
                ts,
                category,
                published_at,
                cover_page_index,
            ],
        )
        .map_err(|e| e.to_string())?;

        for v in acc.volumes {
            tx.execute(
                "INSERT INTO comic_volumes (
                    id, work_id, volume_label, sort_index, is_archive, asset_id,
                    folder_path, file_name, remote_path, page_count, size_bytes, updated_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    v.id,
                    v.work_id,
                    v.volume_label,
                    v.sort_index,
                    if v.is_archive { 1 } else { 0 },
                    v.asset_id,
                    v.folder_path,
                    v.file_name,
                    v.remote_path,
                    v.page_count,
                    v.size_bytes,
                    ts,
                ],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // Re-apply persisted external metadata (E-Hentai / Bangumi) so rebuilds never lose scraped tags/summaries
    let _ = tx.execute(
        "UPDATE comic_works SET
            title = COALESCE((SELECT CASE WHEN m.source = 'bangumi' AND m.title IS NOT NULL AND m.title != '' THEN m.title ELSE NULL END FROM comic_external_meta m WHERE m.work_id = comic_works.id), title),
            authors_json = COALESCE((SELECT CASE WHEN comic_works.authors_json = '[]' AND m.authors_json != '[]' THEN m.authors_json ELSE NULL END FROM comic_external_meta m WHERE m.work_id = comic_works.id), authors_json),
            circle = COALESCE(circle, (SELECT m.circle FROM comic_external_meta m WHERE m.work_id = comic_works.id)),
            parody = COALESCE(parody, (SELECT m.parody FROM comic_external_meta m WHERE m.work_id = comic_works.id)),
            tags_json = COALESCE((SELECT m.tags_json FROM comic_external_meta m WHERE m.work_id = comic_works.id), tags_json),
            source = COALESCE((SELECT m.source FROM comic_external_meta m WHERE m.work_id = comic_works.id), source),
            rating = COALESCE((SELECT m.rating FROM comic_external_meta m WHERE m.work_id = comic_works.id), rating),
            published_at = COALESCE((SELECT m.published_at FROM comic_external_meta m WHERE m.work_id = comic_works.id), published_at),
            summary = COALESCE((SELECT m.summary FROM comic_external_meta m WHERE m.work_id = comic_works.id), summary)
         WHERE EXISTS (SELECT 1 FROM comic_external_meta m WHERE m.work_id = comic_works.id)",
        [],
    );

    // Re-apply manual cover overrides so rebuilds never lose user-chosen covers
    let _ = tx.execute(
        "UPDATE comic_works SET
            cover_asset_id = (SELECT o.cover_asset_id FROM comic_cover_overrides o WHERE o.work_id = comic_works.id),
            cover_page_index = (SELECT o.cover_page_index FROM comic_cover_overrides o WHERE o.work_id = comic_works.id)
         WHERE EXISTS (SELECT 1 FROM comic_cover_overrides o WHERE o.work_id = comic_works.id)",
        [],
    );

    tx.commit().map_err(|e| e.to_string())?;
    get_comic_library_stats_with_conn(conn)
}

pub fn set_comic_work_cover(app: &AppHandle, req: SetComicCoverRequest) -> Result<ComicWorkItem, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_comic_library_schema(&conn)?;
    let page_idx = req.page_index.max(0);
    let ts = now();

    let current_asset: Option<String> = conn
        .query_row("SELECT cover_asset_id FROM comic_works WHERE id = ?1", params![req.work_id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    let asset_id = req
        .asset_id
        .filter(|s| !s.trim().is_empty())
        .or(current_asset)
        .ok_or_else(|| "未找到该作品的封面资源".to_string())?;

    conn.execute(
        "INSERT INTO comic_cover_overrides (work_id, cover_asset_id, cover_page_index, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(work_id) DO UPDATE SET
            cover_asset_id = excluded.cover_asset_id,
            cover_page_index = excluded.cover_page_index,
            updated_at = excluded.updated_at",
        params![req.work_id, asset_id, page_idx, ts],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE comic_works SET cover_asset_id = ?1, cover_page_index = ?2, updated_at = ?3 WHERE id = ?4",
        params![asset_id, page_idx, ts, req.work_id],
    )
    .map_err(|e| e.to_string())?;

    let sql = format!("SELECT {WORK_SELECT_COLS} FROM comic_works WHERE id = ?1");
    conn.query_row(&sql, params![req.work_id], map_work_row)
        .map_err(|e| e.to_string())
}

// ============================================================================
// Query / Stats / Authors / Detail APIs
// ============================================================================

fn map_work_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ComicWorkItem> {
    let authors_raw: String = row.get(3)?;
    let translators_raw: String = row.get(7)?;
    let tags_raw: String = row.get(8)?;
    Ok(ComicWorkItem {
        id: row.get(0)?,
        title: row.get(1)?,
        authors: serde_json::from_str(&authors_raw).unwrap_or_default(),
        circle: row.get(4)?,
        parody: row.get(5)?,
        event_tag: row.get(6)?,
        translators: serde_json::from_str(&translators_raw).unwrap_or_default(),
        tags: serde_json::from_str(&tags_raw).unwrap_or_default(),
        language: row.get(9)?,
        is_uncensored: row.get::<_, i64>(10)? != 0,
        is_full_color: row.get::<_, i64>(11)? != 0,
        is_dl: row.get::<_, i64>(12)? != 0,
        is_ongoing: row.get::<_, i64>(13)? != 0,
        cover_asset_id: row.get(14)?,
        cover_is_archive: row.get::<_, i64>(15)? != 0,
        cover_page_index: row.get::<_, Option<i64>>(24)?.unwrap_or(0),
        volume_count: row.get(16)?,
        total_pages: 0,
        total_bytes: row.get(17)?,
        source: row.get(18)?,
        category: row.get::<_, Option<String>>(22)?.unwrap_or_default(),
        published_at: row.get(23)?,
        rating: row.get(21)?,
        summary: row.get(19)?,
        updated_at: row.get(20)?,
        last_read_volume_id: None,
        last_read_volume_label: None,
        read_page: 0,
        read_total_pages: 0,
        read_percent: 0,
        read_status: "unread".to_string(),
        last_read_at: None,
    })
}

fn load_work_progress_map(conn: &Connection) -> HashMap<String, (String, String, i64, i64, bool, i64)> {
    let mut map = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT work_id, volume_id, volume_label, current_page, total_pages, completed, updated_at
         FROM comic_reading_progress",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)? != 0,
                r.get::<_, i64>(6)?,
            ))
        }) {
            for (wid, vid, vlabel, cur, tot, comp, ts) in rows.flatten() {
                map.insert(wid, (vid, vlabel, cur, tot, comp, ts));
            }
        }
    }
    map
}

fn apply_work_progress(item: &mut ComicWorkItem, prog: &(String, String, i64, i64, bool, i64)) {
    let (vid, vlabel, cur, tot, comp, ts) = prog;
    item.last_read_volume_id = Some(vid.clone());
    if !vlabel.is_empty() {
        item.last_read_volume_label = Some(vlabel.clone());
    }
    item.read_page = *cur;
    item.read_total_pages = *tot;
    item.last_read_at = Some(*ts);
    let pct = if *comp {
        100
    } else if *tot > 0 {
        (((*cur + 1) * 100) / *tot).clamp(1, 99)
    } else if *cur > 0 {
        10
    } else {
        5
    };
    item.read_percent = pct;
    item.read_status = if *comp || pct >= 95 {
        "finished".to_string()
    } else {
        "reading".to_string()
    };
}

pub fn save_comic_reading_progress(app: &AppHandle, req: SaveComicProgressRequest) -> Result<(), String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_comic_library_schema(&conn)?;
    let ts = now();
    let vlabel: String = conn
        .query_row(
            "SELECT volume_label FROM comic_volumes WHERE id = ?1",
            params![&req.volume_id],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None)
        .unwrap_or_default();

    let completed = if req.total_pages > 0 && req.current_page + 1 >= req.total_pages {
        1i64
    } else {
        0i64
    };

    conn.execute(
        "INSERT INTO comic_volume_progress (volume_id, work_id, current_page, total_pages, completed, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(volume_id) DO UPDATE SET
           current_page = excluded.current_page,
           total_pages = MAX(comic_volume_progress.total_pages, excluded.total_pages),
           completed = MAX(comic_volume_progress.completed, excluded.completed),
           updated_at = excluded.updated_at",
        params![req.volume_id, req.work_id, req.current_page, req.total_pages, completed, ts],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO comic_reading_progress (work_id, volume_id, volume_label, asset_id, current_page, total_pages, completed, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(work_id) DO UPDATE SET
           volume_id = excluded.volume_id,
           volume_label = excluded.volume_label,
           asset_id = excluded.asset_id,
           current_page = excluded.current_page,
           total_pages = excluded.total_pages,
           completed = excluded.completed,
           updated_at = excluded.updated_at",
        params![req.work_id, req.volume_id, vlabel, req.asset_id, req.current_page, req.total_pages, completed, ts],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

pub fn list_continue_reading_comics(app: &AppHandle, limit: Option<i64>) -> Result<Vec<ComicWorkItem>, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_comic_library_schema(&conn)?;
    let prog_map = load_work_progress_map(&conn);
    if prog_map.is_empty() {
        return Ok(Vec::new());
    }
    let lim = limit.unwrap_or(12).clamp(1, 30);
    let sql = format!(
        "SELECT w.{cols}
         FROM comic_reading_progress p
         JOIN comic_works w ON w.id = p.work_id
         ORDER BY p.updated_at DESC
         LIMIT ?1",
        cols = WORK_SELECT_COLS.replace(", ", ", w.")
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![lim], map_work_row).map_err(|e| e.to_string())?;
    let mut items = Vec::new();
    for mut item in rows.flatten() {
        if let Some(p) = prog_map.get(&item.id) {
            apply_work_progress(&mut item, p);
        }
        items.push(item);
    }
    Ok(items)
}

const WORK_SELECT_COLS: &str = "id, title, raw_title, authors_json, circle, parody, event_tag, translators_json, tags_json, language, is_uncensored, is_full_color, is_dl, is_ongoing, cover_asset_id, cover_is_archive, volume_count, total_bytes, source, summary, updated_at, rating, category, published_at, cover_page_index";

fn load_work_pages_map(conn: &Connection) -> HashMap<String, i64> {
    let mut map = HashMap::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT v.work_id, SUM(CASE WHEN COALESCE(ca.page_count, 0) > v.page_count THEN ca.page_count ELSE v.page_count END)
         FROM comic_volumes v
         LEFT JOIN comic_archives ca ON ca.asset_id = v.asset_id
         GROUP BY v.work_id",
    ) {
        if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
            for (wid, tp) in rows.flatten() {
                if tp > 0 {
                    map.insert(wid, tp);
                }
            }
        }
    }
    map
}

pub fn get_comic_library_stats(app: &AppHandle) -> Result<ComicLibraryStats, String> {
    four_domains::ensure_v2(app)?;
    let mut conn = four_domains::open_v2(app)?;
    ensure_comic_library_schema(&conn)?;
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM comic_works", [], |r| r.get(0))
        .unwrap_or(0);
    if count == 0 {
        let _ = rebuild_comic_library_with_conn(&mut conn);
    }
    get_comic_library_stats_with_conn(&conn)
}

fn get_comic_library_stats_with_conn(conn: &Connection) -> Result<ComicLibraryStats, String> {
    let total_works: i64 = conn.query_row("SELECT COUNT(*) FROM comic_works", [], |r| r.get(0)).unwrap_or(0);
    let total_volumes: i64 = conn.query_row("SELECT COUNT(*) FROM comic_volumes", [], |r| r.get(0)).unwrap_or(0);
    let total_series: i64 = conn.query_row("SELECT COUNT(*) FROM comic_works WHERE volume_count > 1", [], |r| r.get(0)).unwrap_or(0);
    let zh_count: i64 = conn.query_row("SELECT COUNT(*) FROM comic_works WHERE language = 'zh'", [], |r| r.get(0)).unwrap_or(0);
    let raw_count = total_works.saturating_sub(zh_count);
    let uncensored_count: i64 = conn.query_row("SELECT COUNT(*) FROM comic_works WHERE is_uncensored = 1", [], |r| r.get(0)).unwrap_or(0);
    let archive_volumes: i64 = conn.query_row("SELECT COUNT(*) FROM comic_volumes WHERE is_archive = 1", [], |r| r.get(0)).unwrap_or(0);
    let folder_volumes = total_volumes.saturating_sub(archive_volumes);
    let enriched_count: i64 = conn.query_row("SELECT COUNT(*) FROM comic_works WHERE source != 'local'", [], |r| r.get(0)).unwrap_or(0);

    let mut author_set = BTreeSet::<String>::new();
    if let Ok(mut stmt) = conn.prepare("SELECT authors_json FROM comic_works") {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            for raw in rows.flatten() {
                if let Ok(list) = serde_json::from_str::<Vec<String>>(&raw) {
                    for a in list {
                        author_set.insert(a);
                    }
                }
            }
        }
    }

    Ok(ComicLibraryStats {
        total_works,
        total_volumes,
        total_series,
        total_authors: author_set.len() as i64,
        zh_count,
        raw_count,
        uncensored_count,
        archive_volumes,
        folder_volumes,
        enriched_count,
    })
}

pub fn query_comic_library(app: &AppHandle, req: QueryComicLibraryRequest) -> Result<ComicLibraryPage, String> {
    four_domains::ensure_v2(app)?;
    let mut conn = four_domains::open_v2(app)?;
    ensure_comic_library_schema(&conn)?;

    let existing: i64 = conn.query_row("SELECT COUNT(*) FROM comic_works", [], |r| r.get(0)).unwrap_or(0);
    if existing == 0 {
        let _ = rebuild_comic_library_with_conn(&mut conn);
    }

    let fav_set: BTreeSet<String> = if req.only_favorites.unwrap_or(false) {
        let mut s = BTreeSet::new();
        if let Ok(mut stmt) = conn.prepare("SELECT entity_id FROM entity_favorites WHERE entity_type = 'comic'") {
            if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
                for r in rows.flatten() {
                    s.insert(r);
                }
            }
        }
        s
    } else {
        BTreeSet::new()
    };

    let order_sql = match req.sort.as_deref() {
        Some("title_asc") => "ORDER BY title COLLATE NOCASE ASC",
        Some("volumes_desc") => "ORDER BY volume_count DESC, updated_at DESC, title COLLATE NOCASE ASC",
        Some("size_desc") => "ORDER BY total_bytes DESC",
        _ => "ORDER BY updated_at DESC, volume_count DESC, title COLLATE NOCASE ASC",
    };

    let prog_map = load_work_progress_map(&conn);
    let pages_map = load_work_pages_map(&conn);
    let sql = format!("SELECT {WORK_SELECT_COLS} FROM comic_works {order_sql}");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let all_rows = stmt.query_map([], map_work_row).map_err(|e| e.to_string())?;

    let q_lower = req.query.as_deref().unwrap_or("").trim().to_lowercase();
    let cat_filter = req.category.as_deref().unwrap_or("").trim().to_string();
    let year_filter = req.year.as_deref().unwrap_or("").trim().to_string();
    let author_filter = req.author.as_deref().unwrap_or("").trim().to_lowercase();
    let parody_filter = req.parody.as_deref().unwrap_or("").trim().to_lowercase();
    let tag_filter = req.tag.as_deref().unwrap_or("").trim().to_lowercase();
    let lang_filter = req.language.as_deref().unwrap_or("all").trim().to_lowercase();
    let read_filter = req.read_status.as_deref().unwrap_or("all").trim().to_lowercase();
    let only_series = req.only_series.unwrap_or(false);
    let only_uncensored = req.only_uncensored.unwrap_or(false);
    let only_full_color = req.only_full_color.unwrap_or(false);
    let only_dl = req.only_dl.unwrap_or(false);
    let only_fav = req.only_favorites.unwrap_or(false);

    let mut matched: Vec<ComicWorkItem> = Vec::new();
    for mut item in all_rows.flatten() {
        if let Some(&tp) = pages_map.get(&item.id) {
            item.total_pages = tp;
        }
        if let Some(p) = prog_map.get(&item.id) {
            apply_work_progress(&mut item, p);
        }
        if only_fav && !fav_set.contains(&item.id) {
            continue;
        }
        if !cat_filter.is_empty() && cat_filter != "全部" && item.category != cat_filter {
            continue;
        }
        if !year_filter.is_empty() {
            let hit_year = item.published_at.as_deref().map(|d| d.starts_with(&year_filter)).unwrap_or(false);
            if !hit_year {
                continue;
            }
        }
        if read_filter != "all" && !read_filter.is_empty() && item.read_status != read_filter {
            continue;
        }
        if only_series && item.volume_count <= 1 {
            continue;
        }
        if only_uncensored && !item.is_uncensored {
            continue;
        }
        if only_full_color && !item.is_full_color {
            continue;
        }
        if only_dl && !item.is_dl {
            continue;
        }
        if lang_filter == "zh" && item.language != "zh" {
            continue;
        }
        if lang_filter == "ja" && item.language == "zh" {
            continue;
        }
        if !author_filter.is_empty() {
            let hit_author = item.authors.iter().any(|a| a.to_lowercase() == author_filter)
                || item.circle.as_deref().map(|c| c.to_lowercase() == author_filter).unwrap_or(false);
            if !hit_author {
                continue;
            }
        }
        if !parody_filter.is_empty() {
            let hit_parody = item.parody.as_deref().map(|p| p.to_lowercase() == parody_filter).unwrap_or(false);
            if !hit_parody {
                continue;
            }
        }
        if !tag_filter.is_empty() {
            let hit_tag = item.tags.iter().any(|t| t.to_lowercase() == tag_filter)
                || item.translators.iter().any(|t| t.to_lowercase() == tag_filter);
            if !hit_tag {
                continue;
            }
        }
        if !q_lower.is_empty() {
            let hay = format!(
                "{} {} {} {} {} {}",
                item.title,
                item.authors.join(" "),
                item.circle.as_deref().unwrap_or(""),
                item.parody.as_deref().unwrap_or(""),
                item.tags.join(" "),
                item.translators.join(" ")
            )
            .to_lowercase();
            if !hay.contains(&q_lower) {
                continue;
            }
        }
        matched.push(item);
    }

    if req.sort.as_deref() == Some("random") {
        let mut rng = rand::thread_rng();
        matched.shuffle(&mut rng);
    } else if req.sort.as_deref() == Some("recent_read") {
        matched.sort_by(|a, b| b.last_read_at.unwrap_or(0).cmp(&a.last_read_at.unwrap_or(0)));
    } else if req.sort.as_deref() == Some("published_desc") {
        matched.sort_by(|a, b| {
            b.published_at
                .as_deref()
                .unwrap_or("")
                .cmp(a.published_at.as_deref().unwrap_or(""))
                .then_with(|| a.title.cmp(&b.title))
        });
    } else if req.sort.as_deref() == Some("published_asc") {
        matched.sort_by(|a, b| {
            let da = a.published_at.as_deref().unwrap_or("9999");
            let db = b.published_at.as_deref().unwrap_or("9999");
            da.cmp(db).then_with(|| a.title.cmp(&b.title))
        });
    }

    let total = matched.len() as i64;
    let offset = req.offset.unwrap_or(0).max(0) as usize;
    let limit = req.limit.unwrap_or(48).clamp(1, 200) as usize;
    let slice: Vec<ComicWorkItem> = matched.into_iter().skip(offset).take(limit).collect();
    let has_more = (offset + slice.len()) < (total as usize);

    Ok(ComicLibraryPage {
        items: slice,
        total,
        has_more,
    })
}

pub fn get_comic_work_detail(app: &AppHandle, work_id: &str) -> Result<ComicWorkDetail, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_comic_library_schema(&conn)?;

    let sql = format!("SELECT {WORK_SELECT_COLS} FROM comic_works WHERE id = ?1");
    let mut work = conn
        .query_row(&sql, params![work_id], map_work_row)
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("未找到漫画作品：{work_id}"))?;

    let prog_map = load_work_progress_map(&conn);
    if let Some(p) = prog_map.get(&work.id) {
        apply_work_progress(&mut work, p);
    }

    let mut vol_prog: HashMap<String, (i64, bool)> = HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT volume_id, current_page, completed FROM comic_volume_progress WHERE work_id = ?1") {
        if let Ok(rows) = stmt.query_map(params![work_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)? != 0))) {
            for (vid, cur, comp) in rows.flatten() {
                vol_prog.insert(vid, (cur, comp));
            }
        }
    }

    let mut volumes = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, work_id, volume_label, sort_index, is_archive, asset_id, folder_path, file_name, remote_path, page_count, size_bytes
                 FROM comic_volumes
                 WHERE work_id = ?1
                 ORDER BY sort_index ASC, file_name ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![work_id], |r| {
                Ok(ComicVolumeItem {
                    id: r.get(0)?,
                    work_id: r.get(1)?,
                    volume_label: r.get(2)?,
                    sort_index: r.get(3)?,
                    is_archive: r.get::<_, i64>(4)? != 0,
                    asset_id: r.get(5)?,
                    folder_path: r.get(6)?,
                    file_name: r.get(7)?,
                    remote_path: r.get(8)?,
                    page_count: r.get(9)?,
                    size_bytes: r.get(10)?,
                    read_page: 0,
                    completed: false,
                })
            })
            .map_err(|e| e.to_string())?;
        for mut v in rows.flatten() {
            if let Some((rp, comp)) = vol_prog.get(&v.id) {
                v.read_page = *rp;
                v.completed = *comp;
            }
            volumes.push(v);
        }
    }

    let mut related_by_author = Vec::new();
    let mut related_by_parody = Vec::new();
    let all_sql = format!("SELECT {WORK_SELECT_COLS} FROM comic_works WHERE id != ?1 ORDER BY volume_count DESC, title ASC");
    if let Ok(mut stmt) = conn.prepare(&all_sql) {
        if let Ok(rows) = stmt.query_map(params![work_id], map_work_row) {
            for cand in rows.flatten() {
                if related_by_author.len() < 12 && !work.authors.is_empty() {
                    if cand.authors.iter().any(|a| work.authors.contains(a)) {
                        related_by_author.push(cand.clone());
                    }
                }
                if related_by_parody.len() < 12 {
                    if let (Some(ref p1), Some(ref p2)) = (&work.parody, &cand.parody) {
                        if p1 != "原创" && p1 == p2 {
                            related_by_parody.push(cand);
                        }
                    }
                }
            }
        }
    }

    Ok(ComicWorkDetail {
        work,
        volumes,
        related_by_author,
        related_by_parody,
    })
}

pub fn list_comic_authors(app: &AppHandle, query: Option<String>, limit: Option<i64>) -> Result<Vec<ComicAuthorItem>, String> {
    four_domains::ensure_v2(app)?;
    let mut conn = four_domains::open_v2(app)?;
    ensure_comic_library_schema(&conn)?;
    let existing: i64 = conn.query_row("SELECT COUNT(*) FROM comic_works", [], |r| r.get(0)).unwrap_or(0);
    if existing == 0 {
        let _ = rebuild_comic_library_with_conn(&mut conn);
    }

    let sql = format!("SELECT {WORK_SELECT_COLS} FROM comic_works ORDER BY volume_count DESC, updated_at DESC");
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], map_work_row).map_err(|e| e.to_string())?;

    let mut map: HashMap<String, ComicAuthorItem> = HashMap::new();
    for w in rows.flatten() {
        for author in &w.authors {
            let entry = map.entry(author.clone()).or_insert_with(|| ComicAuthorItem {
                name: author.clone(),
                circle: w.circle.clone(),
                work_count: 0,
                volume_count: 0,
                cover_asset_id: w.cover_asset_id.clone(),
                cover_is_archive: w.cover_is_archive,
            });
            entry.work_count += 1;
            entry.volume_count += w.volume_count;
            if entry.circle.is_none() {
                entry.circle = w.circle.clone();
            }
        }
    }

    let q_lower = query.as_deref().unwrap_or("").trim().to_lowercase();
    let mut list: Vec<ComicAuthorItem> = map
        .into_values()
        .filter(|a| {
            q_lower.is_empty()
                || a.name.to_lowercase().contains(&q_lower)
                || a.circle.as_deref().map(|c| c.to_lowercase().contains(&q_lower)).unwrap_or(false)
        })
        .collect();
    list.sort_by(|a, b| b.work_count.cmp(&a.work_count).then_with(|| b.volume_count.cmp(&a.volume_count)).then_with(|| a.name.cmp(&b.name)));
    list.truncate(limit.unwrap_or(200).clamp(1, 500) as usize);
    Ok(list)
}

pub fn list_comic_filter_options(app: &AppHandle) -> Result<ComicFilterOptions, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_comic_library_schema(&conn)?;

    let mut category_counts: HashMap<String, i64> = HashMap::new();
    let mut year_counts: HashMap<String, i64> = HashMap::new();
    let mut author_counts: HashMap<String, i64> = HashMap::new();
    let mut parody_counts: HashMap<String, i64> = HashMap::new();
    let mut tag_counts: HashMap<String, i64> = HashMap::new();

    let sql = format!("SELECT {WORK_SELECT_COLS} FROM comic_works");
    if let Ok(mut stmt) = conn.prepare(&sql) {
        if let Ok(rows) = stmt.query_map([], map_work_row) {
            for w in rows.flatten() {
                if !w.category.is_empty() {
                    *category_counts.entry(w.category).or_insert(0) += 1;
                }
                if let Some(ref pub_date) = w.published_at {
                    if pub_date.len() >= 4 {
                        *year_counts.entry(pub_date[..4].to_string()).or_insert(0) += 1;
                    }
                }
                for a in w.authors {
                    *author_counts.entry(a).or_insert(0) += 1;
                }
                if let Some(p) = w.parody {
                    *parody_counts.entry(p).or_insert(0) += 1;
                }
                for t in w.tags {
                    *tag_counts.entry(t).or_insert(0) += 1;
                }
            }
        }
    }

    let to_facets = |m: HashMap<String, i64>, cap: usize| -> Vec<ComicFilterFacet> {
        let mut v: Vec<ComicFilterFacet> = m
            .into_iter()
            .map(|(name, count)| ComicFilterFacet { name, count })
            .collect();
        v.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        v.truncate(cap);
        v
    };

    let mut genre_counts: HashMap<String, i64> = HashMap::new();
    for (k, v) in &tag_counts {
        if !is_technical_tag(k) {
            genre_counts.insert(k.clone(), *v);
        }
    }

    let mut years: Vec<ComicFilterFacet> = year_counts
        .into_iter()
        .map(|(name, count)| ComicFilterFacet { name, count })
        .collect();
    years.sort_by(|a, b| b.name.cmp(&a.name));

    Ok(ComicFilterOptions {
        categories: to_facets(category_counts, 20),
        genres: to_facets(genre_counts, 60),
        years,
        authors: to_facets(author_counts, 60),
        parodies: to_facets(parody_counts, 60),
        tags: to_facets(tag_counts, 60),
    })
}
