//! Reconcile an already completed provider move against the canonical V2 index.
//! No provider operations (or direct legacy database opens) belong here.

use std::{collections::HashSet, time::{SystemTime, UNIX_EPOCH}};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use crate::{four_domains, AppHandle};

pub(crate) fn reconcile(app: &AppHandle, source: &str, target: &str) -> Result<(), String> {
    let failure = |error| format!("Physical move already completed, but V2 index reconciliation failed: {error}");
    four_domains::ensure_v2(app).map_err(failure)?;
    let mut connection = four_domains::open_v2(app).map_err(failure)?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?.as_secs() as i64;
    reconcile_connection(&mut connection, source, target, timestamp)
        .map_err(|error| format!("Physical move already completed, but V2 index reconciliation failed: {error}"))
}

#[derive(Debug)]
struct Root { domain: String, id: String, path: String }

fn normalize(path: &str) -> Result<String, String> {
    if !path.starts_with('/') || path.split('/').any(|part| part == "." || part == "..") {
        return Err(format!("Expected an absolute provider path without dot segments: {path}"));
    }
    let path = path.trim_end_matches('/');
    Ok(if path.is_empty() { "/".into() } else { path.into() })
}

/// Boundary checks and suffix mapping operate on Rust strings, never SQLite's
/// character offsets versus Rust byte offsets (important for non-ASCII names).
fn within(path: &str, base: &str) -> bool {
    path == base || if base == "/" { path.starts_with('/') }
    else { path.strip_prefix(base).is_some_and(|suffix| suffix.starts_with('/')) }
}

fn map_path(path: &str, source: &str, target: &str) -> Result<String, String> {
    if !within(path, source) { return Err(format!("Path outside moved subtree: {path}")); }
    let suffix = path.strip_prefix(source).ok_or_else(|| "Invalid source prefix".to_string())?;
    if suffix.is_empty() { return Ok(target.into()); }
    Ok(format!("{}/{}", target.trim_end_matches('/'), suffix.trim_start_matches('/')))
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(base, _)| if base.is_empty() { "/" } else { base }).unwrap_or("/")
}

fn resolve<'a>(roots: &'a [Root], path: &str) -> Option<&'a Root> {
    roots.iter().filter(|root| within(path, &root.path)).max_by_key(|root| root.path.len())
}

fn reconcile_connection(connection: &mut Connection, source: &str, target: &str, now: i64) -> Result<(), String> {
    let source = normalize(source)?;
    let target = normalize(target)?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    reconcile_transaction(&tx, &source, &target, now)?;
    tx.commit().map_err(|e| e.to_string())
}

fn reconcile_transaction(tx: &Transaction<'_>, source: &str, target: &str, now: i64) -> Result<(), String> {
    let roots = {
        let mut statement = tx.prepare("SELECT domain,root_id,remote_path FROM domain_roots").map_err(|e| e.to_string())?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
            .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        rows.into_iter().map(|(domain, id, path)| Ok(Root { domain, id, path: normalize(&path)? }))
            .collect::<Result<Vec<_>, String>>()?
    };
    let destination = resolve(&roots, target).ok_or_else(|| format!(
        "Physical move completed but target is unindexed: no configured V2 domain root contains {target}"))?;
    let origin = resolve(&roots, source);
    let assets = if let Some(origin) = origin {
        let mut statement = tx.prepare("SELECT id,remote_path FROM assets WHERE domain=?1").map_err(|e| e.to_string())?;
        let rows = statement.query_map(params![origin.domain], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
        rows.into_iter().filter(|(_, path)| within(path, source)).collect::<Vec<_>>()
    } else { Vec::new() };
    let mut moves = Vec::new();
    // Preflight every destination before changing any row, including collisions
    // with a different moved row when target overlaps source.
    for (id, path) in &assets {
        let mapped = map_path(path, source, target)?;
        if resolve(&roots, &mapped).map(|root| root.domain.as_str()) != Some(destination.domain.as_str()) {
            return Err(format!("Moved subtree crosses a nested domain root: {mapped}"));
        }
        let conflict: Option<String> = tx.query_row("SELECT id FROM assets WHERE domain=?1 AND remote_path=?2",
            params![destination.domain, mapped], |row| row.get(0)).optional().map_err(|e| e.to_string())?;
        if conflict.as_deref().is_some_and(|other| other != id.as_str()) {
            return Err(format!("Target index conflict at {mapped}; existing identity was preserved"));
        }
        moves.push((id, mapped));
    }
    let mut projects = HashSet::new();
    for (id, mapped) in &moves {
        let project: Option<String> = tx.query_row("SELECT project_id FROM pending_items WHERE asset_id=?1", params![id], |row| row.get(0))
            .optional().map_err(|e| e.to_string())?;
        if let Some(project) = &project { projects.insert(project.clone()); }
        if destination.domain == "vault" {
            tx.execute("INSERT INTO vault_items(asset_id,category_id,author_id,confidence,source,updated_at)
                SELECT asset_id,COALESCE(manual_category_id,recommended_category_id),
                    COALESCE(manual_author_id,recommended_author_id),confidence,'user',?2
                FROM pending_items WHERE asset_id=?1
                ON CONFLICT(asset_id) DO UPDATE SET category_id=excluded.category_id,
                    author_id=excluded.author_id,confidence=excluded.confidence,source='user',updated_at=excluded.updated_at",
                params![id, now]).map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO vault_items(asset_id,source,updated_at) VALUES(?1,'user',?2)
                ON CONFLICT(asset_id) DO NOTHING", params![id, now]).map_err(|e| e.to_string())?;
        } else {
            tx.execute("DELETE FROM vault_items WHERE asset_id=?1", params![id]).map_err(|e| e.to_string())?;
        }
        if destination.domain != "pending" {
            tx.execute("DELETE FROM pending_items WHERE asset_id=?1", params![id]).map_err(|e| e.to_string())?;
        }
        if destination.domain == "recent" {
            tx.execute("INSERT INTO recent_items(asset_id,source,discovered_at) VALUES(?1,'user',?2)
                ON CONFLICT(asset_id) DO NOTHING", params![id, now]).map_err(|e| e.to_string())?;
        } else {
            tx.execute("DELETE FROM recent_items WHERE asset_id=?1", params![id]).map_err(|e| e.to_string())?;
        }
        // comic_pages reference comic_archives with ON DELETE CASCADE. Keep the
        // manifest/pages intact for moves within comics; remove on domain exit.
        if destination.domain == "comics" {
            tx.execute("INSERT INTO comic_archives(asset_id,page_count,updated_at)
                SELECT id,0,?2 FROM assets WHERE id=?1 AND media_type='comicArchive'
                ON CONFLICT(asset_id) DO NOTHING", params![id, now]).map_err(|e| e.to_string())?;
        } else {
            tx.execute("DELETE FROM comic_archives WHERE asset_id=?1", params![id]).map_err(|e| e.to_string())?;
        }
        tx.execute("UPDATE assets SET domain=?2,root_id=?3,remote_path=?4,parent_path=?5,file_name=?6,
            last_seen_at=?7,missing=0,missing_since=NULL,missing_scan_count=0 WHERE id=?1",
            params![id, destination.domain, destination.id, mapped, parent(mapped), mapped.rsplit('/').next().unwrap_or(""), now])
            .map_err(|e| e.to_string())?;
        if destination.domain == "pending" && project.is_none() {
            // Match rebuild_pending_projects_v2 exactly, including its stable
            // project identity, without replacing existing manual assignments.
            let (project, name, source_path, is_loose) = pending_project(&destination.path, mapped);
            tx.execute("INSERT INTO pending_projects(id,name,source_path,root_id,is_loose,status,created_at,updated_at)
                VALUES(?1,?2,?3,?4,?5,'discovered',?6,?6) ON CONFLICT(id) DO NOTHING",
                params![project, name, source_path, destination.id, is_loose, now]).map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO pending_items(asset_id,project_id,assignment_status,updated_at)
                VALUES(?1,?2,'triage',?3) ON CONFLICT(asset_id) DO NOTHING", params![id, project, now]).map_err(|e| e.to_string())?;
            projects.insert(project);
        }
    }
    for project in projects {
        tx.execute("UPDATE pending_projects SET
            total_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=?1),
            triage_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=?1 AND assignment_status='triage' AND conflict_reason IS NULL),
            ready_count=(SELECT COUNT(*) FROM pending_items WHERE project_id=?1 AND assignment_status='ready' AND conflict_reason IS NULL),updated_at=?2 WHERE id=?1",
            params![project, now]).map_err(|e| e.to_string())?;
    }
    // Independent of asset enumeration: moving an empty directory must expire
    // authoritative empty snapshots as well. Rust boundary lists avoid LIKE's
    // wildcard and case-folding behavior for %, _, backslash and Unicode paths.
    for root in &roots {
        for path in [source, target] {
            if within(path, &root.path) || within(&root.path, path) {
                invalidate(tx, root, path)?;
            }
        }
    }
    // An unknown source can legitimately contain no indexed assets (e.g. a
    // provider-only folder). Never synthesize asset identities from cache rows.
    Ok(())
}

fn pending_project(root: &str, path: &str) -> (String, String, String, bool) {
    let root = root.trim_end_matches('/');
    let relative = path.strip_prefix(root).unwrap_or(path).trim_start_matches('/');
    let first = relative.split('/').next().unwrap_or("");
    let lower = first.to_lowercase();
    let grouped = first.starts_with("【Pending") || first.starts_with("【pending")
        || lower.starts_with("pending") || first.starts_with("【待收纳");
    let name = if grouped { first.to_owned() } else { "散装文件".to_owned() };
    let source = if grouped { format!("{root}/{name}") } else { root.to_owned() };
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in format!("{root}|{name}").as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (format!("pending-{hash:016x}"), name, source, !grouped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        // Exercise the real canonical schema without widening its visibility.
        let source = include_str!("four_domains.rs");
        let schema = source.split_once("fn schema_sql()").unwrap().1
            .split_once("r#\"").unwrap().1.split_once("\"#").unwrap().0;
        db.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        db.execute_batch(schema).unwrap();
        db.execute_batch("INSERT INTO domain_roots VALUES
            ('pending','p','/待整理',0),('vault','v','/资料',0),
            ('recent','r','/最近',0),('comics','c','/资料/漫画',0);
            INSERT INTO pending_projects(id,name,source_path,root_id,created_at,updated_at)
            VALUES('project','project','/待整理','p',0,0);").unwrap();
        db
    }

    fn asset(db: &Connection, id: &str, domain: &str, root: &str, path: &str) {
        db.execute("INSERT INTO assets(id,domain,root_id,remote_path,parent_path,file_name,media_type,first_seen_at,last_seen_at)
            VALUES(?1,?2,?3,?4,?5,?6,'video',1,1)",
            params![id,domain,root,path,parent(path),path.rsplit('/').next().unwrap()]).unwrap();
    }

    fn cache(db: &Connection, domain: &str, root: &str, path: &str) {
        db.execute("INSERT INTO provider_directory_snapshots VALUES(?1,?2,?3,1)",params![domain,root,path]).unwrap();
        db.execute("INSERT INTO provider_directory_entries VALUES(?1,?2,?3,?4,'x',1,0,NULL,NULL)",
            params![domain,root,path,format!("{path}/x")]).unwrap();
        db.execute("INSERT INTO directory_sync_state(root_id,remote_path,scanned_at) VALUES(?1,?2,1)",params![root,path]).unwrap();
    }

    #[test]
    fn unicode_pending_to_vault_preserves_identity_state_and_assignments() {
        let mut db = database();
        asset(&db,"a","pending","p","/待整理/作者/影片.mp4");
        db.execute_batch("INSERT INTO pending_items(asset_id,project_id,manual_category_id,recommended_author_id,assignment_status,updated_at)
            VALUES('a','project','manual-category','recommended-author','ready',1);
            INSERT INTO asset_state(asset_id,favorite,progress_seconds,updated_at) VALUES('a',1,42,1);
            UPDATE assets SET missing=1,missing_since=1,missing_scan_count=2 WHERE id='a';").unwrap();
        cache(&db,"pending","p","/待整理");
        cache(&db,"vault","v","/资料");
        reconcile_connection(&mut db,"/待整理/作者","/资料/新作者",99).unwrap();
        let row: (String,String,String,i64,i64) = db.query_row("SELECT domain,root_id,remote_path,missing,last_seen_at FROM assets WHERE id='a'",[],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(row,("vault".into(),"v".into(),"/资料/新作者/影片.mp4".into(),0,99));
        let metadata: (String,String,String) = db.query_row("SELECT category_id,author_id,source FROM vault_items WHERE asset_id='a'",[],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(metadata,("manual-category".into(),"recommended-author".into(),"user".into()));
        assert_eq!(db.query_row("SELECT favorite+progress_seconds FROM asset_state WHERE asset_id='a'",[],|r|r.get::<_,f64>(0)).unwrap(),43.0);
        assert_eq!(db.query_row("SELECT total_count FROM pending_projects WHERE id='project'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        // move_pending_item's follow-up sees a safely deleted membership; retries
        // do not replace identity or wipe already transferred metadata.
        reconcile_connection(&mut db,"/待整理/作者","/资料/新作者",100).unwrap();
        assert_eq!(db.query_row("SELECT COUNT(*) FROM pending_items",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }

    #[test]
    fn collision_rolls_back_all_changes_and_cache_expiry() {
        let mut db = database();
        asset(&db,"a","pending","p","/待整理/f/a");
        asset(&db,"b","pending","p","/待整理/f/b");
        asset(&db,"conflict","vault","v","/资料/f/b");
        cache(&db,"pending","p","/待整理");
        assert!(reconcile_connection(&mut db,"/待整理/f","/资料/f",99).unwrap_err().contains("conflict"));
        assert_eq!(db.query_row("SELECT domain FROM assets WHERE id='a'",[],|r|r.get::<_,String>(0)).unwrap(),"pending");
        assert_eq!(db.query_row("SELECT COUNT(*) FROM provider_directory_snapshots",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }

    #[test]
    fn empty_directory_expires_ancestors_and_subtrees_not_wildcard_siblings() {
        let mut db = database();
        for path in ["/待整理","/待整理/空%_\\","/待整理/空%_\\/子", "/待整理/空XX"] {
            cache(&db,"pending","p",path);
        }
        cache(&db,"vault","v","/资料");
        cache(&db,"vault","v","/资料/目标");
        reconcile_connection(&mut db,"/待整理/空%_\\","/资料/目标",99).unwrap();
        for table in ["provider_directory_entries","provider_directory_snapshots","directory_sync_state"] {
            assert_eq!(db.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),1);
        }
    }

    #[test]
    fn entering_pending_matches_scan_project_identity_and_preserves_assignments() {
        let mut db = database();
        asset(&db,"a","recent","r","/最近/a.mp4");
        reconcile_connection(&mut db,"/最近/a.mp4","/待整理/【Pending甲】/a.mp4",99).unwrap();
        let expected = pending_project("/待整理","/待整理/【Pending甲】/a.mp4");
        let actual: (String,String,String,bool,i64) = db.query_row("SELECT p.id,p.name,p.source_path,p.is_loose,p.total_count
            FROM pending_projects p JOIN pending_items i ON p.id=i.project_id WHERE i.asset_id='a'",[],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(actual,(expected.0.clone(),expected.1,expected.2,false,1));
        db.execute("UPDATE pending_items SET manual_author_id='assigned',assignment_status='ready' WHERE asset_id='a'",[]).unwrap();
        reconcile_connection(&mut db,"/待整理/【Pending甲】/a.mp4","/待整理/【Pending乙】/a.mp4",100).unwrap();
        let assignment: (String,String) = db.query_row("SELECT project_id,manual_author_id FROM pending_items WHERE asset_id='a'",[],
            |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(assignment,(expected.0,"assigned".into()));
        asset(&db,"loose","recent","r","/最近/loose.mp4");
        reconcile_connection(&mut db,"/最近/loose.mp4","/待整理/普通/loose.mp4",101).unwrap();
        let loose = pending_project("/待整理","/待整理/普通/loose.mp4");
        assert_eq!(db.query_row("SELECT project_id FROM pending_items WHERE asset_id='loose'",[],|r|r.get::<_,String>(0)).unwrap(),loose.0);
    }

    #[test]
    fn comic_images_do_not_get_archive_membership() {
        let mut db = database();
        asset(&db,"page","recent","r","/最近/page.jpg");
        db.execute("UPDATE assets SET media_type='image' WHERE id='page'",[]).unwrap();
        reconcile_connection(&mut db,"/最近/page.jpg","/资料/漫画/page.jpg",99).unwrap();
        assert_eq!(db.query_row("SELECT COUNT(*) FROM comic_archives",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT domain FROM assets WHERE id='page'",[],|r|r.get::<_,String>(0)).unwrap(),"comics");
    }

    #[test]
    fn longest_root_comics_membership_and_unknown_target() {
        let mut db = database();
        asset(&db,"a","recent","r","/最近/a.zip");
        db.execute("UPDATE assets SET media_type='comicArchive' WHERE id='a'",[]).unwrap();
        db.execute("INSERT INTO recent_items(asset_id,discovered_at) VALUES('a',1)",[]).unwrap();
        let error = reconcile_connection(&mut db,"/最近/a.zip","/资料外/a.zip",99).unwrap_err();
        assert!(error.contains("Physical move completed but target is unindexed"));
        reconcile_connection(&mut db,"/最近/a.zip","/资料/漫画/a.zip",99).unwrap();
        assert_eq!(db.query_row("SELECT domain FROM assets WHERE id='a'",[],|r|r.get::<_,String>(0)).unwrap(),"comics");
        db.execute("INSERT INTO comic_pages(archive_asset_id,page_index,entry_name) VALUES('a',0,'page')",[]).unwrap();
        reconcile_connection(&mut db,"/资料/漫画/a.zip","/资料/漫画/b.zip",100).unwrap();
        assert_eq!(db.query_row("SELECT COUNT(*) FROM comic_pages",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        reconcile_connection(&mut db,"/资料/漫画/b.zip","/最近/b.zip",101).unwrap();
        assert_eq!(db.query_row("SELECT COUNT(*) FROM comic_pages",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM recent_items",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    }
}

fn invalidate(tx: &Transaction<'_>, root: &Root, moved: &str) -> Result<(), String> {
    for (table, column, has_domain) in [
        ("provider_directory_entries", "parent_path", true),
        ("provider_directory_snapshots", "parent_path", true),
        ("directory_sync_state", "remote_path", false),
    ] {
        let sql = if has_domain { format!("SELECT DISTINCT {column} FROM {table} WHERE root_id=?1 AND domain=?2") }
            else { format!("SELECT DISTINCT {column} FROM {table} WHERE root_id=?1") };
        let mut statement = tx.prepare(&sql).map_err(|e| e.to_string())?;
        let mut rows = if has_domain { statement.query(params![root.id, root.domain]) } else { statement.query(params![root.id]) }.map_err(|e| e.to_string())?;
        let mut paths = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let path: String = row.get(0).map_err(|e| e.to_string())?;
            if within(&path, moved) || within(moved, &path) { paths.push(path); }
        }
        drop(rows);
        drop(statement);
        for path in paths {
            let sql = if has_domain { format!("DELETE FROM {table} WHERE root_id=?1 AND {column}=?2 AND domain=?3") }
                else { format!("DELETE FROM {table} WHERE root_id=?1 AND {column}=?2") };
            if has_domain { tx.execute(&sql, params![root.id, path, root.domain]) } else { tx.execute(&sql, params![root.id, path]) }.map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
