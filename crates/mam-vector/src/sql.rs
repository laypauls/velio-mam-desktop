//! 자산 단위로 `embeddings` 를 찾는 SQL — 질의 계획을 고정한다.
//!
//! 통계(`ANALYZE`)가 없는 DB 에서 SQLite 는 `(자산 = ?, 종류 = ?, 모델 = ?, 버전 = ?, 폐기 IS NULL)` 조건에
//! `idx_embeddings_model` 을 골라 그 모델의 **활성 행 전체**를 훑고 `asset_id` 를 걸러낸다 (P4 후속 측정: 44만 행에서
//! `put` 1건이 빈 DB 보다 150ms 느렸다 = 행 수에 선형). 자산 조건이 가장 선택적이므로 `idx_embeddings_asset` 을 강제한다.
//! (인덱스는 V6 마이그레이션이 만든다 — 없으면 "no such index" 로 바로 드러난다.)

/// 자산의 (종류, 모델) 활성 행을 폐기한다. 파라미터: now, asset_id, kind, model_id, model_version.
pub(crate) const SUPERSEDE: &str =
    "UPDATE embeddings INDEXED BY idx_embeddings_asset SET superseded_at = ?1
     WHERE asset_id = ?2 AND vector_kind = ?3 AND model_id = ?4 AND model_version = ?5
       AND superseded_at IS NULL";

/// 자산의 활성 벡터 raw 오프셋 (프레임 순서). 파라미터: asset_id, kind, model_id, model_version, dimension.
pub(crate) const ASSET_RAW_OFFSETS: &str =
    "SELECT raw_offset FROM embeddings INDEXED BY idx_embeddings_asset
     WHERE asset_id = ?1 AND vector_kind = ?2 AND model_id = ?3 AND model_version = ?4
       AND dimension = ?5 AND superseded_at IS NULL
     ORDER BY embedding_id";

/// 자산의 활성 임베딩 id. 파라미터는 [`ASSET_RAW_OFFSETS`] 와 같다.
pub(crate) const ASSET_ACTIVE_IDS: &str =
    "SELECT embedding_id FROM embeddings INDEXED BY idx_embeddings_asset
     WHERE asset_id = ?1 AND vector_kind = ?2 AND model_id = ?3 AND model_version = ?4
       AND dimension = ?5 AND superseded_at IS NULL";

// 키 단위 조회 — V7 `idx_embeddings_active_map` (커버링) 으로 테이블 조회 없이 읽는다. 파라미터: kind, model_id, model_version, dimension.

/// 활성 `(embedding_id, asset_id)`, 임베딩 id 오름차순 (`ActiveMapCache` 전체 로드).
pub(crate) const ACTIVE_MAP_LOAD: &str = "SELECT embedding_id, asset_id FROM embeddings
     WHERE vector_kind = ?1 AND model_id = ?2 AND model_version = ?3 AND dimension = ?4
       AND superseded_at IS NULL
     ORDER BY embedding_id";

/// 활성 `(embedding_id, raw_offset)`, 임베딩 id 오름차순 (raw 읽기·재구축).
pub(crate) const ACTIVE_RAW_OFFSETS: &str = "SELECT embedding_id, raw_offset FROM embeddings
     WHERE vector_kind = ?1 AND model_id = ?2 AND model_version = ?3 AND dimension = ?4
       AND superseded_at IS NULL
     ORDER BY embedding_id";

/// 키의 raw 참조 하한 (폐기 행 포함 전체의 `MAX(raw_offset)`).
pub(crate) const RAW_FLOOR: &str = "SELECT MAX(raw_offset) FROM embeddings
     WHERE vector_kind = ?1 AND model_id = ?2 AND model_version = ?3 AND dimension = ?4";

#[cfg(test)]
mod tests {
    use super::*;

    /// T-sql-01: 통계가 없는 새 DB 에서도 세 질의가 `idx_embeddings_asset` 으로 찾는다 (행 수에 비례하는 계획 회귀 방지).
    #[test]
    fn t_sql_01_asset_queries_use_the_asset_index_without_analyze() {
        // Arrange
        let dir = tempfile::TempDir::new().unwrap();
        let mut conn = mam_db::open(&dir.path().join("t.db")).unwrap();
        mam_db::migrate(&mut conn, &dir.path().join("b"), &mam_db::SystemClock).unwrap();
        for (sql, params) in [
            (SUPERSEDE, 5),
            (ASSET_RAW_OFFSETS, 5),
            (ASSET_ACTIVE_IDS, 5),
        ] {
            // Act
            let mut st = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let args: Vec<rusqlite::types::Value> =
                (0..params).map(rusqlite::types::Value::Integer).collect();
            let plan: Vec<String> = st
                .query_map(rusqlite::params_from_iter(args), |r| r.get(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            // Assert
            assert!(
                plan.iter().any(|p| p.contains("idx_embeddings_asset")),
                "{plan:?}"
            );
            assert!(
                !plan.iter().any(|p| p.contains("idx_embeddings_model")),
                "{plan:?}"
            );
        }
    }

    /// T-sql-02: 키 단위 조회(활성 맵 로드·활성 raw 읽기·raw 하한)가 V7 커버링 인덱스만으로 풀리고 정렬 단계가 없다.
    #[test]
    fn t_sql_02_key_queries_use_the_covering_active_map_index() {
        // Arrange
        let dir = tempfile::TempDir::new().unwrap();
        let mut conn = mam_db::open(&dir.path().join("t.db")).unwrap();
        mam_db::migrate(&mut conn, &dir.path().join("b"), &mam_db::SystemClock).unwrap();
        for sql in [ACTIVE_MAP_LOAD, ACTIVE_RAW_OFFSETS, RAW_FLOOR] {
            // Act
            let mut st = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let plan: Vec<String> = st
                .query_map(rusqlite::params!["visual", "m", "1", 512], |r| r.get(3))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            // Assert
            assert!(
                plan.iter()
                    .any(|p| p.contains("COVERING INDEX idx_embeddings_active_map")),
                "{sql}: {plan:?}"
            );
            assert!(
                !plan.iter().any(|p| p.contains("TEMP B-TREE")),
                "no sort step: {plan:?}"
            );
        }
    }
}
