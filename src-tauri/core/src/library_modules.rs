use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{four_domains, media_library, AppHandle, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryModuleInfo {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub remote_path: String,
    pub remote_paths: Vec<String>,
    pub domain: String,
    pub enabled: bool,
    pub sort_order: i64,
    pub last_scanned_at: Option<i64>,
    pub asset_count: i64,
    pub total_bytes: i64,
    pub entity_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleRebuildSummary {
    pub module_id: String,
    pub kind: String,
    pub tagged_assets: i64,
    pub entity_count: i64,
    pub item_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleScanResult {
    pub module_id: String,
    pub scan: four_domains::ScanSummary,
    pub rebuild: ModuleRebuildSummary,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn has_column(conn: &Connection, table: &str, col: &str) -> Result<bool, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| e.to_string())?;
    for r in rows.flatten() {
        if r == col {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Strip surrounding brackets like `【】`, `[]`, `「」`, `『』` and whitespace.
pub(crate) fn clean_bracket_name(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    loop {
        let trimmed = s.trim();
        let next = if let Some(inner) = trimmed.strip_prefix('【').and_then(|v| v.strip_suffix('】'))
        {
            inner
        } else if let Some(inner) = trimmed.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            inner
        } else if let Some(inner) = trimmed.strip_prefix('「').and_then(|v| v.strip_suffix('」')) {
            inner
        } else if let Some(inner) = trimmed.strip_prefix('『').and_then(|v| v.strip_suffix('』')) {
            inner
        } else {
            break;
        };
        let next_clean = next.trim();
        if next_clean.is_empty() || next_clean == s {
            break;
        }
        s = next_clean.to_string();
    }
    s.trim().to_string()
}

fn is_archive_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".zip")
        || lower.ends_with(".cbz")
        || lower.ends_with(".rar")
        || lower.ends_with(".cbr")
        || lower.ends_with(".7z")
        || lower.ends_with(".pdf")
}

fn strip_extension(name: &str) -> String {
    if let Some((stem, ext)) = name.rsplit_once('.') {
        if !stem.is_empty() && ext.len() <= 5 {
            return stem.to_string();
        }
    }
    name.to_string()
}

fn stable_id(prefix: &str, key: &str) -> String {
    // Deterministic FNV-1a 64-bit hash so album/set IDs stay stable across rebuilds.
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}-{hash:016x}")
}

/// Additive schema migration for multi-module libraries, creators, collections, and galleries.
pub(crate) fn ensure_module_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS library_modules (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            remote_path TEXT NOT NULL,
            domain TEXT NOT NULL DEFAULT 'vault',
            enabled INTEGER NOT NULL DEFAULT 1,
            sort_order INTEGER NOT NULL DEFAULT 0,
            last_scanned_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS author_assets (
            asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
            author_id TEXT NOT NULL REFERENCES authors(id) ON DELETE CASCADE,
            studio_name TEXT NOT NULL,
            album_name TEXT,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_author_assets_author_studio
            ON author_assets(author_id, studio_name);
        CREATE INDEX IF NOT EXISTS idx_author_assets_studio_author
            ON author_assets(studio_name, author_id);

        CREATE TABLE IF NOT EXISTS author_merge_ignores (
            author_id_a TEXT NOT NULL,
            author_id_b TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(author_id_a, author_id_b)
        );

        CREATE TABLE IF NOT EXISTS collection_albums (
            id TEXT PRIMARY KEY,
            channel TEXT NOT NULL,
            name TEXT NOT NULL,
            remote_path TEXT NOT NULL,
            is_direct INTEGER NOT NULL DEFAULT 0,
            cover_asset_id TEXT,
            item_count INTEGER NOT NULL DEFAULT 0,
            video_count INTEGER NOT NULL DEFAULT 0,
            image_count INTEGER NOT NULL DEFAULT 0,
            total_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL,
            UNIQUE(channel, name)
        );
        CREATE INDEX IF NOT EXISTS idx_collection_albums_channel
            ON collection_albums(channel, item_count DESC);

        CREATE TABLE IF NOT EXISTS collection_items (
            asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
            channel TEXT NOT NULL,
            album_id TEXT NOT NULL REFERENCES collection_albums(id) ON DELETE CASCADE,
            media_type TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_collection_items_channel
            ON collection_items(channel, media_type);
        CREATE INDEX IF NOT EXISTS idx_collection_items_album
            ON collection_items(album_id);

        CREATE TABLE IF NOT EXISTS gallery_sets (
            id TEXT PRIMARY KEY,
            model_name TEXT NOT NULL,
            album_name TEXT NOT NULL,
            remote_path TEXT NOT NULL,
            is_archive INTEGER NOT NULL DEFAULT 0,
            cover_asset_id TEXT,
            item_count INTEGER NOT NULL DEFAULT 0,
            total_bytes INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL,
            UNIQUE(model_name, album_name)
        );
        CREATE INDEX IF NOT EXISTS idx_gallery_sets_model
            ON gallery_sets(model_name, item_count DESC);

        CREATE TABLE IF NOT EXISTS gallery_items (
            asset_id TEXT PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
            set_id TEXT NOT NULL REFERENCES gallery_sets(id) ON DELETE CASCADE,
            sort_order INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_gallery_items_set
            ON gallery_items(set_id, sort_order);

        CREATE TABLE IF NOT EXISTS gallery_cover_overrides (
            set_id TEXT PRIMARY KEY,
            cover_asset_id TEXT NOT NULL,
            cover_page_index INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS entity_favorites (
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(entity_type, entity_id)
        );
        CREATE INDEX IF NOT EXISTS idx_entity_favorites_type_time
            ON entity_favorites(entity_type, created_at DESC);
        "#,
    )
    .map_err(|e| format!("初始化多模块表结构失败：{e}"))?;

    if !has_column(conn, "assets", "module_id")? {
        conn.execute("ALTER TABLE assets ADD COLUMN module_id TEXT", [])
            .map_err(|e| format!("为 assets 添加 module_id 列失败：{e}"))?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS assets_module_missing ON assets(module_id, missing)",
        [],
    )
    .map_err(|e| e.to_string())?;

    if !has_column(conn, "authors", "bio")? {
        conn.execute("ALTER TABLE authors ADD COLUMN bio TEXT", [])
            .map_err(|e| format!("为 authors 添加 bio 列失败：{e}"))?;
    }
    if !has_column(conn, "authors", "social_links_json")? {
        conn.execute(
            "ALTER TABLE authors ADD COLUMN social_links_json TEXT NOT NULL DEFAULT '{}'",
            [],
        )
        .map_err(|e| format!("为 authors 添加 social_links_json 列失败：{e}"))?;
    }

    if !has_column(conn, "library_modules", "extra_paths_json")? {
        conn.execute(
            "ALTER TABLE library_modules ADD COLUMN extra_paths_json TEXT NOT NULL DEFAULT '[]'",
            [],
        )
        .map_err(|e| format!("为 library_modules 添加 extra_paths_json 列失败：{e}"))?;
    }

    if !has_column(conn, "gallery_sets", "cover_page_index")? {
        conn.execute(
            "ALTER TABLE gallery_sets ADD COLUMN cover_page_index INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| format!("为 gallery_sets 添加 cover_page_index 列失败：{e}"))?;
    }

    seed_default_modules(conn)?;
    sync_legacy_directory_configs(conn)?;
    Ok(())
}

fn parse_module_paths(primary: &str, extra_json: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let p0 = primary.trim().trim_end_matches('/').to_string();
    if !p0.is_empty() {
        out.push(p0);
    }
    if let Some(raw) = extra_json {
        if let Ok(extras) = serde_json::from_str::<Vec<String>>(raw) {
            for item in extras {
                let clean = item.trim().trim_end_matches('/').to_string();
                if !clean.is_empty() && !out.contains(&clean) {
                    out.push(clean);
                }
            }
        }
    }
    out
}

fn normalize_module_paths(primary: &str, remote_paths: Option<Vec<String>>) -> (String, String, Vec<String>) {
    let mut all = Vec::new();
    if let Some(list) = remote_paths {
        for item in list {
            let clean = item.trim().trim_end_matches('/').to_string();
            if !clean.is_empty() && !all.contains(&clean) {
                all.push(clean);
            }
        }
    }
    if all.is_empty() {
        let p0 = primary.trim().trim_end_matches('/').to_string();
        if !p0.is_empty() {
            all.push(p0);
        }
    }
    let first = all.first().cloned().unwrap_or_default();
    let extras: Vec<String> = all.iter().skip(1).cloned().collect();
    let extra_json = serde_json::to_string(&extras).unwrap_or_else(|_| "[]".to_string());
    (first, extra_json, all)
}

/// One-time migration: unify legacy `scrape_whitelist` into `library_modules(movies)`
/// so all module paths live in one single source of truth.
fn sync_legacy_directory_configs(conn: &Connection) -> Result<(), String> {
    let done = conn
        .query_row(
            "SELECT 1 FROM schema_meta WHERE key = 'modules_unified_paths_v1'",
            [],
            |_| Ok(true),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(false);
    if done {
        return Ok(());
    }

    let mut wl_paths = Vec::new();
    if let Ok(mut stmt) = conn.prepare("SELECT path FROM scrape_whitelist WHERE enabled = 1 ORDER BY created_at ASC, path ASC") {
        if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
            for r in rows.flatten() {
                let clean = r.trim().trim_end_matches('/').to_string();
                if !clean.is_empty() && !wl_paths.contains(&clean) {
                    wl_paths.push(clean);
                }
            }
        }
    }
    if !wl_paths.is_empty() {
        let (first, extra_json, _) = normalize_module_paths(&wl_paths[0], Some(wl_paths.clone()));
        let _ = conn.execute(
            "UPDATE library_modules SET remote_path = ?1, extra_paths_json = ?2, updated_at = ?3 WHERE kind = 'movies'",
            params![first, extra_json, now()],
        );
    }

    let _ = conn.execute(
        "INSERT OR IGNORE INTO schema_meta(key, value) VALUES('modules_unified_paths_v1', '1')",
        [],
    );
    Ok(())
}

fn seed_default_modules(conn: &Connection) -> Result<(), String> {
    let existing: i64 = conn
        .query_row("SELECT COUNT(*) FROM library_modules", [], |r| r.get(0))
        .unwrap_or(0);
    if existing > 0 {
        return Ok(());
    }

    let vault_root: String = conn
        .query_row(
            "SELECT remote_path FROM domain_roots WHERE domain='vault'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "/115open/文件".to_string());
    let vault_base = vault_root.trim_end_matches('/');

    let comics_root: String = conn
        .query_row(
            "SELECT remote_path FROM domain_roots WHERE domain='comics'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| format!("{vault_base}/漫画"));
    let comics_base = comics_root.trim_end_matches('/');

    let ts = now();
    let defaults = [
        ("movies", "movies", "媒体库", format!("{vault_base}/媒体库"), "vault", 10i64),
        ("creators", "creators", "创作者", format!("{vault_base}/创作者"), "vault", 20),
        ("collections", "collections", "合集", format!("{vault_base}/合集"), "vault", 30),
        ("galleries", "galleries", "图集", format!("{vault_base}/图集"), "vault", 40),
        ("comics", "comics", "漫画", comics_base.to_string(), "comics", 50),
    ];

    for (id, kind, name, path, domain, order) in defaults {
        conn.execute(
            "INSERT OR IGNORE INTO library_modules
             (id, kind, name, remote_path, domain, enabled, sort_order, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?7)",
            params![id, kind, name, path, domain, order, ts],
        )
        .map_err(|e| format!("初始化默认模块 {id} 失败：{e}"))?;
    }
    Ok(())
}

pub fn list_modules(app: &AppHandle) -> Result<Vec<LibraryModuleInfo>, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, kind, name, remote_path, COALESCE(extra_paths_json, '[]'), domain, enabled, sort_order, last_scanned_at
             FROM library_modules
             ORDER BY sort_order ASC, id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)? != 0,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<i64>>(8)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut modules = Vec::new();
    for row in rows {
        let (id, kind, name, remote_path, extra_paths_json, domain, enabled, sort_order, last_scanned_at) =
            row.map_err(|e| e.to_string())?;
        let remote_paths = parse_module_paths(&remote_path, Some(&extra_paths_json));

        let (asset_count, total_bytes): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0)
                 FROM assets
                 WHERE module_id = ?1 AND missing = 0",
                params![&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or((0, 0));

        let entity_count: i64 = match kind.as_str() {
            "creators" => conn
                .query_row(
                    "SELECT COUNT(DISTINCT author_id) FROM author_assets",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0),
            "collections" => conn
                .query_row("SELECT COUNT(*) FROM collection_albums", [], |r| r.get(0))
                .unwrap_or(0),
            "galleries" => conn
                .query_row("SELECT COUNT(*) FROM gallery_sets", [], |r| r.get(0))
                .unwrap_or(0),
            "movies" => conn
                .query_row("SELECT COUNT(*) FROM movie_metadata", [], |r| r.get(0))
                .unwrap_or(0),
            "comics" => conn
                .query_row(
                    "SELECT COUNT(*) FROM assets WHERE module_id = ?1 AND missing = 0",
                    params![&id],
                    |r| r.get(0),
                )
                .unwrap_or(0),
            _ => asset_count,
        };

        modules.push(LibraryModuleInfo {
            id,
            kind,
            name,
            remote_path,
            remote_paths,
            domain,
            enabled,
            sort_order,
            last_scanned_at,
            asset_count,
            total_bytes,
            entity_count,
        });
    }
    Ok(modules)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertModuleRequest {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub remote_path: String,
    #[serde(default)]
    pub remote_paths: Option<Vec<String>>,
    pub domain: Option<String>,
    pub enabled: Option<bool>,
    pub sort_order: Option<i64>,
}

pub fn upsert_module(
    app: &AppHandle,
    req: UpsertModuleRequest,
) -> Result<LibraryModuleInfo, String> {
    four_domains::ensure_v2(app)?;
    let id = req.id.trim().to_string();
    let kind = req.kind.trim().to_string();
    let name = req.name.trim().to_string();
    let (remote_path, extra_paths_json, all_paths) =
        normalize_module_paths(&req.remote_path, req.remote_paths);
    if id.is_empty() || kind.is_empty() || name.is_empty() || remote_path.is_empty() {
        return Err("模块 id、kind、name、remotePath 均不能为空".into());
    }
    let domain = req
        .domain
        .unwrap_or_else(|| if kind == "comics" { "comics".into() } else { "vault".into() });
    let enabled = if req.enabled.unwrap_or(true) { 1i64 } else { 0i64 };
    let sort_order = req.sort_order.unwrap_or(100);
    let ts = now();

    let conn = four_domains::open_v2(app)?;
    conn.execute(
        "INSERT INTO library_modules
         (id, kind, name, remote_path, extra_paths_json, domain, enabled, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(id) DO UPDATE SET
           kind = excluded.kind,
           name = excluded.name,
           remote_path = excluded.remote_path,
           extra_paths_json = excluded.extra_paths_json,
           domain = excluded.domain,
           enabled = excluded.enabled,
           sort_order = excluded.sort_order,
           updated_at = excluded.updated_at",
        params![&id, &kind, &name, &remote_path, &extra_paths_json, &domain, enabled, sort_order, ts],
    )
    .map_err(|e| e.to_string())?;

    // Keep legacy tables strictly in sync so there is only ONE source of truth.
    if kind == "movies" {
        let _ = conn.execute("DELETE FROM scrape_whitelist", []);
        for p in &all_paths {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO scrape_whitelist (path, enabled, created_at) VALUES (?1, 1, ?2)",
                params![p, ts],
            );
        }
    } else if kind == "comics" {
        let _ = conn.execute(
            "UPDATE domain_roots SET remote_path = ?1, updated_at = ?2 WHERE domain = 'comics'",
            params![&remote_path, ts],
        );
    }

    let list = list_modules(app)?;
    list.into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "模块保存后读取失败".into())
}

/// Tag `assets.module_id` for a given module's remote_path prefix and rebuild its business index.
pub fn rebuild_module_index(app: &AppHandle, module_id: &str) -> Result<ModuleRebuildSummary, String> {
    four_domains::ensure_v2(app)?;
    let mut conn = four_domains::open_v2(app)?;
    rebuild_module_index_with_conn(&mut conn, module_id)
}

pub fn rebuild_all_module_indexes(app: &AppHandle) -> Result<Vec<ModuleRebuildSummary>, String> {
    four_domains::ensure_v2(app)?;
    let mut conn = four_domains::open_v2(app)?;
    let module_ids: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT id FROM library_modules WHERE enabled = 1 ORDER BY sort_order ASC, id ASC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.flatten().collect()
    };

    let mut results = Vec::new();
    for mid in module_ids {
        results.push(rebuild_module_index_with_conn(&mut conn, &mid)?);
    }
    Ok(results)
}

pub(crate) fn rebuild_module_index_with_conn(
    conn: &mut Connection,
    module_id: &str,
) -> Result<ModuleRebuildSummary, String> {
    let (kind, remote_path, extra_paths_json): (String, String, String) = conn
        .query_row(
            "SELECT kind, remote_path, COALESCE(extra_paths_json, '[]') FROM library_modules WHERE id = ?1",
            params![module_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("未找到模块：{module_id}"))?;

    let all_paths = parse_module_paths(&remote_path, Some(&extra_paths_json));
    let root = all_paths.first().cloned().unwrap_or_else(|| remote_path.trim_end_matches('/').to_string());
    let prefix = format!("{root}/");

    let tx = conn
        .transaction()
        .map_err(|e| format!("开启索引重建事务失败：{e}"))?;

    // 1. Tag assets belonging to ANY of this module's configured paths.
    tx.execute(
        "UPDATE assets SET module_id = NULL WHERE module_id = ?1",
        params![module_id],
    )
    .map_err(|e| e.to_string())?;

    let mut tagged_assets: i64 = 0;
    for p in &all_paths {
        let lp = format!("{p}/%");
        tagged_assets += tx
            .execute(
                "UPDATE assets SET module_id = ?1
                 WHERE missing = 0 AND (remote_path = ?2 OR remote_path LIKE ?3)",
                params![module_id, p, &lp],
            )
            .map_err(|e| e.to_string())? as i64;
    }

    let (entity_count, item_count) = match kind.as_str() {
        "creators" => rebuild_creators_index(&tx, module_id, &prefix)?,
        "collections" => rebuild_collections_index(&tx, module_id, &root, &prefix)?,
        "galleries" => rebuild_galleries_index(&tx, module_id, &root, &prefix)?,
        _ => {
            let cnt: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM assets WHERE module_id = ?1 AND missing = 0",
                    params![module_id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            (cnt, cnt)
        }
    };

    tx.commit()
        .map_err(|e| format!("提交索引重建事务失败：{e}"))?;

    Ok(ModuleRebuildSummary {
        module_id: module_id.to_string(),
        kind,
        tagged_assets,
        entity_count,
        item_count,
    })
}

fn rebuild_creators_index(
    tx: &rusqlite::Transaction,
    module_id: &str,
    prefix: &str,
) -> Result<(i64, i64), String> {
    let ts = now();

    // Load existing authors and their aliases for case-insensitive matching.
    let mut name_to_author_id: HashMap<String, String> = HashMap::new();
    {
        let mut stmt = tx
            .prepare("SELECT id, name, aliases_json FROM authors")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows.flatten() {
            let (aid, name, aliases_json) = row;
            let clean = clean_bracket_name(&name).to_lowercase();
            if !clean.is_empty() {
                name_to_author_id.entry(clean).or_insert_with(|| aid.clone());
            }
            if let Ok(aliases) = serde_json::from_str::<Vec<String>>(&aliases_json) {
                for alias in aliases {
                    let ac = clean_bracket_name(&alias).to_lowercase();
                    if !ac.is_empty() {
                        name_to_author_id.entry(ac).or_insert_with(|| aid.clone());
                    }
                }
            }
        }
    }

    // Read all active assets under creators module.
    let assets: Vec<(String, String, String)> = {
        let mut stmt = tx
            .prepare(
                "SELECT id, remote_path, media_type
                 FROM assets
                 WHERE module_id = ?1 AND missing = 0",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![module_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.flatten().collect()
    };

    tx.execute("DELETE FROM author_assets", [])
        .map_err(|e| e.to_string())?;

    let mut active_authors: HashSet<String> = HashSet::new();
    let mut item_count: i64 = 0;

    for (asset_id, remote_path, media_type) in assets {
        let Some(rel) = remote_path.strip_prefix(prefix) else {
            continue;
        };
        let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        // Expect at least: studio / author / file (len >= 3)
        if parts.len() < 3 {
            continue;
        }
        let studio_name = clean_bracket_name(parts[0]);
        let author_name = clean_bracket_name(parts[1]);
        if studio_name.is_empty() || author_name.is_empty() {
            continue;
        }

        // If inside a 3rd-level subfolder and it's an image, treat subfolder as album_name.
        let album_name = if parts.len() >= 4 && media_type == "image" {
            Some(parts[2].to_string())
        } else {
            None
        };

        let key = author_name.to_lowercase();
        let author_id = if let Some(existing_id) = name_to_author_id.get(&key) {
            existing_id.clone()
        } else {
            let new_id = stable_id("author", &key);
            tx.execute(
                "INSERT INTO authors(id, name, aliases_json, origin, created_at)
                 VALUES (?1, ?2, '[]', 'creators_module', ?3)
                 ON CONFLICT(name) DO NOTHING",
                params![&new_id, &author_name, ts],
            )
            .map_err(|e| e.to_string())?;
            let resolved_id: String = tx
                .query_row(
                    "SELECT id FROM authors WHERE name = ?1 COLLATE NOCASE",
                    params![&author_name],
                    |r| r.get(0),
                )
                .unwrap_or(new_id);
            name_to_author_id.insert(key, resolved_id.clone());
            resolved_id
        };

        tx.execute(
            "INSERT INTO author_assets(asset_id, author_id, studio_name, album_name, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![&asset_id, &author_id, &studio_name, album_name, ts],
        )
        .map_err(|e| e.to_string())?;

        active_authors.insert(author_id);
        item_count += 1;
    }

    Ok((active_authors.len() as i64, item_count))
}

fn rebuild_collections_index(
    tx: &rusqlite::Transaction,
    module_id: &str,
    root: &str,
    prefix: &str,
) -> Result<(i64, i64), String> {
    let ts = now();

    let assets: Vec<(String, String, String, i64, Option<String>)> = {
        let mut stmt = tx
            .prepare(
                "SELECT id, remote_path, media_type, size_bytes, thumbnail_asset_id
                 FROM assets
                 WHERE module_id = ?1 AND missing = 0
                 ORDER BY remote_path ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![module_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.flatten().collect()
    };

    struct AlbumAgg {
        id: String,
        channel: String,
        name: String,
        remote_path: String,
        is_direct: i64,
        cover_asset_id: Option<String>,
        cover_is_video: bool,
        item_count: i64,
        video_count: i64,
        image_count: i64,
        total_bytes: i64,
    }

    let mut albums: HashMap<(String, String), AlbumAgg> = HashMap::new();
    let mut items: Vec<(String, String, String, String)> = Vec::with_capacity(assets.len());

    for (asset_id, remote_path, media_type, size_bytes, thumb_id) in assets {
        let Some(rel) = remote_path.strip_prefix(prefix) else {
            continue;
        };
        let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() < 2 {
            continue;
        }
        let channel = parts[0].to_string();
        let (album_name, is_direct, album_path) = if parts.len() == 2 {
            ("__direct__".to_string(), 1i64, format!("{root}/{channel}"))
        } else {
            let sub = parts[1].to_string();
            (sub.clone(), 0i64, format!("{root}/{channel}/{sub}"))
        };

        let key = (channel.clone(), album_name.clone());
        let album_id = stable_id("col", &format!("{channel}\0{album_name}"));
        let agg = albums.entry(key).or_insert_with(|| AlbumAgg {
            id: album_id.clone(),
            channel: channel.clone(),
            name: album_name.clone(),
            remote_path: album_path,
            is_direct,
            cover_asset_id: None,
            cover_is_video: false,
            item_count: 0,
            video_count: 0,
            image_count: 0,
            total_bytes: 0,
        });

        agg.item_count += 1;
        agg.total_bytes += size_bytes;
        let is_video = media_type == "video";
        if is_video {
            agg.video_count += 1;
        } else if media_type == "image" {
            agg.image_count += 1;
        }

        // Prefer video asset (or asset with existing thumbnail) as cover.
        let candidate_cover = thumb_id.unwrap_or_else(|| asset_id.clone());
        if agg.cover_asset_id.is_none() || (!agg.cover_is_video && is_video) {
            agg.cover_asset_id = Some(candidate_cover);
            agg.cover_is_video = is_video;
        }

        items.push((asset_id, channel, agg.id.clone(), media_type));
    }

    tx.execute("DELETE FROM collection_items", [])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM collection_albums", [])
        .map_err(|e| e.to_string())?;

    let album_count = albums.len() as i64;
    for agg in albums.into_values() {
        tx.execute(
            "INSERT INTO collection_albums
             (id, channel, name, remote_path, is_direct, cover_asset_id, item_count, video_count, image_count, total_bytes, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                agg.id,
                agg.channel,
                agg.name,
                agg.remote_path,
                agg.is_direct,
                agg.cover_asset_id,
                agg.item_count,
                agg.video_count,
                agg.image_count,
                agg.total_bytes,
                ts
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    let item_count = items.len() as i64;
    for (asset_id, channel, album_id, media_type) in items {
        tx.execute(
            "INSERT INTO collection_items(asset_id, channel, album_id, media_type, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![asset_id, channel, album_id, media_type, ts],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok((album_count, item_count))
}

fn rebuild_galleries_index(
    tx: &rusqlite::Transaction,
    module_id: &str,
    root: &str,
    prefix: &str,
) -> Result<(i64, i64), String> {
    let ts = now();

    let assets: Vec<(String, String, String, i64)> = {
        let mut stmt = tx
            .prepare(
                "SELECT id, remote_path, media_type, size_bytes
                 FROM assets
                 WHERE module_id = ?1 AND missing = 0
                 ORDER BY remote_path ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![module_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        rows.flatten().collect()
    };

    struct SetAgg {
        id: String,
        model_name: String,
        album_name: String,
        remote_path: String,
        is_archive: i64,
        cover_asset_id: Option<String>,
        item_count: i64,
        total_bytes: i64,
    }

    let mut sets: HashMap<(String, String), SetAgg> = HashMap::new();
    let mut items: Vec<(String, String, i64)> = Vec::with_capacity(assets.len());

    for (asset_id, remote_path, media_type, size_bytes) in assets {
        let Some(rel) = remote_path.strip_prefix(prefix) else {
            continue;
        };
        let parts: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() < 2 {
            continue;
        }
        let raw_model = parts[0];
        let model_name = clean_bracket_name(raw_model);
        if model_name.is_empty() {
            continue;
        }

        let (album_name, is_archive, set_path) = if parts.len() == 2 {
            let file_name = parts[1];
            let archive = is_archive_name(file_name) || media_type == "comicArchive";
            let name = if archive {
                strip_extension(file_name)
            } else {
                "直属图集".to_string()
            };
            let path = if archive {
                remote_path.clone()
            } else {
                format!("{root}/{raw_model}")
            };
            (name, if archive { 1i64 } else { 0i64 }, path)
        } else {
            let sub = parts[1].to_string();
            (sub.clone(), 0i64, format!("{root}/{raw_model}/{sub}"))
        };

        let key = (model_name.clone(), album_name.clone());
        let set_id = stable_id("gal", &format!("{model_name}\0{album_name}"));
        let agg = sets.entry(key).or_insert_with(|| SetAgg {
            id: set_id.clone(),
            model_name: model_name.clone(),
            album_name: album_name.clone(),
            remote_path: set_path,
            is_archive,
            cover_asset_id: None,
            item_count: 0,
            total_bytes: 0,
        });

        let sort_order = agg.item_count;
        agg.item_count += 1;
        agg.total_bytes += size_bytes;
        if agg.cover_asset_id.is_none() {
            agg.cover_asset_id = Some(asset_id.clone());
        }
        items.push((asset_id, agg.id.clone(), sort_order));
    }

    tx.execute("DELETE FROM gallery_items", [])
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM gallery_sets", [])
        .map_err(|e| e.to_string())?;

    let set_count = sets.len() as i64;
    for agg in sets.into_values() {
        tx.execute(
            "INSERT INTO gallery_sets
             (id, model_name, album_name, remote_path, is_archive, cover_asset_id, item_count, total_bytes, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                agg.id,
                agg.model_name,
                agg.album_name,
                agg.remote_path,
                agg.is_archive,
                agg.cover_asset_id,
                agg.item_count,
                agg.total_bytes,
                ts
            ],
        )
        .map_err(|e| e.to_string())?;
    }

    let item_count = items.len() as i64;
    for (asset_id, set_id, sort_order) in items {
        tx.execute(
            "INSERT INTO gallery_items(asset_id, set_id, sort_order, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![asset_id, set_id, sort_order, ts],
        )
        .map_err(|e| e.to_string())?;
    }

    let _ = tx.execute(
        "UPDATE gallery_sets
         SET cover_asset_id = (SELECT o.cover_asset_id FROM gallery_cover_overrides o WHERE o.set_id = gallery_sets.id),
             cover_page_index = COALESCE((SELECT o.cover_page_index FROM gallery_cover_overrides o WHERE o.set_id = gallery_sets.id), 0)
         WHERE EXISTS (SELECT 1 FROM gallery_cover_overrides o WHERE o.set_id = gallery_sets.id)",
        [],
    );

    Ok((set_count, item_count))
}

/// Scan a single registered module's 115 directory and rebuild its index.
pub async fn scan_module(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    module_id: &str,
    full: bool,
) -> Result<ModuleScanResult, String> {
    four_domains::ensure_v2(&app)?;
    let (domain, remote_path, extra_paths_json): (String, String, String) = {
        let conn = four_domains::open_v2(&app)?;
        conn.query_row(
            "SELECT domain, remote_path, COALESCE(extra_paths_json, '[]') FROM library_modules WHERE id = ?1",
            params![module_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("未找到模块：{module_id}"))?
    };

    let all_paths = parse_module_paths(&remote_path, Some(&extra_paths_json));
    let mut scan = four_domains::ScanSummary {
        domain: domain.clone(),
        scan_id: String::new(),
        scanned: 0,
        added: 0,
        updated: 0,
        missing: 0,
        deleted: 0,
        conflicts: 0,
    };
    for (idx, p) in all_paths.iter().enumerate() {
        let folder_path = if domain == "comics" && idx == 0 {
            String::new()
        } else {
            p.trim_end_matches('/').to_string()
        };
        let s = four_domains::scan_provider_tree(
            app.clone(),
            state.clone(),
            &domain,
            &folder_path,
            true,
            !full,
        )
        .await?;
        scan.scan_id = s.scan_id;
        scan.scanned += s.scanned;
        scan.added += s.added;
        scan.updated += s.updated;
        scan.missing += s.missing;
        scan.deleted += s.deleted;
        scan.conflicts += s.conflicts;
    }

    let rebuild = {
        let mut conn = four_domains::open_v2(&app)?;
        let summary = rebuild_module_index_with_conn(&mut conn, module_id)?;
        if summary.kind == "comics" {
            let _ = crate::comic_library::rebuild_comic_library_with_conn(&mut conn);
        }
        conn.execute(
            "UPDATE library_modules SET last_scanned_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now(), module_id],
        )
        .map_err(|e| e.to_string())?;
        summary
    };

    Ok(ModuleScanResult {
        module_id: module_id.to_string(),
        scan,
        rebuild,
    })
}

// ============================================================================
// Phase 2.2: Creator V2 APIs (2-layer studio/author + suggested manual merge)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorStudioStat {
    pub studio: String,
    pub creator_count: i64,
    pub asset_count: i64,
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorSummaryV2 {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub avatar_asset_id: Option<String>,
    pub banner_asset_id: Option<String>,
    pub cover_asset_id: Option<String>,
    pub bio: Option<String>,
    pub social_links: HashMap<String, String>,
    pub studios: Vec<String>,
    pub studio_counts: HashMap<String, i64>,
    pub video_count: i64,
    pub image_count: i64,
    pub album_count: i64,
    pub media_count: i64,
    pub total_bytes: i64,
    pub latest_updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorAlbumSummary {
    pub album_name: String,
    pub studio_name: String,
    pub cover_asset_id: Option<String>,
    pub image_count: i64,
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorAssetV2 {
    pub id: String,
    pub root_id: String,
    pub root_kind: String,
    pub remote_path: String,
    pub file_name: String,
    pub parent_path: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub duration_seconds: Option<i64>,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
    pub category_name: Option<String>,
    pub thumbnail_asset_id: Option<String>,
    pub album_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorProfileV2 {
    pub creator: CreatorSummaryV2,
    pub albums: Vec<CreatorAlbumSummary>,
    pub items: Vec<CreatorAssetV2>,
    pub total_items: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeCandidateAuthor {
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub studios: Vec<String>,
    pub paths: Vec<String>,
    pub media_count: i64,
    pub total_bytes: i64,
    pub cover_asset_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorMergeSuggestion {
    pub author_a: MergeCandidateAuthor,
    pub author_b: MergeCandidateAuthor,
    pub reason: String,
    pub score: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyCreatorMergeResult {
    pub keep_author_id: String,
    pub keep_author_name: String,
    pub merged_author_id: String,
    pub merged_author_name: String,
    pub added_aliases: Vec<String>,
    pub remaining_old_assets: i64,
    pub keep_total_assets: i64,
    pub fully_merged: bool,
}

pub fn list_creator_studios(app: &AppHandle) -> Result<Vec<CreatorStudioStat>, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let mut stmt = conn
        .prepare(
            "SELECT aa.studio_name,
                    COUNT(DISTINCT aa.author_id) AS creator_count,
                    COUNT(*) AS asset_count,
                    COALESCE(SUM(a.size_bytes), 0) AS total_bytes
             FROM author_assets aa
             CROSS JOIN assets a ON a.id = aa.asset_id
             WHERE a.missing = 0
             GROUP BY aa.studio_name
             ORDER BY creator_count DESC, asset_count DESC, aa.studio_name ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CreatorStudioStat {
                studio: r.get(0)?,
                creator_count: r.get(1)?,
                asset_count: r.get(2)?,
                total_bytes: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

fn load_creators_summary_map(
    conn: &Connection,
    only_author_id: Option<&str>,
) -> Result<Vec<CreatorSummaryV2>, String> {
    // 1. Load author base metadata for active creators in author_assets.
    struct BaseAuthor {
        id: String,
        name: String,
        aliases: Vec<String>,
        avatar_asset_id: Option<String>,
        banner_asset_id: Option<String>,
        bio: Option<String>,
        social_links: HashMap<String, String>,
    }

    let mut base_map: HashMap<String, BaseAuthor> = HashMap::new();
    if let Some(aid) = only_author_id {
        if let Some(row) = conn
            .query_row(
                "SELECT id, name, aliases_json, avatar_asset_id, banner_asset_id, bio, social_links_json
                 FROM authors WHERE id = ?1",
                params![aid],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, String>(6)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            let (id, name, aliases_json, avatar, banner, bio, links_json) = row;
            base_map.insert(
                id.clone(),
                BaseAuthor {
                    id,
                    name,
                    aliases: serde_json::from_str(&aliases_json).unwrap_or_default(),
                    avatar_asset_id: avatar,
                    banner_asset_id: banner,
                    bio,
                    social_links: serde_json::from_str(&links_json).unwrap_or_default(),
                },
            );
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT au.id, au.name, au.aliases_json, au.avatar_asset_id, au.banner_asset_id, au.bio, au.social_links_json
                 FROM authors au
                 WHERE EXISTS (SELECT 1 FROM author_assets aa WHERE aa.author_id = au.id)",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for r in rows.flatten() {
            let (id, name, aliases_json, avatar, banner, bio, links_json) = r;
            base_map.insert(
                id.clone(),
                BaseAuthor {
                    id,
                    name,
                    aliases: serde_json::from_str(&aliases_json).unwrap_or_default(),
                    avatar_asset_id: avatar,
                    banner_asset_id: banner,
                    bio,
                    social_links: serde_json::from_str(&links_json).unwrap_or_default(),
                },
            );
        }
    }

    if base_map.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Single-pass aggregation over author_assets CROSS JOIN assets.
    struct Agg {
        studio_counts: HashMap<String, i64>,
        albums: HashSet<String>,
        video_count: i64,
        image_count: i64,
        media_count: i64,
        total_bytes: i64,
        latest_updated_at: i64,
        first_image_id: Option<String>,
        first_thumb_video_id: Option<String>,
        first_any_id: Option<String>,
    }

    let mut aggs: HashMap<String, Agg> = HashMap::with_capacity(base_map.len());
    let sql = if only_author_id.is_some() {
        "SELECT aa.author_id, aa.studio_name, aa.album_name, a.id, a.media_type, a.size_bytes, a.thumbnail_asset_id, a.last_seen_at
         FROM author_assets aa
         CROSS JOIN assets a ON a.id = aa.asset_id
         WHERE aa.author_id = ?1 AND a.missing = 0"
    } else {
        "SELECT aa.author_id, aa.studio_name, aa.album_name, a.id, a.media_type, a.size_bytes, a.thumbnail_asset_id, a.last_seen_at
         FROM author_assets aa
         CROSS JOIN assets a ON a.id = aa.asset_id
         WHERE a.missing = 0"
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let mut handle_row = |author_id: String,
                          studio: String,
                          album: Option<String>,
                          asset_id: String,
                          media_type: String,
                          size_bytes: i64,
                          thumb_id: Option<String>,
                          last_seen_at: i64| {
        let entry = aggs.entry(author_id).or_insert_with(|| Agg {
            studio_counts: HashMap::new(),
            albums: HashSet::new(),
            video_count: 0,
            image_count: 0,
            media_count: 0,
            total_bytes: 0,
            latest_updated_at: 0,
            first_image_id: None,
            first_thumb_video_id: None,
            first_any_id: None,
        });
        *entry.studio_counts.entry(studio).or_insert(0) += 1;
        if let Some(alb) = album {
            entry.albums.insert(alb);
        }
        entry.media_count += 1;
        entry.total_bytes += size_bytes;
        if last_seen_at > entry.latest_updated_at {
            entry.latest_updated_at = last_seen_at;
        }
        if media_type == "video" {
            entry.video_count += 1;
            if entry.first_thumb_video_id.is_none() && thumb_id.is_some() {
                entry.first_thumb_video_id = thumb_id;
            }
        } else if media_type == "image" {
            entry.image_count += 1;
            if entry.first_image_id.is_none() {
                entry.first_image_id = Some(asset_id.clone());
            }
        }
        if entry.first_any_id.is_none() {
            entry.first_any_id = Some(asset_id);
        }
    };

    if let Some(aid) = only_author_id {
        let rows = stmt
            .query_map(params![aid], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for r in rows.flatten() {
            handle_row(r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7);
        }
    } else {
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for r in rows.flatten() {
            handle_row(r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7);
        }
    }

    let mut out = Vec::with_capacity(base_map.len());
    for (id, base) in base_map {
        let Some(agg) = aggs.remove(&id) else {
            if only_author_id.is_none() {
                continue;
            }
            out.push(CreatorSummaryV2 {
                id: base.id,
                name: base.name,
                aliases: base.aliases,
                cover_asset_id: base.avatar_asset_id.clone(),
                avatar_asset_id: base.avatar_asset_id,
                banner_asset_id: base.banner_asset_id,
                bio: base.bio,
                social_links: base.social_links,
                studios: Vec::new(),
                studio_counts: HashMap::new(),
                video_count: 0,
                image_count: 0,
                album_count: 0,
                media_count: 0,
                total_bytes: 0,
                latest_updated_at: 0,
            });
            continue;
        };

        let mut studios: Vec<(String, i64)> = agg
            .studio_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        studios.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let studio_names: Vec<String> = studios.into_iter().map(|(k, _)| k).collect();

        // Smart cover priority: explicit avatar -> first image -> video with thumb -> first asset
        let cover_asset_id = base
            .avatar_asset_id
            .clone()
            .or(agg.first_image_id)
            .or(agg.first_thumb_video_id)
            .or(agg.first_any_id);

        out.push(CreatorSummaryV2 {
            id: base.id,
            name: base.name,
            aliases: base.aliases,
            avatar_asset_id: base.avatar_asset_id,
            banner_asset_id: base.banner_asset_id,
            cover_asset_id,
            bio: base.bio,
            social_links: base.social_links,
            studios: studio_names,
            studio_counts: agg.studio_counts,
            video_count: agg.video_count,
            image_count: agg.image_count,
            album_count: agg.albums.len() as i64,
            media_count: agg.media_count,
            total_bytes: agg.total_bytes,
            latest_updated_at: agg.latest_updated_at,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EntityFavoriteIds {
    pub movies: Vec<String>,
    pub performers: Vec<String>,
    pub creators: Vec<String>,
    pub comics: Vec<String>,
}

pub(crate) fn ensure_entity_favorites_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entity_favorites (
            entity_type TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(entity_type, entity_id)
        );
        CREATE INDEX IF NOT EXISTS idx_entity_favorites_type_time
            ON entity_favorites(entity_type, created_at DESC);",
    )
}

pub fn toggle_entity_favorite(
    app: &AppHandle,
    entity_type: &str,
    entity_id: &str,
) -> Result<bool, String> {
    let kind = entity_type.trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "movie" | "performer" | "creator" | "comic") {
        return Err(format!("不支持的收藏类型：{entity_type}"));
    }
    let eid = entity_id.trim();
    if eid.is_empty() {
        return Err("缺少 entityId".into());
    }
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_entity_favorites_table(&conn).map_err(|e| e.to_string())?;
    let existing: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM entity_favorites WHERE entity_type=?1 AND entity_id=?2",
            params![kind, eid],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if existing.is_some() {
        conn.execute(
            "DELETE FROM entity_favorites WHERE entity_type=?1 AND entity_id=?2",
            params![kind, eid],
        )
        .map_err(|e| e.to_string())?;
        Ok(false)
    } else {
        conn.execute(
            "INSERT OR REPLACE INTO entity_favorites(entity_type, entity_id, created_at) VALUES(?1, ?2, ?3)",
            params![kind, eid, now()],
        )
        .map_err(|e| e.to_string())?;
        Ok(true)
    }
}

pub fn list_entity_favorite_ids(app: &AppHandle) -> Result<EntityFavoriteIds, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    ensure_entity_favorites_table(&conn).map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT entity_type, entity_id FROM entity_favorites ORDER BY created_at DESC, entity_id ASC")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut result = EntityFavoriteIds::default();
    for row in rows.flatten() {
        match row.0.as_str() {
            "movie" => result.movies.push(row.1),
            "performer" => result.performers.push(row.1),
            "creator" => result.creators.push(row.1),
            "comic" => result.comics.push(row.1),
            _ => {}
        }
    }
    Ok(result)
}

pub fn list_creators_v2(
    app: &AppHandle,
    studio: Option<&str>,
    query: Option<&str>,
    sort: Option<&str>,
    only_favorites: bool,
) -> Result<Vec<CreatorSummaryV2>, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let mut creators = load_creators_summary_map(&conn, None)?;

    if only_favorites {
        ensure_entity_favorites_table(&conn).map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT entity_id FROM entity_favorites WHERE entity_type='creator'")
            .map_err(|e| e.to_string())?;
        let fav_ids: HashSet<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        creators.retain(|c| fav_ids.contains(&c.id));
    }

    if let Some(st) = studio.map(str::trim).filter(|s| !s.is_empty() && *s != "all") {
        creators.retain(|c| c.studio_counts.contains_key(st));
    }

    if let Some(q) = query.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        creators.retain(|c| {
            c.name.to_lowercase().contains(&q)
                || c.aliases.iter().any(|a| a.to_lowercase().contains(&q))
                || c.studios.iter().any(|s| s.to_lowercase().contains(&q))
        });
    }

    match sort.unwrap_or("mediaCount") {
        "name" => creators.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        "latest" => creators.sort_by(|a, b| {
            b.latest_updated_at
                .cmp(&a.latest_updated_at)
                .then_with(|| b.media_count.cmp(&a.media_count))
        }),
        "size" => creators.sort_by(|a, b| {
            b.total_bytes
                .cmp(&a.total_bytes)
                .then_with(|| b.media_count.cmp(&a.media_count))
        }),
        _ => creators.sort_by(|a, b| {
            b.video_count
                .cmp(&a.video_count)
                .then_with(|| b.media_count.cmp(&a.media_count))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }),
    }

    Ok(creators)
}

pub fn get_creator_profile_v2(
    app: &AppHandle,
    author_id: &str,
    studio: Option<&str>,
    media_type: Option<&str>,
    album_name: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<CreatorProfileV2, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let creator = load_creators_summary_map(&conn, Some(author_id))?
        .into_iter()
        .next()
        .ok_or_else(|| format!("未找到创作者：{author_id}"))?;

    let studio_filter = studio.map(str::trim).filter(|s| !s.is_empty() && *s != "all");
    let type_filter = media_type
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all");
    let album_filter = album_name.map(str::trim).filter(|s| !s.is_empty());

    // Albums summary for this creator (optionally filtered by studio).
    let mut albums = Vec::new();
    {
        let sql = if studio_filter.is_some() {
            "SELECT aa.album_name, aa.studio_name, MIN(a.id), COUNT(*), COALESCE(SUM(a.size_bytes), 0)
             FROM author_assets aa
             CROSS JOIN assets a ON a.id = aa.asset_id
             WHERE aa.author_id = ?1 AND aa.album_name IS NOT NULL AND a.missing = 0 AND aa.studio_name = ?2
             GROUP BY aa.studio_name, aa.album_name
             ORDER BY COUNT(*) DESC, aa.album_name ASC"
        } else {
            "SELECT aa.album_name, aa.studio_name, MIN(a.id), COUNT(*), COALESCE(SUM(a.size_bytes), 0)
             FROM author_assets aa
             CROSS JOIN assets a ON a.id = aa.asset_id
             WHERE aa.author_id = ?1 AND aa.album_name IS NOT NULL AND a.missing = 0
             GROUP BY aa.studio_name, aa.album_name
             ORDER BY COUNT(*) DESC, aa.album_name ASC"
        };
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        if let Some(st) = studio_filter {
            let rows = stmt
                .query_map(params![author_id, st], |r| {
                    Ok(CreatorAlbumSummary {
                        album_name: r.get(0)?,
                        studio_name: r.get(1)?,
                        cover_asset_id: r.get(2)?,
                        image_count: r.get(3)?,
                        total_bytes: r.get(4)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            for r in rows.flatten() {
                albums.push(r);
            }
        } else {
            let rows = stmt
                .query_map(params![author_id], |r| {
                    Ok(CreatorAlbumSummary {
                        album_name: r.get(0)?,
                        studio_name: r.get(1)?,
                        cover_asset_id: r.get(2)?,
                        image_count: r.get(3)?,
                        total_bytes: r.get(4)?,
                    })
                })
                .map_err(|e| e.to_string())?;
            for r in rows.flatten() {
                albums.push(r);
            }
        }
    }

    // Filtered items query
    let mut where_clauses = vec!["aa.author_id = ?".to_string(), "a.missing = 0".to_string()];
    let mut bind_values: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(author_id.to_string())];

    if let Some(st) = studio_filter {
        where_clauses.push("aa.studio_name = ?".to_string());
        bind_values.push(Box::new(st.to_string()));
    }
    if let Some(mt) = type_filter {
        where_clauses.push("a.media_type = ?".to_string());
        bind_values.push(Box::new(mt.to_string()));
    }
    if let Some(alb) = album_filter {
        where_clauses.push("aa.album_name = ?".to_string());
        bind_values.push(Box::new(alb.to_string()));
    }

    let where_sql = where_clauses.join(" AND ");
    let count_sql = format!(
        "SELECT COUNT(*) FROM author_assets aa CROSS JOIN assets a ON a.id = aa.asset_id WHERE {where_sql}"
    );
    let total_items: i64 = {
        let refs: Vec<&dyn rusqlite::ToSql> = bind_values.iter().map(|b| b.as_ref()).collect();
        conn.query_row(&count_sql, refs.as_slice(), |r| r.get(0))
            .map_err(|e| e.to_string())?
    };

    let lim = limit.clamp(1, 500);
    let off = offset.max(0);
    bind_values.push(Box::new(lim));
    bind_values.push(Box::new(off));
    let refs: Vec<&dyn rusqlite::ToSql> = bind_values.iter().map(|b| b.as_ref()).collect();

    let items_sql = format!(
        "SELECT a.id, a.root_id, a.remote_path, a.file_name, a.parent_path,
                a.media_type, a.size_bytes, a.duration_seconds, a.thumbnail_asset_id,
                aa.studio_name, aa.album_name
         FROM author_assets aa
         CROSS JOIN assets a ON a.id = aa.asset_id
         WHERE {where_sql}
         ORDER BY CASE WHEN a.media_type = 'video' THEN 0 ELSE 1 END ASC, a.file_name ASC
         LIMIT ? OFFSET ?"
    );

    let mut stmt = conn.prepare(&items_sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(refs.as_slice(), |r| {
            Ok(CreatorAssetV2 {
                id: r.get(0)?,
                root_id: r.get(1)?,
                root_kind: "media".to_string(),
                remote_path: r.get(2)?,
                file_name: r.get(3)?,
                parent_path: r.get(4)?,
                media_type: r.get(5)?,
                size_bytes: r.get(6)?,
                duration_seconds: r.get(7)?,
                author_id: Some(creator.id.clone()),
                author_name: Some(creator.name.clone()),
                category_name: Some(r.get::<_, String>(9)?),
                thumbnail_asset_id: r.get(8)?,
                album_name: r.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    for r in rows {
        items.push(r.map_err(|e| e.to_string())?);
    }

    Ok(CreatorProfileV2 {
        creator,
        albums,
        items,
        total_items,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCreatorProfileRequest {
    pub author_id: String,
    pub aliases: Option<Vec<String>>,
    pub bio: Option<String>,
    pub social_links: Option<HashMap<String, String>>,
}

pub fn update_creator_profile(
    app: &AppHandle,
    req: UpdateCreatorProfileRequest,
) -> Result<CreatorSummaryV2, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    if let Some(aliases) = req.aliases {
        let mut clean: Vec<String> = Vec::new();
        for a in aliases {
            let trimmed = a.trim().to_string();
            if !trimmed.is_empty() && !clean.iter().any(|x| x.eq_ignore_ascii_case(&trimmed)) {
                clean.push(trimmed);
            }
        }
        let json = serde_json::to_string(&clean).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE authors SET aliases_json = ?1 WHERE id = ?2",
            params![json, &req.author_id],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(bio) = req.bio {
        let trimmed = bio.trim().to_string();
        let val = if trimmed.is_empty() { None } else { Some(trimmed) };
        conn.execute(
            "UPDATE authors SET bio = ?1 WHERE id = ?2",
            params![val, &req.author_id],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(links) = req.social_links {
        let cleaned: HashMap<String, String> = links
            .into_iter()
            .filter_map(|(k, v)| {
                let kk = k.trim().to_string();
                let vv = v.trim().to_string();
                if kk.is_empty() || vv.is_empty() {
                    None
                } else {
                    Some((kk, vv))
                }
            })
            .collect();
        let json = serde_json::to_string(&cleaned).map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE authors SET social_links_json = ?1 WHERE id = ?2",
            params![json, &req.author_id],
        )
        .map_err(|e| e.to_string())?;
    }

    load_creators_summary_map(&conn, Some(&req.author_id))?
        .into_iter()
        .next()
        .ok_or_else(|| "更新后未找到创作者".into())
}

fn normalize_merge_token(name: &str) -> String {
    let cleaned = clean_bracket_name(name).to_lowercase();
    cleaned
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '_' | '-' | '.' | '·' | '/' | '\\' | '（' | '）' | '(' | ')' | '@' | '#'))
        .collect()
}

fn split_name_tokens(name: &str) -> Vec<String> {
    let cleaned = clean_bracket_name(name);
    let mut out = Vec::new();
    for part in cleaned.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '（' | '）' | '(' | ')' | '/' | '／' | '_' | '-' | '|' | '&' | '+' | '·' | ',' | '，'
            )
    }) {
        let t = part.trim().to_lowercase();
        if t.chars().count() >= 2 {
            out.push(t);
        }
    }
    out
}

pub fn list_creator_merge_suggestions(
    app: &AppHandle,
) -> Result<Vec<CreatorMergeSuggestion>, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;

    let creators = load_creators_summary_map(&conn, None)?;
    if creators.len() < 2 {
        return Ok(Vec::new());
    }

    // Load ignored pairs.
    let mut ignored: HashSet<(String, String)> = HashSet::new();
    {
        let mut stmt = conn
            .prepare("SELECT author_id_a, author_id_b FROM author_merge_ignores")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for r in rows.flatten() {
            ignored.insert(r);
        }
    }

    // Map each author_id -> distinct 115 folder paths (up to 2nd level under creators root).
    let creators_root: String = conn
        .query_row(
            "SELECT remote_path FROM library_modules WHERE id = 'creators'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "/115open/文件/创作者".to_string());
    let root_clean = creators_root.trim_end_matches('/');
    let prefix = format!("{root_clean}/");

    let mut author_paths: HashMap<String, HashSet<String>> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT aa.author_id, a.remote_path
                 FROM author_assets aa
                 CROSS JOIN assets a ON a.id = aa.asset_id
                 WHERE a.missing = 0",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?;
        for (aid, rpath) in rows.flatten() {
            if let Some(rel) = rpath.strip_prefix(&prefix) {
                let mut segs = rel.split('/').filter(|s| !s.is_empty());
                if let (Some(s0), Some(s1)) = (segs.next(), segs.next()) {
                    author_paths
                        .entry(aid)
                        .or_default()
                        .insert(format!("{root_clean}/{s0}/{s1}"));
                }
            }
        }
    }

    let norms: Vec<String> = creators.iter().map(|c| normalize_merge_token(&c.name)).collect();
    let tokens: Vec<Vec<String>> = creators.iter().map(|c| split_name_tokens(&c.name)).collect();

    let mut suggestions = Vec::new();
    for i in 0..creators.len() {
        for j in (i + 1)..creators.len() {
            let a = &creators[i];
            let b = &creators[j];
            let pair_key = if a.id < b.id {
                (a.id.clone(), b.id.clone())
            } else {
                (b.id.clone(), a.id.clone())
            };
            if ignored.contains(&pair_key) {
                continue;
            }

            let na = &norms[i];
            let nb = &norms[j];
            if na.is_empty() || nb.is_empty() {
                continue;
            }

            let mut matched: Option<(i64, String)> = None;

            if na == nb {
                matched = Some((100, "去除空格/符号后名称完全一致".to_string()));
            } else {
                // Check alias cross-match
                let a_alias_hit = a.aliases.iter().any(|al| normalize_merge_token(al) == *nb);
                let b_alias_hit = b.aliases.iter().any(|al| normalize_merge_token(al) == *na);
                if a_alias_hit || b_alias_hit {
                    matched = Some((95, "命中已有别名记录".to_string()));
                } else {
                    // Check token exact overlap (e.g. "艾熙" vs "艾熙 Ashley", "花音栗子" vs "花音栗子（金鱼）")
                    let shared = tokens[i]
                        .iter()
                        .find(|t| tokens[j].iter().any(|u| u == *t));
                    if let Some(tok) = shared {
                        if tok.chars().count() >= 2 {
                            matched = Some((85, format!("包含相同名称片段「{tok}」")));
                        }
                    } else {
                        // Prefix / containment check for names >= 2 CJK chars or >= 4 ASCII chars
                        let (shorter, longer) = if na.len() <= nb.len() {
                            (na, nb)
                        } else {
                            (nb, na)
                        };
                        let char_len = shorter.chars().count();
                        let has_cjk = shorter.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c));
                        if ((has_cjk && char_len >= 2) || (!has_cjk && char_len >= 4))
                            && longer.contains(shorter.as_str())
                        {
                            matched = Some((75, format!("名称包含关系（「{shorter}」）")));
                        }
                    }
                }
            }

            if let Some((score, reason)) = matched {
                let to_candidate = |c: &CreatorSummaryV2| {
                    let mut paths: Vec<String> = author_paths
                        .get(&c.id)
                        .map(|s| s.iter().cloned().collect())
                        .unwrap_or_default();
                    paths.sort();
                    MergeCandidateAuthor {
                        id: c.id.clone(),
                        name: c.name.clone(),
                        aliases: c.aliases.clone(),
                        studios: c.studios.clone(),
                        paths,
                        media_count: c.media_count,
                        total_bytes: c.total_bytes,
                        cover_asset_id: c.cover_asset_id.clone(),
                    }
                };
                // Put the one with more assets as author_a (suggested primary).
                let (ca, cb) = if a.media_count >= b.media_count {
                    (to_candidate(a), to_candidate(b))
                } else {
                    (to_candidate(b), to_candidate(a))
                };
                suggestions.push(CreatorMergeSuggestion {
                    author_a: ca,
                    author_b: cb,
                    reason,
                    score,
                });
            }
        }
    }

    suggestions.sort_by(|x, y| {
        y.score
            .cmp(&x.score)
            .then_with(|| (y.author_a.media_count + y.author_b.media_count).cmp(&(x.author_a.media_count + x.author_b.media_count)))
    });
    suggestions.truncate(80);
    Ok(suggestions)
}

pub fn ignore_creator_merge_pair(
    app: &AppHandle,
    author_id_a: &str,
    author_id_b: &str,
) -> Result<(), String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let (a, b) = if author_id_a < author_id_b {
        (author_id_a, author_id_b)
    } else {
        (author_id_b, author_id_a)
    };
    conn.execute(
        "INSERT OR IGNORE INTO author_merge_ignores(author_id_a, author_id_b, created_at)
         VALUES (?1, ?2, ?3)",
        params![a, b, now()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// After the user manually merges two creator folders on 115, run an incremental
/// scan of the creators module, preserve the old name as an alias on the kept
/// author if names differed, and verify whether the old folder is now gone.
pub async fn verify_creator_merge(
    app: AppHandle,
    state: State<'_, media_library::MediaLibraryState>,
    keep_author_id: &str,
    merged_author_id: &str,
    keep_alias: bool,
) -> Result<VerifyCreatorMergeResult, String> {
    if keep_author_id == merged_author_id {
        return Err("主作者与被合并作者不能相同".into());
    }
    four_domains::ensure_v2(&app)?;

    let (keep_name, mut keep_aliases, merged_name, merged_aliases): (
        String,
        Vec<String>,
        String,
        Vec<String>,
    ) = {
        let conn = four_domains::open_v2(&app)?;
        let (kn, ka_json): (String, String) = conn
            .query_row(
                "SELECT name, aliases_json FROM authors WHERE id = ?1",
                params![keep_author_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("未找到主作者：{keep_author_id}"))?;
        let (mn, ma_json): (String, String) = conn
            .query_row(
                "SELECT name, aliases_json FROM authors WHERE id = ?1",
                params![merged_author_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("未找到被合并作者：{merged_author_id}"))?;
        (
            kn,
            serde_json::from_str(&ka_json).unwrap_or_default(),
            mn,
            serde_json::from_str(&ma_json).unwrap_or_default(),
        )
    };

    // 1. Preserve old name & aliases onto kept author BEFORE rescanning so any
    //    remaining files or future incoming files automatically resolve to keep_author_id.
    let mut added_aliases = Vec::new();
    if keep_alias {
        let mut candidates = vec![merged_name.clone()];
        candidates.extend(merged_aliases);
        for cand in candidates {
            let trimmed = clean_bracket_name(&cand);
            if !trimmed.is_empty()
                && !trimmed.eq_ignore_ascii_case(&keep_name)
                && !keep_aliases.iter().any(|x| x.eq_ignore_ascii_case(&trimmed))
            {
                keep_aliases.push(trimmed.clone());
                added_aliases.push(trimmed);
            }
        }
        if !added_aliases.is_empty() {
            let conn = four_domains::open_v2(&app)?;
            let json = serde_json::to_string(&keep_aliases).map_err(|e| e.to_string())?;
            conn.execute(
                "UPDATE authors SET aliases_json = ?1 WHERE id = ?2",
                params![json, keep_author_id],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // 2. Incrementally scan the creators module on 115 and rebuild author_assets.
    let _ = scan_module(app.clone(), state, "creators", false).await?;

    // 3. Check remaining assets under merged_author_id and total under keep_author_id.
    let (remaining_old_assets, keep_total_assets): (i64, i64) = {
        let conn = four_domains::open_v2(&app)?;
        let old_cnt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM author_assets aa CROSS JOIN assets a ON a.id = aa.asset_id
                 WHERE aa.author_id = ?1 AND a.missing = 0",
                params![merged_author_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let keep_cnt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM author_assets aa CROSS JOIN assets a ON a.id = aa.asset_id
                 WHERE aa.author_id = ?1 AND a.missing = 0",
                params![keep_author_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        (old_cnt, keep_cnt)
    };

    Ok(VerifyCreatorMergeResult {
        keep_author_id: keep_author_id.to_string(),
        keep_author_name: keep_name,
        merged_author_id: merged_author_id.to_string(),
        merged_author_name: merged_name,
        added_aliases,
        remaining_old_assets,
        keep_total_assets,
        fully_merged: remaining_old_assets == 0,
    })
}

// ============================================================================
// Phase 2.3: Collections Module V2 APIs (Channels + Album Cards + Flat Video Stream)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionChannelStat {
    pub channel: String,
    pub album_count: i64,
    pub item_count: i64,
    pub video_count: i64,
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionAlbumV2 {
    pub id: String,
    pub channel: String,
    pub name: String,
    pub remote_path: String,
    pub is_direct: bool,
    pub cover_asset_id: Option<String>,
    pub preview_asset_ids: Vec<String>,
    pub item_count: i64,
    pub video_count: i64,
    pub image_count: i64,
    pub total_bytes: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionAlbumsPage {
    pub albums: Vec<CollectionAlbumV2>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionItemsPage {
    pub items: Vec<CreatorAssetV2>,
    pub total: i64,
}

pub fn list_collection_channels(app: &AppHandle) -> Result<Vec<CollectionChannelStat>, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let mut stmt = conn
        .prepare(
            "SELECT channel,
                    COUNT(*) AS album_count,
                    COALESCE(SUM(item_count), 0) AS item_count,
                    COALESCE(SUM(video_count), 0) AS video_count,
                    COALESCE(SUM(total_bytes), 0) AS total_bytes
             FROM collection_albums
             WHERE item_count > 0
             GROUP BY channel
             ORDER BY total_bytes DESC, item_count DESC, channel ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CollectionChannelStat {
                channel: r.get(0)?,
                album_count: r.get(1)?,
                item_count: r.get(2)?,
                video_count: r.get(3)?,
                total_bytes: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn list_collection_albums(
    app: &AppHandle,
    channel: Option<&str>,
    query: Option<&str>,
    sort: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<CollectionAlbumsPage, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;

    let mut where_clauses = vec!["item_count > 0".to_string()];
    let mut bind_vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(ch) = channel.map(str::trim).filter(|s| !s.is_empty() && *s != "all") {
        where_clauses.push("channel = ?".to_string());
        bind_vals.push(Box::new(ch.to_string()));
    }
    if let Some(q) = query.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        where_clauses.push("(LOWER(name) LIKE ? OR LOWER(channel) LIKE ?)".to_string());
        let pat = format!("%{q}%");
        bind_vals.push(Box::new(pat.clone()));
        bind_vals.push(Box::new(pat));
    }

    let where_sql = where_clauses.join(" AND ");
    let count_sql = format!("SELECT COUNT(*) FROM collection_albums WHERE {where_sql}");
    let total: i64 = {
        let refs: Vec<&dyn rusqlite::ToSql> = bind_vals.iter().map(|b| b.as_ref()).collect();
        conn.query_row(&count_sql, refs.as_slice(), |r| r.get(0))
            .map_err(|e| e.to_string())?
    };

    let order_sql = match sort.unwrap_or("size") {
        "count" => "item_count DESC, total_bytes DESC, name ASC",
        "name" => "name COLLATE NOCASE ASC",
        "latest" => "updated_at DESC, total_bytes DESC",
        _ => "total_bytes DESC, item_count DESC, name ASC",
    };

    let lim = limit.clamp(1, 200);
    let off = offset.max(0);
    bind_vals.push(Box::new(lim));
    bind_vals.push(Box::new(off));
    let refs: Vec<&dyn rusqlite::ToSql> = bind_vals.iter().map(|b| b.as_ref()).collect();

    let sql = format!(
        "SELECT id, channel, name, remote_path, is_direct, cover_asset_id,
                item_count, video_count, image_count, total_bytes, updated_at
         FROM collection_albums
         WHERE {where_sql}
         ORDER BY {order_sql}
         LIMIT ? OFFSET ?"
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(refs.as_slice(), |r| {
            Ok(CollectionAlbumV2 {
                id: r.get(0)?,
                channel: r.get(1)?,
                name: r.get(2)?,
                remote_path: r.get(3)?,
                is_direct: r.get::<_, i64>(4)? != 0,
                cover_asset_id: r.get(5)?,
                preview_asset_ids: Vec::new(),
                item_count: r.get(6)?,
                video_count: r.get(7)?,
                image_count: r.get(8)?,
                total_bytes: r.get(9)?,
                updated_at: r.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut albums = Vec::new();
    for r in rows {
        albums.push(r.map_err(|e| e.to_string())?);
    }

    // Fetch up to 4 preview thumbnails per album using indexed lookup on collection_items(album_id).
    let mut prev_stmt = conn
        .prepare(
            "SELECT COALESCE(a.thumbnail_asset_id, a.id)
             FROM collection_items ci
             CROSS JOIN assets a ON a.id = ci.asset_id
             WHERE ci.album_id = ?1 AND a.missing = 0
             ORDER BY CASE WHEN a.media_type = 'video' THEN 0 ELSE 1 END ASC, a.id ASC
             LIMIT 4",
        )
        .map_err(|e| e.to_string())?;

    for alb in &mut albums {
        let ids: Vec<String> = prev_stmt
            .query_map(params![&alb.id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        alb.preview_asset_ids = ids;
    }

    Ok(CollectionAlbumsPage { albums, total })
}

pub fn list_collection_items(
    app: &AppHandle,
    channel: Option<&str>,
    album_id: Option<&str>,
    media_type: Option<&str>,
    query: Option<&str>,
    sort: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<CollectionItemsPage, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;

    let mut where_clauses = vec!["a.missing = 0".to_string()];
    let mut bind_vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(aid) = album_id.map(str::trim).filter(|s| !s.is_empty()) {
        where_clauses.push("ci.album_id = ?".to_string());
        bind_vals.push(Box::new(aid.to_string()));
    } else if let Some(ch) = channel.map(str::trim).filter(|s| !s.is_empty() && *s != "all") {
        where_clauses.push("ci.channel = ?".to_string());
        bind_vals.push(Box::new(ch.to_string()));
    }

    if let Some(mt) = media_type
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
    {
        where_clauses.push("ci.media_type = ?".to_string());
        bind_vals.push(Box::new(mt.to_string()));
    }

    if let Some(q) = query.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        where_clauses.push("(LOWER(a.file_name) LIKE ? OR LOWER(a.parent_path) LIKE ?)".to_string());
        let pat = format!("%{q}%");
        bind_vals.push(Box::new(pat.clone()));
        bind_vals.push(Box::new(pat));
    }

    let where_sql = where_clauses.join(" AND ");
    let count_sql = format!(
        "SELECT COUNT(*) FROM collection_items ci CROSS JOIN assets a ON a.id = ci.asset_id WHERE {where_sql}"
    );
    let total: i64 = {
        let refs: Vec<&dyn rusqlite::ToSql> = bind_vals.iter().map(|b| b.as_ref()).collect();
        conn.query_row(&count_sql, refs.as_slice(), |r| r.get(0))
            .map_err(|e| e.to_string())?
    };

    let order_sql = match sort.unwrap_or("name") {
        "size" => "a.size_bytes DESC, a.file_name ASC",
        "latest" => "a.last_seen_at DESC, a.file_name ASC",
        _ => "CASE WHEN a.media_type = 'video' THEN 0 ELSE 1 END ASC, a.file_name ASC",
    };

    let lim = limit.clamp(1, 200);
    let off = offset.max(0);
    bind_vals.push(Box::new(lim));
    bind_vals.push(Box::new(off));
    let refs: Vec<&dyn rusqlite::ToSql> = bind_vals.iter().map(|b| b.as_ref()).collect();

    let sql = format!(
        "SELECT a.id, a.root_id, a.remote_path, a.file_name, a.parent_path,
                a.media_type, a.size_bytes, a.duration_seconds, a.thumbnail_asset_id,
                ci.channel
         FROM collection_items ci
         CROSS JOIN assets a ON a.id = ci.asset_id
         WHERE {where_sql}
         ORDER BY {order_sql}
         LIMIT ? OFFSET ?"
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(refs.as_slice(), |r| {
            Ok(CreatorAssetV2 {
                id: r.get(0)?,
                root_id: r.get(1)?,
                root_kind: "media".to_string(),
                remote_path: r.get(2)?,
                file_name: r.get(3)?,
                parent_path: r.get(4)?,
                media_type: r.get(5)?,
                size_bytes: r.get(6)?,
                duration_seconds: r.get(7)?,
                author_id: None,
                author_name: None,
                category_name: Some(r.get::<_, String>(9)?),
                thumbnail_asset_id: r.get(8)?,
                album_name: None,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut items = Vec::new();
    for r in rows {
        items.push(r.map_err(|e| e.to_string())?);
    }

    Ok(CollectionItemsPage { items, total })
}

// ============================================================================
// Phase 2.4: Galleries Module V2 APIs (Models + Photo Sets / Archive Sets)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryModelStat {
    pub model_name: String,
    pub set_count: i64,
    pub item_count: i64,
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GallerySetV2 {
    pub id: String,
    pub model_name: String,
    pub album_name: String,
    pub remote_path: String,
    pub is_archive: bool,
    pub cover_asset_id: Option<String>,
    pub cover_page_index: i64,
    pub item_count: i64,
    pub total_bytes: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GallerySetsPage {
    pub sets: Vec<GallerySetV2>,
    pub total: i64,
}

pub fn list_gallery_models(app: &AppHandle) -> Result<Vec<GalleryModelStat>, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let mut stmt = conn
        .prepare(
            "SELECT model_name,
                    COUNT(*) AS set_count,
                    COALESCE(SUM(item_count), 0) AS item_count,
                    COALESCE(SUM(total_bytes), 0) AS total_bytes
             FROM gallery_sets
             WHERE item_count > 0
             GROUP BY model_name
             ORDER BY set_count DESC, total_bytes DESC, model_name ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(GalleryModelStat {
                model_name: r.get(0)?,
                set_count: r.get(1)?,
                item_count: r.get(2)?,
                total_bytes: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

pub fn list_gallery_sets(
    app: &AppHandle,
    model_name: Option<&str>,
    query: Option<&str>,
    sort: Option<&str>,
    kind_filter: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<GallerySetsPage, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;

    let mut where_clauses = vec!["g.item_count > 0".to_string()];
    let mut bind_vals: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(m) = model_name
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
    {
        where_clauses.push("g.model_name = ?".to_string());
        bind_vals.push(Box::new(m.to_string()));
    }
    if let Some(kf) = kind_filter.map(str::trim) {
        match kf {
            "archive" => where_clauses.push("g.is_archive = 1".to_string()),
            "folder" => where_clauses.push("g.is_archive = 0".to_string()),
            _ => {}
        }
    }
    if let Some(q) = query.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        where_clauses.push("(LOWER(g.album_name) LIKE ? OR LOWER(g.model_name) LIKE ?)".to_string());
        let pat = format!("%{q}%");
        bind_vals.push(Box::new(pat.clone()));
        bind_vals.push(Box::new(pat));
    }

    let where_sql = where_clauses.join(" AND ");
    let count_sql = format!("SELECT COUNT(*) FROM gallery_sets g WHERE {where_sql}");
    let total: i64 = {
        let refs: Vec<&dyn rusqlite::ToSql> = bind_vals.iter().map(|b| b.as_ref()).collect();
        conn.query_row(&count_sql, refs.as_slice(), |r| r.get(0))
            .map_err(|e| e.to_string())?
    };

    let order_sql = match sort.unwrap_or("latest") {
        "name" => "g.model_name COLLATE NOCASE ASC, g.album_name COLLATE NOCASE ASC",
        "size" => "g.total_bytes DESC, g.album_name COLLATE NOCASE ASC",
        "count" => "effective_count DESC, g.total_bytes DESC",
        "random" => "RANDOM()",
        _ => "g.updated_at DESC, g.rowid DESC",
    };

    let lim = limit.clamp(1, 200);
    let off = offset.max(0);
    bind_vals.push(Box::new(lim));
    bind_vals.push(Box::new(off));
    let refs: Vec<&dyn rusqlite::ToSql> = bind_vals.iter().map(|b| b.as_ref()).collect();

    let sql = format!(
        "SELECT g.id, g.model_name, g.album_name, g.remote_path, g.is_archive, g.cover_asset_id,
                COALESCE(g.cover_page_index, 0),
                CASE WHEN g.is_archive = 1 AND COALESCE(ca.page_count, 0) > g.item_count THEN ca.page_count ELSE g.item_count END AS effective_count,
                g.total_bytes, g.updated_at
         FROM gallery_sets g
         LEFT JOIN comic_archives ca ON ca.asset_id = g.cover_asset_id
         WHERE {where_sql}
         ORDER BY {order_sql}
         LIMIT ? OFFSET ?"
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(refs.as_slice(), |r| {
            Ok(GallerySetV2 {
                id: r.get(0)?,
                model_name: r.get(1)?,
                album_name: r.get(2)?,
                remote_path: r.get(3)?,
                is_archive: r.get::<_, i64>(4)? != 0,
                cover_asset_id: r.get(5)?,
                cover_page_index: r.get(6)?,
                item_count: r.get(7)?,
                total_bytes: r.get(8)?,
                updated_at: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut sets = Vec::new();
    for r in rows {
        sets.push(r.map_err(|e| e.to_string())?);
    }

    Ok(GallerySetsPage { sets, total })
}

pub fn set_gallery_set_cover(
    app: &AppHandle,
    set_id: &str,
    cover_asset_id: Option<&str>,
    page_index: i64,
) -> Result<GallerySetV2, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let page_idx = page_index.max(0);
    let ts = now();

    let existing_cover: Option<String> = conn
        .query_row(
            "SELECT cover_asset_id FROM gallery_sets WHERE id = ?1",
            params![set_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();

    let resolved_asset = cover_asset_id
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or(existing_cover)
        .ok_or_else(|| "未找到该图集的封面资源".to_string())?;

    conn.execute(
        "INSERT INTO gallery_cover_overrides (set_id, cover_asset_id, cover_page_index, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(set_id) DO UPDATE SET
            cover_asset_id = excluded.cover_asset_id,
            cover_page_index = excluded.cover_page_index,
            updated_at = excluded.updated_at",
        params![set_id, resolved_asset, page_idx, ts],
    )
    .map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE gallery_sets SET cover_asset_id = ?1, cover_page_index = ?2, updated_at = ?3 WHERE id = ?4",
        params![resolved_asset, page_idx, ts, set_id],
    )
    .map_err(|e| e.to_string())?;

    conn.query_row(
        "SELECT g.id, g.model_name, g.album_name, g.remote_path, g.is_archive, g.cover_asset_id,
                COALESCE(g.cover_page_index, 0),
                CASE WHEN g.is_archive = 1 AND COALESCE(ca.page_count, 0) > g.item_count THEN ca.page_count ELSE g.item_count END AS effective_count,
                g.total_bytes, g.updated_at
         FROM gallery_sets g
         LEFT JOIN comic_archives ca ON ca.asset_id = g.cover_asset_id
         WHERE g.id = ?1",
        params![set_id],
        |r| {
            Ok(GallerySetV2 {
                id: r.get(0)?,
                model_name: r.get(1)?,
                album_name: r.get(2)?,
                remote_path: r.get(3)?,
                is_archive: r.get::<_, i64>(4)? != 0,
                cover_asset_id: r.get(5)?,
                cover_page_index: r.get(6)?,
                item_count: r.get(7)?,
                total_bytes: r.get(8)?,
                updated_at: r.get(9)?,
            })
        },
    )
    .map_err(|e| e.to_string())
}

pub fn get_gallery_set_items(
    app: &AppHandle,
    set_id: &str,
) -> Result<Vec<CreatorAssetV2>, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.root_id, a.remote_path, a.file_name, a.parent_path,
                    a.media_type, a.size_bytes, a.duration_seconds, a.thumbnail_asset_id,
                    gs.model_name, gs.album_name
             FROM gallery_items gi
             CROSS JOIN assets a ON a.id = gi.asset_id
             JOIN gallery_sets gs ON gs.id = gi.set_id
             WHERE gi.set_id = ?1 AND a.missing = 0
             ORDER BY gi.sort_order ASC, a.file_name ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![set_id], |r| {
            let media_type: String = r.get(5)?;
            let normalized_type = if media_type == "comicPage" {
                "image".to_string()
            } else {
                media_type
            };
            Ok(CreatorAssetV2 {
                id: r.get(0)?,
                root_id: r.get(1)?,
                root_kind: "media".to_string(),
                remote_path: r.get(2)?,
                file_name: r.get(3)?,
                parent_path: r.get(4)?,
                media_type: normalized_type,
                size_bytes: r.get(6)?,
                duration_seconds: r.get(7)?,
                author_id: None,
                author_name: Some(r.get::<_, String>(9)?),
                category_name: Some(r.get::<_, String>(9)?),
                thumbnail_asset_id: r.get(8)?,
                album_name: Some(r.get::<_, String>(10)?),
            })
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

// ============================================================================
// Folder-Hierarchy Browsing + Single-Video Folder Auto-Unwrap (Creators & Collections)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderCardV2 {
    pub name: String,
    pub sub_path: String,
    pub studio_name: String,
    pub item_count: i64,
    pub video_count: i64,
    pub image_count: i64,
    pub total_bytes: i64,
    pub cover_asset_id: Option<String>,
    pub preview_asset_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderBrowsePage {
    pub sub_path: String,
    pub breadcrumbs: Vec<String>,
    pub folders: Vec<FolderCardV2>,
    pub files: Vec<CreatorAssetV2>,
    pub total_folders: i64,
    pub total_files: i64,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorFolderResponse {
    pub creator: CreatorSummaryV2,
    pub page: FolderBrowsePage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionFolderResponse {
    pub album: CollectionAlbumV2,
    pub page: FolderBrowsePage,
}

fn build_folder_browse_page(
    entries: Vec<(String, String, CreatorAssetV2)>, // (rel_path, studio_or_channel, asset)
    sub_path: Option<&str>,
    query: Option<&str>,
    sort: Option<&str>,
    limit: i64,
    offset: i64,
) -> FolderBrowsePage {
    let clean_sub = sub_path
        .unwrap_or("")
        .trim()
        .trim_matches('/')
        .to_string();
    let sub_prefix = if clean_sub.is_empty() {
        String::new()
    } else {
        format!("{clean_sub}/")
    };
    let breadcrumbs: Vec<String> = if clean_sub.is_empty() {
        Vec::new()
    } else {
        clean_sub
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    };

    struct FolderAgg {
        name: String,
        sub_path: String,
        studio_name: String,
        item_count: i64,
        video_count: i64,
        image_count: i64,
        total_bytes: i64,
        cover_asset_id: Option<String>,
        cover_is_video: bool,
        preview_asset_ids: Vec<String>,
        single_asset: Option<CreatorAssetV2>,
    }

    let mut folder_map: HashMap<(String, String), FolderAgg> = HashMap::new();
    let mut direct_files: Vec<CreatorAssetV2> = Vec::new();

    for (rel_path, studio_name, asset) in entries {
        let rel = rel_path.trim_matches('/');
        let tail = if sub_prefix.is_empty() {
            rel
        } else if let Some(rest) = rel.strip_prefix(&sub_prefix) {
            rest
        } else {
            continue;
        };
        if tail.is_empty() {
            continue;
        }

        if let Some((seg0, _rest)) = tail.split_once('/') {
            if seg0.is_empty() {
                continue;
            }
            let child_sub = if clean_sub.is_empty() {
                seg0.to_string()
            } else {
                format!("{clean_sub}/{seg0}")
            };
            let key = (studio_name.clone(), seg0.to_string());
            let agg = folder_map.entry(key).or_insert_with(|| FolderAgg {
                name: seg0.to_string(),
                sub_path: child_sub,
                studio_name: studio_name.clone(),
                item_count: 0,
                video_count: 0,
                image_count: 0,
                total_bytes: 0,
                cover_asset_id: None,
                cover_is_video: false,
                preview_asset_ids: Vec::new(),
                single_asset: None,
            });
            agg.item_count += 1;
            agg.total_bytes += asset.size_bytes;
            let is_video = asset.media_type == "video";
            if is_video {
                agg.video_count += 1;
            } else if asset.media_type == "image" {
                agg.image_count += 1;
            }
            let thumb = asset
                .thumbnail_asset_id
                .clone()
                .unwrap_or_else(|| asset.id.clone());
            if agg.cover_asset_id.is_none() || (!agg.cover_is_video && is_video) {
                agg.cover_asset_id = Some(thumb.clone());
                agg.cover_is_video = is_video;
            }
            if agg.preview_asset_ids.len() < 4 && !agg.preview_asset_ids.contains(&thumb) {
                agg.preview_asset_ids.push(thumb);
            }
            if agg.item_count == 1 {
                agg.single_asset = Some(asset);
            } else {
                agg.single_asset = None;
            }
        } else {
            direct_files.push(asset);
        }
    }

    let mut folders: Vec<FolderCardV2> = Vec::with_capacity(folder_map.len());
    for agg in folder_map.into_values() {
        // Single-video wrapper folder auto-unwrap: play directly with 1 click!
        if agg.item_count == 1 && agg.video_count == 1 {
            if let Some(mut single) = agg.single_asset {
                // Keep folder name as display title since wrapper folder names are usually descriptive.
                single.file_name = agg.name;
                direct_files.push(single);
                continue;
            }
        }
        folders.push(FolderCardV2 {
            name: agg.name,
            sub_path: agg.sub_path,
            studio_name: agg.studio_name,
            item_count: agg.item_count,
            video_count: agg.video_count,
            image_count: agg.image_count,
            total_bytes: agg.total_bytes,
            cover_asset_id: agg.cover_asset_id,
            preview_asset_ids: agg.preview_asset_ids,
        });
    }

    if let Some(q) = query.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()) {
        folders.retain(|f| {
            f.name.to_lowercase().contains(&q) || f.studio_name.to_lowercase().contains(&q)
        });
        direct_files.retain(|f| {
            f.file_name.to_lowercase().contains(&q)
                || f.category_name
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q)
        });
    }

    match sort.unwrap_or("name") {
        "size" => {
            folders.sort_by(|a, b| {
                b.total_bytes
                    .cmp(&a.total_bytes)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            direct_files.sort_by(|a, b| {
                b.size_bytes
                    .cmp(&a.size_bytes)
                    .then_with(|| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()))
            });
        }
        "count" => {
            folders.sort_by(|a, b| {
                b.item_count
                    .cmp(&a.item_count)
                    .then_with(|| b.total_bytes.cmp(&a.total_bytes))
            });
            direct_files.sort_by(|a, b| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()));
        }
        _ => {
            folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            direct_files.sort_by(|a, b| {
                let av = if a.media_type == "video" { 0 } else { 1 };
                let bv = if b.media_type == "video" { 0 } else { 1 };
                av.cmp(&bv)
                    .then_with(|| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()))
            });
        }
    }

    let total_folders = folders.len() as i64;
    let total_files = direct_files.len() as i64;
    let total = total_folders + total_files;

    // Unified pagination across [folders..., files...]
    let off = offset.max(0) as usize;
    let lim = limit.clamp(1, 200) as usize;
    let f_len = folders.len();

    let paged_folders: Vec<FolderCardV2> = if off < f_len {
        folders.into_iter().skip(off).take(lim).collect()
    } else {
        Vec::new()
    };
    let remaining_lim = lim.saturating_sub(paged_folders.len());
    let file_off = off.saturating_sub(f_len);
    let paged_files: Vec<CreatorAssetV2> = if remaining_lim > 0 && file_off < direct_files.len() {
        direct_files
            .into_iter()
            .skip(file_off)
            .take(remaining_lim)
            .collect()
    } else {
        Vec::new()
    };

    FolderBrowsePage {
        sub_path: clean_sub,
        breadcrumbs,
        folders: paged_folders,
        files: paged_files,
        total_folders,
        total_files,
        total,
    }
}

pub fn browse_creator_folder(
    app: &AppHandle,
    author_id: &str,
    studio: Option<&str>,
    sub_path: Option<&str>,
    query: Option<&str>,
    sort: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<CreatorFolderResponse, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let creator = load_creators_summary_map(&conn, Some(author_id))?
        .into_iter()
        .next()
        .ok_or_else(|| format!("未找到创作者：{author_id}"))?;

    let creators_root: String = conn
        .query_row(
            "SELECT remote_path FROM library_modules WHERE id = 'creators'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "/115open/文件/创作者".to_string());
    let root_prefix = format!("{}/", creators_root.trim_end_matches('/'));

    let studio_filter = studio.map(str::trim).filter(|s| !s.is_empty() && *s != "all");

    let sql = if studio_filter.is_some() {
        "SELECT a.id, a.root_id, a.remote_path, a.file_name, a.parent_path,
                a.media_type, a.size_bytes, a.duration_seconds, a.thumbnail_asset_id,
                aa.studio_name, aa.album_name
         FROM author_assets aa
         CROSS JOIN assets a ON a.id = aa.asset_id
         WHERE aa.author_id = ?1 AND aa.studio_name = ?2 AND a.missing = 0"
    } else {
        "SELECT a.id, a.root_id, a.remote_path, a.file_name, a.parent_path,
                a.media_type, a.size_bytes, a.duration_seconds, a.thumbnail_asset_id,
                aa.studio_name, aa.album_name
         FROM author_assets aa
         CROSS JOIN assets a ON a.id = aa.asset_id
         WHERE aa.author_id = ?1 AND a.missing = 0"
    };

    let mut entries: Vec<(String, String, CreatorAssetV2)> = Vec::new();
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let map_row = |r: &rusqlite::Row| -> rusqlite::Result<(String, String, CreatorAssetV2)> {
        let remote_path: String = r.get(2)?;
        let studio_name: String = r.get(9)?;
        let rel = if let Some(after_root) = remote_path.strip_prefix(&root_prefix) {
            // Strip first 2 segments: studio / author_dir
            let segs: Vec<&str> = after_root.split('/').filter(|s| !s.is_empty()).collect();
            if segs.len() >= 3 {
                segs[2..].join("/")
            } else {
                r.get::<_, String>(3)?
            }
        } else {
            r.get::<_, String>(3)?
        };
        let asset = CreatorAssetV2 {
            id: r.get(0)?,
            root_id: r.get(1)?,
            root_kind: "media".to_string(),
            remote_path,
            file_name: r.get(3)?,
            parent_path: r.get(4)?,
            media_type: r.get(5)?,
            size_bytes: r.get(6)?,
            duration_seconds: r.get(7)?,
            author_id: Some(creator.id.clone()),
            author_name: Some(creator.name.clone()),
            category_name: Some(studio_name.clone()),
            thumbnail_asset_id: r.get(8)?,
            album_name: r.get(10)?,
        };
        Ok((rel, studio_name, asset))
    };

    if let Some(st) = studio_filter {
        let rows = stmt
            .query_map(params![author_id, st], map_row)
            .map_err(|e| e.to_string())?;
        for r in rows {
            entries.push(r.map_err(|e| e.to_string())?);
        }
    } else {
        let rows = stmt
            .query_map(params![author_id], map_row)
            .map_err(|e| e.to_string())?;
        for r in rows {
            entries.push(r.map_err(|e| e.to_string())?);
        }
    }

    let page = build_folder_browse_page(entries, sub_path, query, sort, limit, offset);
    Ok(CreatorFolderResponse { creator, page })
}

pub fn browse_collection_folder(
    app: &AppHandle,
    album_id: &str,
    sub_path: Option<&str>,
    query: Option<&str>,
    sort: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<CollectionFolderResponse, String> {
    four_domains::ensure_v2(app)?;
    let conn = four_domains::open_v2(app)?;
    let album: CollectionAlbumV2 = conn
        .query_row(
            "SELECT id, channel, name, remote_path, is_direct, cover_asset_id,
                    item_count, video_count, image_count, total_bytes, updated_at
             FROM collection_albums WHERE id = ?1",
            params![album_id],
            |r| {
                Ok(CollectionAlbumV2 {
                    id: r.get(0)?,
                    channel: r.get(1)?,
                    name: r.get(2)?,
                    remote_path: r.get(3)?,
                    is_direct: r.get::<_, i64>(4)? != 0,
                    cover_asset_id: r.get(5)?,
                    preview_asset_ids: Vec::new(),
                    item_count: r.get(6)?,
                    video_count: r.get(7)?,
                    image_count: r.get(8)?,
                    total_bytes: r.get(9)?,
                    updated_at: r.get(10)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("未找到合集：{album_id}"))?;

    let album_prefix = format!("{}/", album.remote_path.trim_end_matches('/'));
    let channel_name = album.channel.clone();

    let mut stmt = conn
        .prepare(
            "SELECT a.id, a.root_id, a.remote_path, a.file_name, a.parent_path,
                    a.media_type, a.size_bytes, a.duration_seconds, a.thumbnail_asset_id
             FROM collection_items ci
             CROSS JOIN assets a ON a.id = ci.asset_id
             WHERE ci.album_id = ?1 AND a.missing = 0",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(params![album_id], |r| {
            let remote_path: String = r.get(2)?;
            let rel = remote_path
                .strip_prefix(&album_prefix)
                .map(str::to_string)
                .unwrap_or_else(|| r.get::<_, String>(3).unwrap_or_default());
            let asset = CreatorAssetV2 {
                id: r.get(0)?,
                root_id: r.get(1)?,
                root_kind: "media".to_string(),
                remote_path,
                file_name: r.get(3)?,
                parent_path: r.get(4)?,
                media_type: r.get(5)?,
                size_bytes: r.get(6)?,
                duration_seconds: r.get(7)?,
                author_id: None,
                author_name: None,
                category_name: Some(channel_name.clone()),
                thumbnail_asset_id: r.get(8)?,
                album_name: Some(album.name.clone()),
            };
            Ok((rel, channel_name.clone(), asset))
        })
        .map_err(|e| e.to_string())?;

    let mut entries = Vec::new();
    for r in rows {
        entries.push(r.map_err(|e| e.to_string())?);
    }

    let page = build_folder_browse_page(entries, sub_path, query, sort, limit, offset);
    Ok(CollectionFolderResponse { album, page })
}
