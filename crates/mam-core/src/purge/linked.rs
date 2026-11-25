//! 연결 데이터 삭제 엔진. 호출자의 트랜잭션 안에서 실행한다 — 자산 변경과 `search_outbox` 는 같은 트랜잭션 (db.md §3).

use rusqlite::{Connection, params};

/// 대량 정리의 outbox 우선순위 (사용자 편집 0 보다 낮다)
const BULK_PRIORITY: i64 = 1;

/// `asset_id` 열로 자산에 매인 행을 **지우는** 테이블.
/// 지우지 않는 것: `tags`(태그 이름)·`collections`(컬렉션 자체)·`ai_usage`(사용량 이력 — 자산 삭제 시 `asset_id` 만 끊긴다)·
/// `asset_relations`·`media_assets`(소프트 삭제로 남김)·`search_outbox`·`index_progress`(색인 워커가 쓴다).
/// `transcript_segments` 는 `transcripts` 를 거쳐 따로 지운다.
pub const LINKED_TABLES: &[&str] = &[
    "asset_tags",
    "collection_assets",
    "asset_user_data",
    "asset_play_ranges",
    "asset_resume_positions",
    "asset_scene_cuts",
    "asset_metadata_values",
    "media_metadata",
    "asset_derivatives",
    "embeddings",
    "ai_analyses",
    "ai_analysis_replaced",
    "ai_task_state",
    "analysis_runs",
    "plugin_kv",
    "plugin_tasks",
    "jobs",
];

/// `asset_ids` 의 연결 데이터 행을 지우고, 활성 자산은 `deleted` 로 바꿔 revision 을 올리며 `delete` 이벤트를 남긴다.
/// 이미 `deleted`·`superseded` 인 자산은 연결 데이터만 지운다 (상태·revision·이벤트는 그대로). 반복 호출해도 안전하다 (멱등).
/// 호출자가 "다른 곳에 활성 위치가 있는 자산" 을 미리 뺀 목록만 넘겨야 한다.
///
/// 지운 임베딩의 (종류, 모델 ID, 버전, 차원) 를 중복 없이 돌려준다 - 벡터 활성 맵 캐시를 무효화할 키들이다.
pub fn purge_assets(
    conn: &Connection,
    now: i64,
    asset_ids: &[i64],
) -> rusqlite::Result<Vec<EmbeddingKey>> {
    if asset_ids.is_empty() {
        return Ok(Vec::new());
    }
    // 집합 단위 삭제를 위해 임시 테이블에 모은다 (쓰기 연결 하나뿐이라 연결 간 충돌이 없다)
    conn.execute(
        "CREATE TEMP TABLE IF NOT EXISTS purge_batch (asset_id INTEGER PRIMARY KEY) WITHOUT ROWID",
        [],
    )?;
    conn.execute("DELETE FROM purge_batch", [])?;
    {
        let mut ins = conn.prepare_cached("INSERT OR IGNORE INTO purge_batch (asset_id) VALUES (?1)")?;
        for id in asset_ids {
            ins.execute([id])?;
        }
    }
    const SET: &str = "asset_id IN (SELECT asset_id FROM purge_batch)";
    let keys: Vec<EmbeddingKey> = {
        let mut stmt = conn.prepare(&format!(
            "SELECT DISTINCT vector_kind, model_id, model_version, dimension FROM embeddings WHERE {SET}"
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok(EmbeddingKey {
                kind: r.get(0)?,
                model_id: r.get(1)?,
                model_version: r.get(2)?,
                dimension: r.get(3)?,
            })
        })?;
        rows.collect::<Result<_, _>>()?
    };
    conn.execute(
        &format!(
            "DELETE FROM transcript_segments WHERE transcript_id IN
               (SELECT transcript_id FROM transcripts WHERE {SET})"
        ),
        [],
    )?;
    conn.execute(&format!("DELETE FROM transcripts WHERE {SET}"), [])?;
    for table in LINKED_TABLES {
        conn.execute(&format!("DELETE FROM {table} WHERE {SET}"), [])?;
    }
    let changed: Vec<(i64, i64)> = {
        let mut stmt = conn.prepare(&format!(
            "UPDATE media_assets SET status = 'deleted', revision = revision + 1, updated_at = ?1
             WHERE {SET} AND status = 'active'
             RETURNING asset_id, revision"
        ))?;
        let rows = stmt.query_map([now], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut emit = conn.prepare_cached(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, ?2, 'delete', ?3, ?4)",
    )?;
    for (asset_id, revision) in changed {
        emit.execute(params![asset_id, revision, BULK_PRIORITY, now])?;
    }
    conn.execute("DELETE FROM purge_batch", [])?;
    Ok(keys)
}

/// 지워진 임베딩 한 묶음의 모델 키 (`embeddings` 열 값 그대로)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EmbeddingKey {
    pub kind: String,
    pub model_id: String,
    pub model_version: String,
    pub dimension: i64,
}

/// 자산이 `active` 인가. 작업 결과(파생물·AI·장면·임베딩) 쓰기가 같은 트랜잭션 안에서 확인해, 이미 삭제 표시된 자산
/// (폴더 제거·유형 정리 뒤 늦게 끝난 작업)에 연결 행을 되살리지 않게 한다. 행이 없으면 `false`.
pub fn asset_is_active(conn: &Connection, asset_id: i64) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare_cached(
        "SELECT EXISTS (SELECT 1 FROM media_assets WHERE asset_id = ?1 AND status = 'active')",
    )?;
    stmt.query_row([asset_id], |r| r.get(0))
}
