//! 관리 폴더별 변경 감지 방식·재스캔 정책 저장 (값 검증은 호출자 — 감지기·정책 레지스트리 — 가 한다).
//! 색인 대상이 아니라 outbox·revision 은 없다 (별칭과 같음).

use rusqlite::{Connection, params};

use super::IngestError;

/// 감지 방식·옵션 저장. `mode` 가 None 이면 자동(옵션도 NULL). 없는 폴더·등록 해제 중(`enabled = 0`)은 `NotFound`.
pub fn set_watch_mode(
    conn: &Connection,
    folder_id: i64,
    mode: Option<&str>,
    options: Option<&str>,
) -> Result<(), IngestError> {
    let n = conn.execute(
        "UPDATE managed_folders SET watch_mode = ?2, watch_options = ?3 WHERE folder_id = ?1 AND enabled = 1",
        params![folder_id, mode, options],
    )?;
    if n == 0 {
        return Err(IngestError::NotFound);
    }
    Ok(())
}

/// 재스캔 정책 JSON 저장 (None = 전역 기본으로 되돌림). 없는 폴더·등록 해제 중은 `NotFound`.
pub fn set_rescan_policy(
    conn: &Connection,
    folder_id: i64,
    policy: Option<&str>,
) -> Result<(), IngestError> {
    let n = conn.execute(
        "UPDATE managed_folders SET rescan_policy = ?2 WHERE folder_id = ?1 AND enabled = 1",
        params![folder_id, policy],
    )?;
    if n == 0 {
        return Err(IngestError::NotFound);
    }
    Ok(())
}

/// `ManagedFolderView` 의 변경 감지·재스캔 정책 필드 (`#[serde(flatten)]` — JSON 계약은 최상위 필드 그대로).
/// `Deref` 로 `view.watch_mode` 처럼 바로 읽는다.
#[derive(Debug, Clone, Default, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DetectionView {
    /// 저장된 변경 감지 방식 ID (None = 자동)
    pub watch_mode: Option<String>,
    /// 저장된 감지기 옵션 (해석 불가면 None)
    pub watch_options: Option<serde_json::Value>,
    /// 지금 실제로 동작 중인 감지기 ID (None = 감시 중 아님 — 오프라인·꺼내기 준비·제거 중)
    pub watch_active: Option<String>,
    /// 지정한 방식이 아닌 방식으로 동작하는 사유 키 (폴백이 아니면 None)
    pub watch_fallback_key: Option<String>,
    /// 재스캔 정책 (유효하면 정규화본, 해석 불가면 저장된 원문 JSON, NULL·JSON 아님이면 None)
    pub rescan_policy: Option<serde_json::Value>,
    /// 다음 자동 재스캔 예정 시각 (ms) — 수동 정책·오프라인이면 None
    pub next_rescan_at: Option<i64>,
    /// 스캔이 3분 넘게 한 곳에서 멈춰 있다 (아니면 JSON 에서 빠진다 — 프론트 계약상 선택 필드)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan_stall: Option<super::ScanStall>,
    /// 변경 신호원 상태 — 마지막 신호를 받았거나 저하 중일 때만 존재 (정상·신호 없음이면 JSON 에서 빠진다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watch_health: Option<crate::watch::SourceHealth>,
}
