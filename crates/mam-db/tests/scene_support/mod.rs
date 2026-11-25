//! 장면 컷 테스트 공용 헬퍼 (T-scene-04 ~ 15).
#![allow(dead_code)]

use rusqlite::Connection;

pub const NOW: i64 = 1_700_000_000_000;

/// 활성 영상 자산 (+ 메타데이터가 있으면 길이·fps)
pub fn add_video(conn: &Connection, id: i64, duration_ms: Option<i64>, fps: Option<f64>) {
    add_asset(conn, id, "video", "active");
    if duration_ms.is_some() || fps.is_some() {
        conn.execute(
            "INSERT INTO media_metadata (asset_id, duration_ms, fps) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, duration_ms, fps],
        )
        .unwrap();
    }
}

pub fn add_asset(conn: &Connection, id: i64, media_type: &str, status: &str) {
    conn.execute(
        "INSERT INTO media_assets (asset_id, media_type, status, file_size, modified_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, 10, 0, 0, 0)",
        rusqlite::params![id, media_type, status],
    )
    .unwrap();
}

pub fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

/// `jobs` 의 scene 작업 `(priority, status)` 목록 (job_id 순)
pub fn scene_jobs(conn: &Connection, asset_id: i64) -> Vec<(i64, String)> {
    let mut stmt = conn
        .prepare("SELECT priority, status FROM jobs WHERE kind = 'scene' AND asset_id = ?1 ORDER BY job_id")
        .unwrap();
    stmt.query_map([asset_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
