//! One-shot, read-only V1 export into the canonical V2 model.
//!
//! This module is deliberately not a runtime fallback. The durable marker and
//! report are committed atomically with the import. Existing V2 assets/state are
//! authoritative, including missing assets, explicit unfavorites and cleared
//! playback. Deletions without a tombstone/snapshot cannot be distinguished from
//! never-imported files; that limitation is recorded in the durable report.
use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};

use crate::AppHandle;

const MARKER: &str = "full_legacy_import_v1";

fn error(error: rusqlite::Error) -> String { format!("V2 legacy import: {error}") }
fn done(c: &Connection) -> rusqlite::Result<bool> {
    Ok(c.query_row("SELECT 1 FROM schema_meta WHERE key=?1", [MARKER], |_| Ok(()))
        .optional()?.is_some())
}

/// Called after V2 schema, roots, and the original vault identity import.
/// The marker check is made before opening V1 and repeated under the writer lock.
pub(crate) fn migrate(connection: &Connection, app: &AppHandle) -> Result<(), String> {
    if done(connection).map_err(error)? { return Ok(()); }
    let tx = connection.unchecked_transaction().map_err(error)?;
    // Reserve the writer before checking again, serializing other initializers.
    tx.execute("UPDATE schema_meta SET value=value WHERE key=?1", [MARKER]).map_err(error)?;
    if done(&tx).map_err(error)? { return tx.commit().map_err(error); }
    let path = crate::four_domains::legacy_path(app)?;
    if !path.exists() {
        finish(&tx, serde_json::json!({"source":"absent"})).map_err(error)?;
    } else {
        // Never ATTACH or initialize V1: even accidental writes are rejected.
        let legacy = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(error)?;
        legacy.execute_batch("PRAGMA query_only=ON; BEGIN DEFERRED;").map_err(error)?;
        import(&tx, &legacy).map_err(error)?;
    }
    tx.commit().map_err(error)
}

fn finish(tx: &Transaction<'_>, report: serde_json::Value) -> rusqlite::Result<()> {
    tx.execute("INSERT INTO schema_meta(key,value) VALUES('full_legacy_import_v1_report',?1)
        ON CONFLICT(key) DO UPDATE SET value=excluded.value", [report.to_string()])?;
    tx.execute("INSERT INTO schema_meta(key,value) VALUES(?1,'1')", [MARKER])?;
    Ok(())
}
fn columns(c: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    let mut s = c.prepare(&format!("PRAGMA table_info({table})"))?;
    let result = s.query_map([], |r| r.get(1))?.collect();
    result
}
fn table(c: &Connection, name: &str) -> rusqlite::Result<bool> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [name], |r| r.get(0))
}
fn optional_column(cols: &HashSet<String>, name: &str, fallback: &str) -> String {
    if cols.contains(name) { name.to_owned() } else { fallback.to_owned() }
}
fn stable_id(prefix: &str, value: &str) -> String {
    // Match the scanner's project IDs, so subsequent scans reuse seeded groups.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}-{hash:016x}")
}

#[derive(Debug)]
struct Root { domain: String, id: String, path: String }
fn under(path: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    path == root || path.strip_prefix(root).is_some_and(|p| p.starts_with('/'))
}
fn resolve<'a>(roots: &'a [Root], id: &str, path: &str) -> Option<&'a Root> {
    let by_id: Vec<_> = roots.iter().filter(|r| r.id == id && under(path, &r.path)).collect();
    if by_id.len() == 1 { return Some(by_id[0]); }
    // Equal-length matches are ambiguous, not permission to pick a domain.
    let longest = roots.iter().filter(|r| under(path, &r.path))
        .map(|r| r.path.trim_end_matches('/').len()).max()?;
    let mut matches = roots.iter().filter(|r| under(path, &r.path)
        && r.path.trim_end_matches('/').len() == longest);
    let first = matches.next()?;
    if matches.next().is_some() { None } else { Some(first) }
}

/// A newer successful directory listing is positive evidence of a V2 deletion.
/// Ancestor snapshots matter too (an entire subdirectory may have disappeared).
fn deleted_since(c: &Connection, root: &Root, path: &str, scanned: i64) -> rusqlite::Result<bool> {
    let mut s = c.prepare("SELECT parent_path FROM provider_directory_snapshots
        WHERE domain=?1 AND root_id=?2 AND completed_at>=?3")?;
    let parents = s.query_map(params![root.domain,root.id,scanned], |r| r.get::<_,String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for parent in parents {
        if path == parent || !under(path, &parent) { continue; }
        let base = parent.trim_end_matches('/');
        let relative = path.strip_prefix(base).unwrap_or(path).trim_start_matches('/');
        let child = format!("{base}/{}", relative.split('/').next().unwrap_or(""));
        let entry: Option<bool> = c.query_row("SELECT is_directory<>0 FROM provider_directory_entries
            WHERE domain=?1 AND root_id=?2 AND parent_path=?3 AND remote_path=?4",
            params![root.domain,root.id,parent,child], |r| r.get(0)).optional()?;
        if entry.is_none() || (child != path && entry == Some(false)) || (child == path && entry == Some(true)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Shared by disk migration and focused in-memory tests. Caller owns one writer
/// transaction and a stable read snapshot of the legacy connection.
fn import(tx: &Transaction<'_>, old: &Connection) -> rusqlite::Result<()> {
    if done(tx)? { return Ok(()); }
    if !table(old, "assets")? {
        return finish(tx, serde_json::json!({"source":"no_assets_table"}));
    }
    for (t, name, definition) in [("assets","thumbnail_key","TEXT"),
        ("asset_state","favorite_at","INTEGER"),("asset_state","last_played_at","INTEGER")] {
        if !columns(tx,t)?.contains(name) {
            tx.execute_batch(&format!("ALTER TABLE {t} ADD COLUMN {name} {definition}"))?;
        }
    }
    let roots = {
        let mut s = tx.prepare("SELECT domain,root_id,remote_path FROM domain_roots")?;
        let rows = s.query_map([], |r| Ok(Root { domain:r.get(0)?,id:r.get(1)?,path:r.get(2)? }))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let cols = columns(old,"assets")?;
    // Earlier V1 schemas did not contain optional cache/date columns.
    let extra = ["etag","content_hash","thumbnail_asset_id","duration_seconds","modified_at","thumbnail_key"]
        .iter().map(|name| optional_column(&cols,name,"NULL")).collect::<Vec<_>>().join(",");
    let sql = format!("SELECT id,root_id,remote_path,file_name,parent_path,media_type,size_bytes,missing,scanned_at,{extra}
        FROM assets ORDER BY scanned_at DESC,id");
    let mut assets = old.prepare(&sql)?;
    let mut rows = assets.query([])?;
    let mut map = HashMap::<String,(String,String)>::new();
    let mut unmapped = Vec::<String>::new();
    let mut inserted = 0;
    let mut deleted = 0;
    let mut collisions = Vec::<String>::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let old_root: String = row.get(1)?;
        let path: String = row.get(2)?;
        let Some(root) = resolve(&roots,&old_root,&path) else { unmapped.push(id); continue; };
        let scanned: i64 = row.get(8)?;
        let canonical: Option<String> = tx.query_row("SELECT id FROM assets WHERE domain=?1 AND remote_path=?2",
            params![root.domain,path], |r| r.get(0)).optional()?;
        let canonical = if let Some(canonical) = canonical { canonical } else {
            if deleted_since(tx,root,&path,scanned)? { deleted += 1; continue; }
            // Keep IDs/cache references whenever possible; avoid discarding a row
            // merely because an unrelated V2 asset already uses its old ID.
            let occupied: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM assets WHERE id=?1)", [&id], |r|r.get(0))?;
            let new_id = if occupied { collisions.push(id.clone()); stable_id("legacy",&format!("{}|{}",root.domain,path)) } else { id.clone() };
            let missing: i64 = row.get(7)?;
            tx.execute("INSERT OR IGNORE INTO assets(id,domain,root_id,remote_path,file_name,parent_path,media_type,
                size_bytes,missing,missing_since,missing_scan_count,first_seen_at,last_seen_at,
                etag,content_hash,thumbnail_asset_id,duration_seconds,modified_at,thumbnail_key)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,?9,?10,?10,?11,?12,?13,?14,?15,?16)",
                params![new_id,root.domain,root.id,path,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,
                    row.get::<_,i64>(6)?,i64::from(missing != 0),scanned,row.get::<_,Option<String>>(9)?,
                    row.get::<_,Option<String>>(10)?,row.get::<_,Option<String>>(11)?,row.get::<_,Option<i64>>(12)?,
                    row.get::<_,Option<String>>(13)?,row.get::<_,Option<String>>(14)?])?;
            let canonical: Option<String> = tx.query_row("SELECT id FROM assets WHERE domain=?1 AND remote_path=?2",
                params![root.domain,path], |r|r.get(0)).optional()?;
            let Some(canonical) = canonical else { collisions.push(id); continue; };
            inserted += 1;
            canonical
        };
        map.insert(id,(canonical,root.domain.clone()));
    }
    drop(rows);
    drop(assets);
    import_state(tx,old,&map)?;
    import_vault_metadata(tx,old,&map)?;
    // Derived membership is additive: never reset triage, recommendations or
    // move a manually assigned pending item into another project.
    tx.execute("INSERT OR IGNORE INTO recent_items(asset_id,source,discovered_at)
        SELECT id,'migration',first_seen_at FROM assets WHERE domain='recent' AND missing=0", [])?;
    seed_pending(tx,&roots)?;
    finish(tx,serde_json::json!({"inserted_assets":inserted,"mapped_assets":map.len(),
        "unmapped_asset_ids":unmapped,"skipped_deleted_assets":deleted,"id_collisions":collisions,
        "notes":["Existing V2 assets and state rows win, including missing/unfavorite/cleared progress.",
            "Legacy missing flags retained; missing_since unknown (NULL), not fabricated.",
            "V2 newer directory snapshots suppress deleted legacy paths; deletions without tombstone or surviving snapshot cannot be detected.",
            "Original migrate_vault runs first; its writes cannot be distinguished from earlier V2 writes here.",
            "Only vault metadata is imported; non-vault legacy metadata has no canonical mapping in this migration.",
            "Recent/pending memberships are additive; existing project assignments/counts are not rebuilt."]}))
}

fn import_state(tx: &Transaction<'_>, old: &Connection, map: &HashMap<String,(String,String)>) -> rusqlite::Result<()> {
    let mut state: HashMap<String,(Option<i64>,Option<f64>,Option<i64>)> = HashMap::new();
    if table(old,"favorites")? {
        let mut s = old.prepare("SELECT asset_id,created_at FROM favorites ORDER BY created_at DESC,asset_id")?;
        for row in s.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?)))? {
            let (id,time) = row?;
            if let Some((canonical,_)) = map.get(&id) {
                let entry = state.entry(canonical.clone()).or_default();
                entry.0 = Some(entry.0.map_or(time,|t|t.max(time)));
            }
        }
    }
    if table(old,"playback_progress")? {
        let mut s = old.prepare("SELECT asset_id,position_seconds,updated_at FROM playback_progress ORDER BY updated_at DESC,asset_id")?;
        for row in s.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,f64>(1)?,r.get::<_,i64>(2)?)))? {
            let (id,position,time) = row?;
            if !position.is_finite() || position < 0.0 { continue; }
            if let Some((canonical,_)) = map.get(&id) {
                let entry = state.entry(canonical.clone()).or_default();
                if entry.2.is_none() { entry.1=Some(position); entry.2=Some(time); }
            }
        }
    }
    for (id,(favorite,progress,played)) in state {
        // Do not infer that a default zero/NULL in an existing row is unowned:
        // it may be an explicit unfavorite or a cleared playback tombstone.
        tx.execute("INSERT OR IGNORE INTO asset_state(asset_id,favorite,favorite_at,progress_seconds,last_played_at,updated_at)
            VALUES(?1,?2,?3,?4,?5,?6)",params![id,i64::from(favorite.is_some()),favorite,progress,played,
                favorite.into_iter().chain(played).max().unwrap_or(0)])?;
    }
    Ok(())
}

fn identity(tx: &Connection, old: &Connection, table: &str, id: Option<String>) -> rusqlite::Result<Option<String>> {
    let Some(id) = id else { return Ok(None); };
    if let Some(found) = tx.query_row(&format!("SELECT id FROM {table} WHERE id=?1"),[&id],|r|r.get(0)).optional()? { return Ok(Some(found)); }
    // The earlier vault importer uses OR IGNORE; unique-name conflicts can keep
    // a V2 identity under a different ID. Resolve that reference, not a dangling ID.
    if table != "collections" {
        if let Some(name) = old.query_row(&format!("SELECT name FROM {table} WHERE id=?1"),[id],|r|r.get::<_,String>(0)).optional()? {
            return tx.query_row(&format!("SELECT id FROM {table} WHERE name=?1 COLLATE NOCASE"),[name],|r|r.get(0)).optional();
        }
    }
    Ok(None)
}
fn import_vault_metadata(tx: &Transaction<'_>, old: &Connection, map: &HashMap<String,(String,String)>) -> rusqlite::Result<()> {
    if !table(old,"asset_metadata")? { return Ok(()); }
    let mut s = old.prepare("SELECT asset_id,category_id,author_id,collection_id,origin,updated_at FROM asset_metadata ORDER BY updated_at DESC,asset_id")?;
    let mut rows = s.query([])?;
    while let Some(row) = rows.next()? {
        let old_id: String = row.get(0)?;
        let Some((id,domain)) = map.get(&old_id) else { continue; };
        if domain != "vault" { continue; }
        tx.execute("INSERT OR IGNORE INTO vault_items(asset_id,category_id,author_id,collection_id,source,updated_at)
            VALUES(?1,?2,?3,?4,?5,?6)", params![id,identity(tx,old,"categories",row.get(1)?)?,
                identity(tx,old,"authors",row.get(2)?)?,identity(tx,old,"collections",row.get(3)?)?,
                row.get::<_,String>(4)?,row.get::<_,i64>(5)?])?;
    }
    Ok(())
}

fn seed_pending(tx: &Transaction<'_>, roots: &[Root]) -> rusqlite::Result<()> {
    let Some(root) = roots.iter().find(|r|r.domain=="pending") else { return Ok(()); };
    let mut s = tx.prepare("SELECT id,remote_path,first_seen_at FROM assets WHERE domain='pending' AND missing=0
        AND NOT EXISTS(SELECT 1 FROM pending_items p WHERE p.asset_id=assets.id)")?;
    let rows = s.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(s);
    let mut created = HashSet::new();
    for (id,path,time) in rows {
        // Root-ID classification is authoritative, but paths outside that root
        // cannot safely be grouped into a source folder.
        if !under(&path,&root.path) { continue; }
        let base = root.path.trim_end_matches('/');
        let relative = path.strip_prefix(base).unwrap_or("").trim_start_matches('/');
        let first = relative.split('/').next().unwrap_or("");
        let lower = first.to_lowercase();
        let folder = first.starts_with("【Pending") || first.starts_with("【pending")
            || lower.starts_with("pending") || first.starts_with("【待收纳");
        let name = if folder { first } else { "散装文件" };
        let source = if folder { format!("{base}/{first}") } else { base.to_owned() };
        let existing: Option<String> = tx.query_row("SELECT id FROM pending_projects WHERE root_id=?1 AND source_path=?2 ORDER BY id LIMIT 1",
            params![root.id,source],|r|r.get(0)).optional()?;
        let project = existing.unwrap_or_else(||stable_id("pending",&format!("{base}|{name}")));
        if tx.execute("INSERT OR IGNORE INTO pending_projects(id,name,source_path,root_id,is_loose,created_at,updated_at)
            VALUES(?1,?2,?3,?4,?5,?6,?6)",params![project,name,source,root.id,i64::from(!folder),time])? > 0 {
            created.insert(project.clone());
        }
        tx.execute("INSERT OR IGNORE INTO pending_items(asset_id,project_id,updated_at) VALUES(?1,?2,?3)",params![id,project,time])?;
    }
    for id in created {
        tx.execute("UPDATE pending_projects SET total_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=?1),
            triage_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=?1 AND assignment_status='triage') WHERE id=?1",[id])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn databases() -> (Connection,Connection) {
        let v2 = Connection::open_in_memory().unwrap();
        let source = include_str!("four_domains.rs");
        let schema = source.split("fn schema_sql() -> &'static str {").nth(1).unwrap()
            .split("r#\"").nth(1).unwrap().split("\"#").next().unwrap();
        v2.execute_batch(schema).unwrap();
        v2.execute_batch("PRAGMA foreign_keys=ON;
            INSERT INTO domain_roots VALUES('vault','v','/vault',0),('recent','r','/recent',0),
            ('pending','p','/pending',0),('comics','c','/comics',0);").unwrap();
        let old = Connection::open_in_memory().unwrap();
        old.execute_batch("CREATE TABLE assets(id TEXT PRIMARY KEY,root_id TEXT,remote_path TEXT,file_name TEXT,
            parent_path TEXT,media_type TEXT,size_bytes INTEGER,missing INTEGER,scanned_at INTEGER,
            content_hash TEXT,thumbnail_key TEXT,thumbnail_asset_id TEXT);
            CREATE TABLE favorites(asset_id TEXT,created_at INTEGER);
            CREATE TABLE playback_progress(asset_id TEXT,position_seconds REAL,updated_at INTEGER);").unwrap();
        (v2,old)
    }
    fn add(old: &Connection,id:&str,root:&str,path:&str,missing:i64) {
        old.execute("INSERT INTO assets VALUES(?1,?2,?3,'file.jpg','/','image',42,?4,10,'hash','cache','cover')",
            params![id,root,path,missing]).unwrap();
    }
    fn run(v2:&Connection,old:&Connection) {
        let tx=v2.unchecked_transaction().unwrap();
        import(&tx,old).unwrap(); tx.commit().unwrap();
    }
    #[test]
    fn all_domains_cache_missing_and_unmapped() {
        let (v2,old)=databases();
        for (id,root,path,missing) in [("v","v","/vault/a",0),("r","r","/recent/a",0),
            ("p","p","/pending/a",0),("c","old-root","/comics/a",1),("x","unknown","/else/a",0)] {
            add(&old,id,root,path,missing);
            old.execute("INSERT INTO favorites VALUES(?1,20)",[id]).unwrap();
            old.execute("INSERT INTO playback_progress VALUES(?1,7,30)",[id]).unwrap();
        }
        run(&v2,&old);
        let count:i64=v2.query_row("SELECT COUNT(*) FROM assets",[],|r|r.get(0)).unwrap();
        assert_eq!(count,4);
        let value:(String,String,String,i64,Option<i64>)=v2.query_row("SELECT content_hash,thumbnail_key,thumbnail_asset_id,missing,missing_since FROM assets WHERE id='c'",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(value,("hash".into(),"cache".into(),"cover".into(),1,None));
        assert_eq!(v2.query_row("SELECT COUNT(*) FROM asset_state WHERE favorite=1 AND progress_seconds=7 AND last_played_at=30",[],|r|r.get::<_,i64>(0)).unwrap(),4);
        assert_eq!(v2.query_row("SELECT COUNT(*) FROM pending_items",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(v2.query_row("SELECT COUNT(*) FROM recent_items",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        let report:String=v2.query_row("SELECT value FROM schema_meta WHERE key='full_legacy_import_v1_report'",[],|r|r.get(0)).unwrap();
        assert!(report.contains("\"x\""));
    }
    #[test]
    fn canonical_ids_existing_state_and_missing_win_and_marker_is_durable() {
        let (v2,old)=databases();
        add(&old,"old","r","/recent/a",0);
        old.execute_batch("INSERT INTO favorites VALUES('old',20); INSERT INTO playback_progress VALUES('old',9,30);").unwrap();
        v2.execute_batch("INSERT INTO assets(id,domain,root_id,remote_path,file_name,parent_path,media_type,missing,first_seen_at,last_seen_at)
            VALUES('new','recent','r','/recent/a','new-name','/recent','video',1,50,50);
            INSERT INTO asset_state(asset_id,favorite,reading_page,updated_at) VALUES('new',0,8,60);").unwrap();
        run(&v2,&old);
        let state:(i64,Option<f64>,i64)=v2.query_row("SELECT favorite,progress_seconds,reading_page FROM asset_state WHERE asset_id='new'",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(state,(0,None,8));
        assert_eq!(v2.query_row("SELECT missing FROM assets WHERE id='new'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        add(&old,"later","c","/comics/later",0);
        run(&v2,&old);
        assert_eq!(v2.query_row("SELECT COUNT(*) FROM assets",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    #[test]
    fn maps_state_by_path_and_does_not_resurrect_snapshot_deletions() {
        let (v2,old)=databases();
        add(&old,"old","r","/recent/a",0);
        add(&old,"deleted","p","/pending/removed/deep/a",0);
        old.execute_batch("INSERT INTO favorites VALUES('old',20);").unwrap();
        v2.execute_batch("INSERT INTO assets(id,domain,root_id,remote_path,file_name,parent_path,media_type,first_seen_at,last_seen_at)
            VALUES('canonical','recent','r','/recent/a','a','/recent','image',1,1);
            INSERT INTO provider_directory_snapshots VALUES('pending','p','/pending',20);").unwrap();
        run(&v2,&old);
        assert_eq!(v2.query_row("SELECT asset_id FROM asset_state",[],|r|r.get::<_,String>(0)).unwrap(),"canonical");
        assert_eq!(v2.query_row("SELECT COUNT(*) FROM assets",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
    #[test]
    fn longest_prefix_boundary_and_ambiguous_roots() {
        let roots=vec![Root{domain:"vault".into(),id:"shared".into(),path:"/a".into()},
            Root{domain:"recent".into(),id:"shared".into(),path:"/a/b".into()}];
        assert_eq!(resolve(&roots,"shared","/a/b/file").unwrap().domain,"recent");
        assert!(resolve(&roots,"unknown","/ab/file").is_none());
        assert_eq!(resolve(&roots,"unknown","/a/100%_file").unwrap().domain,"vault");
    }
    #[test]
    fn vault_metadata_resolves_canonical_identity_and_pending_is_additive() {
        let (v2,old)=databases();
        add(&old,"old-v","v","/vault/a",0);
        add(&old,"old-p","p","/pending/a",0);
        old.execute_batch("CREATE TABLE authors(id TEXT,name TEXT); INSERT INTO authors VALUES('old-author','Author');
            CREATE TABLE categories(id TEXT,name TEXT); INSERT INTO categories VALUES('old-cat','Category');
            CREATE TABLE collections(id TEXT,name TEXT);
            CREATE TABLE asset_metadata(asset_id TEXT,category_id TEXT,author_id TEXT,collection_id TEXT,origin TEXT,updated_at INTEGER);
            INSERT INTO asset_metadata VALUES('old-v','old-cat','old-author',NULL,'user',25);").unwrap();
        v2.execute_batch("INSERT INTO authors(id,name,origin,created_at) VALUES('author','Author','user',0);
            INSERT INTO categories(id,name,origin,created_at) VALUES('cat','Category','user',0);
            INSERT INTO assets(id,domain,root_id,remote_path,file_name,parent_path,media_type,first_seen_at,last_seen_at)
            VALUES('v','vault','v','/vault/a','a','/vault','image',0,0),('p','pending','p','/pending/a','a','/pending','image',0,0);
            INSERT INTO pending_projects(id,name,source_path,root_id,status,total_count,triage_count,ready_count,created_at,updated_at)
            VALUES('manual','Manual','/pending','p','custom',7,3,4,0,50);
            INSERT INTO pending_items(asset_id,project_id,manual_author_id,assignment_status,updated_at)
            VALUES('p','manual','chosen','ready',50);").unwrap();
        run(&v2,&old);
        let metadata:(String,String)=v2.query_row("SELECT author_id,category_id FROM vault_items WHERE asset_id='v'",[],
            |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(metadata,("author".into(),"cat".into()));
        let pending:(String,String,String,i64)=v2.query_row("SELECT i.project_id,i.manual_author_id,i.assignment_status,p.total_count
            FROM pending_items i JOIN pending_projects p ON p.id=i.project_id WHERE i.asset_id='p'",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(pending,("manual".into(),"chosen".into(),"ready".into(),7));
    }
    #[test]
    fn failure_rolls_back_assets_and_marker() {
        let (v2,old)=databases();
        add(&old,"a","r","/recent/a",0);
        old.execute_batch("DROP TABLE playback_progress; CREATE TABLE playback_progress(broken TEXT);").unwrap();
        { let tx=v2.unchecked_transaction().unwrap(); assert!(import(&tx,&old).is_err()); }
        assert!(!done(&v2).unwrap());
        assert_eq!(v2.query_row("SELECT COUNT(*) FROM assets",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
}
