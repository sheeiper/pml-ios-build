//! Local, explicitly unconfirmed performer candidates derived from verified movie credits.
//!
//! A `(source, raw_name)` group is NOT a verified human identity: unrelated people can
//! share a credit, and different sources are never merged. The stable opaque ID is
//! independent of editable names/aliases. Only an explicit user action may set
//! `confirmed`; even then no other candidate or credit is merged automatically.
//!
//! Public reads synchronize changed credit lists before returning. Synchronization
//! fingerprints the actual actors/source JSON (not timestamps), never overwrites
//! editable person fields, and replaces stale links only for changed verified movies.
//! Database operations perform no networking, face recognition, or sensitive-profile
//! collection. A separate explicit JavBus profile fetch can return a public portrait
//! preview after structural/name/identity validation; it never saves or confirms it.
//! Remote avatars must be displayed through the host's existing safe image proxy.
//! Inline uploads are restricted to JPEG/PNG/WebP base64, at most 2 MiB decoded.
//! `update_performer` replaces all editable fields; None clears either URL. Status
//! is deliberately a separate, explicit operation. Detail movies are capped at 100;
//! `total` and `person.movieCount` count distinct verified movie numbers in this DB,
//! not files, external filmographies, or a claim that identities have been verified.

use crate::jav_scraper::{ensure_scraper_schema, MovieMetadata};
use base64::{engine::general_purpose::STANDARD, Engine};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt};

const MAX_AVATAR_BYTES: usize = 2 * 1024 * 1024;
const MAX_NAME_CHARS: usize = 200;
const MAX_ALIASES: usize = 50;
const VERIFIED: &str = "m.verification_version = 1 AND m.source NOT IN ('javdb_not_found', 'not_found', '') AND m.cover_url IS NOT NULL AND m.cover_url != ''";

#[derive(Debug)]
pub enum PerformerError {
    Database(rusqlite::Error),
    Validation(String),
    NotFound,
}

impl fmt::Display for PerformerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(error) => write!(f, "{error}"),
            Self::Validation(message) => write!(f, "{message}"),
            Self::NotFound => write!(f, "Performer not found"),
        }
    }
}
impl std::error::Error for PerformerError {}
impl From<rusqlite::Error> for PerformerError {
    fn from(error: rusqlite::Error) -> Self { Self::Database(error) }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Performer {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub avatar_url: Option<String>,
    pub profile_url: Option<String>,
    pub source: String,
    pub status: String,
    pub updated_at: i64,
    pub movie_count: u64,
    #[serde(default)]
    pub birth_date: Option<String>,
    #[serde(default)]
    pub height_cm: Option<i64>,
    #[serde(default)]
    pub measurements: Option<String>,
    #[serde(default)]
    pub hometown: Option<String>,
    #[serde(default)]
    pub hobby: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub blood_type: Option<String>,
    #[serde(default)]
    pub debut_year: Option<String>,
    #[serde(default)]
    pub agency: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformerList {
    pub items: Vec<Performer>,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformerDetail {
    pub person: Performer,
    pub movies: Vec<MovieMetadata>,
    pub total: u64,
}

/// Idempotent schema setup. The host should call this during its usual DB setup.
pub fn ensure_performer_schema(conn: &Connection) -> Result<(), PerformerError> {
    ensure_scraper_schema(conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS performers (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            aliases_json TEXT NOT NULL DEFAULT '[]',
            avatar_url TEXT,
            profile_url TEXT,
            source TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'unconfirmed' CHECK(status IN ('unconfirmed','confirmed')),
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS movie_cast (
            number TEXT NOT NULL REFERENCES movie_metadata(number) ON DELETE CASCADE,
            person_id TEXT NOT NULL REFERENCES performers(id) ON DELETE CASCADE,
            raw_name TEXT NOT NULL,
            PRIMARY KEY(number, person_id, raw_name)
        );
        CREATE INDEX IF NOT EXISTS idx_movie_cast_person ON movie_cast(person_id, number);
        CREATE INDEX IF NOT EXISTS idx_performers_name ON performers(name);
        CREATE TABLE IF NOT EXISTS performer_sync_state (
            number TEXT PRIMARY KEY REFERENCES movie_metadata(number) ON DELETE CASCADE,
            fingerprint TEXT NOT NULL
        );",
    )?;
    for col in [
        "ALTER TABLE performers ADD COLUMN birth_date TEXT",
        "ALTER TABLE performers ADD COLUMN height_cm INTEGER",
        "ALTER TABLE performers ADD COLUMN measurements TEXT",
        "ALTER TABLE performers ADD COLUMN hometown TEXT",
        "ALTER TABLE performers ADD COLUMN hobby TEXT",
        "ALTER TABLE performers ADD COLUMN bio TEXT",
        "ALTER TABLE performers ADD COLUMN blood_type TEXT",
        "ALTER TABLE performers ADD COLUMN debut_year TEXT",
        "ALTER TABLE performers ADD COLUMN agency TEXT",
    ] {
        let _ = conn.execute(col, []);
    }
    Ok(())
}

fn digest_parts(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        // Length framing prevents delimiter ambiguity; preserve case and Unicode.
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn candidate_id(source: &str, raw_name: &str) -> String {
    format!("person_{}", digest_parts(&["performer-v1", source, raw_name]))
}

/// Incremental by actual source/actors content, even when updated_at is unchanged.
/// Existing person records are insert-only here, including zero-credit candidates.
/// Orphan links are removed for deleted movies; quarantined metadata is excluded
/// from reads/counts, not interpreted as evidence for new or updated candidates.
pub fn sync_performers(conn: &Connection) -> Result<(), PerformerError> {
    ensure_performer_schema(conn)?;
    let rows = {
        let mut stmt = conn.prepare(&format!(
            "SELECT m.number,m.source,m.actors_json,s.fingerprint
             FROM movie_metadata m LEFT JOIN performer_sync_state s ON s.number=m.number
             WHERE {VERIFIED}"
        ))?;
        let mapped = stmt.query_map([], |row| Ok((
            row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?,
        )))?;
        mapped.collect::<Result<Vec<_>, _>>()?
    };
    let mut changed = Vec::new();
    for (number, source, actors_json, previous) in rows {
        let fingerprint = digest_parts(&["credits-v1", &source, &actors_json]);
        if previous.as_deref() == Some(fingerprint.as_str()) { continue; }
        // Malformed actor data is not evidence to delete previously valid credits.
        let Ok(actors) = serde_json::from_str::<Vec<String>>(&actors_json) else { continue; };
        changed.push((number, source, actors, fingerprint));
    }
    let has_orphans: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM movie_cast c WHERE NOT EXISTS(SELECT 1 FROM movie_metadata m WHERE m.number=c.number))
         OR EXISTS(SELECT 1 FROM performer_sync_state s WHERE NOT EXISTS(SELECT 1 FROM movie_metadata m WHERE m.number=s.number))",
        [], |row| row.get(0),
    )?;
    if changed.is_empty() && !has_orphans { return Ok(()); }

    // A savepoint supports callers already in a transaction; no write transaction
    // is acquired at all for unchanged credits. On error Drop rolls it back.
    let savepoint = SyncSavepoint::new(conn)?;
    for (number, source, actors, fingerprint) in changed {
        // Recheck within the savepoint's snapshot: another connection may have
        // changed the movie between our initial scan and this savepoint. SQLite
        // rejects stale snapshot write upgrades rather than silently losing data.
        let current: Option<(String, String)> = savepoint.query_row(
            &format!("SELECT m.source,m.actors_json FROM movie_metadata m WHERE m.number=?1 AND {VERIFIED}"),
            [&number], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        if !current.as_ref().is_some_and(|(s,a)| digest_parts(&["credits-v1",s,a]) == fingerprint) {
            continue;
        }
        savepoint.execute("DELETE FROM movie_cast WHERE number=?1", [&number])?;
        let unique: BTreeSet<String> = actors.into_iter().collect();
        for raw_name in unique {
            // Preserve the exact raw credit as the identity key and relation.
            // Empty/invalid credits must not become phantom people.
            let Ok(name) = validate_name(&raw_name, "name") else { continue; };
            let id = candidate_id(&source, &raw_name);
            savepoint.execute(
                "INSERT INTO performers(id,name,source,status,updated_at)
                 VALUES (?1,?2,?3,'confirmed',?4) ON CONFLICT(id) DO NOTHING",
                params![id, name, source, chrono::Utc::now().timestamp()],
            )?;
            savepoint.execute(
                "INSERT INTO movie_cast(number,person_id,raw_name) VALUES (?1,?2,?3)
                 ON CONFLICT(number,person_id,raw_name) DO NOTHING",
                params![number, id, raw_name],
            )?;
        }
        savepoint.execute(
            "INSERT INTO performer_sync_state(number,fingerprint) VALUES (?1,?2)
             ON CONFLICT(number) DO UPDATE SET fingerprint=excluded.fingerprint",
            params![number, fingerprint],
        )?;
    }
    if has_orphans {
        savepoint.execute("DELETE FROM movie_cast WHERE NOT EXISTS(SELECT 1 FROM movie_metadata m WHERE m.number=movie_cast.number)", [])?;
        savepoint.execute("DELETE FROM performer_sync_state WHERE NOT EXISTS(SELECT 1 FROM movie_metadata m WHERE m.number=performer_sync_state.number)", [])?;
    }
    savepoint.commit()?;
    Ok(())
}

// rusqlite 0.32's public Savepoint constructors require &mut Connection, while
// this module deliberately matches the host's shared &Connection API. A scoped
// guard provides the same rollback-on-error contract without unsafe casts.
struct SyncSavepoint<'a> {
    conn: &'a Connection,
    released: bool,
}
impl<'a> SyncSavepoint<'a> {
    fn new(conn: &'a Connection) -> rusqlite::Result<Self> {
        conn.execute_batch("SAVEPOINT performer_sync_internal")?;
        Ok(Self { conn, released: false })
    }
    fn commit(mut self) -> rusqlite::Result<()> {
        self.conn.execute_batch("RELEASE SAVEPOINT performer_sync_internal")?;
        self.released = true;
        Ok(())
    }
}
impl std::ops::Deref for SyncSavepoint<'_> {
    type Target = Connection;
    fn deref(&self) -> &Self::Target { self.conn }
}
impl Drop for SyncSavepoint<'_> {
    fn drop(&mut self) {
        if !self.released {
            let _ = self.conn.execute_batch("ROLLBACK TO SAVEPOINT performer_sync_internal; RELEASE SAVEPOINT performer_sync_internal");
        }
    }
}

fn person_select() -> String {
    format!("SELECT p.id,p.name,p.aliases_json,p.avatar_url,p.profile_url,p.source,p.status,p.updated_at,
        (SELECT COUNT(DISTINCT c.number) FROM movie_cast c JOIN movie_metadata m ON m.number=c.number
         WHERE c.person_id=p.id AND {VERIFIED}) AS movie_count,
        p.birth_date,p.height_cm,p.measurements,p.hometown,p.hobby,p.bio,p.blood_type,p.debut_year,p.agency FROM performers p")
}

fn person_from_row(row: &Row<'_>) -> rusqlite::Result<Performer> {
    let json: String = row.get(2)?;
    Ok(Performer {
        id: row.get(0)?, name: row.get(1)?,
        aliases: serde_json::from_str(&json).unwrap_or_default(),
        avatar_url: row.get(3)?, profile_url: row.get(4)?, source: row.get(5)?,
        status: row.get(6)?, updated_at: row.get(7)?, movie_count: row.get(8)?,
        birth_date: row.get(9)?, height_cm: row.get(10)?, measurements: row.get(11)?,
        hometown: row.get(12)?, hobby: row.get(13)?, bio: row.get(14)?,
        blood_type: row.get(15)?, debut_year: row.get(16)?, agency: row.get(17)?,
    })
}

fn read_person(conn: &Connection, id: &str) -> Result<Option<Performer>, PerformerError> {
    Ok(conn.query_row(&format!("{} WHERE p.id=?1", person_select()), [id], person_from_row).optional()?)
}

/// Search current names and aliases as literal substrings (not SQL wildcards).
/// Limit is capped at 100; zero requests an empty page with an accurate total.
/// Retains edited zero-credit profiles rather than silently deleting user data.
pub fn list_performers(conn: &Connection, query: &str, limit: u32, offset: u32) -> Result<PerformerList, PerformerError> {
    if query.chars().count() > 200 { return Err(PerformerError::Validation("Search query is too long".into())); }
    sync_performers(conn)?;
    let query = query.trim();
    let filter = "(?1='' OR instr(lower(p.name),lower(?1))>0 OR EXISTS(
        SELECT 1 FROM json_each(p.aliases_json) a WHERE instr(lower(a.value),lower(?1))>0))";
    let total = conn.query_row(&format!("SELECT COUNT(*) FROM performers p WHERE {filter}"), [query], |row| row.get(0))?;
    let mut stmt = conn.prepare(&format!("{} WHERE {filter} ORDER BY movie_count DESC,p.name COLLATE NOCASE,p.id LIMIT ?2 OFFSET ?3", person_select()))?;
    let items = stmt.query_map(params![query, limit.min(100), offset], person_from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(PerformerList { items, total })
}

/// Return at most 100 locally verified works, newest release first. `total` is
/// the complete local movie count, even when the returned vector is truncated.
pub fn get_performer(conn: &Connection, id: &str) -> Result<Option<PerformerDetail>, PerformerError> {
    sync_performers(conn)?;
    let Some(person) = read_person(conn, id)? else { return Ok(None); };
    let mut stmt = conn.prepare(&format!(
        "SELECT m.number,m.asset_id,m.title,m.original_title,m.actors_json,m.maker,m.label,m.series,
                m.director,m.release_date,m.duration_minutes,m.cover_url,m.sample_images_json,
                m.tags_json,m.source,m.updated_at
         FROM movie_metadata m WHERE {VERIFIED}
           AND EXISTS(SELECT 1 FROM movie_cast c WHERE c.number=m.number AND c.person_id=?1)
         ORDER BY CASE WHEN m.release_date IS NULL OR trim(m.release_date) = '' THEN 1 ELSE 0 END ASC, replace(m.release_date, '/', '-') DESC, m.updated_at DESC, m.number ASC LIMIT 100"
    ))?;
    let movies = stmt.query_map([id], |row| {
        let actors: String = row.get(4)?;
        let samples: String = row.get(12)?;
        let tags: String = row.get(13)?;
        Ok(MovieMetadata {
            number: row.get(0)?, asset_id: row.get(1)?, title: row.get(2)?, original_title: row.get(3)?,
            actors: serde_json::from_str(&actors).unwrap_or_default(), maker: row.get(5)?, label: row.get(6)?,
            series: row.get(7)?, director: row.get(8)?, release_date: row.get(9)?, duration_minutes: row.get(10)?,
            cover_url: row.get(11)?, sample_images: serde_json::from_str(&samples).unwrap_or_default(),
            tags: serde_json::from_str(&tags).unwrap_or_default(), source: row.get(14)?, updated_at: row.get(15)?,
        })
    })?.collect::<Result<Vec<_>, _>>()?;
    let total = person.movie_count;
    Ok(Some(PerformerDetail { person, movies, total }))
}

fn validate_name(value: &str, field: &str) -> Result<String, PerformerError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_NAME_CHARS || value.chars().any(char::is_control) {
        return Err(PerformerError::Validation(format!("{field} must contain 1–{MAX_NAME_CHARS} characters without control characters")));
    }
    Ok(trimmed.to_string())
}

fn validate_http_url(value: &str) -> Result<(), PerformerError> {
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(PerformerError::Validation("URL is too long or contains control characters".into()));
    }
    let url = url::Url::parse(value).map_err(|_| PerformerError::Validation("Invalid URL".into()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
        return Err(PerformerError::Validation("Only HTTP(S) URLs without embedded credentials are allowed".into()));
    }
    Ok(())
}

fn validate_avatar(value: Option<&str>) -> Result<Option<String>, PerformerError> {
    let Some(value) = value else { return Ok(None); };
    let value = value.trim();
    if value.is_empty() { return Ok(None); }
    if !value.starts_with("data:") {
        validate_http_url(value)?;
        return Ok(Some(value.to_string()));
    }
    let (prefix, encoded) = value.split_once(',').ok_or_else(|| PerformerError::Validation("Invalid image data URL".into()))?;
    if !matches!(prefix, "data:image/jpeg;base64" | "data:image/png;base64" | "data:image/webp;base64") {
        return Err(PerformerError::Validation("Only JPEG, PNG or WebP base64 uploads are allowed".into()));
    }
    if encoded.len() > MAX_AVATAR_BYTES.div_ceil(3) * 4 {
        return Err(PerformerError::Validation("Avatar exceeds 2 MiB".into()));
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| PerformerError::Validation("Invalid avatar base64".into()))?;
    if bytes.is_empty() || bytes.len() > MAX_AVATAR_BYTES {
        return Err(PerformerError::Validation("Avatar must be non-empty and at most 2 MiB".into()));
    }
    // Reject MIME spoofing (especially SVG/HTML). This is a signature check, not
    // a full image decoder; the browser still validates image payloads on display.
    let matches_signature = match prefix {
        "data:image/jpeg;base64" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "data:image/png;base64" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "data:image/webp;base64" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    };
    if !matches_signature { return Err(PerformerError::Validation("Avatar content does not match its image MIME type".into())); }
    Ok(Some(value.to_string()))
}

/// Full replacement of editable public-profile fields; this does NOT confirm an
/// identity. Aliases are trimmed, deduplicated, capped at 50 x 200 characters.
/// Names/aliases are display/search fields only, never remap original credits.
pub fn update_performer(conn: &Connection, id: &str, name: &str, aliases: &[String], avatar_url: Option<&str>, profile_url: Option<&str>) -> Result<Performer, PerformerError> {
    let name = validate_name(name, "name")?;
    if aliases.len() > MAX_ALIASES { return Err(PerformerError::Validation("At most 50 aliases are allowed".into())); }
    let mut cleaned = Vec::new();
    for alias in aliases {
        let alias = validate_name(alias, "alias")?;
        if !cleaned.contains(&alias) { cleaned.push(alias); }
    }
    let avatar = validate_avatar(avatar_url)?;
    let profile = profile_url.map(str::trim).filter(|url| !url.is_empty());
    if let Some(profile) = profile { validate_http_url(profile)?; }
    let aliases_json = serde_json::to_string(&cleaned).map_err(|error| PerformerError::Validation(error.to_string()))?;
    sync_performers(conn)?;
    let affected = conn.execute(
        "UPDATE performers SET name=?2,aliases_json=?3,avatar_url=?4,profile_url=?5,updated_at=?6 WHERE id=?1",
        params![id, name, aliases_json, avatar, profile, chrono::Utc::now().timestamp()],
    )?;
    if affected == 0 { return Err(PerformerError::NotFound); }
    read_person(conn, id)?.ok_or(PerformerError::NotFound)
}

/// Call only after an explicit user choice to confirm public professional data
/// (or to revert to unconfirmed). Never infers identity from a name or an image.
pub fn set_performer_status(conn: &Connection, id: &str, status: &str) -> Result<Performer, PerformerError> {
    if !matches!(status, "unconfirmed" | "confirmed") {
        return Err(PerformerError::Validation("Status must be unconfirmed or confirmed".into()));
    }
    sync_performers(conn)?;
    let affected = conn.execute("UPDATE performers SET status=?2,updated_at=?3 WHERE id=?1", params![id, status, chrono::Utc::now().timestamp()])?;
    if affected == 0 { return Err(PerformerError::NotFound); }
    read_person(conn, id)?.ok_or(PerformerError::NotFound)
}

/// A preview only: fetching never writes a profile, confirms a person, or changes
/// movie credits. The caller must show this to the user before saving it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileCandidate {
    pub avatar_url: String,
    pub profile_url: String,
    pub name: String,
}

const MAX_PROFILE_BYTES: usize = 1024 * 1024;

fn javbus_profile_url(value: &str) -> Result<(url::Url, String), String> {
    if value.len() > 4096 || value.chars().any(char::is_control) || value.contains('\\') {
        return Err("Invalid profile URL".into());
    }
    let url = url::Url::parse(value).map_err(|_| "Invalid profile URL".to_string())?;
    if url.scheme() != "https" || !matches!(url.host_str(), Some("javbus.com" | "www.javbus.com"))
        || !url.username().is_empty() || url.password().is_some() || url.port().is_some()
        || url.query().is_some() || url.fragment().is_some()
    {
        return Err("Only HTTPS javbus.com/star/<id> profile URLs are supported".into());
    }
    let id = url.path().strip_prefix("/star/").unwrap_or("").trim_end_matches('/');
    if id.is_empty() || id.len() > 100 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || url.path() != format!("/star/{id}") && url.path() != format!("/star/{id}/")
    {
        return Err("Profile URL must identify exactly one JavBus performer".into());
    }
    let id = id.to_string();
    Ok((url, id))
}

fn public_profile_address(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let o = ip.octets();
            !ip.is_private() && !ip.is_loopback() && !ip.is_link_local()
                && !ip.is_unspecified() && !ip.is_multicast() && !ip.is_broadcast()
                && !ip.is_documentation() && o[0] != 0 && o[0] < 224
                && !(o[0] == 100 && (64..=127).contains(&o[1]))
                && !(o[0] == 198 && (18..=19).contains(&o[1]))
                && !(o[0] == 192 && o[1] == 0 && o[2] == 0)
        }
        std::net::IpAddr::V6(ip) => {
            // Only global unicast; reject mapped IPv4, local/link-local and
            // documentation ranges. DNS is pinned to the validated address.
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000 && !(s[0] == 0x2001 && (s[1] <= 0x01ff || s[1] == 0x0db8))
                && s[0] != 0x2002 && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

/// Fetch an explicitly supplied public professional profile for manual preview.
/// `source` must be "javbus". `expected_names` must come from existing local
/// credits/name/aliases, not from the fetched page. Exact trimmed matching only.
/// No DB handle is held across await, so this future can be used by Axum handlers.
/// Fixed HTTPS host/path allowlist, public DNS pinning, no ambient proxy, no
/// cookies, max 3 redirects retaining performer ID, 15s total and 1MiB HTML cap.
/// Failure is fail-closed and has no persistent side effects. Avatar URLs are
/// returned, not fetched; the existing safe image proxy must render the preview.
pub async fn fetch_profile_candidate(source: &str, profile_url: &str, expected_names: &[String]) -> Result<ProfileCandidate, String> {
    if source != "javbus" { return Err("Only the JavBus public profile provider is supported".into()); }
    if expected_names.is_empty() || expected_names.len() > 100
        || expected_names.iter().any(|name| validate_name(name, "expected name").is_err())
    { return Err("Expected names must be non-empty local credits, name or aliases".into()); }
    let (initial, identity) = javbus_profile_url(profile_url)?;
    let fetch = async {
        use futures_util::StreamExt;
        let mut current = initial.clone();
        for hop in 0..=3 {
            let host = current.host_str().ok_or("Missing profile host")?;
            let addresses: Vec<_> = tokio::net::lookup_host((host, 443)).await
                .map_err(|_| "Could not resolve profile provider".to_string())?.collect();
            if addresses.is_empty() || addresses.iter().any(|address| !public_profile_address(address.ip())) {
                return Err("Profile provider resolved to a non-public address".into());
            }
            let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
                .no_proxy().timeout(std::time::Duration::from_secs(15))
                .resolve_to_addrs(host, &addresses).user_agent("PersonalMediaLibrary/1.0 public-profile-preview")
                .build().map_err(|_| "Could not create profile client".to_string())?;
            let response = client.get(current.clone()).header(reqwest::header::ACCEPT, "text/html")
                .send().await.map_err(|_| "无法连接资料来源，请手动上传头像或稍后重试".to_string())?;
            if response.status().is_redirection() {
                if hop == 3 { return Err("Too many profile redirects".into()); }
                let location = response.headers().get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok()).ok_or("Profile redirect has no valid location")?;
                let next = current.join(location).map_err(|_| "Invalid profile redirect".to_string())?;
                if matches!(next.host_str(), Some("javbus.com" | "www.javbus.com")) && next.path().contains("/doc/driver-verify") {
                    return Err("来源要求访问验证，请手动上传头像或稍后重试".into());
                }
                let (next, next_id) = javbus_profile_url(next.as_str())?;
                if next_id != identity { return Err("Profile redirect changed performer identity".into()); }
                current = next;
                continue;
            }
            if matches!(response.status().as_u16(), 403 | 429) {
                return Err("来源限制访问，请手动上传头像或稍后重试".into());
            }
            if !response.status().is_success() { return Err(format!("来源请求失败（HTTP {}），请手动上传头像或稍后重试", response.status().as_u16())); }
            let content_type = response.headers().get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").trim();
            if !content_type.eq_ignore_ascii_case("text/html") && !content_type.eq_ignore_ascii_case("application/xhtml+xml") {
                return Err("Profile response is not HTML".into());
            }
            if response.content_length().is_some_and(|length| length > MAX_PROFILE_BYTES as u64) {
                return Err("Profile response exceeds 1 MiB".into());
            }
            let mut stream = response.bytes_stream();
            let mut bytes = Vec::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| "Could not read profile response".to_string())?;
                if chunk.len() > MAX_PROFILE_BYTES - bytes.len() { return Err("Profile response exceeds 1 MiB".into()); }
                bytes.extend_from_slice(&chunk);
            }
            let html = std::str::from_utf8(&bytes).map_err(|_| "Profile must be UTF-8 HTML".to_string())?;
            return parse_profile_candidate(initial.as_str(), current.as_str(), expected_names, html);
        }
        Err("Profile redirect limit exceeded".into())
    };
    tokio::time::timeout(std::time::Duration::from_secs(15), fetch).await
        .map_err(|_| "资料来源请求超时（15秒），请手动上传头像或稍后重试".to_string())?
}

// A deliberately small, fail-closed HTML structural reader. Unlike substring
// scraping it tracks real ancestor tags, quoted attributes, and inert raw-text
// regions. It does not implement browser HTML error repair: malformed/ambiguous
// profile markup is rejected rather than guessed. No new parser dependency.
#[derive(Debug)]
struct ProfileNode {
    tag: String,
    attrs: std::collections::BTreeMap<String, String>,
    parent: Option<usize>,
    text: String,
}
impl ProfileNode {
    fn attr(&self, name: &str) -> &str { self.attrs.get(name).map(String::as_str).unwrap_or("") }
    fn class(&self, name: &str) -> bool { self.attr("class").split_ascii_whitespace().any(|s| s == name) }
}

fn decode_profile_entities(text: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once('&') {
        output.push_str(before);
        // Raw ampersands are common in image query strings. Unknown named
        // entities stay literal and therefore cannot create a false name match.
        let Some(end) = after.find(';').filter(|&n| n < 16) else { output.push('&'); rest = after; continue; };
        let entity = &after[..end];
        let decoded = match entity {
            "amp" => Some('&'), "quot" => Some('"'), "apos" | "#39" => Some('\''),
            "lt" => Some('<'), "gt" => Some('>'), "nbsp" => Some(' '),
            _ if entity.starts_with("#x") || entity.starts_with("#X") => u32::from_str_radix(&entity[2..],16).ok().and_then(char::from_u32),
            _ if entity.starts_with('#') => entity[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        if let Some(value) = decoded { output.push(value); } else { output.push('&'); output.push_str(entity); output.push(';'); }
        rest = &after[end+1..];
    }
    output.push_str(rest);
    Ok(output)
}

fn profile_nodes(html: &str) -> Result<Vec<ProfileNode>, String> {
    if html.len() > MAX_PROFILE_BYTES { return Err("Profile exceeds parser size limit".into()); }
    let mut nodes: Vec<ProfileNode> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut rest = html;
    while !rest.is_empty() {
        if !rest.starts_with('<') {
            let end = rest.find('<').unwrap_or(rest.len());
            if let Some(&parent) = stack.last() { nodes[parent].text.push_str(&decode_profile_entities(&rest[..end])?); }
            rest = &rest[end..];
            continue;
        }
        if rest.starts_with("<!--") {
            let end = rest.find("-->").ok_or("Unterminated HTML comment")?;
            rest = &rest[end+3..]; continue;
        }
        let mut quote = None;
        let mut end = None;
        for (index, ch) in rest.char_indices().skip(1) {
            if let Some(q) = quote { if ch == q { quote = None; } }
            else if ch == '\'' || ch == '"' { quote = Some(ch); }
            else if ch == '>' { end = Some(index); break; }
        }
        let end = end.ok_or("Unterminated HTML tag")?;
        let inside = rest[1..end].trim();
        rest = &rest[end+1..];
        if inside.starts_with('!') || inside.starts_with('?') { continue; }
        if let Some(closing) = inside.strip_prefix('/') {
            let tag = closing.trim().to_ascii_lowercase();
            if let Some(position) = stack.iter().rposition(|&n| nodes[n].tag == tag) {
                // Do not repair malformed profile markup into trusted evidence.
                if position + 1 != stack.len() && stack.iter().any(|&n| nodes[n].class("avatar-box")) {
                    return Err("Malformed main profile structure".into());
                }
                stack.truncate(position);
            }
            continue;
        }
        let tag_end = inside.find(|c: char| c.is_ascii_whitespace() || c == '/').unwrap_or(inside.len());
        let tag = inside[..tag_end].to_ascii_lowercase();
        if tag.is_empty() || !tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') { return Err("Invalid HTML tag".into()); }
        if matches!(tag.as_str(), "script" | "style" | "textarea" | "title") {
            let lower = rest.to_ascii_lowercase();
            let marker = format!("</{tag}");
            let close = lower.match_indices(&marker).find_map(|(index, _)| {
                lower.as_bytes().get(index + marker.len()).filter(|&&b| b == b'>' || b == b'/' || b.is_ascii_whitespace()).map(|_| index)
            }).ok_or("Unterminated raw text element")?;
            let close_end = rest[close..].find('>').ok_or("Unterminated raw text close")?;
            rest = &rest[close+close_end+1..]; continue;
        }
        let mut attrs = std::collections::BTreeMap::new();
        let mut attr_rest = inside[tag_end..].trim();
        while !attr_rest.is_empty() && attr_rest != "/" {
            let n = attr_rest.find(|c: char| c.is_ascii_whitespace() || c == '=' || c == '/').unwrap_or(attr_rest.len());
            if n == 0 { return Err("Invalid HTML attribute".into()); }
            let name = attr_rest[..n].to_ascii_lowercase();
            attr_rest = attr_rest[n..].trim_start();
            let mut value = String::new();
            if let Some(after) = attr_rest.strip_prefix('=') {
                attr_rest = after.trim_start();
                if let Some(q) = attr_rest.chars().next().filter(|&c| c == '\'' || c == '"') {
                    let after_quote = &attr_rest[1..];
                    let finish = after_quote.find(q).ok_or("Unclosed HTML attribute")?;
                    value = decode_profile_entities(&after_quote[..finish])?;
                    attr_rest = after_quote[finish+1..].trim_start();
                } else {
                    let finish = attr_rest.find(char::is_whitespace).unwrap_or(attr_rest.len());
                    value = decode_profile_entities(&attr_rest[..finish])?;
                    attr_rest = attr_rest[finish..].trim_start();
                }
            }
            if attrs.insert(name, value).is_some() { return Err("Ambiguous duplicate HTML attribute".into()); }
        }
        let index = nodes.len();
        let is_void = matches!(tag.as_str(), "img" | "meta" | "link" | "br" | "hr" | "input" | "area" | "base" | "col" | "embed" | "param" | "source" | "track" | "wbr");
        nodes.push(ProfileNode { tag, attrs, parent: stack.last().copied(), text: String::new() });
        if nodes.len() > 20000 || stack.len() > 128 { return Err("Profile HTML is too complex".into()); }
        if !is_void && !inside.ends_with('/') { stack.push(index); }
    }
    if stack.iter().any(|&n| nodes[n].class("avatar-box")) { return Err("Unclosed main profile structure".into()); }
    Ok(nodes)
}

fn ancestor_matches(nodes: &[ProfileNode], mut index: usize, predicate: impl Fn(&ProfileNode) -> bool) -> bool {
    loop {
        if predicate(&nodes[index]) { return true; }
        let Some(parent) = nodes[index].parent else { return false; };
        index = parent;
    }
}

fn in_profile_box(nodes: &[ProfileNode], mut index: usize, profile: usize) -> bool {
    loop {
        if index == profile { return true; }
        let Some(parent) = nodes[index].parent else { return false; };
        index = parent;
    }
}

fn excluded_profile_region(node: &ProfileNode) -> bool {
    matches!(node.tag.as_str(), "aside" | "footer" | "nav" | "template" | "noscript")
        || node.attrs.contains_key("hidden") || node.attr("aria-hidden") == "true"
        || node.attr("style").to_ascii_lowercase().replace(' ', "").contains("display:none")
        || format!("{} {}",node.attr("id"),node.attr("class")).to_ascii_lowercase()
            .split_ascii_whitespace().any(|s| s.contains("recommend") || s.contains("related") || s.contains("waterfall"))
}

fn parse_profile_candidate(original: &str, final_url: &str, expected_names: &[String], html: &str) -> Result<ProfileCandidate, String> {
    let (_, original_id) = javbus_profile_url(original)?;
    let (base, final_id) = javbus_profile_url(final_url)?;
    if original_id != final_id { return Err("Final profile identity differs from requested identity".into()); }
    if html.contains("/doc/driver-verify") && (html.contains("Age Verification") || html.contains("你是否已經成年") || html.contains("地区年龄检测")) {
        return Err("来源要求访问验证，请手动上传头像或稍后重试".into());
    }
    let nodes = profile_nodes(html)?;
    for node in &nodes {
        if node.tag == "base" { return Err("Profile HTML changes the document base URL".into()); }
        if node.tag == "link" && node.attr("rel").split_ascii_whitespace().any(|s| s.eq_ignore_ascii_case("canonical")) {
            let canonical = base.join(node.attr("href")).map_err(|_| "Invalid profile canonical URL")?;
            let (_, canonical_id) = javbus_profile_url(canonical.as_str())?;
            if canonical_id != original_id { return Err("Canonical profile identity mismatch".into()); }
        }
    }
    let boxes: Vec<_> = nodes.iter().enumerate().filter(|(i,n)| n.tag == "div" && n.class("avatar-box")
        && !ancestor_matches(&nodes,*i,excluded_profile_region)).map(|(i,_)| i).collect();
    if boxes.len() != 1 { return Err("无法确认页面主体头像区域，请手动上传头像".into()); }
    let profile = boxes[0];
    // JavBus main profile uses photo-info > span.pb10 for its professional
    // display name, and photo-frame > img for the portrait. Movie cards, page
    // titles, alt text and recommendation thumbnails are NOT identity evidence.
    let names: Vec<_> = nodes.iter().enumerate().filter(|(i,n)| in_profile_box(&nodes,*i,profile)
        && (n.tag == "span" && n.class("pb10") || matches!(n.tag.as_str(),"h1" | "h2"))
        && n.parent.is_some_and(|p| nodes[p].class("photo-info"))
        && !ancestor_matches(&nodes,*i,excluded_profile_region)).map(|(_,n)| n.text.trim()).collect();
    if names.len() != 1 || names[0].is_empty() || !expected_names.iter().any(|name| name.trim() == names[0]) {
        return Err("来源人物姓名与本库姓名或别名不一致，已拒绝获取头像".into());
    }
    let images: Vec<_> = nodes.iter().enumerate().filter(|(i,n)| n.tag == "img" && in_profile_box(&nodes,*i,profile)
        && n.parent.is_some_and(|p| nodes[p].class("photo-frame"))
        && !ancestor_matches(&nodes,*i,excluded_profile_region)).map(|(_,n)| n).collect();
    if images.len() != 1 { return Err("Main profile portrait is absent or ambiguous".into()); }
    let image = images[0];
    for label in [image.attr("alt"), image.attr("title")] {
        if !label.is_empty() && label.trim() != names[0] { return Err("Portrait name conflicts with main profile name".into()); }
    }
    let avatar = base.join(image.attr("src")).map_err(|_| "Invalid portrait URL")?;
    if avatar.scheme() != "https" || !matches!(avatar.host_str(), Some("javbus.com" | "www.javbus.com"))
        || !avatar.username().is_empty() || avatar.password().is_some() || avatar.port().is_some()
        || avatar.query().is_some() || avatar.fragment().is_some()
        || !avatar.path().starts_with("/pics/actress/")
        || !avatar.path()["/pics/actress/".len()..].bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b,b'-' | b'_' | b'.'))
        || ![".jpg",".jpeg",".png",".webp"].iter().any(|ext| avatar.path().to_ascii_lowercase().ends_with(ext))
    { return Err("Portrait is not an allowlisted JavBus actress image".into()); }
    Ok(ProfileCandidate { avatar_url: avatar.to_string(), profile_url: base.to_string(), name: names[0].to_string() })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrichedPerformerInfo {
    pub avatar_url: Option<String>,
    pub profile_url: Option<String>,
    pub birth_date: Option<String>,
    pub height_cm: Option<i64>,
    pub measurements: Option<String>,
    pub hometown: Option<String>,
    pub hobby: Option<String>,
    pub bio: Option<String>,
    pub blood_type: Option<String>,
    pub debut_year: Option<String>,
    pub agency: Option<String>,
    pub aliases: Vec<String>,
}

fn clean_html_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => { in_tag = true; out.push(' '); }
            '>' => { in_tag = false; }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    decode_profile_entities(&out)
        .unwrap_or(out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_attr_value(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{name}={quote}");
        if let Some((_, rest)) = tag.split_once(&needle) {
            if let Some((val, _)) = rest.split_once(quote) {
                return Some(val.replace("&amp;", "&"));
            }
        }
    }
    None
}

fn safe_utf8_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn parse_javbus_star_page(base_url: &url::Url, html: &str) -> EnrichedPerformerInfo {
    let mut info = EnrichedPerformerInfo {
        profile_url: Some(base_url.to_string()),
        ..Default::default()
    };
    if let Some((_, after_box)) = html.split_once("class=\"avatar-box\"") {
        let box_slice = safe_utf8_prefix(after_box, 8192);
        if let Some((_, after_frame)) = box_slice.split_once("class=\"photo-frame\"") {
            if let Some((_, after_img)) = after_frame.split_once("<img") {
                if let Some((img_tag, _)) = after_img.split_once('>') {
                    if let Some(src) = extract_attr_value(img_tag, "src") {
                        let lower = src.to_ascii_lowercase();
                        if !lower.contains("nowprinting") && !lower.contains("default") {
                            if let Ok(resolved) = base_url.join(src.trim()) {
                                info.avatar_url = Some(resolved.to_string());
                            }
                        }
                    }
                }
            }
        }
        if let Some((_, after_info)) = box_slice.split_once("class=\"photo-info\"") {
            let info_slice = after_info.split("</div>").next().unwrap_or(after_info);
            let mut cup = String::new();
            let mut bust = String::new();
            let mut waist = String::new();
            let mut hip = String::new();
            for p_seg in info_slice.split("<p").skip(1) {
                let Some((_, body)) = p_seg.split_once('>') else { continue; };
                let body = body.split("</p>").next().unwrap_or(body);
                let line = clean_html_text(body);
                let (label, val) = match line.split_once(':').or_else(|| line.split_once('：')) {
                    Some((k, v)) => (k.trim(), v.trim()),
                    None => continue,
                };
                if val.is_empty() || val == "-" { continue; }
                if label.contains("生日") {
                    info.birth_date = Some(val.to_string());
                } else if label.contains("身高") {
                    let digits: String = val.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(cm) = digits.parse::<i64>() {
                        if (120..=220).contains(&cm) { info.height_cm = Some(cm); }
                    }
                } else if label.contains("罩杯") {
                    cup = val.to_string();
                } else if label.contains("胸圍") || label.contains("胸围") {
                    bust = val.replace("cm", "").trim().to_string();
                } else if label.contains("腰圍") || label.contains("腰围") {
                    waist = val.replace("cm", "").trim().to_string();
                } else if label.contains("臀圍") || label.contains("臀围") {
                    hip = val.replace("cm", "").trim().to_string();
                } else if label.contains("出生地") {
                    info.hometown = Some(val.to_string());
                } else if label.contains("愛好") || label.contains("爱好") {
                    info.hobby = Some(val.to_string());
                }
            }
            let mut parts = Vec::new();
            if !cup.is_empty() { parts.push(format!("{cup}罩杯")); }
            if !bust.is_empty() || !waist.is_empty() || !hip.is_empty() {
                parts.push(format!("B{bust} / W{waist} / H{hip}"));
            }
            if !parts.is_empty() {
                info.measurements = Some(parts.join(" · "));
            }
        }
    }
    info
}

fn find_javbus_star_url(search_base: &url::Url, html: &str, expected_name: &str) -> Option<(String, Option<String>)> {
    let target = expected_name.trim().to_ascii_lowercase();
    let mut candidates: Vec<(String, String, Option<String>)> = Vec::new();
    for seg in html.split("class=\"avatar-box\"").skip(1) {
        let block = seg.split("</a>").next().unwrap_or("");
        // Look backwards or inside for href="/star/..." or https://www.javbus.com/star/...
        let href = extract_attr_value(block, "href").or_else(|| {
            // Often `<a class="avatar-box text-center" href="https://www.javbus.com/star/xxx">`
            None
        });
        let Some(raw_href) = href else { continue; };
        if !raw_href.contains("/star/") { continue; }
        let Ok(resolved_url) = search_base.join(raw_href.trim()) else { continue; };
        let avatar = block.split("<img").nth(1)
            .and_then(|s| s.split('>').next())
            .and_then(|tag| extract_attr_value(tag, "src"))
            .filter(|src| !src.to_ascii_lowercase().contains("nowprinting"))
            .and_then(|src| search_base.join(src.trim()).ok())
            .map(|u| u.to_string());
        let name = block.split("<span").nth(1)
            .and_then(|s| s.split_once('>').map(|(_, b)| b))
            .and_then(|s| s.split("</span>").next())
            .map(clean_html_text)
            .unwrap_or_default();
        if !name.is_empty() {
            candidates.push((name, resolved_url.to_string(), avatar));
        }
    }
    // Also support `<a href=".../star/..." class="avatar-box ...">` where href precedes class
    if candidates.is_empty() {
        for seg in html.split("<a ").skip(1) {
            let Some((open_tag, rest)) = seg.split_once('>') else { continue; };
            if !open_tag.contains("avatar-box") { continue; }
            let Some(raw_href) = extract_attr_value(open_tag, "href") else { continue; };
            if !raw_href.contains("/star/") { continue; }
            let Ok(resolved_url) = search_base.join(raw_href.trim()) else { continue; };
            let block = rest.split("</a>").next().unwrap_or("");
            let avatar = block.split("<img").nth(1)
                .and_then(|s| s.split('>').next())
                .and_then(|tag| extract_attr_value(tag, "src"))
                .filter(|src| !src.to_ascii_lowercase().contains("nowprinting"))
                .and_then(|src| search_base.join(src.trim()).ok())
                .map(|u| u.to_string());
            let name = block.split("<span").nth(1)
                .and_then(|s| s.split_once('>').map(|(_, b)| b))
                .and_then(|s| s.split("</span>").next())
                .map(clean_html_text)
                .unwrap_or_default();
            if !name.is_empty() {
                candidates.push((name, resolved_url.to_string(), avatar));
            }
        }
    }
    if let Some((_, url, avatar)) = candidates.iter().find(|(n, _, _)| n.trim().to_ascii_lowercase() == target) {
        return Some((url.clone(), avatar.clone()));
    }
    if candidates.len() == 1 {
        let (_, url, avatar) = &candidates[0];
        return Some((url.clone(), avatar.clone()));
    }
    None
}

async fn fetch_wikipedia_summary(client: &reqwest::Client, lang: &str, name: &str) -> Option<(Option<String>, Option<String>, String)> {
    let mut url = url::Url::parse(&format!("https://{lang}.wikipedia.org/api/rest_v1/page/summary/")).ok()?;
    url.path_segments_mut().ok()?.push(name.trim());
    let resp = client.get(url.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .send().await.ok()?;
    if !resp.status().is_success() { return None; }
    let json: serde_json::Value = resp.json().await.ok()?;
    if json.get("type").and_then(|v| v.as_str()) == Some("disambiguation") { return None; }
    let avatar = json.pointer("/thumbnail/source")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| s.starts_with("https://"))
        .map(str::to_string);
    let bio = json.get("extract")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(260).collect::<String>());
    let page_url = json.pointer("/content_urls/desktop/page")
        .and_then(|v| v.as_str())
        .unwrap_or(url.as_str())
        .to_string();
    if avatar.is_some() || bio.is_some() { Some((avatar, bio, page_url)) } else { None }
}

static GFRIENDS_INDEX: std::sync::OnceLock<tokio::sync::RwLock<Option<(std::collections::HashMap<String, String>, std::time::Instant)>>> = std::sync::OnceLock::new();
const GFRIENDS_TTL: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);
const WIKI_USER_AGENT: &str = "PersonalMediaLibrary/1.0 (https://personal-media-library.local)";

fn normalize_lookup_key(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '　')
        .collect::<String>()
        .to_lowercase()
}

fn build_gfriends_url(folder: &str, file_with_query: &str) -> Option<String> {
    let (file_part, query_part) = match file_with_query.split_once('?') {
        Some((f, q)) => (f, Some(q)),
        None => (file_with_query, None),
    };
    let mut url = url::Url::parse("https://cdn.jsdelivr.net/gh/gfriends/gfriends@master/Content/").ok()?;
    {
        let mut segs = url.path_segments_mut().ok()?;
        segs.push(folder);
        segs.push(file_part);
    }
    if let Some(q) = query_part {
        if !q.is_empty() { url.set_query(Some(q)); }
    }
    Some(url.to_string())
}

async fn lookup_gfriends_avatar(client: &reqwest::Client, names: &[String]) -> Option<String> {
    let cell = GFRIENDS_INDEX.get_or_init(|| tokio::sync::RwLock::new(None));
    {
        let guard = cell.read().await;
        if let Some((map, loaded_at)) = guard.as_ref() {
            if loaded_at.elapsed() < GFRIENDS_TTL {
                for n in names {
                    if let Some(url) = map.get(&normalize_lookup_key(n)) {
                        return Some(url.clone());
                    }
                }
                return None;
            }
        }
    }
    // Fetch Filetree.json from jsDelivr CDN (fallback to raw.githubusercontent.com)
    let mut parsed_json: Option<serde_json::Value> = None;
    for tree_url in [
        "https://cdn.jsdelivr.net/gh/gfriends/gfriends@master/Filetree.json",
        "https://raw.githubusercontent.com/gfriends/gfriends/master/Filetree.json",
    ] {
        if let Ok(resp) = client.get(tree_url).send().await {
            if resp.status().is_success() {
                if let Ok(val) = resp.json::<serde_json::Value>().await {
                    parsed_json = Some(val);
                    break;
                }
            }
        }
    }
    let val = parsed_json?;
    let content = val.get("Content")?.as_object()?;
    let mut map = std::collections::HashMap::<String, String>::with_capacity(24000);
    for (folder, files_val) in content {
        let Some(files) = files_val.as_object() else { continue; };
        for (display_file, actual_val) in files {
            let Some(actual) = actual_val.as_str() else { continue; };
            let stem = display_file
                .rsplit_once('.')
                .map(|(s, _)| s)
                .unwrap_or(display_file);
            let key = normalize_lookup_key(stem);
            if key.is_empty() || map.contains_key(&key) { continue; }
            if let Some(cdn_url) = build_gfriends_url(folder, actual) {
                map.insert(key, cdn_url);
            }
        }
    }
    let found = names.iter().find_map(|n| map.get(&normalize_lookup_key(n)).cloned());
    {
        let mut guard = cell.write().await;
        *guard = Some((map, std::time::Instant::now()));
    }
    found
}

fn clean_wikitext_value(raw: &str) -> String {
    let mut text = raw.to_string();
    // Strip <ref>...</ref> and HTML comments
    while let Some(start) = text.find("<!--") {
        if let Some(end) = text[start..].find("-->") {
            text.replace_range(start..start + end + 3, "");
        } else { break; }
    }
    while let Some(start) = text.find("<ref") {
        if let Some(end) = text[start..].find("</ref>") {
            text.replace_range(start..start + end + 6, "");
        } else if let Some(end) = text[start..].find("/>") {
            text.replace_range(start..start + end + 2, "");
        } else { break; }
    }
    // Replace common templates like {{JPN}} or {{生年月日と年齢|...}}
    text = text.replace("{{JPN}}", "日本").replace("{{Flagicon|Japan}}", "日本");
    // Strip remaining simple {{...}} templates
    while let Some(start) = text.find("{{") {
        if let Some(end) = text[start..].find("}}") {
            text.replace_range(start..start + end + 2, "");
        } else { break; }
    }
    // Convert [[Target|Label]] -> Label and [[Target]] -> Target
    while let Some(start) = text.find("[[") {
        if let Some(end) = text[start..].find("]]") {
            let inner = &text[start + 2..start + end];
            let label = inner.rsplit_once('|').map(|(_, l)| l).unwrap_or(inner).to_string();
            text.replace_range(start..start + end + 2, &label);
        } else { break; }
    }
    clean_html_text(&text)
        .replace("'''", "")
        .replace("''", "")
        .trim_matches(|c: char| c == '・' || c == '、' || c == ',' || c == ' ')
        .trim()
        .to_string()
}

fn parse_ja_wikipedia_infobox(wikitext: &str, info: &mut EnrichedPerformerInfo) {
    let mut year = String::new();
    let mut month = String::new();
    let mut day = String::new();
    let mut bwh = String::new();
    let mut cup = String::new();

    for line in wikitext.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix('|') else { continue; };
        let Some((raw_key, raw_val)) = rest.split_once('=') else { continue; };
        let key = raw_key.trim();
        let val = clean_wikitext_value(raw_val);
        if val.is_empty() || val == "-" || val == "不明" || val == "非公開" { continue; }
        match key {
            "生年" => { year = val.chars().filter(|c| c.is_ascii_digit()).collect(); }
            "生月" => { month = val.chars().filter(|c| c.is_ascii_digit()).collect(); }
            "生日" => { day = val.chars().filter(|c| c.is_ascii_digit()).collect(); }
            "出身地" | "出生地" => {
                if info.hometown.is_none() { info.hometown = Some(val); }
            }
            "血液型" => {
                if info.blood_type.is_none() {
                    let clean = if val.ends_with('型') { val } else { format!("{val}型") };
                    info.blood_type = Some(clean);
                }
            }
            "身長" => {
                if info.height_cm.is_none() {
                    let digits: String = val.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(cm) = digits.parse::<i64>() {
                        if (120..=220).contains(&cm) { info.height_cm = Some(cm); }
                    }
                }
            }
            "スリーサイズ" => { bwh = val; }
            "ブラのサイズ" | "カップ" => { cup = val; }
            "趣味" | "趣味・特技" | "特技" => {
                if info.hobby.is_none() { info.hobby = Some(val); }
            }
            "活動期間" | "デビュー" => {
                if info.debut_year.is_none() { info.debut_year = Some(val); }
            }
            "所属事務所" | "事務所" | "専属契約" => {
                if info.agency.is_none() { info.agency = Some(val); }
            }
            "別名" | "愛称" => {
                for alias in val.split(['、', '・', '/', ',', '，']) {
                    let a = alias.trim();
                    if !a.is_empty() && a.chars().count() <= 30 && !info.aliases.contains(&a.to_string()) {
                        info.aliases.push(a.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    if info.birth_date.is_none() && year.len() == 4 {
        if let (Ok(m), Ok(d)) = (month.parse::<u32>(), day.parse::<u32>()) {
            if (1..=12).contains(&m) && (1..=31).contains(&d) {
                info.birth_date = Some(format!("{year}-{m:02}-{d:02}"));
            }
        }
    }
    if info.measurements.is_none() {
        let mut parts = Vec::new();
        if !cup.is_empty() { parts.push(format!("{cup}罩杯")); }
        if !bwh.is_empty() { parts.push(bwh); }
        if !parts.is_empty() { info.measurements = Some(parts.join(" · ")); }
    }
}

async fn fetch_ja_wikipedia_infobox(client: &reqwest::Client, name: &str, info: &mut EnrichedPerformerInfo) {
    let Ok(mut url) = url::Url::parse("https://ja.wikipedia.org/w/api.php") else { return; };
    url.query_pairs_mut()
        .append_pair("action", "query")
        .append_pair("titles", name.trim())
        .append_pair("redirects", "1")
        .append_pair("prop", "revisions")
        .append_pair("rvprop", "content")
        .append_pair("rvsection", "0")
        .append_pair("format", "json")
        .append_pair("formatversion", "2");
    let Ok(resp) = client.get(url).send().await else { return; };
    if !resp.status().is_success() { return; }
    let Ok(json) = resp.json::<serde_json::Value>().await else { return; };
    if let Some(content) = json.pointer("/query/pages/0/revisions/0/content").and_then(|v| v.as_str()) {
        parse_ja_wikipedia_infobox(content, info);
    }
}

/// Automatically fetch performer portrait and rich profile info (birthday, blood type, height,
/// measurements, hometown, debut year, agency, hobby, bio) from gfriends + Wikipedia + JavBus.
pub async fn fetch_performer_enrichment(name: &str, existing_profile_url: Option<&str>) -> Result<EnrichedPerformerInfo, String> {
    let clean_name = name.trim();
    if clean_name.is_empty() { return Err("演员姓名为空".into()); }
    let wiki_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .user_agent(WIKI_USER_AGENT)
        .build()
        .map_err(|_| "初始化资料抓取客户端失败".to_string())?;
    let web_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36")
        .build()
        .map_err(|_| "初始化网页客户端失败".to_string())?;

    let mut result = EnrichedPerformerInfo::default();
    let mut lookup_names = vec![clean_name.to_string()];

    // 1. Japanese Wikipedia structured Infobox (birthday, height, B/W/H, blood type, hometown, debut, agency, hobby)
    fetch_ja_wikipedia_infobox(&wiki_client, clean_name, &mut result).await;
    for alias in &result.aliases {
        if !lookup_names.contains(alias) { lookup_names.push(alias.clone()); }
    }

    // 2. gfriends CDN portrait lookup (instant in-memory index after first load)
    if let Some(avatar) = lookup_gfriends_avatar(&wiki_client, &lookup_names).await {
        result.avatar_url = Some(avatar);
    }

    // 3. Wikipedia REST Summary (prefer zh for Chinese bio, fallback to ja; also fallback portrait)
    for lang in ["zh", "ja"] {
        if let Some((wiki_avatar, wiki_bio, wiki_page)) = fetch_wikipedia_summary(&wiki_client, lang, clean_name).await {
            if result.avatar_url.is_none() { result.avatar_url = wiki_avatar; }
            if result.bio.is_none() { result.bio = wiki_bio; }
            if result.profile_url.is_none() { result.profile_url = Some(wiki_page); }
            if result.bio.is_some() && result.avatar_url.is_some() { break; }
        }
    }

    // 4. Fallback to JavBus (with age-verification cookie) if portrait or core measurements still missing
    if result.avatar_url.is_none() || (result.birth_date.is_none() && result.measurements.is_none()) {
        let mut star_url: Option<String> = existing_profile_url
            .map(str::trim)
            .filter(|u| u.contains("javbus.com/star/"))
            .map(str::to_string);

        if star_url.is_none() {
            if let Ok(mut search_url) = url::Url::parse("https://www.javbus.com/searchstar/") {
                if let Ok(mut segs) = search_url.path_segments_mut() { segs.push(clean_name); }
                if let Ok(resp) = web_client.get(search_url.clone())
                    .header(reqwest::header::COOKIE, "existmag=all; age=verified; dv=1")
                    .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,ja;q=0.8")
                    .send().await
                {
                    if resp.status().is_success() {
                        if let Ok(html) = resp.text().await {
                            if let Some((found_url, thumb)) = find_javbus_star_url(&search_url, &html, clean_name) {
                                star_url = Some(found_url);
                                if result.avatar_url.is_none() && thumb.is_some() { result.avatar_url = thumb; }
                            }
                        }
                    }
                }
            }
        }

        if let Some(ref target_url) = star_url {
            if let Ok(parsed_url) = url::Url::parse(target_url) {
                if let Ok(resp) = web_client.get(parsed_url.clone())
                    .header(reqwest::header::COOKIE, "existmag=all; age=verified; dv=1")
                    .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,ja;q=0.8")
                    .send().await
                {
                    if resp.status().is_success() {
                        if let Ok(html) = resp.text().await {
                            let parsed = parse_javbus_star_page(&parsed_url, &html);
                            if result.avatar_url.is_none() && parsed.avatar_url.is_some() { result.avatar_url = parsed.avatar_url; }
                            if parsed.profile_url.is_some() { result.profile_url = parsed.profile_url; }
                            if result.birth_date.is_none() { result.birth_date = parsed.birth_date; }
                            if result.height_cm.is_none() { result.height_cm = parsed.height_cm; }
                            if result.measurements.is_none() { result.measurements = parsed.measurements; }
                            if result.hometown.is_none() { result.hometown = parsed.hometown; }
                            if result.hobby.is_none() { result.hobby = parsed.hobby; }
                        }
                    }
                }
            }
        }
    }

    if result.avatar_url.is_none()
        && result.birth_date.is_none()
        && result.height_cm.is_none()
        && result.measurements.is_none()
        && result.bio.is_none()
        && result.blood_type.is_none()
        && result.hometown.is_none()
    {
        return Err(format!("未能从公开站点获取到「{clean_name}」的头像或资料"));
    }
    Ok(result)
}

pub fn apply_performer_enrichment(
    conn: &Connection,
    id: &str,
    info: &EnrichedPerformerInfo,
    force: bool,
) -> Result<Performer, PerformerError> {
    ensure_performer_schema(conn)?;
    let existing = read_person(conn, id)?.ok_or(PerformerError::NotFound)?;
    let avatar = if force || existing.avatar_url.as_deref().unwrap_or("").is_empty() {
        info.avatar_url.clone().or(existing.avatar_url)
    } else {
        existing.avatar_url
    };
    let profile = if force || existing.profile_url.as_deref().unwrap_or("").is_empty() {
        info.profile_url.clone().or(existing.profile_url)
    } else {
        existing.profile_url
    };
    let birth_date = info.birth_date.clone().or(existing.birth_date);
    let height_cm = info.height_cm.or(existing.height_cm);
    let measurements = info.measurements.clone().or(existing.measurements);
    let hometown = info.hometown.clone().or(existing.hometown);
    let hobby = info.hobby.clone().or(existing.hobby);
    let bio = info.bio.clone().or(existing.bio);
    let blood_type = info.blood_type.clone().or(existing.blood_type);
    let debut_year = info.debut_year.clone().or(existing.debut_year);
    let agency = info.agency.clone().or(existing.agency);
    let mut merged_aliases = existing.aliases.clone();
    for a in &info.aliases {
        if a != &existing.name && !merged_aliases.contains(a) && merged_aliases.len() < MAX_ALIASES {
            merged_aliases.push(a.clone());
        }
    }
    let aliases_json = serde_json::to_string(&merged_aliases).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "UPDATE performers SET aliases_json=?2, avatar_url=?3, profile_url=?4, birth_date=?5, height_cm=?6,
         measurements=?7, hometown=?8, hobby=?9, bio=?10, blood_type=?11, debut_year=?12, agency=?13,
         status='confirmed', updated_at=?14 WHERE id=?1",
        params![
            id, aliases_json, avatar, profile, birth_date, height_cm,
            measurements, hometown, hobby, bio, blood_type, debut_year, agency,
            chrono::Utc::now().timestamp()
        ],
    )?;
    read_person(conn, id)?.ok_or(PerformerError::NotFound)
}

pub fn list_performers_missing_profile(conn: &Connection, limit: u32) -> Result<(Vec<(String, String, Option<String>)>, u64), PerformerError> {
    sync_performers(conn)?;
    let total_missing: u64 = conn.query_row(
        "SELECT COUNT(*) FROM performers WHERE avatar_url IS NULL OR trim(avatar_url) = ''",
        [],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(&format!(
        "SELECT p.id, p.name, p.profile_url,
            (SELECT COUNT(DISTINCT c.number) FROM movie_cast c JOIN movie_metadata m ON m.number=c.number
             WHERE c.person_id=p.id AND {VERIFIED}) AS movie_count
         FROM performers p
         WHERE p.avatar_url IS NULL OR trim(p.avatar_url) = ''
         ORDER BY movie_count DESC, p.name COLLATE NOCASE ASC
         LIMIT ?1"
    ))?;
    let rows = stmt.query_map([limit.min(50)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((rows, total_missing))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformerEnrichStats {
    pub total_performers: u64,
    pub with_avatar: u64,
    pub with_profile_info: u64,
    pub missing_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformerEnrichProgress {
    pub running: bool,
    pub paused: bool,
    pub total: usize,
    pub completed: usize,
    pub enriched: usize,
    pub failed: usize,
    pub current_name: Option<String>,
    pub last_message: Option<String>,
}

fn enrich_progress_cell() -> &'static std::sync::Mutex<PerformerEnrichProgress> {
    static CELL: std::sync::OnceLock<std::sync::Mutex<PerformerEnrichProgress>> = std::sync::OnceLock::new();
    CELL.get_or_init(|| std::sync::Mutex::new(PerformerEnrichProgress::default()))
}

fn enrich_pause_flag() -> &'static std::sync::atomic::AtomicBool {
    static FLAG: std::sync::OnceLock<std::sync::atomic::AtomicBool> = std::sync::OnceLock::new();
    FLAG.get_or_init(|| std::sync::atomic::AtomicBool::new(false))
}

fn enrich_stop_flag() -> &'static std::sync::atomic::AtomicBool {
    static FLAG: std::sync::OnceLock<std::sync::atomic::AtomicBool> = std::sync::OnceLock::new();
    FLAG.get_or_init(|| std::sync::atomic::AtomicBool::new(false))
}

pub fn get_performer_enrich_stats(conn: &Connection) -> Result<PerformerEnrichStats, PerformerError> {
    sync_performers(conn)?;
    let total_performers: u64 = conn.query_row("SELECT COUNT(*) FROM performers", [], |r| r.get(0))?;
    let with_avatar: u64 = conn.query_row(
        "SELECT COUNT(*) FROM performers WHERE avatar_url IS NOT NULL AND trim(avatar_url) != ''",
        [],
        |r| r.get(0),
    )?;
    let with_profile_info: u64 = conn.query_row(
        "SELECT COUNT(*) FROM performers WHERE (birth_date IS NOT NULL AND trim(birth_date) != '')
            OR height_cm IS NOT NULL
            OR (measurements IS NOT NULL AND trim(measurements) != '')
            OR (bio IS NOT NULL AND trim(bio) != '')",
        [],
        |r| r.get(0),
    )?;
    let missing_count: u64 = conn.query_row(
        "SELECT COUNT(*) FROM performers WHERE (avatar_url IS NULL OR trim(avatar_url) = '')
            OR ((birth_date IS NULL OR trim(birth_date) = '') AND height_cm IS NULL AND (bio IS NULL OR trim(bio) = ''))",
        [],
        |r| r.get(0),
    )?;
    Ok(PerformerEnrichStats {
        total_performers,
        with_avatar,
        with_profile_info,
        missing_count,
    })
}

pub fn get_performer_enrich_progress() -> PerformerEnrichProgress {
    enrich_progress_cell().lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn pause_performer_enrich() -> PerformerEnrichProgress {
    enrich_pause_flag().store(true, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut g) = enrich_progress_cell().lock() {
        if g.running { g.paused = true; }
        return g.clone();
    }
    PerformerEnrichProgress::default()
}

pub fn resume_performer_enrich() -> PerformerEnrichProgress {
    enrich_pause_flag().store(false, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut g) = enrich_progress_cell().lock() {
        g.paused = false;
        return g.clone();
    }
    PerformerEnrichProgress::default()
}

pub fn stop_performer_enrich() -> PerformerEnrichProgress {
    enrich_stop_flag().store(true, std::sync::atomic::Ordering::SeqCst);
    enrich_pause_flag().store(false, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut g) = enrich_progress_cell().lock() {
        g.paused = false;
        g.last_message = Some("正在停止任务…".to_string());
        return g.clone();
    }
    PerformerEnrichProgress::default()
}

pub fn list_performers_for_enrichment(conn: &Connection, force_all: bool) -> Result<Vec<(String, String, Option<String>)>, PerformerError> {
    sync_performers(conn)?;
    let where_clause = if force_all {
        "1 = 1"
    } else {
        "(p.avatar_url IS NULL OR trim(p.avatar_url) = '')
         OR ((p.birth_date IS NULL OR trim(p.birth_date) = '') AND p.height_cm IS NULL AND (p.bio IS NULL OR trim(p.bio) = ''))"
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT p.id, p.name, p.profile_url,
            (SELECT COUNT(DISTINCT c.number) FROM movie_cast c JOIN movie_metadata m ON m.number=c.number
             WHERE c.person_id=p.id AND {VERIFIED}) AS movie_count
         FROM performers p
         WHERE {where_clause}
         ORDER BY movie_count DESC, p.name COLLATE NOCASE ASC"
    ))?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn start_performer_enrich_task<F>(open_db: F, force_all: bool) -> Result<PerformerEnrichProgress, String>
where
    F: Fn() -> Result<Connection, String> + Send + Sync + 'static,
{
    {
        let mut g = enrich_progress_cell().lock().map_err(|_| "状态锁异常".to_string())?;
        if g.running {
            return Ok(g.clone());
        }
        enrich_stop_flag().store(false, std::sync::atomic::Ordering::SeqCst);
        enrich_pause_flag().store(false, std::sync::atomic::Ordering::SeqCst);
        *g = PerformerEnrichProgress {
            running: true,
            paused: false,
            total: 0,
            completed: 0,
            enriched: 0,
            failed: 0,
            current_name: None,
            last_message: Some("正在扫描待补全演员列表…".to_string()),
        };
    }

    let open_db = std::sync::Arc::new(open_db);
    let open_db_list = open_db.clone();
    tokio::spawn(async move {
        struct EnrichRunGuard;
        impl Drop for EnrichRunGuard {
            fn drop(&mut self) {
                if let Ok(mut g) = enrich_progress_cell().lock() {
                    if g.running {
                        g.running = false;
                        g.paused = false;
                        g.current_name = None;
                        if g.last_message.is_none() {
                            g.last_message = Some("任务已结束".to_string());
                        }
                    }
                }
            }
        }
        let _guard = EnrichRunGuard;

        let queue = tokio::task::spawn_blocking(move || -> Result<Vec<(String, String, Option<String>)>, String> {
            let conn = open_db_list()?;
            list_performers_for_enrichment(&conn, force_all).map_err(|e| e.to_string())
        }).await;

        let items = match queue {
            Ok(Ok(list)) => list,
            Ok(Err(e)) => {
                if let Ok(mut g) = enrich_progress_cell().lock() {
                    g.running = false;
                    g.last_message = Some(format!("扫描失败：{e}"));
                }
                return;
            }
            Err(e) => {
                if let Ok(mut g) = enrich_progress_cell().lock() {
                    g.running = false;
                    g.last_message = Some(format!("任务异常：{e}"));
                }
                return;
            }
        };

        if let Ok(mut g) = enrich_progress_cell().lock() {
            g.total = items.len();
            if items.is_empty() {
                g.running = false;
                g.last_message = Some("所有演员资料均已补全，无需重复获取。".to_string());
                return;
            }
            g.last_message = Some(format!("共 {} 位待处理演员，开始联网获取…", items.len()));
        }

        for (id, name, existing_url) in items {
            if enrich_stop_flag().load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            while enrich_pause_flag().load(std::sync::atomic::Ordering::SeqCst) {
                if enrich_stop_flag().load(std::sync::atomic::Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            if enrich_stop_flag().load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }

            if let Ok(mut g) = enrich_progress_cell().lock() {
                g.current_name = Some(name.clone());
            }

            let fetch_outcome = tokio::select! {
                res = fetch_performer_enrichment(&name, existing_url.as_deref()) => Some(res),
                _ = async {
                    while !enrich_stop_flag().load(std::sync::atomic::Ordering::SeqCst) {
                        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
                    }
                } => None,
            };

            let Some(outcome) = fetch_outcome else {
                break;
            };

            match outcome {
                Ok(info) => {
                    let open_save = open_db.clone();
                    let id_clone = id.clone();
                    let saved = tokio::task::spawn_blocking(move || -> Result<(), String> {
                        let conn = open_save()?;
                        apply_performer_enrichment(&conn, &id_clone, &info, force_all)
                            .map_err(|e| e.to_string())?;
                        Ok(())
                    }).await;
                    if let Ok(mut g) = enrich_progress_cell().lock() {
                        g.completed += 1;
                        if matches!(saved, Ok(Ok(()))) {
                            g.enriched += 1;
                            g.last_message = Some(format!("已更新「{name}」资料"));
                        } else {
                            g.failed += 1;
                            g.last_message = Some(format!("保存「{name}」失败"));
                        }
                    }
                }
                Err(err) => {
                    if let Ok(mut g) = enrich_progress_cell().lock() {
                        g.completed += 1;
                        g.failed += 1;
                        g.last_message = Some(err);
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }

        if let Ok(mut g) = enrich_progress_cell().lock() {
            let stopped = enrich_stop_flag().load(std::sync::atomic::Ordering::SeqCst);
            g.running = false;
            g.paused = false;
            g.current_name = None;
            g.last_message = Some(if stopped {
                format!("已手动停止：成功 {}，未匹配 {}", g.enriched, g.failed)
            } else {
                format!("补全完成：成功 {}，未匹配 {}", g.enriched, g.failed)
            });
        }
    });

    Ok(get_performer_enrich_progress())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE_FIXTURE: &str = r#"<!doctype html><html><head><link rel="canonical" href="https://www.javbus.com/star/abc"></head><body><div class="container"><div class="row"><div class="avatar-box"><div class="photo-frame"><img src="/pics/actress/abc_a.jpg" title="Known Name" alt="Known Name"></div><div class="photo-info"><span class="pb10">Known Name</span><p>Public professional profile</p></div></div></div><div id="related-waterfall"><div class="avatar-box"><div class="photo-frame"><img src="/pics/actress/other.jpg"></div><div class="photo-info"><span class="pb10">Other Name</span></div></div></div></div></body></html>"#;

    fn parse_fixture(html: &str) -> Result<ProfileCandidate, String> {
        parse_profile_candidate("https://www.javbus.com/star/abc", "https://www.javbus.com/star/abc", &["Known Name".into()], html)
    }

    #[test]
    fn profile_preview_accepts_main_portrait_not_recommendation() {
        let candidate = parse_fixture(PROFILE_FIXTURE).unwrap();
        assert_eq!(candidate.name, "Known Name");
        assert_eq!(candidate.avatar_url, "https://www.javbus.com/pics/actress/abc_a.jpg");
        assert_eq!(candidate.profile_url, "https://www.javbus.com/star/abc");
        assert!(serde_json::to_value(candidate).unwrap().get("avatarUrl").is_some());
        let entities = PROFILE_FIXTURE.replace("Known Name", "Known&#32;Name");
        assert!(parse_fixture(&entities).is_ok());
        let fake_script = format!("<script>var x='<div class=avatar-box><img src=evil></div>';</script>{PROFILE_FIXTURE}");
        assert!(parse_fixture(&fake_script).is_ok());
    }

    #[test]
    fn profile_parser_handles_doctype_void_tags_and_raw_script_less_than() {
        let prefix = r#"<!DOCTYPE html><meta charset='UTF-8'><link rel='stylesheet' href='/style.css'/><input disabled><br><hr><img src='/logo.png'><script>if (a < b && c > d) { var x = '<div class="avatar-box">'; } var sentinel = '</scripture><div class="avatar-box">';</script><style>.x::before{content:'<fake>';}</style><!-- <div class='avatar-box'> -->"#;
        assert!(parse_fixture(&format!("{prefix}{PROFILE_FIXTURE}")).is_ok());
        let broken = PROFILE_FIXTURE.replace("</span><p>", "</div><p>");
        assert!(parse_fixture(&broken).is_err());
        let age_gate = "<html><title>Age Verification JavBus</title><a href='/doc/driver-verify'>你是否已經成年?</a></html>";
        assert_eq!(parse_fixture(age_gate).unwrap_err(), "来源要求访问验证，请手动上传头像或稍后重试");
    }

    #[test]
    fn profile_preview_rejects_wrong_person_and_canonical() {
        assert!(parse_fixture(&PROFILE_FIXTURE.replace("Known Name", "Another Person")).is_err());
        assert!(parse_fixture(&PROFILE_FIXTURE.replace("/star/abc", "/star/wrong")).is_err());
        assert!(parse_profile_candidate("https://www.javbus.com/star/abc", "https://www.javbus.com/star/other", &["Known Name".into()], PROFILE_FIXTURE).is_err());
        assert!(parse_fixture(&PROFILE_FIXTURE.replace("alt=\"Known Name\"", "alt=\"Other Person\"")).is_err());
    }

    #[test]
    fn profile_preview_rejects_recommendation_only_or_ambiguous_markup() {
        let recommendation_only = format!("<div id='related-waterfall'>{PROFILE_FIXTURE}</div>");
        assert!(parse_fixture(&recommendation_only).is_err());
        assert!(parse_fixture(&PROFILE_FIXTURE.replace("class=\"avatar-box\"", "class=\"movie-box\"")).is_err());
        assert!(parse_fixture(&format!("{PROFILE_FIXTURE}{PROFILE_FIXTURE}")).is_err());
        assert!(parse_fixture(&PROFILE_FIXTURE.replace("class=\"photo-frame\"", "class=\"recommendations\"")).is_err());
        assert!(parse_fixture(&PROFILE_FIXTURE.replace("<body>", "<body><base href='https://evil.example/'>")).is_err());
        assert!(parse_fixture(&PROFILE_FIXTURE.replace("class=\"avatar-box\"", "class=\"avatar-box\" hidden")).is_err());
        assert!(parse_fixture(&PROFILE_FIXTURE.replace("src=\"/pics/actress/abc_a.jpg\"", "src=\"/pics/actress/abc_a.jpg\" src=\"/pics/actress/evil.jpg\"")).is_err());
    }

    #[test]
    fn profile_preview_rejects_invalid_scheme_hosts_and_portraits() {
        for url in ["http://www.javbus.com/star/abc", "https://javbus.com.evil.example/star/abc", "https://user@www.javbus.com/star/abc", "https://www.javbus.com:444/star/abc", "https://www.javbus.com/star/a/b", "https://www.javbus.com/star/%61bc", "https://www.javbus.com/star/abc?x=1", "file:///star/abc"] {
            assert!(javbus_profile_url(url).is_err(), "{url}");
        }
        for image in ["http://www.javbus.com/pics/actress/abc.jpg", "https://evil.example/pics/actress/abc.jpg", "data:image/png;base64,AAAA", "/pics/cover/abc.jpg", "/pics/actress/abc.svg", "/pics/actress/abc.jpg?redirect=evil"] {
            assert!(parse_fixture(&PROFILE_FIXTURE.replace("/pics/actress/abc_a.jpg", image)).is_err(), "{image}");
        }
        assert!(javbus_profile_url("https://javbus.com/star/abc/").is_ok());
        for ip in ["127.0.0.1", "10.0.0.1", "100.64.0.1", "169.254.1.1", "192.0.2.1", "::1", "::ffff:127.0.0.1", "fd00::1", "2001:db8::1"] {
            assert!(!public_profile_address(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_profile_address("1.1.1.1".parse().unwrap()));
        assert!(public_profile_address("2606:4700:4700::1111".parse().unwrap()));
    }

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        ensure_performer_schema(&conn).unwrap();
        conn
    }

    fn movie(conn: &Connection, number: &str, source: &str, actors: &[&str], verified: i32) {
        conn.execute("INSERT INTO movie_metadata(number,title,source,actors_json,cover_url,updated_at,verification_version)
            VALUES(?1,?1,?2,?3,'https://example.com/cover.jpg',1,?4)
            ON CONFLICT(number) DO UPDATE SET source=excluded.source,actors_json=excluded.actors_json,verification_version=excluded.verification_version",
            params![number,source,serde_json::to_string(actors).unwrap(),verified]).unwrap();
    }

    #[test]
    fn migration_idempotence_and_sync_noop() {
        let conn = database();
        ensure_performer_schema(&conn).unwrap();
        movie(&conn, "ABC-001", "javdb", &["A"], 1);
        sync_performers(&conn).unwrap();
        let changes = conn.total_changes();
        let first = list_performers(&conn, "", 100, 0).unwrap();
        ensure_performer_schema(&conn).unwrap();
        sync_performers(&conn).unwrap();
        assert_eq!(changes, conn.total_changes());
        assert_eq!(first.total, 1);
        assert_eq!(first.items[0].status, "unconfirmed");
        assert_eq!(first.items[0].id, candidate_id("javdb", "A"));
    }

    #[test]
    fn manual_preservation_and_immutable_raw_identity() {
        let conn = database();
        movie(&conn, "ABC-001", "javdb", &["Original"], 1);
        let id = list_performers(&conn, "", 10, 0).unwrap().items[0].id.clone();
        update_performer(&conn, &id, "Edited", &["Alias".into()], Some("https://example.com/avatar.jpg"), Some("https://example.com/person")).unwrap();
        set_performer_status(&conn, &id, "confirmed").unwrap();
        movie(&conn, "ABC-002", "javdb", &["Original"], 1);
        let person = get_performer(&conn, &id).unwrap().unwrap().person;
        assert_eq!(person.name, "Edited");
        assert_eq!(person.aliases, vec!["Alias"]);
        assert_eq!(person.avatar_url.as_deref(), Some("https://example.com/avatar.jpg"));
        assert_eq!(person.profile_url.as_deref(), Some("https://example.com/person"));
        assert_eq!(person.status, "confirmed");
        assert_eq!(person.movie_count, 2);
        assert_eq!(list_performers(&conn, "alias", 10, 0).unwrap().total, 1);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM movie_cast WHERE raw_name='Original'", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }

    #[test]
    fn stale_links_follow_changed_credits_and_source_without_timestamp_change() {
        let conn = database();
        movie(&conn, "ABC-001", "javdb", &["A", "B"], 1);
        sync_performers(&conn).unwrap();
        movie(&conn, "ABC-001", "javdb", &["B"], 1);
        assert_eq!(get_performer(&conn, &candidate_id("javdb", "A")).unwrap().unwrap().total, 0);
        assert_eq!(get_performer(&conn, &candidate_id("javdb", "B")).unwrap().unwrap().total, 1);
        movie(&conn, "ABC-001", "other", &["B"], 1);
        assert_eq!(get_performer(&conn, &candidate_id("javdb", "B")).unwrap().unwrap().total, 0);
        assert_eq!(get_performer(&conn, &candidate_id("other", "B")).unwrap().unwrap().total, 1);
        movie(&conn, "ABC-001", "other", &[], 1);
        assert_eq!(get_performer(&conn, &candidate_id("other", "B")).unwrap().unwrap().total, 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM movie_cast", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn identity_separation_and_explicit_confirmation() {
        let conn = database();
        movie(&conn, "ABC-001", "javdb", &["Same"], 1);
        movie(&conn, "ABC-002", "other", &["Same"], 1);
        let list = list_performers(&conn, "Same", 100, 0).unwrap();
        assert_eq!(list.total, 2);
        assert_ne!(list.items[0].id, list.items[1].id);
        set_performer_status(&conn, &list.items[0].id, "confirmed").unwrap();
        assert_eq!(get_performer(&conn, &list.items[1].id).unwrap().unwrap().person.status, "unconfirmed");
        assert!(set_performer_status(&conn, &list.items[0].id, "verified").is_err());
        assert_ne!(candidate_id("a|b", "c"), candidate_id("a", "b|c"));
    }

    #[test]
    fn counts_distinct_movies_only_verified_and_detail_cap() {
        let conn = database();
        movie(&conn, "LEGACY-001", "javdb", &["Legacy"], 0);
        movie(&conn, "EMPTY-001", "not_found", &["Missing"], 1);
        for n in 0..103 { movie(&conn, &format!("ABC-{n:03}"), "javdb", &["A", "A"], 1); }
        let list = list_performers(&conn, "", 1, 0).unwrap();
        assert_eq!(list.total, 1);
        assert_eq!(list.items[0].movie_count, 103);
        let detail = get_performer(&conn, &list.items[0].id).unwrap().unwrap();
        assert_eq!(detail.movies.len(), 100);
        assert_eq!(detail.total, 103);
        movie(&conn, "ABC-000", "javdb", &["A"], 0);
        assert_eq!(get_performer(&conn, &list.items[0].id).unwrap().unwrap().total, 102);
        conn.execute("DELETE FROM movie_metadata WHERE number='ABC-001'", []).unwrap();
        assert_eq!(get_performer(&conn, &list.items[0].id).unwrap().unwrap().total, 101);
        assert_eq!(list_performers(&conn, "", 0, 0).unwrap().items.len(), 0);
        assert_eq!(list_performers(&conn, "", 1, 1).unwrap().total, 1);
        let json = serde_json::to_value(detail).unwrap();
        assert!(json["person"].get("movieCount").is_some());
        assert!(json["person"].get("avatarUrl").is_some());
        assert!(json["person"].get("updatedAt").is_some());
    }

    #[test]
    fn alias_validation_and_literal_search() {
        let conn = database();
        movie(&conn, "ABC-001", "javdb", &["A"], 1);
        let id = list_performers(&conn, "", 10, 0).unwrap().items[0].id.clone();
        let updated = update_performer(&conn, &id, " A ", &[" 別名 ".into(), "別名".into(), "100%_".into()], None, None).unwrap();
        assert_eq!(updated.aliases, vec!["別名", "100%_"]);
        assert_eq!(list_performers(&conn, "別名", 10, 0).unwrap().total, 1);
        assert_eq!(list_performers(&conn, "%_", 10, 0).unwrap().total, 1);
        assert_eq!(list_performers(&conn, "' OR 1=1 --", 10, 0).unwrap().total, 0);
        for aliases in [vec!["".into()], vec!["bad\nname".into()], vec!["x".repeat(201)], vec!["x".into(); 51]] {
            assert!(matches!(update_performer(&conn, &id, "A", &aliases, None, None), Err(PerformerError::Validation(_))));
        }
        assert!(update_performer(&conn, &id, " ", &[], None, None).is_err());
        assert!(matches!(update_performer(&conn, "missing", "A", &[], None, None), Err(PerformerError::NotFound)));
        assert!(get_performer(&conn, "missing").unwrap().is_none());
    }

    #[test]
    fn avatar_validation_and_upload_limit() {
        assert!(validate_avatar(Some("https://example.com/image.jpg")).is_ok());
        for invalid in ["file:///tmp/image", "javascript:alert(1)", "https://user:pass@example.com/a", "data:image/svg+xml;base64,PHN2Zz4=", "data:image/png;base64,aGVsbG8=", "data:image/png;base64,***"] {
            assert!(validate_avatar(Some(invalid)).is_err(), "{invalid}");
        }
        let png = format!("data:image/png;base64,{}", STANDARD.encode(b"\x89PNG\r\n\x1a\n"));
        assert!(validate_avatar(Some(&png)).is_ok());
        let jpeg = format!("data:image/jpeg;base64,{}", STANDARD.encode([0xff, 0xd8, 0xff]));
        assert!(validate_avatar(Some(&jpeg)).is_ok());
        let webp = format!("data:image/webp;base64,{}", STANDARD.encode(b"RIFFxxxxWEBP"));
        assert!(validate_avatar(Some(&webp)).is_ok());
        let mut bytes = vec![0u8; MAX_AVATAR_BYTES];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        let at_limit = format!("data:image/png;base64,{}", STANDARD.encode(&bytes));
        assert!(validate_avatar(Some(&at_limit)).is_ok());
        bytes.push(0);
        let over_limit = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
        assert!(validate_avatar(Some(&over_limit)).is_err());
    }

    #[test]
    fn invalid_credit_json_does_not_destroy_previous_links() {
        let conn = database();
        movie(&conn, "ABC-001", "javdb", &["A"], 1);
        sync_performers(&conn).unwrap();
        conn.execute("UPDATE movie_metadata SET actors_json='broken'", []).unwrap();
        assert_eq!(get_performer(&conn, &candidate_id("javdb", "A")).unwrap().unwrap().total, 1);
    }

    #[test]
    fn orphan_cleanup_without_foreign_keys_preserves_profiles() {
        let conn = database();
        conn.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        movie(&conn, "ABC-001", "javdb", &["A"], 1);
        sync_performers(&conn).unwrap();
        conn.execute("DELETE FROM movie_metadata", []).unwrap();
        sync_performers(&conn).unwrap();
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM movie_cast", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM performer_sync_state", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
        assert_eq!(list_performers(&conn, "A", 10, 0).unwrap().total, 1);
    }

    #[test]
    fn failed_sync_rolls_back_links_and_releases_savepoint() {
        let conn = database();
        movie(&conn, "ABC-001", "javdb", &["A"], 1);
        sync_performers(&conn).unwrap();
        movie(&conn, "ABC-001", "javdb", &["B"], 1);
        conn.execute_batch("CREATE TRIGGER reject_cast BEFORE INSERT ON movie_cast BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
        assert!(sync_performers(&conn).is_err());
        assert!(conn.is_autocommit());
        assert_eq!(conn.query_row("SELECT raw_name FROM movie_cast", [], |r| r.get::<_, String>(0)).unwrap(), "A");
        assert_eq!(conn.query_row("SELECT COUNT(*) FROM performers", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
        conn.execute_batch("DROP TRIGGER reject_cast").unwrap();
        sync_performers(&conn).unwrap();
        assert_eq!(conn.query_row("SELECT raw_name FROM movie_cast", [], |r| r.get::<_, String>(0)).unwrap(), "B");
    }

    #[test]
    fn sync_can_run_inside_host_transaction() {
        let conn = database();
        conn.execute_batch("BEGIN").unwrap();
        movie(&conn, "ABC-001", "javdb", &["A"], 1);
        assert_eq!(list_performers(&conn, "", 10, 0).unwrap().total, 1);
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(list_performers(&conn, "", 10, 0).unwrap().total, 0);
    }
}
