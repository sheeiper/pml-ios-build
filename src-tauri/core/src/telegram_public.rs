//! Public Telegram channel preview.
//!
//! This is a separate source from the four-domain library. The server fetches
//! `https://t.me/s/<username>` HTML, stores only the channel the user added,
//! and returns post text plus CDN URLs. It does not log in, does not use
//! TDLib, and does not proxy media bytes.
//!
//! Signed CDN URLs expire. Callers must use `telegram_resolve_media` before
//! playback instead of reusing a listed video URL.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::{four_domains, AppHandle};

static FEED_CACHE: LazyLock<RwLock<HashMap<String, (i64, TelegramFeed)>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

const PREVIEW_TTL_SECS: i64 = 20 * 60;
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramChannel {
    pub username: String,
    pub title: String,
    pub photo_url: Option<String>,
    pub description: Option<String>,
    pub subscribers: Option<String>,
    pub open_url: String,
    pub added_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramMedia {
    pub kind: String,
    pub url: Option<String>,
    pub poster_url: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<String>,
    pub expires_at: Option<i64>,
    pub resolve_before_play: bool,
    pub open_url: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramPost {
    pub id: i64,
    pub text: String,
    pub date: Option<String>,
    pub link: String,
    pub views: Option<String>,
    pub preview_limited: bool,
    pub media: Vec<TelegramMedia>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramFeed {
    pub channel: TelegramChannel,
    pub posts: Vec<TelegramPost>,
    pub next_before: Option<i64>,
    pub preview_unavailable: bool,
    pub notice: Option<String>,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewKind {
    Channel,
    Bot,
    Unavailable,
}

#[derive(Debug, Clone)]
struct ParsedPreview {
    kind: PreviewKind,
    username: Option<String>,
    title: Option<String>,
    photo_url: Option<String>,
    description: Option<String>,
    subscribers: Option<String>,
    posts: Vec<TelegramPost>,
    next_before: Option<i64>,
}

pub async fn add_channel(app: AppHandle, input: String) -> Result<TelegramChannel, String> {
    let username = normalize_username(&input)?;
    let html = fetch_preview(&channel_url(&username, None)).await?;
    let parsed = parse_preview(&html, &username, now_secs());
    match parsed.kind {
        PreviewKind::Bot => {
            return Err("这是机器人，不是公开频道。本版不支持机器人。".to_owned())
        }
        PreviewKind::Unavailable => {
            return Err(
                "没有公开预览。请确认是 t.me/频道名；私有频道和邀请链接不能在这里打开。"
                    .to_owned(),
            )
        }
        PreviewKind::Channel => {}
    }
    let canonical = parsed
        .username
        .clone()
        .ok_or_else(|| "公开页没有频道用户名".to_owned())?;
    if canonical != username {
        return Err("公开页用户名与输入不一致".to_owned());
    }
    with_db(app, move |connection| upsert_channel(connection, &parsed)).await
}

pub fn list_channels(app: &AppHandle) -> Result<Vec<TelegramChannel>, String> {
    let connection = open(app)?;
    list_saved(&connection)
}

pub fn remove_channel(app: &AppHandle, input: &str) -> Result<bool, String> {
    let username = normalize_username(input)?;
    if let Ok(mut cache) = FEED_CACHE.try_write() {
        cache.remove(&username);
    }
    let connection = open(app)?;
    let deleted = connection
        .execute(
            "DELETE FROM telegram_channels WHERE username=?1",
            params![username],
        )
        .map_err(database_error)?;
    if deleted == 0 {
        return Err("尚未添加该频道".to_owned());
    }
    Ok(true)
}

pub async fn list_posts(
    app: AppHandle,
    input: String,
    before: Option<i64>,
    refresh: Option<bool>,
) -> Result<TelegramFeed, String> {
    let username = normalize_username(&input)?;
    if let Some(id) = before {
        if id <= 0 {
            return Err("before 必须是正整数".to_owned());
        }
    }
    if before.is_none() && refresh != Some(true) {
        let cache = FEED_CACHE.read().await;
        if let Some((ts, feed)) = cache.get(&username) {
            if now_secs().saturating_sub(*ts) < 90 {
                return Ok(feed.clone());
            }
        }
    }
    let saved = with_db(app.clone(), {
        let username = username.clone();
        move |connection| load_channel(connection, &username)
    })
    .await?;
    let Some(saved) = saved else {
        return Err("请先添加这个公开频道".to_owned());
    };
    let fetched_at = now_secs();
    let html = fetch_preview(&channel_url(&username, before)).await?;
    let parsed = parse_preview(&html, &username, fetched_at);
    if parsed.kind != PreviewKind::Channel {
        return Ok(TelegramFeed {
            channel: saved,
            posts: Vec::new(),
            next_before: None,
            preview_unavailable: true,
            notice: Some(
                "这个频道没有公开预览，可能已转为私有。请在 Telegram 中打开。".to_owned(),
            ),
            fetched_at,
        });
    }
    let _ = with_db(app, {
        let parsed = parsed.clone();
        move |connection| upsert_channel(connection, &parsed)
    })
    .await;
    let mut channel = saved;
    if let Some(title) = parsed.title.clone() {
        channel.title = title;
    }
    channel.photo_url = parsed.photo_url.clone().or(channel.photo_url);
    channel.description = parsed.description.clone().or(channel.description);
    channel.subscribers = parsed.subscribers.clone().or(channel.subscribers);
    channel.updated_at = fetched_at;
    let feed = TelegramFeed {
        channel,
        posts: parsed.posts,
        next_before: parsed.next_before,
        preview_unavailable: false,
        notice: None,
        fetched_at,
    };
    if before.is_none() && !feed.preview_unavailable {
        let mut cache = FEED_CACHE.write().await;
        cache.insert(username, (fetched_at, feed.clone()));
    }
    Ok(feed)
}

pub async fn resolve_media(
    app: AppHandle,
    input: String,
    post_id: i64,
) -> Result<TelegramPost, String> {
    if post_id <= 0 {
        return Err("postId 必须是正整数".to_owned());
    }
    let username = normalize_username(&input)?;
    let saved = with_db(app, {
        let username = username.clone();
        move |connection| load_channel(connection, &username)
    })
    .await?;
    if saved.is_none() {
        return Err("请先添加这个公开频道".to_owned());
    }
    let fetched_at = now_secs();
    let embed = fetch_preview(&format!(
        "https://t.me/{username}/{post_id}?embed=1&mode=tme"
    ))
    .await?;
    let mut parsed = parse_preview(&embed, &username, fetched_at);
    if !parsed.posts.iter().any(|post| post.id == post_id) {
        let page = fetch_preview(&format!("https://t.me/s/{username}/{post_id}")).await?;
        parsed = parse_preview(&page, &username, fetched_at);
    }
    if let Some(post) = parsed.posts.into_iter().find(|post| post.id == post_id) {
        return Ok(post);
    }
    if too_big_label(&embed) {
        return Ok(too_big_post(&username, post_id, None, fetched_at));
    }
    Err("没有解析到这条公开消息，请在 Telegram 中打开".to_owned())
}

fn open(app: &AppHandle) -> Result<Connection, String> {
    four_domains::ensure_v2(app)?;
    let connection = four_domains::open_v2(app)?;
    ensure_schema(&connection)?;
    Ok(connection)
}

async fn with_db<T, F>(app: AppHandle, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let connection = open(&app)?;
        task(&connection)
    })
    .await
    .map_err(|error| format!("Telegram 数据库任务失败：{error}"))?
}

fn ensure_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS telegram_channels (
                username TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                photo_url TEXT,
                description TEXT,
                subscribers TEXT,
                added_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );",
        )
        .map_err(database_error)
}

fn upsert_channel(connection: &Connection, parsed: &ParsedPreview) -> Result<TelegramChannel, String> {
    let username = parsed
        .username
        .clone()
        .ok_or_else(|| "公开页没有频道用户名".to_owned())?;
    let title = parsed
        .title
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| username.clone());
    let timestamp = now_secs();
    connection
        .execute(
            "INSERT INTO telegram_channels(username,title,photo_url,description,subscribers,added_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?6)
             ON CONFLICT(username) DO UPDATE SET
               title=excluded.title,
               photo_url=excluded.photo_url,
               description=excluded.description,
               subscribers=excluded.subscribers,
               updated_at=excluded.updated_at",
            params![
                username,
                title,
                parsed.photo_url,
                parsed.description,
                parsed.subscribers,
                timestamp
            ],
        )
        .map_err(database_error)?;
    load_channel(connection, &username)?.ok_or_else(|| "频道保存后读取失败".to_owned())
}

fn list_saved(connection: &Connection) -> Result<Vec<TelegramChannel>, String> {
    let mut statement = connection
        .prepare(
            "SELECT username,title,photo_url,description,subscribers,added_at,updated_at
             FROM telegram_channels ORDER BY added_at DESC, username",
        )
        .map_err(database_error)?;
    let rows = statement
        .query_map([], channel_from_row)
        .map_err(database_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(database_error)
}

fn load_channel(connection: &Connection, username: &str) -> Result<Option<TelegramChannel>, String> {
    connection
        .query_row(
            "SELECT username,title,photo_url,description,subscribers,added_at,updated_at
             FROM telegram_channels WHERE username=?1",
            params![username],
            channel_from_row,
        )
        .optional()
        .map_err(database_error)
}

fn channel_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TelegramChannel> {
    let username: String = row.get(0)?;
    Ok(TelegramChannel {
        open_url: format!("https://t.me/{username}"),
        username,
        title: row.get(1)?,
        photo_url: row.get(2)?,
        description: row.get(3)?,
        subscribers: row.get(4)?,
        added_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn database_error(error: rusqlite::Error) -> String {
    format!("Telegram 频道库错误：{error}")
}

pub(crate) fn normalize_username(input: &str) -> Result<String, String> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err("请填写公开频道用户名".to_owned());
    }
    let lowered = raw.to_ascii_lowercase();
    if lowered.contains("joinchat")
        || lowered.contains("/+")
        || lowered.contains("t.me/+")
        || lowered.contains("/c/")
        || lowered.contains("tg://join")
    {
        return Err("私有邀请链接不能用公开预览，本版不登录 Telegram".to_owned());
    }
    let mut value = raw;
    for prefix in ["https://", "http://"] {
        if let Some(rest) = value.get(prefix.len()..) {
            if value.to_ascii_lowercase().starts_with(prefix) {
                value = rest;
            }
        }
    }
    let lower_value = value.to_ascii_lowercase();
    if let Some(rest) = lower_value.strip_prefix("tg://resolve?domain=") {
        value = &value[value.len() - rest.len()..];
    } else if let Some(rest) = lower_value.strip_prefix("t.me/") {
        value = &value[value.len() - rest.len()..];
    } else if let Some(rest) = lower_value.strip_prefix("telegram.me/") {
        value = &value[value.len() - rest.len()..];
    }
    if let Some(rest) = value.strip_prefix("s/") {
        value = rest;
    }
    value = value.trim_start_matches('@');
    value = value.split(['/', '?', '#']).next().unwrap_or("");
    let username = value.trim().to_ascii_lowercase();
    if !is_public_username(&username) {
        return Err("不是公开频道用户名。请用 @name 或 t.me/name，不要用私有链接。".to_owned());
    }
    Ok(username)
}

fn is_public_username(username: &str) -> bool {
    let mut chars = username.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    username.len() >= 5
        && username.len() <= 32
        && first.is_ascii_alphabetic()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn channel_url(username: &str, before: Option<i64>) -> String {
    match before {
        Some(id) => format!("https://t.me/s/{username}?before={id}"),
        None => format!("https://t.me/s/{username}"),
    }
}

async fn fetch_preview(url: &str) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(4))
        .build()
        .map_err(|error| format!("Telegram 请求初始化失败：{error}"))?;
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "text/html,application/xhtml+xml")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en")
        .send()
        .await
        .map_err(|error| format!("无法打开 Telegram 公开页：{error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Telegram 公开页返回 {status}"));
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("Telegram 公开页读取失败：{error}"))?;
    if body.len() > 2_000_000 {
        return Err("Telegram 公开页过大".to_owned());
    }
    Ok(body)
}

fn parse_preview(html: &str, requested: &str, now: i64) -> ParsedPreview {
    let kind = if html.contains("tgme_channel_info") {
        PreviewKind::Channel
    } else if html.contains("Start Bot") || html.contains("monthly users") {
        PreviewKind::Bot
    } else {
        PreviewKind::Unavailable
    };
    let username = inner_text(html, "tgme_channel_info_header_username")
        .and_then(|value| normalize_username(&value).ok())
        .or_else(|| {
            if kind == PreviewKind::Channel && is_public_username(requested) {
                Some(requested.to_owned())
            } else {
                None
            }
        });
    let mut posts = message_posts(html, requested, now);
    posts.reverse();
    ParsedPreview {
        kind,
        username,
        title: inner_text(html, "tgme_channel_info_header_title"),
        photo_url: channel_photo(html),
        description: inner_text(html, "tgme_channel_info_description"),
        subscribers: subscribers(html),
        posts,
        next_before: data_before(html),
    }
}

fn message_posts(html: &str, requested: &str, now: i64) -> Vec<TelegramPost> {
    let marker = "data-post=\"";
    let mut posts = Vec::new();
    let mut search = 0;
    while let Some(rel) = html[search..].find(marker) {
        let value_start = search + rel + marker.len();
        let Some(value_end_rel) = html[value_start..].find('"') else {
            break;
        };
        let value_end = value_start + value_end_rel;
        let post_ref = &html[value_start..value_end];
        let next = html[value_end..]
            .find(marker)
            .map(|offset| value_end + offset)
            .unwrap_or(html.len());
        if let Some(post) = parse_post(post_ref, &html[value_end..next], requested, now) {
            posts.push(post);
        }
        search = value_end;
        if posts.len() == 40 {
            break;
        }
    }
    posts
}

fn parse_post(post_ref: &str, block: &str, requested: &str, now: i64) -> Option<TelegramPost> {
    let (owner, id) = post_ref.split_once('/')?;
    if normalize_username(owner).ok().as_deref() != Some(requested) {
        return None;
    }
    let id = id.parse::<i64>().ok()?;
    if id <= 0 {
        return None;
    }
    let open_url = format!("https://t.me/{requested}/{id}");
    let bubble = block
        .find("tgme_widget_message_bubble")
        .map(|index| &block[index..])
        .unwrap_or(block);
    let videos = video_media(bubble, &open_url, now);
    let photos = photo_media(bubble, &open_url, now);
    let too_big = videos.is_empty() && too_big_label(bubble);
    let mut media = Vec::new();
    if too_big {
        media.push(TelegramMedia {
            kind: "tooBig".to_owned(),
            url: None,
            poster_url: first_background(bubble, "tgme_widget_message_video_thumb"),
            width: None,
            height: None,
            duration: None,
            expires_at: None,
            resolve_before_play: false,
            open_url: open_url.clone(),
        });
    }
    media.extend(videos);
    media.extend(photos);
    let text = inner_html(bubble, "js-message_text")
        .map(|value| html_to_text(&value))
        .unwrap_or_default();
    Some(TelegramPost {
        id,
        text,
        date: attr_value(bubble, "datetime"),
        link: open_url,
        views: inner_text(bubble, "tgme_widget_message_views"),
        preview_limited: media.is_empty() && bubble.contains("media_not_supported_cont"),
        media,
    })
}

fn too_big_post(username: &str, id: i64, poster: Option<String>, _now: i64) -> TelegramPost {
    let open_url = format!("https://t.me/{username}/{id}");
    TelegramPost {
        id,
        text: String::new(),
        date: None,
        link: open_url.clone(),
        views: None,
        preview_limited: false,
        media: vec![TelegramMedia {
            kind: "tooBig".to_owned(),
            url: None,
            poster_url: poster,
            width: None,
            height: None,
            duration: None,
            expires_at: None,
            resolve_before_play: false,
            open_url,
        }],
    }
}

fn video_media(bubble: &str, open_url: &str, now: i64) -> Vec<TelegramMedia> {
    let posters = backgrounds(bubble, "tgme_widget_message_video_thumb");
    let durations = all_inner_text(bubble, "message_video_duration");
    let mut media = Vec::new();
    let mut search = 0;
    while let Some(rel) = bubble[search..].find("<video") {
        let tag_start = search + rel;
        let tag_end = bubble[tag_start..]
            .find('>')
            .map(|offset| tag_start + offset + 1)
            .unwrap_or(bubble.len());
        let tag = &bubble[tag_start..tag_end];
        search = tag_end;
        if !tag.contains("js-message_video") && !tag.contains("tgme_widget_message_video") {
            continue;
        }
        let Some(url) = attr_value(tag, "src").and_then(|value| normalize_url(&value)) else {
            continue;
        };
        let (width, height) = video_size_before(bubble, tag_start);
        let index = media.len();
        media.push(TelegramMedia {
            kind: "video".to_owned(),
            expires_at: Some(expires_at(&url, now)),
            resolve_before_play: true,
            poster_url: posters.get(index).cloned(),
            duration: durations.get(index).cloned(),
            width,
            height,
            url: Some(url),
            open_url: open_url.to_owned(),
        });
    }
    media
}

fn photo_media(bubble: &str, open_url: &str, now: i64) -> Vec<TelegramMedia> {
    backgrounds(bubble, "tgme_widget_message_photo_wrap")
        .into_iter()
        .filter(|url| !url.contains("/img/emoji/"))
        .map(|url| TelegramMedia {
            kind: "image".to_owned(),
            expires_at: Some(expires_at(&url, now)),
            resolve_before_play: signed_cdn(&url),
            poster_url: None,
            width: None,
            height: None,
            duration: None,
            url: Some(url),
            open_url: open_url.to_owned(),
        })
        .collect()
}

fn video_size_before(html: &str, before: usize) -> (Option<i64>, Option<i64>) {
    let head = &html[..before];
    let Some(index) = head.rfind("tgme_widget_message_video_wrap") else {
        return (None, None);
    };
    let tag_end = head[index..]
        .find('>')
        .map(|offset| index + offset + 1)
        .unwrap_or(head.len());
    let style = attr_value(&head[index..tag_end], "style").unwrap_or_default();
    let width = css_px(&style, "width");
    let ratio = css_percent(&style, "padding-top");
    let height = width.and_then(|value| ratio.map(|percent| (value as f64 * percent / 100.0).round() as i64));
    (width, height)
}

fn css_px(style: &str, name: &str) -> Option<i64> {
    let token = format!("{name}:");
    let index = style.find(&token)?;
    let rest = style[index + token.len()..].trim_start();
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn css_percent(style: &str, name: &str) -> Option<f64> {
    let token = format!("{name}:");
    let index = style.find(&token)?;
    let rest = style[index + token.len()..].trim_start();
    let number: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect();
    number.parse().ok()
}

fn backgrounds(html: &str, class_name: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut search = 0;
    while let Some(index) = find_class(html, class_name, search) {
        let tag_end = html[index..]
            .find('>')
            .map(|offset| index + offset + 1)
            .unwrap_or(html.len());
        if let Some(url) = attr_value(&html[index..tag_end], "style").and_then(|style| css_url(&style))
        {
            urls.push(url);
        }
        search = tag_end;
    }
    urls
}

fn first_background(html: &str, class_name: &str) -> Option<String> {
    backgrounds(html, class_name).into_iter().next()
}

fn too_big_label(html: &str) -> bool {
    all_inner_text(html, "message_media_not_supported_label")
        .into_iter()
        .any(|label| {
            let label = label.to_ascii_lowercase();
            label.contains("too big") || label.contains("过大") || label.contains("太大")
        })
}

fn subscribers(html: &str) -> Option<String> {
    let mut search = 0;
    while let Some(rel) = html[search..].find("tgme_channel_info_counter") {
        let start = search + rel;
        let end = html[start..]
            .find("</div>")
            .map(|offset| start + offset)
            .unwrap_or(html.len());
        let slice = &html[start..end];
        if slice.contains(">subscribers<") || slice.contains(">subscriber<") {
            return inner_text(slice, "counter_value");
        }
        search = end;
    }
    None
}

fn channel_photo(html: &str) -> Option<String> {
    let index = html.find("tgme_page_photo_image")?;
    let end = html[index..]
        .find("tgme_channel_info_counters")
        .map(|offset| index + offset)
        .unwrap_or(html.len().min(index + 2000));
    attr_value(&html[index..end], "src").and_then(|value| normalize_url(&value))
}

fn data_before(html: &str) -> Option<i64> {
    let marker = "data-before=\"";
    let index = html.find(marker)?;
    let start = index + marker.len();
    let end = html[start..].find('"')? + start;
    let id = html[start..end].parse::<i64>().ok()?;
    (id > 0).then_some(id)
}

fn inner_text(html: &str, class_name: &str) -> Option<String> {
    let text = html_to_text(&inner_html(html, class_name)?);
    (!text.is_empty()).then_some(text)
}

fn all_inner_text(html: &str, class_name: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut search = 0;
    while let Some(index) = find_class(html, class_name, search) {
        let Some(inner) = element_inner(html, index) else {
            break;
        };
        let text = html_to_text(inner);
        if !text.is_empty() {
            values.push(text);
        }
        search = index + class_name.len();
    }
    values
}

fn inner_html(html: &str, class_name: &str) -> Option<String> {
    let index = find_class(html, class_name, 0)?;
    element_inner(html, index).map(str::to_owned)
}

fn element_inner(html: &str, class_at: usize) -> Option<&str> {
    let start = html[..class_at].rfind('<')?;
    let tag = tag_name(html, start)?;
    let inner_start = html[class_at..].find('>')? + class_at + 1;
    Some(take_until_close(html, inner_start, tag))
}

fn tag_name(html: &str, lt: usize) -> Option<&str> {
    let rest = html.get(lt + 1..)?;
    let end = rest
        .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
        .unwrap_or(rest.len());
    let name = &rest[..end];
    (!name.is_empty() && !name.starts_with('!')).then_some(name)
}

fn take_until_close<'a>(html: &'a str, start: usize, tag: &str) -> &'a str {
    let mut depth = 1;
    let mut index = start;
    while index < html.len() && depth > 0 {
        let next_open = find_tag(html, index, tag, false);
        let next_close = find_tag(html, index, tag, true);
        match (next_open, next_close) {
            (_, Some(close)) if next_open.map(|open| close < open).unwrap_or(true) => {
                depth -= 1;
                if depth == 0 {
                    return &html[start..close];
                }
                index = close + tag.len() + 2;
            }
            (Some(open), _) => {
                depth += 1;
                index = open + tag.len() + 1;
            }
            _ => break,
        }
        if index > start + 50_000 {
            break;
        }
    }
    let end = html.len().min(start + 20_000);
    &html[start..end]
}

fn find_tag(html: &str, from: usize, tag: &str, closing: bool) -> Option<usize> {
    let needle = if closing {
        format!("</{tag}")
    } else {
        format!("<{tag}")
    };
    let mut search = from;
    while let Some(rel) = html[search..].find(&needle) {
        let index = search + rel;
        let after = index + needle.len();
        let boundary = html.as_bytes().get(after).copied().unwrap_or(b' ');
        if boundary.is_ascii_whitespace() || boundary == b'>' || boundary == b'/' {
            return Some(index);
        }
        search = after;
    }
    None
}

fn find_class(html: &str, class_name: &str, from: usize) -> Option<usize> {
    let mut search = from;
    while search < html.len() {
        let rel = html[search..].find("class=")?;
        let attr = search + rel;
        let quote_at = attr + "class=".len();
        let quote = *html.as_bytes().get(quote_at)?;
        if quote != b'"' && quote != b'\'' {
            search = quote_at + 1;
            continue;
        }
        let value_start = quote_at + 1;
        let value_end_rel = html[value_start..].find(quote as char)?;
        let value = &html[value_start..value_start + value_end_rel];
        if value.split_whitespace().any(|token| token == class_name) {
            return Some(attr);
        }
        search = value_start + value_end_rel + 1;
    }
    None
}

fn attr_value(tag: &str, name: &str) -> Option<String> {
    let mut search = 0;
    let pattern = format!("{name}=");
    while let Some(rel) = tag[search..].find(&pattern) {
        let at = search + rel;
        let boundary = at == 0
            || tag.as_bytes()[at - 1].is_ascii_whitespace()
            || tag.as_bytes()[at - 1] == b'<';
        if boundary {
            return Some(decode_entities(read_attr(&tag[at + pattern.len()..])?));
        }
        search = at + pattern.len();
    }
    None
}

fn read_attr(value: &str) -> Option<&str> {
    let value = value.trim_start();
    let mut chars = value.chars();
    let quote = chars.next()?;
    if quote == '"' || quote == '\'' {
        let end = value[1..].find(quote)?;
        Some(&value[1..1 + end])
    } else {
        let end = value
            .find(|ch: char| ch.is_whitespace() || ch == '>')
            .unwrap_or(value.len());
        Some(&value[..end])
    }
}

fn css_url(style: &str) -> Option<String> {
    let index = style.find("url(")?;
    let rest = style[index + 4..].trim_start();
    let raw = if rest.starts_with(['"', '\'']) {
        let quote = rest.chars().next()?;
        let end = rest[1..].find(quote)?;
        &rest[1..1 + end]
    } else {
        let end = rest.find(')')?;
        rest[..end].trim()
    };
    normalize_url(raw)
}

fn normalize_url(raw: &str) -> Option<String> {
    let decoded = decode_entities(raw.trim());
    let url = if let Some(rest) = decoded.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        decoded
    };
    if url.starts_with("https://") || url.starts_with("http://") {
        Some(url)
    } else {
        None
    }
}

fn signed_cdn(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("token=")
        || lower.contains("telesco.pe")
        || lower.contains("telegram-cdn")
        || lower.contains("cdn-telegram")
}

fn expires_at(url: &str, now: i64) -> i64 {
    let cap = now + PREVIEW_TTL_SECS;
    match explicit_expiry(url) {
        Some(value) if value > now => value.min(cap),
        Some(_) => now,
        None => cap,
    }
}

fn explicit_expiry(url: &str) -> Option<i64> {
    let query = url.split_once('?')?.1;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if matches!(key, "expires" | "expire" | "exp" | "expires_at") {
            let parsed = value.parse::<i64>().ok()?;
            if parsed > 1_000_000_000 {
                return Some(parsed);
            }
        }
    }
    None
}

fn html_to_text(html: &str) -> String {
    let with_breaks = html
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    let mut text = String::new();
    let mut in_tag = false;
    for ch in with_breaks.chars() {
        if ch == '<' {
            in_tag = true;
            continue;
        }
        if ch == '>' {
            in_tag = false;
            continue;
        }
        if !in_tag {
            text.push(ch);
        }
    }
    let decoded = decode_entities(&text);
    decoded
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_entities(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        let Some(end) = rest.find(';') else {
            out.push_str(rest);
            return out;
        };
        let entity = &rest[1..end];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" | "#39" => out.push('\''),
            "nbsp" => out.push(' '),
            _ => {
                if let Some(hex) = entity.strip_prefix("#x").or_else(|| entity.strip_prefix("#X")) {
                    if let Some(ch) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    } else {
                        out.push_str(&rest[..=end]);
                    }
                } else if let Some(decimal) = entity.strip_prefix('#') {
                    if let Some(ch) = decimal.parse::<u32>().ok().and_then(char::from_u32) {
                        out.push(ch);
                    } else {
                        out.push_str(&rest[..=end]);
                    }
                } else {
                    out.push_str(&rest[..=end]);
                }
            }
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    fn fixture() -> String {
        r#"
        <div class="tgme_channel_info">
          <i class="tgme_page_photo_image"><img src="https://cdn.example/photo.jpg"></i>
          <div class="tgme_channel_info_header_title"><span dir="auto">Example News</span></div>
          <div class="tgme_channel_info_header_username"><a href="https://t.me/example">@Example</a></div>
          <div class="tgme_channel_info_counters">
            <div class="tgme_channel_info_counter"><span class="counter_value">12K</span> <span class="counter_type">subscribers</span></div>
          </div>
          <div class="tgme_channel_info_description">Hello &amp; welcome</div>
        </div>
        <a class="tme_messages_more" href="/s/example?before=9" data-before="9"></a>
        <div class="tgme_widget_message" data-post="example/11">
          <img src="https://cdn.example/avatar.jpg">
          <div class="tgme_widget_message_bubble">
            <a class="tgme_widget_message_video_player" href="https://t.me/example/11">
              <i class="tgme_widget_message_video_thumb" style="background-image:url('https://cdn.example/poster.jpg')"></i>
              <div class="tgme_widget_message_video_wrap" style="width:800px;padding-top:50%">
                <video src="https://cdn.example/file/a.mp4?token=abc&amp;x=1" class="tgme_widget_message_video js-message_video"></video>
              </div>
              <time class="message_video_duration">0:06</time>
              <div class="message_media_not_supported_label">This media is not supported in your browser</div>
              <div class="message_media_not_supported_label">Please open Telegram to view this post</div>
            </a>
            <div class="tgme_widget_message_text js-message_text" dir="auto">Line&amp;1<br/>Line 2</div>
            <div class="tgme_widget_message_link_preview"><img src="https://cdn.example/card.jpg"></div>
            <span class="tgme_widget_message_views">9</span>
            <time datetime="2026-07-19T17:57:15+00:00"></time>
          </div>
        </div>
        <div class="tgme_widget_message" data-post="example/10">
          <div class="tgme_widget_message_bubble">
            <a class="tgme_widget_message_photo_wrap" style="background-image:url('https://cdn.example/one.jpg')"></a>
            <a class="tgme_widget_message_photo_wrap" style="background-image:url('https://cdn.example/two.jpg')"></a>
            <div class="tgme_widget_message_text js-message_text">Photo</div>
          </div>
        </div>
        <div class="tgme_widget_message" data-post="example/9">
          <div class="tgme_widget_message_bubble">
            <i class="tgme_widget_message_video_thumb" style="background-image:url('https://cdn.example/big.jpg')"></i>
            <div class="message_media_not_supported_label">Media is too big</div>
            <div class="tgme_widget_message_text js-message_text">Big</div>
          </div>
        </div>
        "#
        .to_owned()
    }

    #[test]
    fn normalizes_public_names_and_rejects_private_links() {
        assert_eq!(normalize_username("@Example").unwrap(), "example");
        assert_eq!(
            normalize_username("https://t.me/s/Example/11?single").unwrap(),
            "example"
        );
        assert_eq!(
            normalize_username("tg://resolve?domain=Example").unwrap(),
            "example"
        );
        assert!(normalize_username("https://t.me/+abcde").is_err());
        assert!(normalize_username("https://t.me/joinchat/abcde").is_err());
        assert!(normalize_username("https://t.me/c/123/4").is_err());
        assert!(normalize_username("ab").is_err());
    }

    #[test]
    fn parses_direct_video_photo_and_too_big_without_copying_page_noise() {
        let parsed = parse_preview(&fixture(), "example", NOW);
        assert_eq!(parsed.kind, PreviewKind::Channel);
        assert_eq!(parsed.username.as_deref(), Some("example"));
        assert_eq!(parsed.title.as_deref(), Some("Example News"));
        assert_eq!(parsed.subscribers.as_deref(), Some("12K"));
        assert_eq!(parsed.description.as_deref(), Some("Hello & welcome"));
        assert_eq!(
            parsed.photo_url.as_deref(),
            Some("https://cdn.example/photo.jpg")
        );
        assert_eq!(parsed.next_before, Some(9));
        assert_eq!(
            parsed.posts.iter().map(|post| post.id).collect::<Vec<_>>(),
            vec![11, 10, 9]
        );

        let video = &parsed.posts[0];
        assert_eq!(video.text, "Line&1\nLine 2");
        assert_eq!(video.views.as_deref(), Some("9"));
        assert_eq!(video.date.as_deref(), Some("2026-07-19T17:57:15+00:00"));
        assert!(!video.preview_limited);
        assert_eq!(video.media.len(), 1);
        assert_eq!(video.media[0].kind, "video");
        assert_eq!(
            video.media[0].url.as_deref(),
            Some("https://cdn.example/file/a.mp4?token=abc&x=1")
        );
        assert_eq!(
            video.media[0].poster_url.as_deref(),
            Some("https://cdn.example/poster.jpg")
        );
        assert_eq!(video.media[0].duration.as_deref(), Some("0:06"));
        assert_eq!(video.media[0].width, Some(800));
        assert_eq!(video.media[0].height, Some(400));
        assert_eq!(video.media[0].expires_at, Some(NOW + PREVIEW_TTL_SECS));
        assert!(video.media[0].resolve_before_play);

        let photos = &parsed.posts[1].media;
        assert_eq!(photos.len(), 2);
        assert_eq!(photos[0].kind, "image");
        assert_eq!(photos[0].url.as_deref(), Some("https://cdn.example/one.jpg"));
        assert_eq!(photos[1].url.as_deref(), Some("https://cdn.example/two.jpg"));

        let too_big = &parsed.posts[2].media[0];
        assert_eq!(too_big.kind, "tooBig");
        assert_eq!(too_big.url, None);
        assert!(!too_big.resolve_before_play);
        assert_eq!(too_big.open_url, "https://t.me/example/9");
        assert_eq!(
            too_big.poster_url.as_deref(),
            Some("https://cdn.example/big.jpg")
        );
    }

    #[test]
    fn classifies_bots_and_missing_previews() {
        let bot = parse_preview(
            r#"<a href="tg://resolve?domain=BotFather">Start Bot</a>"#,
            "botfather",
            NOW,
        );
        assert_eq!(bot.kind, PreviewKind::Bot);
        let missing = parse_preview("<div>If you have Telegram</div>", "missing", NOW);
        assert_eq!(missing.kind, PreviewKind::Unavailable);
    }

    #[test]
    fn rich_text_fallback_is_not_a_playable_media_item() {
        let html = r#"
            <div class="tgme_channel_info"></div>
            <div data-post="example/8">
              <div class="tgme_widget_message_bubble">
                <div class="media_supported_cont">
                  <div class="tgme_widget_message_text js-message_text">Only text</div>
                </div>
                <div class="media_not_supported_cont">
                  <div class="message_media_not_supported_label">Please open Telegram to view this post</div>
                </div>
              </div>
            </div>"#;
        let parsed = parse_preview(html, "example", NOW);
        assert_eq!(parsed.posts.len(), 1);
        assert!(parsed.posts[0].media.is_empty());
        assert!(parsed.posts[0].preview_limited);
        assert_eq!(parsed.posts[0].link, "https://t.me/example/8");
    }

    #[test]
    fn explicit_expiry_is_capped_at_the_refresh_window() {
        assert_eq!(
            expires_at("https://cdn.example/a.mp4?expires=1800000000", NOW),
            NOW + PREVIEW_TTL_SECS
        );
        assert_eq!(
            expires_at("https://cdn.example/a.mp4?exp=1700000100", NOW),
            1_700_000_100
        );
    }

    #[test]
    fn channel_upsert_keeps_the_original_added_time() {
        let connection = Connection::open_in_memory().unwrap();
        ensure_schema(&connection).unwrap();
        let mut parsed = parse_preview(&fixture(), "example", NOW);
        let first = upsert_channel(&connection, &parsed).unwrap();
        parsed.title = Some("Renamed".to_owned());
        let second = upsert_channel(&connection, &parsed).unwrap();
        assert_eq!(second.added_at, first.added_at);
        assert_eq!(second.title, "Renamed");
        assert!(second.updated_at >= first.updated_at);
        assert_eq!(list_saved(&connection).unwrap().len(), 1);
    }
}
