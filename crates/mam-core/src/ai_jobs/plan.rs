//! 등록 계획 - 단계 선택 규칙과 대상 자산 조회 SQL (스케줄러가 사용).

use rusqlite::{Connection, params};
use serde::Serialize;

use super::config::AiRuntimeSettings;
use super::types::StepKind;

pub(super) const CHUNK: usize = 500;

/// 미분석(= 단계 행이 하나도 없는) 활성 자산, `?1` 초과 id 부터 `?2` 개 (keyset). `ai_task_state` PK 로 탐색한다.
pub const UNANALYZED_SQL: &str = "SELECT a.asset_id FROM media_assets a
     WHERE a.asset_id > ?1 AND a.status = 'active'
       AND NOT EXISTS (SELECT 1 FROM ai_task_state s WHERE s.asset_id = a.asset_id)
     ORDER BY a.asset_id LIMIT ?2";
/// 임베딩 백필 대상: embedding 단계 행이 없는(어떤 상태든 행이 있으면 제외) 활성 자산, `?1` 초과 id 부터 `?2` 개.
/// `ai_task_state` PK(asset_id, kind) 탐색. `media_type` 은 스키마 CHECK 로 image/video/audio 뿐이고 임베딩은
/// 셋 모두에 적용되므로 유형 조건을 두지 않는다(IN 조건은 정렬용 임시 B-tree 를 만들어 전체 스캔이 된다).
pub const EMBEDDING_BACKFILL_SQL: &str = "SELECT a.asset_id FROM media_assets a
     WHERE a.asset_id > ?1 AND a.status = 'active'
       AND NOT EXISTS (SELECT 1 FROM ai_task_state s WHERE s.asset_id = a.asset_id AND s.kind = 'embedding')
     ORDER BY a.asset_id LIMIT ?2";
/// 전체 재분석용 활성 자산 keyset.
pub const ACTIVE_SQL: &str = "SELECT asset_id FROM media_assets
     WHERE asset_id > ?1 AND status = 'active' ORDER BY asset_id LIMIT ?2";
/// 실패 단계가 있는 자산 (`idx_ai_task_state_kind_state` 를 종류 IN 목록으로 탐색).
pub const FAILED_SQL: &str = "SELECT DISTINCT asset_id FROM ai_task_state
     WHERE kind IN ('describe', 'keywords', 'ocr', 'summary', 'stt', 'embedding') AND state = 'failed'";

/// UI 계약 `QueueResult`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueResult {
    pub queued: u64,
    /// 동의 없음·적용할 단계 없음·비활성 자산 등으로 건너뛴 수
    pub skipped: u64,
}

impl QueueResult {
    pub(super) fn add(&mut self, o: QueueResult) {
        self.queued += o.queued;
        self.skipped += o.skipped;
    }
}

/// 등록할 단계: 명시 목록이면 그 목록, 아니면 설정 기본(외부 종류 + stt + embedding). 미디어 유형에 맞는 것만,
/// 동의 전에는 외부 종류를 뺀다.
pub(super) fn plan(
    media_type: &str,
    requested: Option<&[StepKind]>,
    st: &AiRuntimeSettings,
) -> Vec<StepKind> {
    let base: Vec<StepKind> = match requested {
        Some(k) => k.to_vec(),
        None => {
            let mut v = st.external_kinds.clone();
            if st.stt_enabled {
                v.push(StepKind::Stt);
            }
            if st.semantic_enabled {
                v.push(StepKind::Embedding);
            }
            v
        }
    };
    let mut out: Vec<StepKind> = Vec::new();
    for k in base {
        let allowed = k.applies_to(media_type) && (st.consent_given || !k.is_external());
        if allowed && !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

pub(super) fn ids_page(
    conn: &Connection,
    sql: &str,
    after: i64,
    n: usize,
) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params![after, i64::try_from(n).unwrap_or(i64::MAX)], |r| {
        r.get(0)
    })?;
    rows.collect()
}

/// 적용 가능한 모든 단계를 skipped(disabled) 로 표시한다 (스윕 재선택 방지).
pub(super) fn mark_skipped(
    conn: &Connection,
    now: i64,
    id: i64,
    media: &str,
) -> rusqlite::Result<()> {
    if !crate::purge::asset_is_active(conn, id)? {
        return Ok(()); // 삭제 표시된 자산에는 단계 행을 만들지 않는다
    }
    for k in StepKind::ALL.into_iter().filter(|k| k.applies_to(media)) {
        conn.execute(
            "INSERT OR IGNORE INTO ai_task_state (asset_id, kind, state, error_code, attempts, updated_at)
             VALUES (?1, ?2, 'skipped', 'disabled', 0, ?3)",
            params![id, k.as_str(), now],
        )?;
    }
    Ok(())
}
