//! Four-domain backend (V2).
//!
//! The legacy database remains the provider/scan cache. This module owns the
//! new recent/pending/vault/comics schema and exposes the domain APIs used by
//! the new server and iOS client.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Mutex as StdMutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{media_library, AppHandle, State};

pub const DOMAINS: [&str; 4] = ["recent", "pending", "vault", "comics"];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainSummary {
    pub domain: String,
    pub root_id: Option<String>,
    pub root_path: Option<String>,
    pub total: i64,
    pub present: i64,
    pub missing: i64,
    pub last_scan_at: Option<i64>,
    pub last_scan_status: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainAsset {
    pub id: String,
    pub domain: String,
    pub remote_path: String,
    pub file_name: String,
    pub parent_path: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub missing: bool,
    pub modified_at: Option<String>,
    pub thumbnail_asset_id: Option<String>,
    pub duration_seconds: Option<i64>,
    pub progress_seconds: Option<f64>,
    pub favorite: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingProjectSummary {
    pub id: String,
    pub name: String,
    pub source_path: String,
    pub status: String,
    pub is_loose: bool,
    pub total_count: i64,
    pub triage_count: i64,
    pub ready_count: i64,
    pub conflict_count: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingItemView {
    pub asset_id: String,
    pub remote_path: String,
    pub file_name: String,
    pub parent_path: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub recommended_category_id: Option<String>,
    pub recommended_category_name: Option<String>,
    pub recommended_author_id: Option<String>,
    pub recommended_author_name: Option<String>,
    pub candidate_authors: Value,
    pub candidate_categories: Value,
    pub confidence: Option<f64>,
    pub reason: Option<String>,
    pub manual_category_id: Option<String>,
    pub manual_author_id: Option<String>,
    pub assignment_status: String,
    pub conflict_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub domain: String,
    pub scan_id: String,
    pub scanned: i64,
    pub added: i64,
    pub updated: i64,
    pub missing: i64,
    pub deleted: i64,
    pub conflicts: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanSummary {
    pub domain: String,
    pub dry_run: bool,
    pub count: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveResult {
    pub success: bool,
    pub status: String,
    pub message: String,
    pub source_path: String,
    pub target_path: String,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn stable_id(prefix: &str, value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}-{hash:016x}")
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(value) = std::env::var("PML_DATA_DIR") {
        let path = PathBuf::from(value);
        fs::create_dir_all(&path).map_err(|error| format!("无法创建数据目录：{error}"))?;
        return Ok(path);
    }
    #[cfg(feature = "tauri")]
    {
        use tauri::Manager;
        let path = app
            .path()
            .app_data_dir()
            .map_err(|error| format!("无法获取应用数据目录：{error}"))?;
        fs::create_dir_all(&path).map_err(|error| format!("无法创建数据目录：{error}"))?;
        return Ok(path);
    }
    #[cfg(not(feature = "tauri"))]
    {
        let path = app
            .path()
            .app_data_dir()
            .map_err(|error| format!("无法获取应用数据目录：{error}"))?;
        fs::create_dir_all(&path).map_err(|error| format!("无法创建数据目录：{error}"))?;
        Ok(path)
    }
}

pub(crate) fn legacy_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("personal-media-library.sqlite3"))
}

fn v2_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("personal-media-library-v2.sqlite3"))
}

pub(crate) fn open_v2(app: &AppHandle) -> Result<Connection, String> {
    let connection = Connection::open(v2_path(app)?)
        .map_err(|error| format!("V2 数据库打开失败：{error}"))?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 30000;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -65536;
             PRAGMA mmap_size = 268435456;",
        )
        .map_err(|error| format!("V2 数据库初始化失败：{error}"))?;
    Ok(connection)
}

fn schema_sql() -> &'static str {
    r#"
    CREATE TABLE IF NOT EXISTS schema_meta (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS domain_roots (
        domain TEXT PRIMARY KEY CHECK(domain IN ('recent','pending','vault','comics')),
        root_id TEXT NOT NULL,
        remote_path TEXT NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS assets (
        id TEXT PRIMARY KEY,
        domain TEXT NOT NULL CHECK(domain IN ('recent','pending','vault','comics')),
        root_id TEXT NOT NULL,
        remote_path TEXT NOT NULL,
        file_name TEXT NOT NULL,
        parent_path TEXT NOT NULL,
        media_type TEXT NOT NULL,
        size_bytes INTEGER NOT NULL DEFAULT 0,
        etag TEXT,
        content_hash TEXT,
        thumbnail_asset_id TEXT,
        duration_seconds INTEGER,
        modified_at TEXT,
        missing INTEGER NOT NULL DEFAULT 0,
        missing_since INTEGER,
        missing_scan_count INTEGER NOT NULL DEFAULT 0,
        first_seen_at INTEGER NOT NULL,
        last_seen_at INTEGER NOT NULL,
        last_scan_id TEXT,
        UNIQUE(domain, remote_path)
    );
    CREATE INDEX IF NOT EXISTS assets_domain_path ON assets(domain, remote_path);
    CREATE INDEX IF NOT EXISTS assets_domain_parent ON assets(domain, parent_path, file_name COLLATE NOCASE);
    CREATE INDEX IF NOT EXISTS assets_domain_missing ON assets(domain, missing);
    CREATE INDEX IF NOT EXISTS assets_domain_media ON assets(domain, media_type, missing);

    -- Completion is recorded separately: an empty successful listing is authoritative too.
    CREATE TABLE IF NOT EXISTS directory_sync_state (
        root_id TEXT NOT NULL,
        remote_path TEXT NOT NULL,
        scanned_at INTEGER NOT NULL,
        revision TEXT,
        cover_asset_id TEXT,
        PRIMARY KEY(root_id, remote_path)
    );
    CREATE TABLE IF NOT EXISTS provider_directory_snapshots (
        domain TEXT NOT NULL,
        root_id TEXT NOT NULL,
        parent_path TEXT NOT NULL,
        completed_at INTEGER NOT NULL,
        PRIMARY KEY(domain, root_id, parent_path)
    );
    CREATE TABLE IF NOT EXISTS provider_directory_entries (
        domain TEXT NOT NULL,
        root_id TEXT NOT NULL,
        parent_path TEXT NOT NULL,
        remote_path TEXT NOT NULL,
        name TEXT NOT NULL,
        is_directory INTEGER NOT NULL,
        size_bytes INTEGER NOT NULL,
        modified_at TEXT,
        etag TEXT,
        PRIMARY KEY(domain, root_id, parent_path, remote_path)
    );

    CREATE TABLE IF NOT EXISTS recent_items (
        asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
        batch_id TEXT,
        source TEXT,
        discovered_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS pending_projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        source_path TEXT NOT NULL,
        root_id TEXT NOT NULL,
        is_loose INTEGER NOT NULL DEFAULT 0,
        status TEXT NOT NULL DEFAULT 'discovered',
        total_count INTEGER NOT NULL DEFAULT 0,
        triage_count INTEGER NOT NULL DEFAULT 0,
        ready_count INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS pending_projects_source ON pending_projects(source_path);

    CREATE TABLE IF NOT EXISTS pending_items (
        asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
        project_id TEXT NOT NULL REFERENCES pending_projects(id) ON DELETE CASCADE,
        recommended_category_id TEXT,
        recommended_author_id TEXT,
        candidate_authors_json TEXT,
        candidate_categories_json TEXT,
        confidence REAL,
        reason TEXT,
        manual_category_id TEXT,
        manual_author_id TEXT,
        assignment_status TEXT NOT NULL DEFAULT 'triage',
        conflict_reason TEXT,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS pending_items_project ON pending_items(project_id, assignment_status);

    CREATE TABLE IF NOT EXISTS vault_items (
        asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
        category_id TEXT,
        author_id TEXT,
        collection_id TEXT,
        confidence REAL,
        source TEXT,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS vault_items_category ON vault_items(category_id);
    CREATE INDEX IF NOT EXISTS vault_items_author ON vault_items(author_id);
    CREATE INDEX IF NOT EXISTS vault_items_collection ON vault_items(collection_id);

    CREATE TABLE IF NOT EXISTS comic_archives (
        asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
        page_count INTEGER NOT NULL DEFAULT 0,
        manifest_json TEXT,
        cover_asset_id TEXT,
        updated_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS comic_pages (
        archive_asset_id TEXT NOT NULL REFERENCES comic_archives(asset_id) ON DELETE CASCADE,
        page_index INTEGER NOT NULL,
        entry_name TEXT NOT NULL,
        data_offset INTEGER,
        compressed_size INTEGER,
        uncompressed_size INTEGER,
        PRIMARY KEY(archive_asset_id, page_index)
    );

    CREATE TABLE IF NOT EXISTS authors (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL COLLATE NOCASE UNIQUE,
        aliases_json TEXT NOT NULL DEFAULT '[]',
        avatar_asset_id TEXT,
        banner_asset_id TEXT,
        origin TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS categories (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL COLLATE NOCASE,
        parent_id TEXT,
        origin TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        UNIQUE(name)
    );

    CREATE TABLE IF NOT EXISTS collections (
        id TEXT PRIMARY KEY,
        root_id TEXT NOT NULL,
        author_id TEXT,
        name TEXT NOT NULL,
        anchor_path TEXT,
        parent_id TEXT,
        origin TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS asset_state (
        asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
        favorite INTEGER NOT NULL DEFAULT 0,
        progress_seconds REAL,
        reading_page INTEGER,
        last_opened_at INTEGER,
        updated_at INTEGER NOT NULL
    );

    CREATE TABLE IF NOT EXISTS scan_runs (
        id TEXT PRIMARY KEY,
        domain TEXT NOT NULL,
        status TEXT NOT NULL,
        started_at INTEGER NOT NULL,
        finished_at INTEGER,
        scanned_count INTEGER,
        added_count INTEGER,
        updated_count INTEGER,
        missing_count INTEGER,
        deleted_count INTEGER,
        conflict_count INTEGER,
        error TEXT
    );
    CREATE INDEX IF NOT EXISTS scan_runs_domain ON scan_runs(domain, started_at DESC);

    CREATE TABLE IF NOT EXISTS move_conflicts (
        id TEXT PRIMARY KEY,
        asset_id TEXT NOT NULL,
        project_id TEXT,
        source_path TEXT NOT NULL,
        target_path TEXT NOT NULL,
        kind TEXT NOT NULL,
        detected_at INTEGER NOT NULL,
        resolved_at INTEGER,
        resolution TEXT,
        UNIQUE(asset_id, target_path)
    );
    CREATE INDEX IF NOT EXISTS move_conflicts_open ON move_conflicts(resolved_at, detected_at DESC);
    "#
}

pub(crate) fn ensure_v2(app: &AppHandle) -> Result<(), String> {
    static INITIALIZED: OnceLock<StdMutex<HashSet<PathBuf>>> = OnceLock::new();
    let path = v2_path(app)?;
    let mut guard = INITIALIZED
        .get_or_init(|| StdMutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.contains(&path) {
        return Ok(());
    }
    // Serialize the complete one-time bootstrap. Releasing this guard before
    // additive schema/import work lets concurrent first requests race migrations.
    let mut connection = open_v2(app)?;
    connection
        .execute_batch(schema_sql())
        .map_err(|error| format!("V2 schema 创建失败：{error}"))?;
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_meta(key, value) VALUES('schema_version','2')",
            [],
        )
        .map_err(|error| error.to_string())?;

    crate::v2_state::ensure_state_schema(&mut connection).map_err(|error| error.to_string())?;
    ensure_domain_roots(&connection, app)?;
    if !migration_done(&connection)? {
        migrate_vault(&connection, app)?;
    }
    crate::v2_import::migrate(&connection, app)?;

    guard.insert(path);
    Ok(())
}

fn migration_done(connection: &Connection) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key='vault_migrated'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map(|value| value.as_deref() == Some("1"))
        .map_err(|error| error.to_string())
}

fn ensure_domain_roots(connection: &Connection, app: &AppHandle) -> Result<(), String> {
    let initialized = connection.query_row(
        "SELECT 1 FROM schema_meta WHERE key='domain_roots_initialized'", [], |_| Ok(true))
        .optional().map_err(|error| error.to_string())?.unwrap_or(false);
    if initialized { return Ok(()); }
    let legacy = legacy_path(app)?;
    if !legacy.exists() {
        return Ok(());
    }
    let legacy_conn = Connection::open_with_flags(&legacy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| format!("旧数据库打开失败：{error}"))?;
    let mut roots = Vec::new();
    if let Ok(mut statement) = legacy_conn.prepare("SELECT id, kind, remote_path FROM library_roots") {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        }) {
            for row in rows.flatten() {
                roots.push(row);
            }
        }
    }
    let mut settings = HashMap::new();
    if let Ok(mut statement) = legacy_conn.prepare("SELECT key, value FROM settings") {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                settings.insert(row.0, row.1);
            }
        }
    }
    let configured = |key: &str| settings.get(key).map(|value| value.trim_end_matches('/').to_owned());
    let recent = configured("dir_recent");
    let pending = configured("dir_pending");
    let vault = configured("dir_vault");
    let comics = configured("dir_comics");

    let mut chosen: HashMap<&str, (String, String)> = HashMap::new();
    for (id, kind, path) in &roots {
        let normalized = path.trim_end_matches('/');
        let domain = if kind == "comics"
            || comics.as_deref() == Some(normalized)
            || normalized.contains("漫画")
        {
            "comics"
        } else if recent.as_deref() == Some(normalized) || normalized.contains("云下载") {
            "recent"
        } else if pending.as_deref() == Some(normalized)
            || normalized.contains("暂存")
            || normalized.contains("未分类")
        {
            "pending"
        } else {
            "vault"
        };
        chosen.entry(domain).or_insert((id.clone(), path.clone()));
    }
    // Fallbacks for a fresh/partial legacy database.
    for (domain, path) in [
        ("recent", "/115open/云下载"),
        ("pending", "/115open/暂存文件/{未分类}"),
        ("vault", "/115open/文件"),
        ("comics", "/115open/漫画"),
    ] {
        if !chosen.contains_key(domain) {
            if let Some((id, _, _)) = roots
                .iter()
                .find(|(_, _, p)| p.trim_end_matches('/') == path)
            {
                chosen.insert(domain, (id.clone(), path.to_owned()));
            }
        }
    }
    for (domain, (root_id, remote_path)) in chosen {
        connection
            .execute(
                "INSERT INTO domain_roots(domain, root_id, remote_path, updated_at)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(domain) DO NOTHING",
                params![domain, root_id, remote_path, now()],
            )
            .map_err(|error| format!("域根目录写入失败：{error}"))?;
    }
    connection.execute("INSERT OR IGNORE INTO schema_meta(key,value) VALUES('domain_roots_initialized','1')", []).map_err(|error| error.to_string())?;
    Ok(())
}

fn migrate_vault(connection: &Connection, app: &AppHandle) -> Result<(), String> {
    let legacy = legacy_path(app)?;
    if !legacy.exists() {
        connection
            .execute(
                "INSERT OR REPLACE INTO schema_meta(key, value) VALUES('vault_migrated','1')",
                [],
            )
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
    // Consistent snapshot before touching the legacy database.
    let backup = data_dir(app)?.join("personal-media-library-v1-fourdomain-backup.sqlite3");
    if !backup.exists() {
        if let Ok(legacy_conn) = Connection::open_with_flags(&legacy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            let _ = legacy_conn.execute(
                "VACUUM INTO ?1",
                params![backup.to_string_lossy().to_string()],
            );
        }
    }

    let mut legacy_uri = url::Url::from_file_path(&legacy)
        .map_err(|_| "旧数据库路径无法转换为只读 URI".to_string())?;
    legacy_uri.query_pairs_mut().append_pair("mode", "ro");
    connection
        .execute(
            "ATTACH DATABASE ?1 AS legacy",
            params![legacy_uri.as_str()],
        )
        .map_err(|error| format!("挂载旧数据库失败：{error}"))?;

    let result = (|| -> Result<(), String> {
        connection
            .execute_batch(
                "INSERT OR IGNORE INTO authors(id,name,aliases_json,avatar_asset_id,banner_asset_id,origin,created_at)
                 SELECT id,name,aliases_json,avatar_asset_id,banner_asset_id,origin,created_at FROM legacy.authors;
                 INSERT OR IGNORE INTO categories(id,name,parent_id,origin,created_at)
                 SELECT id,name,parent_id,origin,created_at FROM legacy.categories;
                 INSERT OR IGNORE INTO collections(id,root_id,author_id,name,anchor_path,parent_id,origin,created_at)
                 SELECT id,root_id,author_id,name,anchor_path,parent_id,origin,created_at FROM legacy.collections;",
            )
            .map_err(|error| format!("迁移作者/分类/合集失败：{error}"))?;

        Ok(())
    })();

    let detach = connection.execute("DETACH DATABASE legacy", []);
    let _ = detach;
    result?;
    connection
        .execute(
            "INSERT OR REPLACE INTO schema_meta(key, value) VALUES('vault_migrated','1')",
            [],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn domain_root(connection: &Connection, domain: &str) -> Result<(String, String), String> {
    connection
        .query_row(
            "SELECT root_id, remote_path FROM domain_roots WHERE domain=?1",
            params![domain],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("{domain} 域还没有配置根目录"))
}

fn domain_valid(domain: &str) -> bool {
    DOMAINS.contains(&domain)
}

pub fn summaries(app: &AppHandle) -> Result<Vec<DomainSummary>, String> {
    ensure_v2(app)?;
    let connection = open_v2(app)?;
    let mut result = Vec::new();
    for domain in DOMAINS {
        let root = connection
            .query_row(
                "SELECT root_id, remote_path FROM domain_roots WHERE domain=?1",
                params![domain],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let total: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM assets WHERE domain=?1",
                params![domain],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let present: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM assets WHERE domain=?1 AND missing=0",
                params![domain],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let scan = connection
            .query_row(
                "SELECT finished_at, status FROM scan_runs WHERE domain=?1 ORDER BY started_at DESC LIMIT 1",
                params![domain],
                |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        result.push(DomainSummary {
            domain: domain.to_owned(),
            root_id: root.as_ref().map(|pair| pair.0.clone()),
            root_path: root.as_ref().map(|pair| pair.1.clone()),
            total,
            present,
            missing: total - present,
            last_scan_at: scan.as_ref().and_then(|pair| pair.0),
            last_scan_status: scan.map(|pair| pair.1),
        });
    }
    Ok(result)
}

pub fn list_assets(
    app: &AppHandle,
    domain: &str,
    media_type: Option<&str>,
    query: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<DomainAsset>, String> {
    if !domain_valid(domain) {
        return Err("未知域".into());
    }
    ensure_v2(app)?;
    let connection = open_v2(app)?;
    list_assets_from_connection(&connection, domain, media_type, query, limit, offset)
}

fn list_assets_from_connection(
    connection: &Connection,
    domain: &str,
    media_type: Option<&str>,
    query: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<DomainAsset>, String> {
    let limit = limit.clamp(1, 500);
    let offset = offset.max(0);
    let query_like = query.map(|value| format!("%{}%", value.replace('%', "\\%").replace('_', "\\_")));
    let mut statement = connection
        .prepare(
            "SELECT a.id,a.domain,a.remote_path,a.file_name,a.parent_path,a.media_type,a.size_bytes,
                    a.missing,a.modified_at,a.thumbnail_asset_id,a.duration_seconds,
                    s.progress_seconds,COALESCE(s.favorite,0)
             FROM assets a
             LEFT JOIN asset_state s ON s.asset_id=a.id
             WHERE a.domain=?1 AND a.missing=0
               AND (?2 IS NULL OR a.media_type=?2)
               AND (?3 IS NULL OR a.file_name LIKE ?3 ESCAPE '\\')
             ORDER BY COALESCE(a.modified_at,'') DESC, a.file_name COLLATE NOCASE
             LIMIT ?4 OFFSET ?5",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![domain, media_type, query_like, limit, offset], |row| {
            Ok(DomainAsset {
                id: row.get(0)?,
                domain: row.get(1)?,
                remote_path: row.get(2)?,
                file_name: row.get(3)?,
                parent_path: row.get(4)?,
                media_type: row.get(5)?,
                size_bytes: row.get(6)?,
                missing: row.get::<_, i64>(7)? != 0,
                modified_at: row.get(8)?,
                thumbnail_asset_id: row.get(9)?,
                duration_seconds: row.get(10)?,
                progress_seconds: row.get(11)?,
                favorite: row.get::<_, i64>(12)? != 0,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

pub fn projects(app: &AppHandle) -> Result<Vec<PendingProjectSummary>, String> {
    ensure_v2(app)?;
    let connection = open_v2(app)?;
    let mut statement = connection
        .prepare(
            "SELECT p.id,p.name,p.source_path,p.status,p.is_loose,p.total_count,p.triage_count,p.ready_count,
                    (SELECT COUNT(*) FROM pending_items i WHERE i.project_id=p.id AND i.conflict_reason IS NOT NULL)
             FROM pending_projects p ORDER BY p.updated_at DESC, p.name COLLATE NOCASE",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(PendingProjectSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                source_path: row.get(2)?,
                status: row.get(3)?,
                is_loose: row.get::<_, i64>(4)? != 0,
                total_count: row.get(5)?,
                triage_count: row.get(6)?,
                ready_count: row.get(7)?,
                conflict_count: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

pub fn project_items(
    app: &AppHandle,
    project_id: &str,
) -> Result<Vec<PendingItemView>, String> {
    ensure_v2(app)?;
    let connection = open_v2(app)?;
    let mut statement = connection
        .prepare(
            "SELECT a.id,a.remote_path,a.file_name,a.parent_path,a.media_type,a.size_bytes,
                    i.recommended_category_id,c.name,i.recommended_author_id,au.name,
                    i.candidate_authors_json,i.candidate_categories_json,i.confidence,i.reason,
                    i.manual_category_id,i.manual_author_id,i.assignment_status,i.conflict_reason
             FROM pending_items i
             JOIN assets a ON a.id=i.asset_id
             LEFT JOIN categories c ON c.id=i.recommended_category_id
             LEFT JOIN authors au ON au.id=i.recommended_author_id
             WHERE i.project_id=?1 AND a.missing=0
             ORDER BY a.file_name COLLATE NOCASE",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok(PendingItemView {
                asset_id: row.get(0)?,
                remote_path: row.get(1)?,
                file_name: row.get(2)?,
                parent_path: row.get(3)?,
                media_type: row.get(4)?,
                size_bytes: row.get(5)?,
                recommended_category_id: row.get(6)?,
                recommended_category_name: row.get(7)?,
                recommended_author_id: row.get(8)?,
                recommended_author_name: row.get(9)?,
                candidate_authors: row
                    .get::<_, Option<String>>(10)?
                    .and_then(|value| serde_json::from_str(&value).ok())
                    .unwrap_or(Value::Array(Vec::new())),
                candidate_categories: row
                    .get::<_, Option<String>>(11)?
                    .and_then(|value| serde_json::from_str(&value).ok())
                    .unwrap_or(Value::Array(Vec::new())),
                confidence: row.get(12)?,
                reason: row.get(13)?,
                manual_category_id: row.get(14)?,
                manual_author_id: row.get(15)?,
                assignment_status: row.get(16)?,
                conflict_reason: row.get(17)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

pub fn clean(app: &AppHandle, domain: &str, dry_run: bool) -> Result<CleanSummary, String> {
    if !domain_valid(domain) {
        return Err("未知域".into());
    }
    ensure_v2(app)?;
    let connection = open_v2(app)?;
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM assets WHERE domain=?1 AND missing=1",
            params![domain],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !dry_run && count > 0 {
        connection
            .execute(
                "DELETE FROM assets WHERE domain=?1 AND missing=1",
                params![domain],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(CleanSummary {
        domain: domain.to_owned(),
        dry_run,
        count,
    })
}

pub async fn scan_domain(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    domain: &str,
) -> Result<ScanSummary, String> {
    scan_provider_tree(app, state, domain, "", true).await
}

/// Recursive scoped scan for original scan/retry commands; unlike interactive
/// refresh this enumerates the full subtree before committing reconciliation.
pub(crate) async fn scan_domain_subtree(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    domain: &str,
    path: &str,
) -> Result<ScanSummary, String> {
    scan_provider_tree(app, state, domain, path, true).await
}

pub async fn refresh_domain_path(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    domain: &str,
    path: &str,
) -> Result<ScanSummary, String> {
    scan_path_v2(app, state, domain, path).await
}


fn domain_media_type(domain: &str, path: &str) -> Option<&'static str> {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    if domain == "comics" {
        return match extension.as_str() {
            "zip" | "cbz" | "rar" | "cbr" | "7z" | "pdf" => Some("comicArchive"),
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "heic" | "heif" | "avif" | "bmp" | "tif" | "tiff" => Some("comicPage"),
            _ => None,
        };
    }
    match extension.as_str() {
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "heic" | "heif" | "avif" | "bmp" | "tif" | "tiff" | "jxl" | "svg" => Some("image"),
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "m4v" | "ts" | "m2ts" | "mts" | "m4s" => Some("video"),
        "zip" | "cbz" | "rar" | "cbr" | "7z" | "pdf" => Some("comicArchive"),
        _ => None,
    }
}

async fn scan_path_v2(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    domain: &str,
    folder_path: &str,
) -> Result<ScanSummary, String> {
    scan_provider_tree(app, state, domain, folder_path, false).await
}

// Fetch everything before opening a write transaction. A failed or malformed listing
// must never make a partial tree authoritative.
async fn scan_provider_tree(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    domain: &str,
    folder_path: &str,
    recursive: bool,
) -> Result<ScanSummary, String> {
    if !domain_valid(domain) {
        return Err("未知域".into());
    }
    ensure_v2(&app)?;
    let (root_id, root_path) = {
        let connection = open_v2(&app)?;
        domain_root(&connection, domain)?
    };
    let start_path = if folder_path.trim().is_empty() {
        root_path.trim_end_matches('/').to_owned()
    } else {
        folder_path.trim().trim_end_matches('/').to_owned()
    };
    if start_path != root_path.trim_end_matches('/')
        && !start_path.starts_with(&format!("{}/", root_path.trim_end_matches('/')))
    {
        return Err("刷新目录不在该域根目录下".into());
    }
    let scan_id = format!("scan-path-{}-{}", domain, SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos());
    {
        let connection = open_v2(&app)?;
        connection
            .execute(
                "INSERT INTO scan_runs(id,domain,status,started_at) VALUES(?1,?2,'running',?3)",
                params![scan_id, domain, now()],
            )
            .map_err(|error| error.to_string())?;
    }

    let mut queue = std::collections::VecDeque::from([start_path.clone()]);
    let mut visited = HashSet::new();
    let mut files: Vec<(String, String, String, Option<String>, String, i64, Option<String>, Option<String>)> = Vec::new();
    let mut listings = Vec::new();
    let fetched: Result<(), String> = async {
    while let Some(directory) = queue.pop_front() {
        if !visited.insert(directory.clone()) {
            continue;
        }
        let entries = media_library::list_provider_entries(state.clone(), directory.clone()).await
            .map_err(|error| format!("目录读取失败 {directory}: {error}"))?;
        validate_provider_listing(&directory, &entries)?;
        listings.push((directory.clone(), entries.clone()));
        if recursive { tokio::time::sleep(Duration::from_millis(80)).await; }
        for entry in entries {
            if entry.is_directory {
                if recursive { queue.push_back(entry.path); }
            } else if let Some(media_type) = domain_media_type(domain, &entry.path) {
                let name = entry.name.clone();
                let parent = parent_path(&entry.path);
                files.push((
                    entry.path,
                    name,
                    parent,
                    entry.etag,
                    media_type.to_owned(),
                    entry.size_bytes,
                    entry.modified_at,
                    None,
                ));
                if files.len() > 250_000 {
                    return Err("目录超过 250000 项".into());
                }
            }
        }
    }

    Ok(())
    }.await;
    if let Err(error) = fetched {
        let connection = open_v2(&app)?;
        connection.execute("UPDATE scan_runs SET status='failed',finished_at=?1,error=?2 WHERE id=?3",
            params![now(), error, scan_id]).map_err(|error| error.to_string())?;
        return Err(error);
    }

    let connection = open_v2(&app)?;
    let tx = connection.unchecked_transaction().map_err(|error| error.to_string())?;
    let mut missing = 0i64;
    let mut deleted = 0i64;
    for (directory, entries) in &listings {
        let counts = reconcile_directory_snapshot(&tx, domain, &root_id, directory, entries)?;
        missing += counts.0;
        deleted += counts.1;
    }
    let mut added = 0i64;
    let mut updated = 0i64;
    for (remote_path, file_name, parent, etag, media_type, size_bytes, modified_at, _) in &files {
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM assets WHERE domain=?1 AND remote_path=?2",
                params![domain, remote_path],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let id = existing.unwrap_or_else(|| stable_id("asset", &format!("{domain}|{remote_path}")));
        let was_present: bool = tx
            .query_row("SELECT 1 FROM assets WHERE id=?1", params![&id], |_| Ok(true))
            .optional()
            .map_err(|error| error.to_string())?
            .unwrap_or(false);
        tx.execute(
            "INSERT INTO assets(
                id,domain,root_id,remote_path,file_name,parent_path,media_type,size_bytes,
                etag,content_hash,thumbnail_asset_id,duration_seconds,modified_at,missing,
                missing_since,missing_scan_count,first_seen_at,last_seen_at,last_scan_id
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,NULL,NULL,?10,0,NULL,0,?11,?11,?12)
             ON CONFLICT(id) DO UPDATE SET domain=excluded.domain, root_id=excluded.root_id,
                remote_path=excluded.remote_path, file_name=excluded.file_name,
                parent_path=excluded.parent_path, media_type=excluded.media_type,
                content_hash=CASE WHEN assets.size_bytes IS NOT excluded.size_bytes OR assets.etag IS NOT excluded.etag OR assets.modified_at IS NOT excluded.modified_at THEN NULL ELSE assets.content_hash END,
                 thumbnail_asset_id=CASE WHEN assets.size_bytes IS NOT excluded.size_bytes OR assets.etag IS NOT excluded.etag OR assets.modified_at IS NOT excluded.modified_at THEN NULL ELSE assets.thumbnail_asset_id END,
                 duration_seconds=CASE WHEN assets.size_bytes IS NOT excluded.size_bytes OR assets.etag IS NOT excluded.etag OR assets.modified_at IS NOT excluded.modified_at THEN NULL ELSE assets.duration_seconds END,
                 size_bytes=excluded.size_bytes, etag=excluded.etag, modified_at=excluded.modified_at,
                missing=0, missing_since=NULL, missing_scan_count=0,
                last_seen_at=excluded.last_seen_at, last_scan_id=excluded.last_scan_id",
            params![
                &id,
                domain,
                root_id,
                remote_path,
                file_name,
                parent,
                media_type,
                size_bytes,
                etag,
                modified_at,
                now(),
                scan_id
            ],
        )
        .map_err(|error| error.to_string())?;
        if was_present { updated += 1; } else { added += 1; }
        match domain {
            "vault" => {
                tx.execute(
                    "INSERT INTO vault_items(asset_id,updated_at) VALUES(?1,?2)
                     ON CONFLICT(asset_id) DO NOTHING",
                    params![&id, now()],
                )
                .map_err(|error| error.to_string())?;
            }
            "recent" => {
                tx.execute(
                    "INSERT INTO recent_items(asset_id,discovered_at) VALUES(?1,?2)
                     ON CONFLICT(asset_id) DO NOTHING",
                    params![&id, now()],
                )
                .map_err(|error| error.to_string())?;
            }
            "comics" => {
                tx.execute(
                    "INSERT INTO comic_archives(asset_id,page_count,updated_at) VALUES(?1,0,?2)
                     ON CONFLICT(asset_id) DO NOTHING",
                    params![&id, now()],
                )
                .map_err(|error| error.to_string())?;
            }
            _ => {}
        }
    }

    if domain == "pending" {
        rebuild_pending_projects_v2(&tx, root_path.trim_end_matches('/'))?;
    }
    tx.execute(
        "UPDATE scan_runs SET status='success', finished_at=?1, scanned_count=?2,
            added_count=?3, updated_count=?4, missing_count=?5, deleted_count=?6 WHERE id=?7",
        params![now(), files.len() as i64, added, updated, missing, deleted, scan_id],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(ScanSummary {
        domain: domain.to_owned(),
        scan_id,
        scanned: files.len() as i64,
        added,
        updated,
        missing,
        deleted,
        conflicts: 0,
    })
}

/// Case-insensitive, numeric-aware name order: "第2集" sorts before "第10集".
/// Chinese characters are compared by code point here; clients that can use a
/// locale collator (pinyin) still re-sort locally for display.
fn natural_name_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let left: Vec<char> = left.to_lowercase().chars().collect();
    let right: Vec<char> = right.to_lowercase().chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    while i < left.len() && j < right.len() {
        if left[i].is_ascii_digit() && right[j].is_ascii_digit() {
            let start_left = i;
            while i < left.len() && left[i].is_ascii_digit() { i += 1; }
            let start_right = j;
            while j < right.len() && right[j].is_ascii_digit() { j += 1; }
            let a: String = left[start_left..i].iter().collect();
            let b: String = right[start_right..j].iter().collect();
            let a = a.trim_start_matches('0');
            let b = b.trim_start_matches('0');
            let ordering = a.len().cmp(&b.len()).then_with(|| a.cmp(b));
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        } else {
            if left[i] != right[j] {
                return left[i].cmp(&right[j]);
            }
            i += 1;
            j += 1;
        }
    }
    (left.len() - i).cmp(&(right.len() - j))
}

fn validate_provider_listing(parent: &str, entries: &[media_library::ProviderEntry]) -> Result<(), String> {    let mut seen = HashSet::new();
    for entry in entries {
        if parent_path(&entry.path) != parent || entry.path.ends_with('/')
            || matches!(entry.path.rsplit('/').next(), Some("" | "." | ".."))
            || !seen.insert(&entry.path)
        {
            return Err(format!("目录返回了无效或重复的直接子项: {}", entry.path));
        }
    }
    Ok(())
}

fn has_directory_snapshot(connection: &Connection, domain: &str, root_id: &str, parent: &str) -> Result<bool, String> {
    connection.query_row(
        "SELECT 1 FROM provider_directory_snapshots WHERE domain=?1 AND root_id=?2 AND parent_path=?3",
        params![domain, root_id, parent], |_| Ok(true))
        .optional().map(|value| value.unwrap_or(false)).map_err(|error| error.to_string())
}

// Called only with a successful, complete direct listing, inside the asset transaction.
fn reconcile_directory_snapshot(
    tx: &rusqlite::Transaction, domain: &str, root_id: &str, parent: &str,
    entries: &[media_library::ProviderEntry],
) -> Result<(i64, i64), String> {
    validate_provider_listing(parent, entries)?;
    let prefix = format!("{parent}/");
    let mut candidates = HashSet::new();
    // Include inferred old children as well: databases predating snapshots still
    // need to reconcile removed folders, without assuming present folders empty.
    let mut statement = tx.prepare(
        "SELECT remote_path FROM assets WHERE domain=?1 AND substr(remote_path,1,length(?2))=?2
         UNION SELECT remote_path FROM provider_directory_entries
         WHERE domain=?1 AND root_id=?3 AND parent_path=?4")
        .map_err(|error| error.to_string())?;
    let rows = statement.query_map(params![domain, prefix, root_id, parent], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    for row in rows {
        let path = row.map_err(|error| error.to_string())?;
        if let Some(relative) = path.strip_prefix(&prefix) {
            candidates.insert(format!("{prefix}{}", relative.split('/').next().unwrap_or("")));
        }
    }
    drop(statement);
    let present: HashMap<&str, bool> = entries.iter().map(|entry| (entry.path.as_str(), entry.is_directory)).collect();
    let mut missing = 0;
    let mut deleted = 0;
    for path in candidates {
        let kind = present.get(path.as_str()).copied();
        let subtree = format!("{path}/");
        // A file replacing a directory also invalidates its former descendants;
        // a directory replacing a file invalidates only the former file asset.
        let absent = kind.is_none();
        missing += tx.execute(
            "UPDATE assets SET missing=1,missing_since=COALESCE(missing_since,?1),missing_scan_count=missing_scan_count+1
             WHERE domain=?2 AND ((remote_path=?3 AND ?5) OR
                (substr(remote_path,1,length(?4))=?4 AND ?6))",
            params![now(), domain, path, subtree, absent || kind == Some(true), kind != Some(true)])
            .map_err(|error| error.to_string())? as i64;
        deleted += tx.execute(
            "DELETE FROM assets WHERE domain=?1 AND missing=1 AND
                ((remote_path=?2 AND ?4) OR (substr(remote_path,1,length(?3))=?3 AND ?5))
                AND (?1!='vault' OR missing_scan_count>=2 OR missing_since<=?6)",
            params![domain, path, subtree, absent || kind == Some(true), kind != Some(true), now()-86400])
            .map_err(|error| error.to_string())? as i64;
        if kind != Some(true) {
            for table in ["provider_directory_snapshots", "provider_directory_entries"] {
                tx.execute(&format!("DELETE FROM {table} WHERE domain=?1 AND root_id=?2 AND
                    (parent_path=?3 OR substr(parent_path,1,length(?4))=?4)"),
                    params![domain, root_id, path, subtree]).map_err(|error| error.to_string())?;
            }
        }
    }
    tx.execute("DELETE FROM provider_directory_entries WHERE domain=?1 AND root_id=?2 AND parent_path=?3",
        params![domain, root_id, parent]).map_err(|error| error.to_string())?;
    for entry in entries {
        tx.execute("INSERT INTO provider_directory_entries
            (domain,root_id,parent_path,remote_path,name,is_directory,size_bytes,modified_at,etag)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![domain, root_id, parent, entry.path, entry.name, entry.is_directory,
                entry.size_bytes, entry.modified_at, entry.etag]).map_err(|error| error.to_string())?;
    }
    tx.execute("INSERT INTO provider_directory_snapshots(domain,root_id,parent_path,completed_at)
        VALUES(?1,?2,?3,?4) ON CONFLICT(domain,root_id,parent_path) DO UPDATE SET completed_at=excluded.completed_at",
        params![domain, root_id, parent, now()]).map_err(|error| error.to_string())?;
    Ok((missing, deleted))
}

fn rebuild_pending_projects_v2(tx: &rusqlite::Transaction, root_path: &str) -> Result<(), String> {
    use std::collections::BTreeMap;
    let root = root_path.trim_end_matches('/');
    let mut statement = tx
        .prepare("SELECT id,remote_path FROM assets WHERE domain='pending' AND missing=0")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|error| error.to_string())?;
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        let (id, remote_path) = row.map_err(|error| error.to_string())?;
        let relative = remote_path.strip_prefix(root).unwrap_or(&remote_path).trim_start_matches('/');
        let first = relative.split('/').next().unwrap_or("");
        let name = if is_pending_folder(first) { first.to_owned() } else { "散装文件".to_owned() };
        groups.entry(name).or_default().push(id);
    }
    drop(statement);
    for (name, ids) in groups {
        let is_loose = name == "散装文件";
        let source_path = if is_loose { root.to_owned() } else { format!("{root}/{name}") };
        let project_id = stable_id("pending", &format!("{root}|{name}"));
        let total = ids.len() as i64;
        tx.execute(
            "INSERT INTO pending_projects(id,name,source_path,root_id,is_loose,status,total_count,triage_count,ready_count,created_at,updated_at)
             VALUES(?1,?2,?3,(SELECT root_id FROM domain_roots WHERE domain='pending'),?4,'discovered',?5,?5,0,?6,?6)
             ON CONFLICT(id) DO UPDATE SET total_count=excluded.total_count,
                updated_at=excluded.updated_at",
            params![project_id, name, source_path, if is_loose { 1 } else { 0 }, total, now()],
        )
        .map_err(|error| error.to_string())?;
        for id in ids {
            tx.execute(
                "INSERT INTO pending_items(asset_id,project_id,assignment_status,updated_at)
                 VALUES(?1,?2,'triage',?3)
                 ON CONFLICT(asset_id) DO UPDATE SET project_id=pending_items.project_id,
                    updated_at=excluded.updated_at",
                params![id, project_id, now()],
            )
            .map_err(|error| error.to_string())?;
        }
    }
    let mut statement = tx.prepare("SELECT id FROM pending_projects").map_err(|error| error.to_string())?;
    let project_ids = statement.query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    drop(statement);
    for project_id in project_ids { refresh_project_counts_for_project(tx, &project_id)?; }
    // A shallow directory refresh is not evidence that an empty or manually
    // created project was deleted. Preserve project identity and assignments.
    Ok(())
}

fn is_pending_folder(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.starts_with("【Pending")
        || name.starts_with("【pending")
        || lower.starts_with("pending")
        || name.starts_with("【待收纳")
}

struct Recommendation {
    category: Option<String>,
    author: Option<String>,
    candidate_authors: Value,
    candidate_categories: Value,
    confidence: Option<f64>,
    reason: Option<String>,
}

fn recommend_for(tx: &Connection, asset_id: &str) -> Result<Recommendation, String> {
    let (file_name, relative): (String, String) = tx
        .query_row(
            "SELECT file_name, remote_path FROM assets WHERE id=?1",
            params![asset_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    let lower = format!("{relative}/{file_name}").to_lowercase();

    let mut candidate_authors = Vec::new();
    if let Ok(mut statement) = tx.prepare(
        "SELECT id,name FROM authors ORDER BY length(name) DESC LIMIT 500",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                if !row.1.is_empty()
                    && lower.contains(&row.1.to_lowercase())
                    && !candidate_authors.iter().any(|value: &Value| value["id"] == row.0)
                {
                    candidate_authors.push(serde_json::json!({"id": row.0, "name": row.1}));
                    if candidate_authors.len() >= 5 { break; }
                }
            }
        }
    }

    let mut candidate_categories = Vec::new();
    if let Ok(mut statement) = tx.prepare(
        "SELECT id,name FROM categories ORDER BY length(name) DESC LIMIT 500",
    ) {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for row in rows.flatten() {
                let last = row.1.rsplit('/').next().unwrap_or(&row.1);
                if !last.is_empty()
                    && lower.contains(&last.to_lowercase())
                    && !candidate_categories.iter().any(|value: &Value| value["id"] == row.0)
                {
                    candidate_categories.push(serde_json::json!({"id": row.0, "name": row.1}));
                    if candidate_categories.len() >= 5 { break; }
                }
            }
        }
    }

    let author = candidate_authors.first().and_then(|value| value["id"].as_str()).map(str::to_owned);
    let category = candidate_categories.first().and_then(|value| value["id"].as_str()).map(str::to_owned);
    let confidence = match (author.is_some(), category.is_some()) {
        (true, true) => Some(0.9),
        (true, false) => Some(0.8),
        (false, true) => Some(0.6),
        (false, false) => None,
    };
    let reason = match (author.is_some(), category.is_some()) {
        (true, true) => Some("命中已有作者与分类".to_owned()),
        (true, false) => Some("命中已有作者".to_owned()),
        (false, true) => Some("命中已有分类".to_owned()),
        (false, false) => None,
    };
    Ok(Recommendation {
        category,
        author,
        candidate_authors: Value::Array(candidate_authors),
        candidate_categories: Value::Array(candidate_categories),
        confidence,
        reason,
    })
}

pub fn assign_pending(
    app: &AppHandle,
    asset_id: &str,
    category_id: Option<String>,
    author_id: Option<String>,
) -> Result<(), String> {
    ensure_v2(app)?;
    let connection = open_v2(app)?;
    let changed = connection
        .execute(
            "UPDATE pending_items SET manual_category_id=?1, manual_author_id=?2,
                assignment_status='ready', conflict_reason=NULL, updated_at=?3
             WHERE asset_id=?4",
            params![category_id, author_id, now(), asset_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("待整理文件不存在".into());
    }
    refresh_project_counts(&connection, asset_id)?;
    Ok(())
}

fn refresh_project_counts(connection: &Connection, asset_id: &str) -> Result<(), String> {
    let project_id: Option<String> = connection
        .query_row(
            "SELECT project_id FROM pending_items WHERE asset_id=?1",
            params![asset_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some(project_id) = project_id {
        refresh_project_counts_for_project(connection, &project_id)?;
    }
    Ok(())
}

fn refresh_project_counts_for_project(connection: &Connection, project_id: &str) -> Result<(), String> {
    connection
        .execute(
            "UPDATE pending_projects SET
                total_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=?1),
                triage_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=?1 AND assignment_status='triage' AND conflict_reason IS NULL),
                ready_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=?1 AND assignment_status='ready' AND conflict_reason IS NULL),
                updated_at=?2
             WHERE id=?1",
            params![project_id, now()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn conflicts(app: &AppHandle) -> Result<Vec<Value>, String> {
    ensure_v2(app)?;
    let connection = open_v2(app)?;
    let mut statement = connection
        .prepare(
            "SELECT id,asset_id,project_id,source_path,target_path,kind,detected_at,resolved_at
             FROM move_conflicts WHERE resolved_at IS NULL ORDER BY detected_at DESC LIMIT 200",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "assetId": row.get::<_, String>(1)?,
                "projectId": row.get::<_, Option<String>>(2)?,
                "sourcePath": row.get::<_, String>(3)?,
                "targetPath": row.get::<_, String>(4)?,
                "kind": row.get::<_, String>(5)?,
                "detectedAt": row.get::<_, i64>(6)?,
                "resolvedAt": row.get::<_, Option<i64>>(7)?,
            }))
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}

pub async fn move_pending_item(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    asset_id: String,
) -> Result<MoveResult, String> {
    ensure_v2(&app)?;
    let connection = open_v2(&app)?;
    let (source_path, project_id, category_id, author_id, status): (
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    ) = connection
        .query_row(
            "SELECT a.remote_path,i.project_id,
                    COALESCE(i.manual_category_id,i.recommended_category_id),
                    COALESCE(i.manual_author_id,i.recommended_author_id),i.assignment_status
             FROM pending_items i JOIN assets a ON a.id=i.asset_id
             WHERE i.asset_id=?1",
            params![asset_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "待整理文件不存在".to_string())?;
    if status != "ready" {
        return Err("该文件还没有指定分类/作者".into());
    }
    let vault_root: String = connection
        .query_row(
            "SELECT remote_path FROM domain_roots WHERE domain='vault'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let category_name: Option<String> = if let Some(id) = &category_id {
        connection
            .query_row(
                "SELECT name FROM categories WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    let author_name: Option<String> = if let Some(id) = &author_id {
        connection
            .query_row(
                "SELECT name FROM authors WHERE id=?1",
                params![id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    let file_name = source_path.rsplit('/').next().unwrap_or(&source_path).to_owned();
    let mut target = vault_root.trim_end_matches('/').to_owned();
    if let Some(category) = category_name.filter(|value| !value.is_empty()) {
        target.push('/');
        target.push_str(category.trim_matches('/'));
    }
    if let Some(author) = author_name.filter(|value| !value.is_empty()) {
        target.push('/');
        target.push_str(author.trim());
    }
    target.push('/');
    target.push_str(&file_name);

    let request = media_library::CurateMoveItemRequest {
        source_path: source_path.clone(),
        target_path: target.clone(),
        is_directory: Some(false),
    };
    let result = media_library::curate_move_item(app.clone(), state.clone(), request).await?;
    let now_value = now();
    let mut connection = open_v2(&app)?;
    let tx = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    if result.status == "MOVED" {
        tx.execute(
            "UPDATE assets SET domain='vault', root_id=(SELECT root_id FROM domain_roots WHERE domain='vault'), remote_path=?1, file_name=?2,
                parent_path=?3, last_seen_at=?4, missing=0, missing_since=NULL, missing_scan_count=0
             WHERE id=?5",
            params![target, file_name, parent_path(&target), now_value, asset_id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "INSERT INTO vault_items(asset_id,category_id,author_id,collection_id,confidence,source,updated_at)
             VALUES(?1,?2,?3,NULL,NULL,'user',?4)
             ON CONFLICT(asset_id) DO UPDATE SET category_id=excluded.category_id,
                author_id=excluded.author_id, source='user', updated_at=excluded.updated_at",
            params![&asset_id, &category_id, &author_id, now_value],
        )
        .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM pending_items WHERE asset_id=?1", params![asset_id])
            .map_err(|error| error.to_string())?;
        refresh_project_counts_for_project(&tx, &project_id)?;
        tx.commit().map_err(|error| error.to_string())?;
        return Ok(MoveResult {
            success: true,
            status: result.status,
            message: result.message,
            source_path,
            target_path: target,
        });
    }
    let conflict_id = stable_id("conflict", &format!("{asset_id}|{target}"));
    tx.execute(
        "INSERT INTO move_conflicts(id,asset_id,project_id,source_path,target_path,kind,detected_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(asset_id,target_path) DO UPDATE SET detected_at=excluded.detected_at",
        params![
            conflict_id,
            asset_id,
            project_id,
            source_path,
            target,
            if result.status == "SKIPPED_EXIST" { "exists" } else { "failed" },
            now_value
        ],
    )
    .map_err(|error| error.to_string())?;
    tx.execute(
        "UPDATE pending_items SET conflict_reason=?1, updated_at=?2 WHERE asset_id=?3",
        params![result.message, now_value, asset_id],
    )
    .map_err(|error| error.to_string())?;
    tx.commit().map_err(|error| error.to_string())?;
    refresh_project_counts(&connection, &asset_id)?;
    Ok(MoveResult {
        success: false,
        status: result.status,
        message: result.message,
        source_path,
        target_path: target,
    })
}


pub fn metadata(app: &AppHandle) -> Result<Value, String> {
    ensure_v2(app)?;
    let connection = open_v2(app)?;
    let mut authors_stmt = connection
        .prepare("SELECT id,name FROM authors ORDER BY name COLLATE NOCASE")
        .map_err(|error| error.to_string())?;
    let authors = authors_stmt
        .query_map([], |row| {
            Ok(serde_json::json!({"id": row.get::<_, String>(0)?, "name": row.get::<_, String>(1)?}))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut categories_stmt = connection
        .prepare("SELECT id,name,parent_id FROM categories ORDER BY name COLLATE NOCASE")
        .map_err(|error| error.to_string())?;
    let categories = categories_stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "name": row.get::<_, String>(1)?,
                "parentId": row.get::<_, Option<String>>(2)?,
            }))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({"authors": authors, "categories": categories}))
}

pub async fn create_pending_project(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    name: String,
    asset_ids: Vec<String>,
) -> Result<PendingProjectSummary, String> {
    ensure_v2(&app)?;
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("项目名不能为空".into());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("项目名不能包含路径分隔符".into());
    }
    if asset_ids.is_empty() {
        return Err("请选择要生成项目的文件".into());
    }
    let folder_name = if trimmed.starts_with('【') && trimmed.ends_with('】') {
        trimmed.to_owned()
    } else {
        format!("【{}】", trimmed)
    };
    let connection = open_v2(&app)?;
    let pending_root: String = connection
        .query_row(
            "SELECT remote_path FROM domain_roots WHERE domain='pending'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let root_id: String = connection
        .query_row(
            "SELECT root_id FROM domain_roots WHERE domain='pending'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let project_id = stable_id("pending", &format!("{pending_root}|{folder_name}"));
    let source_path = format!("{}/{}", pending_root.trim_end_matches('/'), folder_name);
    let created = now();
    connection
        .execute(
            "INSERT INTO pending_projects(id,name,source_path,root_id,is_loose,status,total_count,triage_count,ready_count,created_at,updated_at)
             VALUES(?1,?2,?3,?4,0,'discovered',0,0,0,?5,?5)
             ON CONFLICT(id) DO UPDATE SET updated_at=excluded.updated_at",
            params![project_id, folder_name, source_path, root_id, created],
        )
        .map_err(|error| error.to_string())?;

    let mut moved = 0i64;
    for asset_id in asset_ids {
        let source: Option<String> = connection
            .query_row(
                "SELECT remote_path FROM assets WHERE id=?1 AND domain='recent' AND missing=0",
                params![asset_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(source) = source else { continue };
        let file_name = source
            .rsplit('/')
            .next()
            .unwrap_or(&source)
            .to_owned();
        let target = format!("{}/{}", source_path, file_name);
        let request = media_library::CurateMoveItemRequest {
            source_path: source.clone(),
            target_path: target.clone(),
            is_directory: Some(false),
        };
        let result = media_library::curate_move_item(app.clone(), state.clone(), request).await?;
        if result.status == "MOVED" {
            moved += 1;
            connection
                .execute(
                    "UPDATE assets SET domain='pending', remote_path=?1, file_name=?2,
                        parent_path=?3, last_seen_at=?4, missing=0, missing_since=NULL, missing_scan_count=0
                     WHERE id=?5",
                    params![target, file_name, source_path, now(), asset_id],
                )
                .map_err(|error| error.to_string())?;
            let recommendation = recommend_for(&connection, &asset_id)?;
            connection
                .execute(
                    "INSERT INTO pending_items(asset_id,project_id,recommended_category_id,
                        recommended_author_id,candidate_authors_json,candidate_categories_json,
                        confidence,reason,assignment_status,updated_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'triage',?9)
                     ON CONFLICT(asset_id) DO UPDATE SET project_id=pending_items.project_id,
                        recommended_category_id=excluded.recommended_category_id,
                        recommended_author_id=excluded.recommended_author_id,
                        candidate_authors_json=excluded.candidate_authors_json,
                        candidate_categories_json=excluded.candidate_categories_json,
                        confidence=excluded.confidence, reason=excluded.reason,
                        updated_at=excluded.updated_at",
                    params![
                        asset_id,
                        project_id,
                        recommendation.category,
                        recommendation.author,
                        recommendation.candidate_authors.to_string(),
                        recommendation.candidate_categories.to_string(),
                        recommendation.confidence,
                        recommendation.reason,
                        now()
                    ],
                )
                .map_err(|error| error.to_string())?;
        } else {
            let conflict_id = stable_id("conflict", &format!("{asset_id}|{target}"));
            connection
                .execute(
                    "INSERT INTO move_conflicts(id,asset_id,project_id,source_path,target_path,kind,detected_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7)
                     ON CONFLICT(asset_id,target_path) DO UPDATE SET detected_at=excluded.detected_at",
                    params![conflict_id, asset_id, project_id, source, target, result.status, now()],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    if moved == 0 {
        let _ = connection.execute("DELETE FROM pending_projects WHERE id=?1", params![project_id]);
        return Err("没有文件被移动；目标可能已存在，请查看冲突列表".into());
    }
    connection
        .execute(
            "UPDATE pending_projects SET status='discovered', total_count=(
                SELECT COUNT(*) FROM pending_items WHERE project_id=?1
             ), triage_count=(
                SELECT COUNT(*) FROM pending_items WHERE project_id=?1 AND assignment_status='triage'
             ), ready_count=(
                SELECT COUNT(*) FROM pending_items WHERE project_id=?1 AND assignment_status='ready'
             ), updated_at=?2 WHERE id=?1",
            params![project_id, now()],
        )
        .map_err(|error| error.to_string())?;
    let summary = connection
        .query_row(
            "SELECT id,name,source_path,status,is_loose,total_count,triage_count,ready_count,
                    (SELECT COUNT(*) FROM pending_items i WHERE i.project_id=p.id AND i.conflict_reason IS NOT NULL)
             FROM pending_projects p WHERE id=?1",
            params![project_id],
            |row| {
                Ok(PendingProjectSummary {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    source_path: row.get(2)?,
                    status: row.get(3)?,
                    is_loose: row.get::<_, i64>(4)? != 0,
                    total_count: row.get(5)?,
                    triage_count: row.get(6)?,
                    ready_count: row.get(7)?,
                    conflict_count: row.get(8)?,
                })
            },
        )
        .map_err(|error| error.to_string())?;
    Ok(summary)
}


#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainChild {
    pub path: String,
    pub name: String,
    pub is_directory: bool,
    pub asset_id: Option<String>,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub modified_at: Option<String>,
    pub thumbnail_asset_id: Option<String>,
    pub child_count: i64,
}

pub fn children(
    app: &AppHandle,
    domain: &str,
    path: Option<&str>,
) -> Result<Vec<DomainChild>, String> {
    if !domain_valid(domain) {
        return Err("未知域".into());
    }
    ensure_v2(app)?;
    let connection = open_v2(app)?;
    let root_path: String = connection
        .query_row(
            "SELECT remote_path FROM domain_roots WHERE domain=?1",
            params![domain],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "该域还没有配置根目录".to_string())?;
    let parent = path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&root_path)
        .trim_end_matches('/')
        .to_owned();
    let prefix = if parent.is_empty() {
        "%".to_owned()
    } else {
        format!("{parent}/%")
    };
    let mut statement = connection
        .prepare(
            "SELECT id,remote_path,file_name,media_type,size_bytes,modified_at,thumbnail_asset_id
             FROM assets WHERE domain=?1 AND missing=0 AND remote_path LIKE ?2
             ORDER BY remote_path COLLATE NOCASE",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![domain, prefix], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut directories: BTreeMap<String, (String, i64, i64, Option<String>)> = BTreeMap::new();
    let mut files: Vec<DomainChild> = Vec::new();
    for row in rows {
        let (id, remote_path, file_name, media_type, size_bytes, modified_at, thumbnail) =
            row.map_err(|error| error.to_string())?;
        let relative = remote_path
            .strip_prefix(&parent)
            .unwrap_or(&remote_path)
            .trim_start_matches('/');
        if relative.is_empty() {
            continue;
        }
        if let Some((first, _rest)) = relative.split_once('/') {
            if first.is_empty() {
                continue;
            }
            let directory_path = format!("{}/{}", parent.trim_end_matches('/'), first);
            let entry = directories
                .entry(directory_path)
                .or_insert_with(|| (first.to_owned(), 0, 0, None));
            entry.1 += 1;
            entry.2 += size_bytes;
            if let Some(value) = &modified_at {
                if entry.3.as_ref().map_or(true, |current| value > current) {
                    entry.3 = Some(value.clone());
                }
            }
        } else {
            files.push(DomainChild {
                path: remote_path,
                name: file_name,
                is_directory: false,
                asset_id: Some(id),
                media_type: Some(media_type),
                size_bytes,
                modified_at,
                thumbnail_asset_id: thumbnail,
                child_count: 0,
            });
        }
    }
    let root_id: String = connection.query_row("SELECT root_id FROM domain_roots WHERE domain=?1",
        params![domain], |row| row.get(0)).map_err(|error| error.to_string())?;
    if has_directory_snapshot(&connection, domain, &root_id, &parent)? {
        let mut statement = connection.prepare(
            "SELECT e.remote_path,e.name,e.is_directory,e.size_bytes,e.modified_at,
                a.id,a.media_type,a.thumbnail_asset_id
             FROM provider_directory_entries e LEFT JOIN assets a
                ON a.domain=e.domain AND a.remote_path=e.remote_path AND a.missing=0
             WHERE e.domain=?1 AND e.root_id=?2 AND e.parent_path=?3")
            .map_err(|error| error.to_string())?;
        let rows = statement.query_map(params![domain, root_id, parent], |row| Ok(DomainChild {
            path: row.get(0)?, name: row.get(1)?, is_directory: row.get(2)?,
            size_bytes: row.get(3)?, modified_at: row.get(4)?, asset_id: row.get(5)?,
            media_type: row.get(6)?, thumbnail_asset_id: row.get(7)?, child_count: 0,
        })).map_err(|error| error.to_string())?;
        let old_directories = std::mem::take(&mut directories);
        files.clear();
        for row in rows {
            let mut entry = row.map_err(|error| error.to_string())?;
            if entry.is_directory {
                let count = old_directories.get(&entry.path).map(|value| value.1).unwrap_or(0);
                directories.insert(entry.path, (entry.name, count, entry.size_bytes, entry.modified_at));
            } else {
                // A provider media file remains visible even if no asset mapping exists.
                if entry.media_type.is_none() {
                    entry.media_type = domain_media_type(domain, &entry.path).map(str::to_owned);
                }
                files.push(entry);
            }
        }
    }
    files.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    let mut result: Vec<DomainChild> = directories
        .into_iter()
        .map(|(path, (name, count, size_bytes, modified_at))| DomainChild {
            path,
            name,
            is_directory: true,
            asset_id: None,
            media_type: None,
            size_bytes,
            modified_at,
            thumbnail_asset_id: None,
            child_count: count,
        })
        .collect();
    result.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    result.extend(files);
    Ok(result)
}


fn modified_key(value: Option<&str>) -> i64 {
    let Some(value) = value else { return i64::MIN };
    chrono::DateTime::parse_from_rfc3339(value)
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(value))
        .map(|date| date.timestamp())
        .unwrap_or(i64::MIN)
}

fn browse_progress(progress: Option<f64>) -> Option<f64> {
    progress.filter(|seconds| seconds.is_finite())
}

pub async fn compat_remote_children(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    root_id: &str,
    path: &str,
    refresh: bool,
    sort: &str,
) -> Result<Value, String> {
    ensure_v2(&app)?;
    let connection = open_v2(&app)?;
    let domain: Option<String> = connection
        .query_row(
            "SELECT domain FROM domain_roots WHERE root_id=?1 LIMIT 1",
            params![root_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(domain) = domain else {
        return Err("V2 没有该根目录".into());
    };
    drop(connection);

    let scan_path = if path.trim().is_empty() {
        let connection = open_v2(&app)?;
        domain_root(&connection, &domain)?.1
    } else {
        path.to_owned()
    };
    let scan_path = scan_path.trim().trim_end_matches('/').to_owned();
    let connection = open_v2(&app)?;
    let has_snapshot = has_directory_snapshot(&connection, &domain, root_id, &scan_path)?;
    drop(connection);
    // A provider success is the only source of these counters; a cached read
    // must not claim to have checked CD2.
    let refresh_summary = if refresh || !has_snapshot {
        Some(refresh_domain_path(app.clone(), state, &domain, &scan_path).await?)
    } else {
        None
    };

    let mut entries = children(&app, &domain, Some(&scan_path))?;
    let sort_key = sort.to_owned();
    entries.sort_by(|left, right| {
        if left.is_directory != right.is_directory {
            return right.is_directory.cmp(&left.is_directory);
        }
        let result = match sort_key.as_str() {
            "newest" => modified_key(right.modified_at.as_deref())
                .cmp(&modified_key(left.modified_at.as_deref())),
            "size" => right.size_bytes.cmp(&left.size_bytes),
            _ => natural_name_cmp(&left.name, &right.name),
        };
        if result == std::cmp::Ordering::Equal {
            natural_name_cmp(&left.name, &right.name)
        } else {
            result
        }
    });
    let connection = open_v2(&app)?;
    let mut statement = connection.prepare("SELECT remote_path,cover_asset_id FROM directory_sync_state WHERE root_id=?1 AND cover_asset_id IS NOT NULL")
        .map_err(|error| error.to_string())?;
    let covers = statement.query_map(params![root_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|error| error.to_string())?.collect::<Result<HashMap<_, _>, _>>()
        .map_err(|error| error.to_string())?;
    let mut metadata = connection.prepare(
        "SELECT a.etag,a.duration_seconds,s.progress_seconds,a.thumbnail_asset_id
         FROM assets a LEFT JOIN asset_state s ON s.asset_id=a.id WHERE a.id=?1 AND a.missing=0")
        .map_err(|error| error.to_string())?;
    let mut provider_metadata = connection.prepare(
        "SELECT etag FROM provider_directory_entries WHERE domain=?1 AND root_id=?2 AND parent_path=?3 AND remote_path=?4")
        .map_err(|error| error.to_string())?;
    let mut default_cover = connection.prepare(
        "SELECT id FROM assets WHERE domain=?1 AND missing=0 AND substr(remote_path,1,length(?2))=?2
         AND media_type IN ('image','video','comicPage','comicArchive')
         ORDER BY CASE WHEN media_type IN ('image','comicPage') THEN 0 ELSE 1 END,
            remote_path COLLATE NOCASE,id LIMIT 1")
        .map_err(|error| error.to_string())?;
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let cover = if entry.is_directory {
            if let Some(cover) = covers.get(&entry.path).filter(|value| !value.is_empty()) {
                Some(cover.clone())
            } else {
                default_cover.query_row(params![domain, format!("{}/", entry.path)], |row| row.get::<_, String>(0))
                    .optional().map_err(|error| error.to_string())?
            }
        } else { None };
        let asset_metadata = if let Some(id) = &entry.asset_id {
            metadata.query_row(params![id], |row| Ok((row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<i64>>(1)?, row.get::<_, Option<f64>>(2)?, row.get::<_, Option<String>>(3)?)))
                .optional().map_err(|error| error.to_string())?
        } else { None };
        let provider_etag = provider_metadata.query_row(params![domain, root_id, scan_path, entry.path], |row| row.get::<_, Option<String>>(0))
            .optional().map_err(|error| error.to_string())?.flatten();
        let (asset_etag, duration, progress, thumbnail_id) = asset_metadata.unwrap_or((None,None,None,None));
        let thumbnail_source = if let Some(id) = entry.asset_id.as_ref().or(cover.as_ref()) {
            media_library::cached_asset_cover_url(&app, id)
        } else { None };
        items.push(serde_json::json!({
            "path": entry.path,
            "name": entry.name,
            "isDirectory": entry.is_directory,
            "sizeBytes": entry.size_bytes,
            "modifiedAt": entry.modified_at,
            "etag": provider_etag.or(asset_etag),
            "assetId": entry.asset_id,
            "mediaType": entry.media_type,
            "thumbnailAssetId": thumbnail_id,
            "thumbnailSource": thumbnail_source,
            "thumbnailUrl": thumbnail_source,
            "coverAssetId": cover,
            "durationSeconds": duration,
            "progressSeconds": browse_progress(progress),
        }));
    }
    let from_cache = refresh_summary.is_none();
    // "total" is what the user just saw listed (files and folders), while the
    // added/updated/removed counters describe asset-level reconciliation.
    let listed = items.len() as i64;
    let sync_summary = refresh_summary.map(|summary| {
        serde_json::json!({
            "total": listed,
            "added": summary.added,
            "updated": summary.updated,
            "removed": summary.deleted,
        })
    });
    Ok(serde_json::json!({
        "items": items,
        "nextCursor": null,
        "fromLocalCache": from_cache,
        "syncSummary": sync_summary,
    }))
}

pub async fn compat_media_source(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    asset_id: String,
) -> Result<String, String> {
    media_library::open_media_source(app, state, asset_id).await
}

fn parent_path(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(parent, _)| parent.to_owned())
        .unwrap_or_else(|| "/".to_owned())
}

#[cfg(test)]
mod directory_snapshot_tests {
    use super::*;

    fn database() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(schema_sql()).unwrap();
        connection
    }

    fn entry(path: &str, directory: bool) -> media_library::ProviderEntry {
        media_library::ProviderEntry {
            path: path.into(), name: path.rsplit('/').next().unwrap().into(),
            is_directory: directory, size_bytes: 42, modified_at: None, etag: None,
        }
    }

    fn asset(connection: &Connection, domain: &str, path: &str) {
        connection.execute("INSERT INTO assets(id,domain,root_id,remote_path,file_name,parent_path,
            media_type,first_seen_at,last_seen_at) VALUES(?1,?2,'root',?3,'a.jpg',?4,'image',0,0)",
            params![format!("legacy-{path}"), domain, path, parent_path(path)]).unwrap();
    }

    #[test]
    fn playback_real_progress_preserves_domain_and_browse_precision() {
        let connection = database();
        asset(&connection, "recent", "/root/a.jpg");
        connection.execute("INSERT INTO asset_state(asset_id,progress_seconds,updated_at) VALUES('legacy-/root/a.jpg',12.75,0)", []).unwrap();
        let items = list_assets_from_connection(&connection, "recent", None, None, 100, 0).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].progress_seconds, Some(12.75));
        let domain_json = serde_json::to_value(&items[0]).unwrap();
        assert_eq!(domain_json["progressSeconds"].as_f64(), Some(12.75));
        let browse_json = serde_json::json!({"progressSeconds": browse_progress(items[0].progress_seconds)});
        assert_eq!(browse_json["progressSeconds"].as_f64(), Some(12.75));
        assert_eq!(browse_progress(None), None);
        // SQLite REAL affinity also stores integral values as REAL.
        connection.execute("UPDATE asset_state SET progress_seconds=12.0", []).unwrap();
        assert_eq!(list_assets_from_connection(&connection, "recent", None, None, 100, 0).unwrap()[0].progress_seconds, Some(12.0));
    }

    #[test]
    fn present_folder_preserves_descendants_and_empty_nonmedia_entries() {
        let connection = database();
        asset(&connection, "recent", "/root/keep/deep/a.jpg");
        asset(&connection, "recent", "/root/gone/a.jpg");
        let tx = connection.unchecked_transaction().unwrap();
        let counts = reconcile_directory_snapshot(&tx, "recent", "root", "/root",
            &[entry("/root/keep", true), entry("/root/empty", true), entry("/root/notes.txt", false)]).unwrap();
        assert_eq!(counts, (1, 1));
        tx.commit().unwrap();
        assert_eq!(connection.query_row("SELECT id FROM assets", [], |row| row.get::<_, String>(0)).unwrap(),
            "legacy-/root/keep/deep/a.jpg");
        assert!(has_directory_snapshot(&connection, "recent", "root", "/root").unwrap());
        assert_eq!(connection.query_row("SELECT count(*) FROM provider_directory_entries", [], |row| row.get::<_, i64>(0)).unwrap(), 3);
    }

    #[test]
    fn empty_complete_listing_respects_vault_grace_and_invalidates_subtree() {
        let connection = database();
        asset(&connection, "vault", "/root/gone/a.jpg");
        let tx = connection.unchecked_transaction().unwrap();
        assert_eq!(reconcile_directory_snapshot(&tx, "vault", "root", "/root/gone", &[]).unwrap(), (1, 0));
        tx.commit().unwrap();
        let tx = connection.unchecked_transaction().unwrap();
        assert_eq!(reconcile_directory_snapshot(&tx, "vault", "root", "/root", &[]).unwrap(), (1, 1));
        tx.commit().unwrap();
        assert!(!has_directory_snapshot(&connection, "vault", "root", "/root/gone").unwrap());
        assert!(has_directory_snapshot(&connection, "vault", "root", "/root").unwrap());
    }

    #[test]
    fn pending_refresh_preserves_manual_project_and_ready_assignment() {
        let connection = database();
        connection.execute("INSERT INTO domain_roots VALUES('pending','root','/root',0)", []).unwrap();
        connection.execute("INSERT INTO pending_projects(id,name,source_path,root_id,created_at,updated_at)
            VALUES('manual','Manual','/root','root',0,0),('empty','Empty','/root/empty','root',0,0)", []).unwrap();
        asset(&connection, "pending", "/root/a.jpg");
        connection.execute("INSERT INTO pending_items(asset_id,project_id,assignment_status,manual_author_id,updated_at)
            VALUES('legacy-/root/a.jpg','manual','ready','chosen-author',0)", []).unwrap();
        let tx = connection.unchecked_transaction().unwrap();
        rebuild_pending_projects_v2(&tx, "/root").unwrap();
        tx.commit().unwrap();
        let assignment: (String,String,String) = connection.query_row(
            "SELECT project_id,assignment_status,manual_author_id FROM pending_items", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(assignment, ("manual".into(),"ready".into(),"chosen-author".into()));
        let counts: (i64,i64,i64) = connection.query_row(
            "SELECT total_count,triage_count,ready_count FROM pending_projects WHERE id='manual'", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(counts, (1,0,1));
        assert_eq!(connection.query_row("SELECT count(*) FROM pending_projects WHERE id='empty'", [], |row| row.get::<_,i64>(0)).unwrap(), 1);
    }

    #[test]
    fn natural_name_sort_orders_numbers_and_ignores_case() {
        assert_eq!(natural_name_cmp("第2集", "第10集"), std::cmp::Ordering::Less);
        assert_eq!(natural_name_cmp("ep2", "EP10"), std::cmp::Ordering::Less);
        assert_eq!(natural_name_cmp("a2", "a02"), std::cmp::Ordering::Equal);
        assert_eq!(natural_name_cmp("abc", "abd"), std::cmp::Ordering::Less);
        assert_eq!(natural_name_cmp("2.jpg", "10.jpg"), std::cmp::Ordering::Less);
    }

    #[test]
    fn invalid_listing_and_rolled_back_reconcile_preserve_data() {
        let connection = database();
        asset(&connection, "recent", "/root/a.jpg");
        {
            let tx = connection.unchecked_transaction().unwrap();
            assert!(reconcile_directory_snapshot(&tx, "recent", "root", "/root", &[entry("/elsewhere/a.jpg", false)]).is_err());
            reconcile_directory_snapshot(&tx, "recent", "root", "/root", &[]).unwrap();
            // Failure before commit rolls back both assets and completion marker.
        }
        assert_eq!(connection.query_row("SELECT count(*) FROM assets", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        assert!(!has_directory_snapshot(&connection, "recent", "root", "/root").unwrap());
        assert!(validate_provider_listing("/root", &[entry("/root/a.jpg", false), entry("/root/a.jpg", false)]).is_err());
    }
}
