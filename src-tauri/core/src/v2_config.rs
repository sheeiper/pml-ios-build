//! V2-resident runtime configuration. Legacy import is read-only and performed once.
use crate::{four_domains, AppHandle};
use rusqlite::{Connection, OpenFlags, OptionalExtension};

pub(crate) fn open(app: &AppHandle) -> Result<Connection, String> {
    four_domains::ensure_v2(app)?;
    let mut db = four_domains::open_v2(app)?;
    // The durable marker is committed with both auxiliary tables. Ordinary
    // configuration/media requests must not take a SQLite writer lock.
    let ready = db.query_row(
        "SELECT 1 FROM schema_meta WHERE key='runtime_config_imported'", [], |_| Ok(())
    ).optional().map_err(|e| e.to_string())?.is_some();
    if ready { return Ok(db); }
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS library_roots(id TEXT PRIMARY KEY,kind TEXT NOT NULL,remote_path TEXT NOT NULL,created_at INTEGER NOT NULL);")
        .map_err(|e| e.to_string())?;
    let done = tx.query_row("SELECT value FROM schema_meta WHERE key='runtime_config_imported'", [], |r| r.get::<_, String>(0))
        .optional().map_err(|e| e.to_string())?.is_some();
    if !done {
        let directory = if let Ok(value) = std::env::var("PML_DATA_DIR") {
            std::path::PathBuf::from(value)
        } else {
            #[cfg(feature = "tauri")]
            use tauri::Manager;
            app.path().app_data_dir().map_err(|e| e.to_string())?
        };
        let legacy = directory.join("personal-media-library.sqlite3");
        if legacy.exists() {
            let old = Connection::open_with_flags(legacy, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|e| format!("配置导入失败：{e}"))?;
            let mut settings = old.prepare("SELECT key,value FROM settings").map_err(|e| e.to_string())?;
            let rows = settings.query_map([], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?)))
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (key,value) = row.map_err(|e| e.to_string())?;
                tx.execute("INSERT OR IGNORE INTO settings(key,value) VALUES(?1,?2)", rusqlite::params![key,value])
                    .map_err(|e| e.to_string())?;
            }
            let mut roots = old.prepare("SELECT id,kind,remote_path,created_at FROM library_roots").map_err(|e| e.to_string())?;
            let rows = roots.query_map([], |r| Ok((r.get::<_, String>(0)?,r.get::<_, String>(1)?,r.get::<_, String>(2)?,r.get::<_, i64>(3)?)))
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (id,kind,path,created) = row.map_err(|e| e.to_string())?;
                tx.execute("INSERT OR IGNORE INTO library_roots(id,kind,remote_path,created_at) VALUES(?1,?2,?3,?4)", rusqlite::params![id,kind,path,created])
                    .map_err(|e| e.to_string())?;
            }
        }
        tx.execute("INSERT INTO schema_meta(key,value) VALUES('runtime_config_imported','1')", []).map_err(|e| e.to_string())?;
    }
    tx.execute("INSERT OR IGNORE INTO library_roots(id,kind,remote_path,created_at) SELECT root_id,CASE WHEN domain='comics' THEN 'comics' ELSE 'media' END,remote_path,updated_at FROM domain_roots", [])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(db)
}
