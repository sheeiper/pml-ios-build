use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    pub asset_id: String,
    pub file_name: String,
    pub remote_path: String,
    pub size_bytes: i64,
    pub category: String, // "text" | "image" | "video"
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub deleted_count: usize,
    pub freed_bytes: i64,
}

/// Scan for advertisement and garbage files in library assets
pub fn scan_cleanup_items(conn: &Connection, root_path: Option<&str>) -> Result<Vec<CleanupItem>, rusqlite::Error> {
    let mut sql = "SELECT id, file_name, remote_path, size_bytes, media_type, duration_seconds
                   FROM assets
                   WHERE missing = 0".to_string();

    if let Some(rp) = root_path {
        let clean = rp.trim_end_matches('/');
        if !clean.is_empty() {
            sql.push_str(&format!(" AND remote_path LIKE '{}%'", clean));
        }
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let file_name: String = row.get(1)?;
        let remote_path: String = row.get(2)?;
        let size_bytes: i64 = row.get(3)?;
        let media_type: String = row.get(4)?;
        let duration_seconds: Option<i32> = row.get(5)?;
        Ok((id, file_name, remote_path, size_bytes, media_type, duration_seconds))
    })?;

    let mut cleanup_items = Vec::new();

    let text_keywords = [
        "最新地址", "永久发布", "永久地址", "备用", "网址", "解压密码",
        "防屏蔽", "发布页", "发布器", "点击访问", "免费看", "一手资源"
    ];

    let image_keywords = [
        "banner", "promo", "poster_ad", "ad_", "扫码", "关注", "飞机群", "电报群", "telegram"
    ];

    let video_ad_keywords = [
        "promo", "sample", "ad", "广告", "片头", "最新", "永久", "网址", "欢迎访问"
    ];

    for r in rows.flatten() {
        let (id, file_name, remote_path, size_bytes, media_type, duration_seconds) = r;
        let lower_name = file_name.to_lowercase();

        // 1. URL / Text garbage
        if lower_name.ends_with(".url") || lower_name.ends_with(".lnk") || lower_name.ends_with(".html") || lower_name.ends_with(".htm") {
            cleanup_items.push(CleanupItem {
                asset_id: id,
                file_name,
                remote_path,
                size_bytes,
                category: "text".to_string(),
                reason: "网页推广或跳转链接文件".to_string(),
            });
            continue;
        }

        if lower_name.ends_with(".txt") {
            for kw in &text_keywords {
                if lower_name.contains(kw) {
                    cleanup_items.push(CleanupItem {
                        asset_id: id,
                        file_name: file_name.clone(),
                        remote_path: remote_path.clone(),
                        size_bytes,
                        category: "text".to_string(),
                        reason: format!("推广文本（含关键词【{}】）", kw),
                    });
                    break;
                }
            }
            continue;
        }

        // 2. Image Banner Ad
        if media_type == "image" {
            let mut is_ad = false;
            let mut reason = String::new();
            for kw in &image_keywords {
                if lower_name.contains(kw) {
                    is_ad = true;
                    reason = format!("广告水印或海报横幅（含关键词【{}】）", kw);
                    break;
                }
            }
            if !is_ad && size_bytes > 0 && size_bytes < 50 * 1024 {
                if lower_name.contains("ad") || lower_name.contains("tuiguang") || lower_name.contains("qun") {
                    is_ad = true;
                    reason = "超小体积广告引导图".to_string();
                }
            }
            if is_ad {
                cleanup_items.push(CleanupItem {
                    asset_id: id,
                    file_name,
                    remote_path,
                    size_bytes,
                    category: "image".to_string(),
                    reason,
                });
                continue;
            }
        }

        // 3. Short video advertisement bumper
        if media_type == "video" {
            if let Some(dur) = duration_seconds {
                if dur > 0 && dur <= 25 {
                    for kw in &video_ad_keywords {
                        if lower_name.contains(kw) {
                            cleanup_items.push(CleanupItem {
                                asset_id: id,
                                file_name: file_name.clone(),
                                remote_path: remote_path.clone(),
                                size_bytes,
                                category: "video".to_string(),
                                reason: format!("时长仅 {} 秒且带有推广关键词【{}】的片头广告切片", dur, kw),
                            });
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(cleanup_items)
}

/// Delete selected garbage items from the database index and mark as removed
pub fn delete_cleanup_items(conn: &mut Connection, asset_ids: &[String]) -> Result<CleanupReport, rusqlite::Error> {
    if asset_ids.is_empty() {
        return Ok(CleanupReport { deleted_count: 0, freed_bytes: 0 });
    }

    let tx = conn.transaction()?;
    let mut freed_bytes = 0i64;
    let mut deleted_count = 0usize;

    for id in asset_ids {
        let size: Option<i64> = tx.query_row(
            "SELECT size_bytes FROM assets WHERE id = ?1",
            params![id],
            |row| row.get(0)
        ).optional()?;

        if let Some(bytes) = size {
            freed_bytes += bytes;
            deleted_count += 1;
            tx.execute("DELETE FROM assets WHERE id = ?1", params![id])?;
            tx.execute("DELETE FROM asset_state WHERE asset_id = ?1", params![id])?;
        }
    }

    tx.commit()?;
    Ok(CleanupReport {
        deleted_count,
        freed_bytes,
    })
}
