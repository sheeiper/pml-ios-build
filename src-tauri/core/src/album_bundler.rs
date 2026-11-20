use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumItem {
    pub file_name: String,
    pub asset_id: String,
    pub remote_path: String,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumInfo {
    pub dir_path: String,
    pub title: String,
    pub image_count: usize,
    pub total_size_bytes: i64,
    pub cover_asset_id: Option<String>,
    pub preview_asset_ids: Vec<String>,
    pub items: Vec<AlbumItem>,
}

/// Query images in a directory (and immediate subdirectories) to form a bundled Album view
pub fn get_album_details(conn: &Connection, dir_path: &str) -> Result<AlbumInfo, rusqlite::Error> {
    let clean_dir = dir_path.trim_end_matches('/');
    let title = clean_dir.rsplit_once('/').map(|(_, name)| name).unwrap_or(clean_dir).to_string();
    let dir_with_slash = format!("{}/", clean_dir);
    let prefix_like = format!("{}%", dir_with_slash);

    let mut stmt = conn.prepare(
        "SELECT id, file_name, remote_path, size_bytes
         FROM assets
         WHERE (parent_path = ?1 OR parent_path = ?2 OR remote_path LIKE ?3)
           AND (media_type = 'image' OR media_type = 'comicPage' OR file_name LIKE '%.jpg' OR file_name LIKE '%.jpeg' OR file_name LIKE '%.png' OR file_name LIKE '%.webp')
           AND missing = 0
         ORDER BY remote_path ASC, file_name ASC"
    )?;

    let rows = stmt.query_map(params![clean_dir, dir_with_slash, prefix_like], |row| {
        Ok(AlbumItem {
            asset_id: row.get(0)?,
            file_name: row.get(1)?,
            remote_path: row.get(2)?,
            size_bytes: row.get(3)?,
        })
    })?;

    let mut items = Vec::new();
    let mut total_size_bytes = 0i64;
    for r in rows.flatten() {
        total_size_bytes += r.size_bytes;
        items.push(r);
    }

    let image_count = items.len();
    let cover_asset_id = items.first().map(|i| i.asset_id.clone());
    let preview_asset_ids = items.iter().take(6).map(|i| i.asset_id.clone()).collect();

    Ok(AlbumInfo {
        dir_path: clean_dir.to_string(),
        title,
        image_count,
        total_size_bytes,
        cover_asset_id,
        preview_asset_ids,
        items,
    })
}
