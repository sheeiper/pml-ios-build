//! Explicit V2 implementations of the legacy catalog API. No V1 tables or views.
//! The response types remain owned by `media_library`.
use rusqlite::{params_from_iter, types::Value, Connection, Row};
use crate::{four_domains, AppHandle};
use crate::media_library::{LibraryAsset, LibraryAuthor, LibraryCategory, LibraryCollection,
    LibraryOverview, LibraryRootKind, ShortVideoFeedPage};

fn database(app: &AppHandle) -> Result<Connection, String> {
    four_domains::ensure_v2(app)?;
    four_domains::open_v2(app)
}
fn error(e: rusqlite::Error) -> String { format!("V2 catalog: {e}") }

// Match the legacy persisted ordering, including arbitrary-length numeric runs
// and RFC2822 WebDAV dates. Source columns detect scanner edits on every sorted
// request; this is deliberately not a process-wide OnceLock migration.
fn natural_name_key(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut chars = lower.chars().peekable();
    let mut key = String::new();
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() {
            let mut digits = String::from(ch);
            while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                digits.push(chars.next().unwrap());
            }
            let value = digits.trim_start_matches('0');
            let value = if value.is_empty() { "0" } else { value };
            key.push_str(&format!("\u{1}{:010}:{}\u{2}", value.len(), value));
        } else { key.push(ch); }
    }
    key
}
fn modified_timestamp(value: Option<&str>) -> i64 {
    value.and_then(|v| chrono::DateTime::parse_from_rfc3339(v)
        .or_else(|_| chrono::DateTime::parse_from_rfc2822(v)).ok())
        .map(|v| v.timestamp_millis()).unwrap_or(i64::MIN)
}

/// Refresh derived sort keys only for new/changed names or dates. The write
/// transaction serializes concurrent schema initialization and scan updates.
/// Scanners may optionally maintain these four columns themselves; otherwise
/// lazy refresh preserves correctness without sorting the whole result in Rust.
pub(crate) fn refresh_sort_keys(connection: &Connection) -> Result<(), String> {
    let tx = connection.unchecked_transaction().map_err(error)?;
    // Acquire the writer lock before inspecting columns (avoids concurrent ALTER).
    tx.execute("UPDATE assets SET id=id WHERE 0", []).map_err(error)?;
    let existing = {
        let mut statement = tx.prepare("PRAGMA table_info(assets)").map_err(error)?;
        let rows = statement.query_map([], |r| r.get::<_, String>(1)).map_err(error)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(error)?
    };
    for (name, definition) in [
        ("name_sort", "TEXT NOT NULL DEFAULT ''"),
        ("modified_ts", "INTEGER NOT NULL DEFAULT -9223372036854775808"),
        ("catalog_sort_file_name", "TEXT"),
        ("catalog_sort_modified_at", "TEXT"),
    ] {
        if !existing.iter().any(|column| column == name) {
            tx.execute_batch(&format!("ALTER TABLE assets ADD COLUMN {name} {definition}")).map_err(error)?;
        }
    }
    let dirty = {
        let mut statement = tx.prepare("SELECT id,file_name,modified_at FROM assets
          WHERE catalog_sort_file_name IS NOT file_name OR catalog_sort_modified_at IS NOT modified_at").map_err(error)?;
        let rows = statement.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).map_err(error)?;
        rows.collect::<Result<Vec<_>,_>>().map_err(error)?
    };
    {
        let mut update = tx.prepare("UPDATE assets SET name_sort=?1,modified_ts=?2,
          catalog_sort_file_name=?3,catalog_sort_modified_at=?4 WHERE id=?5").map_err(error)?;
        for (id,name,date) in dirty {
            update.execute(rusqlite::params![natural_name_key(&name),modified_timestamp(date.as_deref()),name,date,id]).map_err(error)?;
        }
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS assets_catalog_recent ON assets(missing,modified_ts DESC,name_sort,id);
      CREATE INDEX IF NOT EXISTS assets_catalog_size ON assets(missing,size_bytes DESC,name_sort,id);").map_err(error)?;
    tx.commit().map_err(error)
}

const JOINS: &str = "FROM assets asset
 LEFT JOIN vault_items metadata ON metadata.asset_id = asset.id
 LEFT JOIN categories category ON category.id = metadata.category_id
 LEFT JOIN authors author ON author.id = metadata.author_id
 LEFT JOIN collections collection ON collection.id = metadata.collection_id";
const COLUMNS: &str = "asset.id, asset.root_id, asset.domain, asset.remote_path,
 asset.file_name, asset.parent_path, asset.media_type, asset.size_bytes,
 asset.duration_seconds, metadata.category_id, category.name, metadata.author_id,
 author.name, metadata.collection_id, collection.name, metadata.source,
 asset.content_hash, asset.thumbnail_asset_id";
const VISIBLE_PATH: &str = "asset.remote_path NOT LIKE '%inbox%' AND asset.remote_path NOT LIKE '%暂存%'
 AND asset.remote_path NOT LIKE '%未分类%' AND asset.remote_path NOT LIKE '%pending%'";
fn asset_row(row: &Row<'_>) -> rusqlite::Result<LibraryAsset> {
    let domain: String = row.get(2)?;
    Ok(LibraryAsset {
        id: row.get(0)?, root_id: row.get(1)?,
        root_kind: if domain == "comics" { LibraryRootKind::Comics } else { LibraryRootKind::Media },
        remote_path: row.get(3)?, file_name: row.get(4)?, parent_path: row.get(5)?,
        media_type: row.get(6)?, size_bytes: row.get(7)?, duration_seconds: row.get(8)?,
        category_id: row.get(9)?, category_name: row.get(10)?, author_id: row.get(11)?,
        author_name: row.get(12)?, collection_id: row.get(13)?, collection_name: row.get(14)?,
        metadata_origin: row.get(15)?, content_hash: row.get(16)?, thumbnail_asset_id: row.get(17)?,
    })
}
fn assets(connection: &Connection, clauses: &[String], values: Vec<Value>, order: &str) -> Result<Vec<LibraryAsset>, String> {
    let sql = format!("SELECT {COLUMNS} {JOINS} WHERE {} {order}", clauses.join(" AND "));
    let mut stmt = connection.prepare(&sql).map_err(error)?;
    let rows = stmt.query_map(params_from_iter(values), asset_row).map_err(error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(error)
}
fn media_filter(filter: Option<&str>) -> Result<&'static str, String> {
    match filter.unwrap_or("all") {
        "all" => Ok("asset.domain != 'comics'"),
        "images" => Ok("asset.domain != 'comics' AND asset.media_type = 'image'"),
        "videos" | "shortVideos" => Ok("asset.domain != 'comics' AND asset.media_type = 'video'"),
        "comics" => Ok("asset.domain = 'comics'"),
        _ => Err("不支持的媒体筛选条件".into()),
    }
}
fn category_filter(category: Option<String>, values: &mut Vec<Value>) -> String {
    match category.filter(|s| !s.is_empty()) {
        Some(s) if s == "uncategorized" => "metadata.category_id IS NULL".into(),
        Some(s) => {
            values.push(s.into());
            "metadata.category_id IN (WITH RECURSIVE sub(id) AS (SELECT id FROM categories WHERE id = ? UNION SELECT c.id FROM categories c JOIN sub ON c.parent_id = sub.id) SELECT id FROM sub)".into()
        }
        None => "1 = 1".into(),
    }
}

pub fn list_library_assets(
    app: AppHandle, filter: Option<String>, query: Option<String>, author_id: Option<String>,
    collection_id: Option<String>, category_id: Option<String>, limit: Option<i64>,
    offset: Option<i64>, parent_path: Option<String>,
) -> Result<Vec<LibraryAsset>, String> {
    list_assets_sql(&database(&app)?, filter, query, author_id, collection_id, category_id, limit, offset, parent_path)
}
fn list_assets_sql(
    connection: &Connection, filter: Option<String>, query: Option<String>, author_id: Option<String>,
    collection_id: Option<String>, category_id: Option<String>, limit: Option<i64>,
    offset: Option<i64>, parent_path: Option<String>,
) -> Result<Vec<LibraryAsset>, String> {
    let filter = filter.unwrap_or_else(|| "all".into());
    let mut clauses = vec!["asset.missing = 0".into()];
    let mut values = Vec::new();
    if let Some(path) = parent_path.filter(|v| !v.trim().is_empty()) {
        clauses.push("asset.parent_path = ?".into()); values.push(path.into());
    }
    let predicate = match filter.as_str() {
        "all" | "images" | "videos" | "shortVideos" | "comics" => media_filter(Some(&filter))?,
        "recent" => "asset.domain != 'comics'",
        "unplayed" => "asset.domain != 'comics' AND NOT EXISTS (SELECT 1 FROM asset_state watched WHERE watched.asset_id = asset.id AND watched.progress_seconds IS NOT NULL)",
        "missingCovers" => "asset.domain != 'comics' AND asset.media_type IN ('image','video') AND asset.thumbnail_asset_id IS NULL",
        "largeFiles" => "asset.domain != 'comics' AND asset.size_bytes >= 104857600",
        "longVideos" => "asset.domain != 'comics' AND asset.media_type = 'video' AND asset.duration_seconds >= 600",
        // A duplicate group requires both matching fingerprint and size.
        "duplicates" => "asset.domain != 'comics' AND asset.content_hash IS NOT NULL AND EXISTS (SELECT 1 FROM assets duplicate WHERE duplicate.missing=0 AND duplicate.content_hash=asset.content_hash AND duplicate.size_bytes=asset.size_bytes AND duplicate.id<>asset.id)",
        _ => return Err("不支持的媒体筛选条件".into()),
    };
    clauses.push(predicate.into());
    if let Some(q) = query.filter(|v| !v.trim().is_empty()) {
        clauses.push("(asset.file_name LIKE ? OR asset.remote_path LIKE ? OR author.name LIKE ? OR collection.name LIKE ?)".into());
        for _ in 0..4 { values.push(format!("%{}%", q.trim()).into()); }
    }
    if let Some(id) = author_id.filter(|v| !v.is_empty()) {
        if id == "unassigned" { clauses.push("metadata.author_id IS NULL".into()); }
        else { clauses.push("metadata.author_id = ?".into()); values.push(id.into()); }
    }
    if let Some(id) = collection_id.filter(|v| !v.is_empty()) {
        if id == "unassigned" { clauses.push("metadata.collection_id IS NULL".into()); }
        else {
            clauses.push("metadata.collection_id IN (WITH RECURSIVE sub(id) AS (SELECT id FROM collections WHERE id = ? UNION SELECT c.id FROM collections c JOIN sub ON c.parent_id=sub.id) SELECT id FROM sub)".into());
            values.push(id.into());
        }
    }
    clauses.push(category_filter(category_id, &mut values));
    values.push(limit.unwrap_or(1000).clamp(1, 5000).into());
    values.push(offset.unwrap_or(0).max(0).into());
    if matches!(filter.as_str(), "recent" | "largeFiles" | "longVideos") {
        refresh_sort_keys(connection)?;
    }
    let order = match filter.as_str() {
        "recent" => "ORDER BY asset.modified_ts DESC, asset.name_sort, asset.id LIMIT ? OFFSET ?",
        "largeFiles" | "longVideos" => "ORDER BY asset.size_bytes DESC, asset.name_sort, asset.id LIMIT ? OFFSET ?",
        _ => "ORDER BY asset.file_name COLLATE NOCASE, asset.id LIMIT ? OFFSET ?",
    };
    assets(connection, &clauses, values, order)
}

/// Deterministic keyset pagination; the transport's cursor remains an opaque string.
/// Old v8 random tokens had no position, so they start a new traversal.
pub fn list_short_video_feed(app: AppHandle, cursor: Option<String>, limit: Option<i64>,
    author_id: Option<String>, max_duration_seconds: Option<i64>) -> Result<ShortVideoFeedPage, String> {
    feed_sql(&database(&app)?, cursor, limit, author_id, max_duration_seconds)
}
fn feed_sql(connection: &Connection, cursor: Option<String>, limit: Option<i64>,
    author_id: Option<String>, max_duration_seconds: Option<i64>) -> Result<ShortVideoFeedPage, String> {
    let limit = limit.unwrap_or(24).clamp(1,48) as usize;
    let mut clauses = vec!["asset.missing=0 AND asset.media_type='video'".into(),
        "(asset.file_name LIKE '%.mp4' OR asset.file_name LIKE '%.webm' OR asset.file_name LIKE '%.mov' OR asset.file_name LIKE '%.m4v')".into()];
    let mut values = Vec::new();
    if let Some(id) = author_id.filter(|v| !v.trim().is_empty()) {
        clauses.push("metadata.author_id = ?".into()); values.push(id.into());
    } else { clauses.push(VISIBLE_PATH.into()); }
    if let Some(max) = max_duration_seconds.filter(|v| *v > 0) {
        clauses.push("(asset.duration_seconds IS NULL OR asset.duration_seconds <= ?)".into()); values.push(max.into());
    }
    if let Some(cursor) = cursor {
        if let Some(id) = cursor.strip_prefix("v2catalog|") {
            clauses.push("asset.id > ?".into()); values.push(id.to_string().into());
        } // Legacy v8 and other old tokens were ignored by the old implementation.
    }
    values.push(((limit+1) as i64).into());
    let mut items = assets(connection, &clauses, values, "ORDER BY asset.id LIMIT ?")?;
    let has_more = items.len() > limit;
    items.truncate(limit);
    for item in &mut items { item.metadata_origin = None; }
    let next_cursor = if has_more { items.last().map(|a| format!("v2catalog|{}",a.id)) } else { None };
    let stream_sources = if items.is_empty() { None } else {
        Some(items.iter().map(|a| (a.id.clone(),format!("/media/{}",a.id))).collect())
    };
    Ok(ShortVideoFeedPage { items, next_cursor, stream_sources })
}

pub fn list_library_authors(app: AppHandle, category_id: Option<String>, filter: Option<String>) -> Result<Vec<LibraryAuthor>, String> {
    authors_sql(&database(&app)?, category_id, filter)
}
fn authors_sql(connection: &Connection, category_id: Option<String>, filter: Option<String>) -> Result<Vec<LibraryAuthor>, String> {
    let filter = media_filter(filter.as_deref())?;
    let mut values = Vec::new();
    let category = category_filter(category_id, &mut values);
    let sql = format!("SELECT author.id,author.name,author.aliases_json,author.avatar_asset_id,author.banner_asset_id,
      COUNT(DISTINCT asset.id),
      (SELECT a.id FROM assets a JOIN vault_items m ON m.asset_id=a.id
       WHERE m.author_id=author.id AND a.missing=0
       AND a.remote_path NOT LIKE '%inbox%' AND a.remote_path NOT LIKE '%暂存%'
       AND a.remote_path NOT LIKE '%未分类%' AND a.remote_path NOT LIKE '%pending%'
       ORDER BY (a.media_type='image') DESC,a.rowid DESC LIMIT 1)
      {JOINS} WHERE asset.missing=0 AND author.id IS NOT NULL AND metadata.author_id IS NOT NULL
      AND {VISIBLE_PATH} AND ({category}) AND ({filter})
      GROUP BY author.id HAVING COUNT(DISTINCT asset.id)>0
      ORDER BY COUNT(DISTINCT asset.id) DESC,author.name COLLATE NOCASE");
    let mut stmt = connection.prepare(&sql).map_err(error)?;
    let rows = stmt.query_map(params_from_iter(values.iter()), |r| {
        let avatar: Option<String> = r.get(3)?;
        let latest: Option<String> = r.get(6)?;
        let aliases: String = r.get(2)?;
        Ok(LibraryAuthor { id:r.get(0)?,name:r.get(1)?,aliases:serde_json::from_str(&aliases).unwrap_or_default(),
            media_count:r.get(5)?,collection_count:0,avatar_asset_id:avatar.or_else(||latest.clone()),
            cover_asset_id:latest,banner_asset_id:r.get(4)? })
    }).map_err(error)?;
    let mut result = rows.collect::<Result<Vec<_>,_>>().map_err(error)?;
    let sql = format!("SELECT EXISTS(SELECT 1 {JOINS} WHERE asset.missing=0 AND metadata.author_id IS NULL AND ({category}) AND ({filter}) LIMIT 1)");
    let unassigned: bool = connection.query_row(&sql, params_from_iter(values.iter()), |r|r.get(0)).map_err(error)?;
    if unassigned { result.push(LibraryAuthor { id:"unassigned".into(),name:"未识别作者".into(),aliases:vec![],media_count:0,collection_count:0,avatar_asset_id:None,cover_asset_id:None,banner_asset_id:None }); }
    Ok(result)
}

fn natural_category_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn extract(s:&str)->Option<(u64,&str)> {
        let s=s.trim(); let end=s.bytes().take_while(|c|c.is_ascii_digit()).count();
        if end==0 { return None; }
        Some((s[..end].parse().ok()?,s[end..].trim_start_matches(|c:char| matches!(c,'-'|'_'|'.'|' '|'、'))))
    }
    match (extract(a),extract(b)) {
        (Some((an,ar)),Some((bn,br)))=>an.cmp(&bn).then_with(||ar.cmp(br)),
        (Some(_),None)=>std::cmp::Ordering::Less,
        (None,Some(_))=>std::cmp::Ordering::Greater,
        _=>a.cmp(b),
    }
}
pub fn list_library_categories(app:AppHandle,filter:Option<String>,parent_id:Option<String>)->Result<Vec<LibraryCategory>,String> {
    categories_sql(&database(&app)?,filter,parent_id)
}
fn categories_sql(connection:&Connection,filter:Option<String>,parent_id:Option<String>)->Result<Vec<LibraryCategory>,String> {
    let parent=parent_id.filter(|s|!s.is_empty());
    let values:Vec<Value>=parent.iter().cloned().map(Value::from).collect();
    let parent_clause=if parent.is_some() { "category.parent_id=?" } else { "category.parent_id IS NULL" };
    // Preserve legacy behavior: real category counts are not media-filtered.
    let sql=format!("WITH RECURSIVE target AS (SELECT id,name,parent_id FROM categories category WHERE {parent_clause}),
      tree(root_id,id) AS (SELECT id,id FROM target UNION SELECT t.root_id,c.id FROM categories c JOIN tree t ON c.parent_id=t.id)
      SELECT t.id,t.name,t.parent_id,COUNT(DISTINCT a.id),COUNT(DISTINCT m.author_id),COUNT(DISTINCT m.collection_id)
      FROM target t LEFT JOIN tree tr ON tr.root_id=t.id LEFT JOIN vault_items m ON m.category_id=tr.id
      LEFT JOIN assets a ON a.id=m.asset_id AND a.missing=0 GROUP BY t.id ORDER BY t.name COLLATE NOCASE");
    let mut stmt=connection.prepare(&sql).map_err(error)?;
    let rows=stmt.query_map(params_from_iter(values),|r|Ok(LibraryCategory { id:r.get(0)?,name:r.get(1)?,parent_id:r.get(2)?,media_count:r.get(3)?,author_count:r.get(4)?,collection_count:r.get(5)? })).map_err(error)?;
    let mut result=rows.collect::<Result<Vec<_>,_>>().map_err(error)?;
    result.sort_by(|a,b|natural_category_cmp(&a.name,&b.name));
    if parent.is_none() {
        let filter=media_filter(filter.as_deref())?;
        let sql=format!("SELECT COUNT(DISTINCT asset.id),COUNT(DISTINCT metadata.author_id) {JOINS} WHERE asset.missing=0 AND metadata.category_id IS NULL AND {filter}");
        let (media_count,author_count):(i64,i64)=connection.query_row(&sql,[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(error)?;
        if media_count>0 { result.push(LibraryCategory { id:"uncategorized".into(),name:"未分类".into(),parent_id:None,media_count,author_count,collection_count:0 }); }
    }
    Ok(result)
}

pub fn list_library_collections(app:AppHandle,author_id:Option<String>,parent_id:Option<String>,category_id:Option<String>)->Result<Vec<LibraryCollection>,String> {
    collections_sql(&database(&app)?,author_id,parent_id,category_id)
}
fn collections_sql(connection:&Connection,author_id:Option<String>,parent_id:Option<String>,category_id:Option<String>)->Result<Vec<LibraryCollection>,String> {
    let mut clauses: Vec<String> = vec!["asset.missing=0 AND asset.domain != 'comics'".into()];
    let mut values=Vec::<Value>::new();
    if let Some(id)=author_id.filter(|s|!s.is_empty()) {
        if id=="unassigned" { clauses.push("collection.author_id IS NULL".into()); }
        else { clauses.push("collection.author_id=?".into());values.push(id.into()); }
    }
    clauses.push(category_filter(category_id,&mut values));
    if let Some(id)=parent_id.filter(|s|!s.is_empty()) { clauses.push("collection.parent_id=?".into());values.push(id.into()); }
    else { clauses.push("collection.parent_id IS NULL".into()); }
    let sql=format!("WITH RECURSIVE descendants(ancestor_id,id) AS (SELECT id,id FROM collections UNION SELECT d.ancestor_id,c.id FROM collections c JOIN descendants d ON c.parent_id=d.id)
      SELECT DISTINCT collection.id,collection.name,collection.author_id,author.name,collection.root_id
      FROM collections collection JOIN descendants d ON d.ancestor_id=collection.id
      JOIN vault_items metadata ON metadata.collection_id=d.id JOIN assets asset ON asset.id=metadata.asset_id
      LEFT JOIN authors author ON author.id=collection.author_id WHERE {} ORDER BY collection.name COLLATE NOCASE",clauses.join(" AND "));
    let mut stmt=connection.prepare(&sql).map_err(error)?;
    let rows=stmt.query_map(params_from_iter(values),|r|Ok(LibraryCollection { id:r.get(0)?,name:r.get(1)?,author_id:r.get(2)?,author_name:r.get(3)?,root_id:r.get(4)?,media_count:0,image_count:0,video_count:0,comic_count:0 })).map_err(error)?;
    rows.collect::<Result<Vec<_>,_>>().map_err(error)
}

pub fn get_library_overview(app:AppHandle)->Result<LibraryOverview,String> { overview_sql(&database(&app)?) }
fn overview_sql(connection:&Connection)->Result<LibraryOverview,String> {
    // State rows can exist for favorites only; only non-NULL progress means played.
    // V2 directory snapshots record completed listings including empty folders.
    connection.query_row("SELECT (SELECT COUNT(*) FROM domain_roots),COUNT(*),
      COALESCE(SUM(media_type='image'),0),COALESCE(SUM(media_type='video'),0),COALESCE(SUM(media_type='video'),0),
      COALESCE(SUM(media_type IN ('comicPage','comicArchive','pdf')),0),
      (SELECT COUNT(*) FROM authors),(SELECT COUNT(*) FROM collections),COALESCE(SUM(size_bytes),0),
      (SELECT COUNT(*) FROM provider_directory_snapshots),
      (SELECT COUNT(*) FROM assets a WHERE a.missing=0 AND NOT EXISTS(SELECT 1 FROM asset_state s WHERE s.asset_id=a.id AND s.progress_seconds IS NOT NULL)),
      COALESCE(SUM(media_type IN ('image','video') AND thumbnail_asset_id IS NULL),0),
      (SELECT MAX(completed_at) FROM provider_directory_snapshots),
      (SELECT COUNT(*) FROM (SELECT content_hash,size_bytes FROM assets WHERE missing=0 AND content_hash IS NOT NULL GROUP BY content_hash,size_bytes HAVING COUNT(*)>1))
      FROM assets WHERE missing=0",[],|r|Ok(LibraryOverview {
        roots:r.get(0)?,assets:r.get(1)?,images:r.get(2)?,videos:r.get(3)?,short_videos:r.get(4)?,comics:r.get(5)?,authors:r.get(6)?,collections:r.get(7)?,total_bytes:r.get(8)?,directories:r.get(9)?,unplayed:r.get(10)?,missing_thumbnails:r.get(11)?,last_scan_time:r.get(12)?,duplicate_groups:r.get(13)?
    })).map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        // Exercise the actual canonical schema without needing an AppHandle or
        // initialization/migration touching a user's persistent database.
        let source = include_str!("four_domains.rs");
        let schema = source.split("fn schema_sql() -> &'static str {").nth(1).unwrap()
            .split("r#\"").nth(1).unwrap().split("\"#").next().unwrap();
        connection.execute_batch(schema).unwrap();
        connection
    }
    fn add(c: &Connection, id: &str, domain: &str, media: &str, name: &str, path: &str) {
        c.execute("INSERT INTO assets(id,domain,root_id,remote_path,file_name,parent_path,media_type,first_seen_at,last_seen_at) VALUES(?1,?2,'root',?3,?4,'/',?5,0,0)",rusqlite::params![id,domain,path,name,media]).unwrap();
    }
    fn listing(c: &Connection, filter: &str) -> Vec<LibraryAsset> {
        list_assets_sql(c,Some(filter.into()),None,None,None,None,None,None,None).unwrap()
    }
    #[test]
    fn media_and_comics_are_separate_but_recent_and_pending_remain_media() {
        let c=db();
        add(&c,"v","vault","image","v.jpg","/v.jpg");
        add(&c,"r","recent","video","r.mp4","/r.mp4");
        add(&c,"p","pending","image","p.jpg","/p.jpg");
        add(&c,"c","comics","image","c.jpg","/c.jpg");
        assert_eq!(listing(&c,"all").len(),3);
        assert_eq!(listing(&c,"images").len(),2);
        assert_eq!(listing(&c,"comics")[0].id,"c");
        assert_eq!(listing(&c,"shortVideos")[0].id,"r");
        c.execute("INSERT INTO asset_state(asset_id,favorite,updated_at) VALUES('v',1,0)",[]).unwrap();
        assert_eq!(listing(&c,"unplayed").len(),3);
        c.execute("UPDATE asset_state SET progress_seconds=0 WHERE asset_id='v'",[]).unwrap();
        assert_eq!(listing(&c,"unplayed").len(),2);
        assert_eq!(overview_sql(&c).unwrap().unplayed,3);
    }
    #[test]
    fn duplicates_require_matching_size_and_fingerprint() {
        let c=db();
        for id in ["a","b","c"] { add(&c,id,"vault","video",&format!("{id}.mp4"),&format!("/{id}.mp4")); }
        c.execute("UPDATE assets SET content_hash='same',size_bytes=10",[]).unwrap();
        c.execute("UPDATE assets SET size_bytes=20 WHERE id='c'",[]).unwrap();
        let mut ids=listing(&c,"duplicates").into_iter().map(|a|a.id).collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids,vec!["a","b"]);
    }
    #[test]
    fn feed_pagination_is_deterministic_and_does_not_repeat() {
        let c=db();
        for id in ["a","b","c","d","e"] { add(&c,id,"vault","video",&format!("{id}.mp4"),&format!("/{id}.mp4")); }
        add(&c,"z","pending","video","z.mp4","/pending/z.mp4");
        add(&c,"x","vault","video","x.mkv","/x.mkv");
        let one=feed_sql(&c,None,Some(2),None,None).unwrap();
        let same=feed_sql(&c,None,Some(2),None,None).unwrap();
        assert_eq!(one.items.iter().map(|a|&a.id).collect::<Vec<_>>(),same.items.iter().map(|a|&a.id).collect::<Vec<_>>());
        let two=feed_sql(&c,one.next_cursor,Some(2),None,None).unwrap();
        let three=feed_sql(&c,two.next_cursor,Some(2),None,None).unwrap();
        let ids=one.items.into_iter().chain(two.items).chain(three.items).map(|a|a.id).collect::<Vec<_>>();
        assert_eq!(ids,vec!["a","b","c","d","e"]);
        assert!(three.next_cursor.is_none());
        assert_eq!(feed_sql(&c,Some("v8|123".into()),Some(2),None,None).unwrap().items[0].id,"a");
    }
    #[test]
    fn feed_cursor_uses_id_not_filename_and_binds_author_duration() {
        let c=db();
        c.execute_batch("INSERT INTO authors(id,name,origin,created_at) VALUES('a','Author','user',0)").unwrap();
        // Filenames collide ignoring case and contain SQL/cursor delimiters;
        // pagination must use the exact stable id, never a split filename token.
        for (id,name,duration) in [("a|one","same|name.MP4",Some(10)),("b' OR 1=1 --","SAME|NAME.mp4",None),("c","third.mp4",Some(90))] {
            add(&c,id,"pending","video",name,&format!("/pending/{id}.mp4"));
            c.execute("UPDATE assets SET duration_seconds=? WHERE id=?",rusqlite::params![duration,id]).unwrap();
            c.execute("INSERT INTO vault_items(asset_id,author_id,source,updated_at) VALUES(?,'a','user',0)",rusqlite::params![id]).unwrap();
        }
        let first=feed_sql(&c,None,Some(1),Some("a".into()),Some(30)).unwrap();
        assert_eq!(first.items[0].id,"a|one");
        let second=feed_sql(&c,first.next_cursor,Some(1),Some("a".into()),Some(30)).unwrap();
        assert_eq!(second.items[0].id,"b' OR 1=1 --");
        assert!(second.next_cursor.is_none());
        assert!(feed_sql(&c,None,None,Some("a' OR 1=1 --".into()),None).unwrap().items.is_empty());
        assert!(feed_sql(&c,None,None,None,None).unwrap().items.is_empty());
    }
    #[test]
    fn list_assets_binds_all_optional_parameters_in_order() {
        let c=db();
        c.execute_batch("INSERT INTO authors(id,name,origin,created_at) VALUES('a','Author','user',0);
          INSERT INTO categories(id,name,origin,created_at) VALUES('cat','Category','user',0);
          INSERT INTO collections(id,root_id,name,origin,created_at) VALUES('col','root','Collection','user',0)").unwrap();
        for id in ["a","b","c"] {
            add(&c,id,"vault","image",&format!("{id}.jpg"),&format!("/{id}.jpg"));
            c.execute("INSERT INTO vault_items(asset_id,author_id,category_id,collection_id,source,updated_at) VALUES(?,'a','cat','col','user',0)",rusqlite::params![id]).unwrap();
        }
        let result=list_assets_sql(&c,Some("images".into()),Some("Author".into()),Some("a".into()),Some("col".into()),Some("cat".into()),Some(1),Some(1),Some("/".into())).unwrap();
        assert_eq!(result.len(),1); assert_eq!(result[0].id,"b");
        let result=list_assets_sql(&c,None,Some("' OR 1=1 --".into()),None,None,None,None,None,None).unwrap();
        assert!(result.is_empty());
        assert_eq!(listing(&c,"images")[0].id,"a");
    }
    #[test]
    fn sorted_filters_preserve_rfc_dates_subseconds_and_natural_names_after_updates() {
        let c=db();
        for (id,name,date) in [
            ("a","clip10.mp4",Some("2024-01-01T00:00:00.100Z")),
            ("b","clip2.mp4",Some("Mon, 01 Jan 2024 00:00:00 +0000")),
            ("c","clip1.mp4",Some("2024-01-01T00:00:00.200Z")),
            ("d","clip99999999999999999999999999.mp4",None),
        ] {
            add(&c,id,"vault","video",name,&format!("/{id}.mp4"));
            c.execute("UPDATE assets SET modified_at=?,size_bytes=104857600,duration_seconds=700 WHERE id=?",rusqlite::params![date,id]).unwrap();
        }
        let ids=|items:Vec<LibraryAsset>|items.into_iter().map(|a|a.id).collect::<Vec<_>>();
        assert_eq!(ids(listing(&c,"recent")),vec!["c","a","b","d"]);
        assert_eq!(ids(listing(&c,"largeFiles")),vec!["c","b","a","d"]);
        assert_eq!(ids(listing(&c,"longVideos")),vec!["c","b","a","d"]);
        c.execute("UPDATE assets SET file_name='clip0.mp4',modified_at='2025-01-01T00:00:00Z' WHERE id='d'",[]).unwrap();
        assert_eq!(ids(listing(&c,"recent")),vec!["d","c","a","b"]);
        assert_eq!(ids(listing(&c,"largeFiles")),vec!["d","c","b","a"]);
        c.execute("UPDATE assets SET modified_at=NULL WHERE id='d'",[]).unwrap();
        assert_eq!(ids(listing(&c,"recent")),vec!["c","a","b","d"]);
        // Refresh is idempotent and leaves unaffected rows untouched.
        refresh_sort_keys(&c).unwrap();
        let dirty:i64=c.query_row("SELECT COUNT(*) FROM assets WHERE catalog_sort_file_name IS NOT file_name OR catalog_sort_modified_at IS NOT modified_at",[],|r|r.get(0)).unwrap();
        assert_eq!(dirty,0);
        assert!(natural_name_key("clip2")<natural_name_key("clip10"));
        assert!(natural_name_key("clip10")<natural_name_key("clip999999999999999999999999999"));
    }
    #[test]
    fn vault_metadata_aliases_and_recursive_collection_category_queries() {
        let c=db();
        c.execute_batch("INSERT INTO authors(id,name,aliases_json,origin,created_at) VALUES('author','A','[\"alias\"]','user',0);
          INSERT INTO categories(id,name,parent_id,origin,created_at) VALUES('cat','2. Category',NULL,'user',0),('child','Nested','cat','user',0);
          INSERT INTO collections(id,root_id,author_id,name,parent_id,origin,created_at) VALUES('col','root','author','Parent',NULL,'user',0),('leaf','root','author','Leaf','col','user',0);").unwrap();
        add(&c,"a","vault","image","a.jpg","/a.jpg");
        c.execute_batch("INSERT INTO vault_items(asset_id,category_id,author_id,collection_id,source,updated_at) VALUES('a','child','author','leaf','user',0)").unwrap();
        let authors=authors_sql(&c,Some("cat".into()),Some("images".into())).unwrap();
        assert_eq!(authors.len(),1); assert_eq!(authors[0].aliases,vec!["alias"]);
        assert_eq!(authors[0].cover_asset_id.as_deref(),Some("a"));
        assert!(authors_sql(&c,None,Some("videos".into())).unwrap().is_empty());
        let categories=categories_sql(&c,None,None).unwrap();
        assert_eq!(categories[0].media_count,1);
        let collections=collections_sql(&c,None,None,Some("cat".into())).unwrap();
        assert_eq!(collections[0].id,"col");
        assert_eq!(collections[0].media_count,0); // intentional legacy DTO semantics
        let assets=list_assets_sql(&c,None,None,None,Some("col".into()),Some("cat".into()),None,None,None).unwrap();
        assert_eq!(assets[0].metadata_origin.as_deref(),Some("user"));
    }
}
