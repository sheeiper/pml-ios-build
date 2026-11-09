//! Legacy UI playback/favorite commands backed exclusively by canonical V2 state.
//!
//! State mutations deliberately name only the fields they own: playback, favorites,
//! and comic reading state must never overwrite one another.

use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use crate::{
    four_domains,
    media_library::{LibraryAsset, LibraryRootKind, PlaybackHistoryItem},
    AppHandle,
};

fn database_error(error: rusqlite::Error) -> String {
    format!("V2 状态数据库错误：{error}")
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Kept independent of four_domains' initializer: that initializer may already
/// have run before this module's additive schema migration is first requested.
fn open_state(app: &AppHandle) -> Result<Connection, String> {
    static INITIALIZED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    four_domains::ensure_v2(app)?;
    let mut connection = four_domains::open_v2(app)?;
    let path = connection.path().unwrap_or_default().to_owned();
    let mut initialized = INITIALIZED
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // In-memory connections are independent databases, never a process-wide key.
    if path.is_empty() || !initialized.contains(&path) {
        ensure_state_schema(&mut connection).map_err(database_error)?;
        if !path.is_empty() {
            initialized.insert(path);
        }
    }
    Ok(connection)
}

/// Bootstrap hook: call after creating the base V2 schema and before importing
/// states. This does not call ensure_v2/open_v2 and cannot recurse into bootstrap.
pub(crate) fn ensure_state_schema(connection: &mut Connection) -> rusqlite::Result<()> {
    // Serialize introspection + ALTER across connections/processes as well.
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let columns = {
        let mut statement = tx.prepare("PRAGMA table_info(asset_state)")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<rusqlite::Result<HashSet<_>>>()?
    };
    if !columns.contains("favorite_at") {
        tx.execute_batch("ALTER TABLE asset_state ADD COLUMN favorite_at INTEGER;")?;
    }
    if !columns.contains("last_played_at") {
        tx.execute_batch("ALTER TABLE asset_state ADD COLUMN last_played_at INTEGER;")?;
    }
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    let migrated = tx
        .query_row(
            "SELECT 1 FROM schema_meta WHERE key='asset_state_playback_favorites_v1'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !migrated {
        // Only progress-bearing rows represent playback. Reading-only rows must
        // not become history. Old migrations sometimes have only updated_at.
        tx.execute_batch(
            "UPDATE asset_state
             SET last_played_at=COALESCE(last_opened_at,updated_at)
             WHERE progress_seconds IS NOT NULL AND last_played_at IS NULL;
             UPDATE asset_state SET favorite_at=updated_at
             WHERE favorite<>0 AND favorite_at IS NULL;
             INSERT INTO schema_meta(key,value)
             VALUES('asset_state_playback_favorites_v1','1');",
        )?;
    }
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS asset_state_playback_time ON asset_state(last_played_at DESC);
         CREATE INDEX IF NOT EXISTS asset_state_favorite_time ON asset_state(favorite,favorite_at DESC);",
    )?;
    tx.commit()
}

fn playback_progress(connection: &Connection, asset_id: &str) -> rusqlite::Result<f64> {
    let position = connection
        .query_row(
            "SELECT progress_seconds FROM asset_state WHERE asset_id=?1",
            [asset_id],
            |row| row.get::<_, Option<f64>>(0),
        )
        .optional()?
        .flatten()
        .unwrap_or(0.0);
    Ok(if position.is_finite() { position.max(0.0) } else { 0.0 })
}

/// Connection-level writer for other V2 APIs; requires ensure_state_schema first.
/// Accepts a transaction via deref coercion without opening another connection.
pub(crate) fn set_progress(connection: &Connection, asset_id: &str, position: f64, timestamp: i64) -> rusqlite::Result<()> {
    if !position.is_finite() || position < 0.0 {
        return Ok(());
    }
    connection.execute(
        "INSERT INTO asset_state(asset_id,progress_seconds,last_played_at,last_opened_at,updated_at)
         VALUES(?1,?2,?3,?3,?3)
         ON CONFLICT(asset_id) DO UPDATE SET
           progress_seconds=excluded.progress_seconds,
           last_played_at=excluded.last_played_at,
           last_opened_at=excluded.last_opened_at,
           updated_at=excluded.updated_at",
        params![asset_id, position.round().max(0.0) as i64, timestamp],
    )?;
    Ok(())
}

/// Idempotent favorite setter shared by domain APIs. Re-applying `true` retains
/// the original favorite timestamp, so unrelated updates cannot reorder favorites.
/// Requires ensure_state_schema first; preserves playback and reading columns.
pub(crate) fn set_favorite(
    connection: &Connection,
    asset_id: &str,
    favorite: bool,
    timestamp: i64,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO asset_state(asset_id,favorite,favorite_at,updated_at)
         VALUES(?1,?2,CASE WHEN ?2<>0 THEN ?3 ELSE NULL END,?3)
         ON CONFLICT(asset_id) DO UPDATE SET favorite=excluded.favorite,
           favorite_at=CASE WHEN excluded.favorite=0 THEN NULL
                            WHEN asset_state.favorite<>0 THEN COALESCE(asset_state.favorite_at,excluded.favorite_at)
                            ELSE excluded.favorite_at END,
           updated_at=excluded.updated_at",
        params![asset_id, favorite, timestamp],
    )?;
    Ok(())
}

fn clear_history(connection: &Connection, timestamp: i64) -> rusqlite::Result<()> {
    // last_opened_at is shared with reading; leave it (and reading_page) alone.
    connection.execute(
        "UPDATE asset_state SET progress_seconds=NULL,last_played_at=NULL,updated_at=?1
         WHERE progress_seconds IS NOT NULL OR last_played_at IS NOT NULL",
        [timestamp],
    )?;
    Ok(())
}

fn toggle(connection: &mut Connection, asset_id: &str, timestamp: i64) -> rusqlite::Result<bool> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // A single UPSERT flips the value while retaining all non-favorite fields.
    // The FK deliberately rejects unknown assets rather than creating orphans.
    tx.execute(
        "INSERT INTO asset_state(asset_id,favorite,favorite_at,updated_at) VALUES(?1,1,?2,?2)
         ON CONFLICT(asset_id) DO UPDATE SET
           favorite=CASE WHEN asset_state.favorite=0 THEN 1 ELSE 0 END,
           favorite_at=CASE WHEN asset_state.favorite=0 THEN excluded.favorite_at ELSE NULL END,
           updated_at=excluded.updated_at",
        params![asset_id, timestamp],
    )?;
    let favorite = tx.query_row(
        "SELECT favorite<>0 FROM asset_state WHERE asset_id=?1",
        [asset_id],
        |row| row.get(0),
    )?;
    tx.commit()?;
    Ok(favorite)
}

fn sync(connection: &mut Connection, asset_ids: &[String], timestamp: i64) -> rusqlite::Result<()> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    {
        let mut statement = tx.prepare(
            "INSERT INTO asset_state(asset_id,favorite,favorite_at,updated_at)
             SELECT id,1,?2,?2 FROM assets WHERE id=?1
             ON CONFLICT(asset_id) DO UPDATE SET favorite=1,
               favorite_at=excluded.favorite_at,updated_at=excluded.updated_at
             WHERE asset_state.favorite=0",
        )?;
        for id in asset_ids {
            statement.execute(params![id, timestamp])?;
        }
    }
    tx.commit()
}

pub fn get_asset_playback_progress(app: AppHandle, asset_id: String) -> Result<f64, String> {
    playback_progress(&open_state(&app)?, &asset_id).map_err(database_error)
}

pub fn set_asset_playback_progress(app: AppHandle, asset_id: String, position_seconds: f64) -> Result<(), String> {
    if !position_seconds.is_finite() || position_seconds < 0.0 {
        return Ok(());
    }
    set_progress(&open_state(&app)?, &asset_id, position_seconds, now()).map_err(database_error)
}

fn asset_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LibraryAsset> {
    let domain: String = row.get(2)?;
    Ok(LibraryAsset {
        id: row.get(0)?,
        root_id: row.get(1)?,
        root_kind: if domain == "comics" { LibraryRootKind::Comics } else { LibraryRootKind::Media },
        remote_path: row.get(3)?,
        file_name: row.get(4)?,
        parent_path: row.get(5)?,
        media_type: row.get(6)?,
        size_bytes: row.get(7)?,
        duration_seconds: row.get(8)?,
        category_id: None,
        category_name: None,
        author_id: None,
        author_name: None,
        collection_id: None,
        collection_name: None,
        metadata_origin: None,
        content_hash: row.get(9)?,
        thumbnail_asset_id: row.get(10)?,
    })
}

pub fn list_playback_history(app: AppHandle, limit: Option<i64>) -> Result<Vec<PlaybackHistoryItem>, String> {
    let connection = open_state(&app)?;
    let mut statement = connection.prepare(
        "SELECT a.id,a.root_id,r.domain,a.remote_path,a.file_name,a.parent_path,
                a.media_type,a.size_bytes,a.duration_seconds,a.content_hash,a.thumbnail_asset_id,
                CAST(COALESCE(s.progress_seconds,0) AS INTEGER),s.last_played_at
         FROM asset_state s JOIN assets a ON a.id=s.asset_id
         JOIN domain_roots r ON r.domain=a.domain AND r.root_id=a.root_id
         WHERE a.missing=0 AND a.media_type='video' AND s.last_played_at IS NOT NULL
         ORDER BY s.last_played_at DESC,a.id LIMIT ?1",
    ).map_err(database_error)?;
    let rows = statement.query_map([limit.unwrap_or(100).clamp(1,500)], |row| {
        Ok(PlaybackHistoryItem {
            asset: asset_from_row(row)?,
            position_seconds: row.get::<_, i64>(11)?.max(0),
            updated_at: row.get(12)?,
        })
    }).map_err(database_error)?;
    rows.map(|row| row.map_err(database_error)).collect()
}

pub fn list_playback_history_ids(app: AppHandle) -> Result<Vec<String>, String> {
    history_ids(&open_state(&app)?).map_err(database_error)
}

fn history_ids(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT s.asset_id FROM asset_state s JOIN assets a ON a.id=s.asset_id
         WHERE a.missing=0 AND a.media_type='video' AND s.last_played_at IS NOT NULL
         ORDER BY s.last_played_at DESC,s.asset_id",
    )?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect()
}

pub fn clear_playback_history(app: AppHandle) -> Result<(), String> {
    clear_history(&open_state(&app)?, now()).map_err(database_error)
}

pub fn toggle_favorite(app: AppHandle, asset_id: String) -> Result<bool, String> {
    toggle(&mut open_state(&app)?, &asset_id, now()).map_err(database_error)
}

pub fn list_favorite_assets(app: AppHandle, limit: Option<i64>, offset: Option<i64>) -> Result<Vec<LibraryAsset>, String> {
    let connection = open_state(&app)?;
    let mut statement = connection.prepare(
        "SELECT a.id,a.root_id,r.domain,a.remote_path,a.file_name,a.parent_path,
                a.media_type,a.size_bytes,a.duration_seconds,a.content_hash,a.thumbnail_asset_id
         FROM asset_state s JOIN assets a ON a.id=s.asset_id
         JOIN domain_roots r ON r.domain=a.domain AND r.root_id=a.root_id
         WHERE a.missing=0 AND s.favorite<>0
         ORDER BY s.favorite_at DESC,a.id LIMIT ?1 OFFSET ?2",
    ).map_err(database_error)?;
    let rows = statement.query_map(
        params![limit.unwrap_or(50).clamp(1,200),offset.unwrap_or(0).max(0)],
        asset_from_row,
    ).map_err(database_error)?;
    rows.map(|row| row.map_err(database_error)).collect()
}

pub fn list_favorite_ids(app: AppHandle) -> Result<Vec<String>, String> {
    let connection = open_state(&app)?;
    let mut statement = connection.prepare(
        "SELECT s.asset_id FROM asset_state s JOIN assets a ON a.id=s.asset_id
         WHERE s.favorite<>0 ORDER BY s.favorite_at DESC,s.asset_id",
    ).map_err(database_error)?;
    let rows = statement.query_map([], |row| row.get(0)).map_err(database_error)?;
    rows.map(|row| row.map_err(database_error)).collect()
}

pub fn sync_favorites(app: AppHandle, asset_ids: Vec<String>) -> Result<(), String> {
    if asset_ids.is_empty() {
        return Ok(());
    }
    sync(&mut open_state(&app)?, &asset_ids, now()).map_err(database_error)
}

pub fn get_favorite_count(app: AppHandle) -> Result<i64, String> {
    open_state(&app)?.query_row(
        "SELECT COUNT(*) FROM asset_state s JOIN assets a ON a.id=s.asset_id
         WHERE s.favorite<>0 AND a.missing=0",
        [], |row| row.get(0),
    ).map_err(database_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE assets(id TEXT PRIMARY KEY);
             INSERT INTO assets VALUES('video'),('comic'),('other');
             CREATE TABLE asset_state(
               asset_id TEXT PRIMARY KEY REFERENCES assets(id),
               favorite INTEGER NOT NULL DEFAULT 0,progress_seconds REAL,reading_page INTEGER,
               last_opened_at INTEGER,updated_at INTEGER NOT NULL);
             INSERT INTO asset_state VALUES('video',1,12,NULL,40,50);
             INSERT INTO asset_state VALUES('comic',0,NULL,7,60,70);",
        ).unwrap();
        ensure_state_schema(&mut connection).unwrap();
        connection
    }

    #[test]
    fn history_ids_are_unbounded_and_include_zero_progress_but_not_comic_reading() {
        let mut c=memory_db();
        c.execute_batch("ALTER TABLE assets ADD COLUMN missing INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE assets ADD COLUMN media_type TEXT NOT NULL DEFAULT 'video';
            UPDATE assets SET media_type='image' WHERE id='comic';
            UPDATE asset_state SET progress_seconds=0 WHERE asset_id='video';").unwrap();
        let tx=c.transaction().unwrap();
        for index in 0..601 {
            let id=format!("extra-{index}");
            tx.execute("INSERT INTO assets(id) VALUES(?1)",[&id]).unwrap();
            tx.execute("INSERT INTO asset_state(asset_id,progress_seconds,last_played_at,updated_at) VALUES(?1,0,100,100)",[&id]).unwrap();
        }
        tx.commit().unwrap();
        let ids=history_ids(&c).unwrap();
        assert_eq!(ids.len(),602);
        assert!(ids.contains(&"video".to_string()));
        assert!(!ids.contains(&"comic".to_string()));
        clear_history(&c,200).unwrap();
        assert!(history_ids(&c).unwrap().is_empty());
    }

    #[test]
    fn additive_schema_backfills_only_playback_and_does_not_resurrect_clear() {
        let mut connection = memory_db();
        let times: (Option<i64>,Option<i64>) = connection.query_row(
            "SELECT last_played_at,favorite_at FROM asset_state WHERE asset_id='video'", [],
            |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert_eq!(times,(Some(40),Some(50)));
        let comic_playback: Option<i64> = connection.query_row(
            "SELECT last_played_at FROM asset_state WHERE asset_id='comic'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(comic_playback,None);
        clear_history(&connection,80).unwrap();
        ensure_state_schema(&mut connection).unwrap();
        let state: (i64,Option<f64>,Option<i64>,i64) = connection.query_row(
            "SELECT favorite,progress_seconds,last_played_at,last_opened_at FROM asset_state WHERE asset_id='video'", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(state,(1,None,None,40));
        let reading: (i64,i64) = connection.query_row(
            "SELECT reading_page,last_opened_at FROM asset_state WHERE asset_id='comic'", [],
            |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert_eq!(reading,(7,60));
    }

    #[test]
    fn playback_and_favorites_preserve_independent_reading_state() {
        let mut connection = memory_db();
        set_progress(&connection,"video",18.0,90).unwrap();
        let favorite: (i64,i64) = connection.query_row(
            "SELECT favorite,favorite_at FROM asset_state WHERE asset_id='video'", [],
            |row| Ok((row.get(0)?,row.get(1)?)),
        ).unwrap();
        assert_eq!(favorite,(1,50));
        set_progress(&connection,"comic",25.6,100).unwrap();
        assert!(toggle(&mut connection,"comic",110).unwrap());
        assert!(!toggle(&mut connection,"comic",120).unwrap());
        let state: (i64,i64,f64,i64,Option<i64>) = connection.query_row(
            "SELECT reading_page,favorite,progress_seconds,last_played_at,favorite_at FROM asset_state WHERE asset_id='comic'", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
        ).unwrap();
        assert_eq!(state,(7,0,26.0,100,None));
        assert!(toggle(&mut connection,"other",130).unwrap());
        assert!(!toggle(&mut connection,"other",140).unwrap());
        assert!(toggle(&mut connection,"invalid",150).is_err());
        assert_eq!(playback_progress(&connection,"absent").unwrap(),0.0);
        set_progress(&connection,"comic",f64::NAN,160).unwrap();
        assert_eq!(playback_progress(&connection,"comic").unwrap(),26.0);
    }

    #[test]
    fn domain_favorite_setter_is_idempotent_and_preserves_other_state() {
        let connection = memory_db();
        set_favorite(&connection,"video",true,100).unwrap();
        let timestamp: i64 = connection.query_row(
            "SELECT favorite_at FROM asset_state WHERE asset_id='video'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(timestamp,50);
        set_favorite(&connection,"video",false,110).unwrap();
        set_favorite(&connection,"video",true,120).unwrap();
        let state: (i64,f64,i64) = connection.query_row(
            "SELECT favorite_at,progress_seconds,last_played_at FROM asset_state WHERE asset_id='video'", [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).unwrap();
        assert_eq!(state,(120,12.0,40));
        set_favorite(&connection,"comic",true,130).unwrap();
        let reading: i64 = connection.query_row(
            "SELECT reading_page FROM asset_state WHERE asset_id='comic'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(reading,7);
    }

    #[test]
    fn sync_is_additive_validated_and_does_not_reorder_existing_favorites() {
        let mut connection = memory_db();
        sync(&mut connection,&["video".into(),"comic".into(),"invalid".into(),"comic".into()],200).unwrap();
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM asset_state WHERE favorite=1", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(count,2);
        let original: i64 = connection.query_row(
            "SELECT favorite_at FROM asset_state WHERE asset_id='video'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(original,50);
        assert_eq!(playback_progress(&connection,"video").unwrap(),12.0);
        sync(&mut connection,&[],300).unwrap();
        let page: i64 = connection.query_row(
            "SELECT reading_page FROM asset_state WHERE asset_id='comic'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(page,7);
    }
}
