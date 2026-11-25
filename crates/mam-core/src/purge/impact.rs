//! 폴더 제거 확인 Dialog 용 영향 개수 (프론트 계약 `FolderRemovalImpact`). 읽기 전용.

use rusqlite::{Connection, params};
use serde::Serialize;

use crate::keyrange::key_range;

/// 폴더를 제거하면 잃는 항목 수 (계약 `FolderRemovalImpact`).
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FolderRemovalImpact {
    /// 폴더 범위의 활성 미디어 수
    pub media_count: i64,
    /// 태그 연결 수
    pub tag_links: i64,
    /// 메모가 있는 미디어 수
    pub notes: i64,
    /// 컬렉션 소속 수
    pub collection_items: i64,
    /// AI 분석 결과(설명·키워드·OCR 요약 또는 음성 인식 원문)가 있는 미디어 수
    pub ai_results: i64,
}

/// 범위 안 활성 위치를 가진 활성 자산 (자산당 활성 위치는 최대 1개라 중복이 없다)
const SCOPE: &str = "WITH s AS (
    SELECT l.asset_id AS asset_id FROM file_locations l
    JOIN media_assets a ON a.asset_id = l.asset_id AND a.status = 'active'
    WHERE l.volume_id = ?1 AND l.detached_at IS NULL AND l.path_key >= ?2 AND l.path_key < ?3)";

/// `volume_id` 의 `root_key` 하위 활성 자산이 지워질 때 잃는 항목 수.
pub fn folder_removal_impact(
    conn: &Connection,
    volume_id: i64,
    root_key: &str,
) -> rusqlite::Result<FolderRemovalImpact> {
    let (lo, hi) = key_range(root_key);
    let count = |select: &str| -> rusqlite::Result<i64> {
        conn.query_row(
            &format!("{SCOPE} {select}"),
            params![volume_id, lo, hi],
            |r| r.get(0),
        )
    };
    Ok(FolderRemovalImpact {
        media_count: count("SELECT COUNT(*) FROM s")?,
        tag_links: count("SELECT COUNT(*) FROM s JOIN asset_tags t ON t.asset_id = s.asset_id")?,
        notes: count(
            "SELECT COUNT(*) FROM s JOIN asset_user_data u ON u.asset_id = s.asset_id WHERE u.memo <> ''",
        )?,
        collection_items: count(
            "SELECT COUNT(*) FROM s JOIN collection_assets c ON c.asset_id = s.asset_id",
        )?,
        ai_results: count(
            "SELECT COUNT(*) FROM s
             WHERE EXISTS (SELECT 1 FROM ai_analyses x WHERE x.asset_id = s.asset_id)
                OR EXISTS (SELECT 1 FROM transcripts x WHERE x.asset_id = s.asset_id)",
        )?,
    })
}
