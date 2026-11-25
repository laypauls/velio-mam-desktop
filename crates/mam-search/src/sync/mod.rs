//! SQLite Outbox -> Tantivy 색인 워커 (기술 검토서 §8, db.md §3).
//!
//! - [`SyncWorker::process_batch`]: `search_outbox` 의 `pending` 이벤트를 우선순위·id 순으로 읽어
//!   자산별 최신 상태만 인덱스에 반영한다. **SQLite 가 원천**이다 — 이벤트의 `event_type` 이 아니라
//!   읽은 시점의 자산 상태(활성 여부, revision)로 upsert/delete 를 결정한다.
//! - 역전 방지: `index_progress.applied_revision` 이하의 revision 은 건너뛴다. 삭제도 revision 을
//!   기록하므로 삭제 뒤 늦게 온 낮은 revision 이벤트가 자산을 되살리지 못한다.
//! - 인덱스 commit 성공 **뒤에** 한 SQLite 트랜잭션으로 `index_progress` 갱신 + 이벤트 `done`.
//!   중간에 죽으면 이벤트가 `pending` 으로 남아 다시 처리된다(멱등).
//! - 실패: 이벤트의 `attempts` 증가 + `last_error` 기록, `max_attempts` 번째 실패에서 `failed`
//!   (이력 보존 — 성공해도 `last_error`/`attempts` 는 지우지 않는다).
//! - [`SyncWorker::rebuild`]: 인덱스 손상·형식/분석기 불일치·문서 수 이탈 시 SQLite 에서 새 디렉터리에
//!   전체 재구축한 뒤 교체한다.
//!
//! 이 모듈은 V1 스키마 테이블(`media_assets`, `file_locations`, `media_metadata`, `tags`,
//! `asset_tags`, `search_outbox`, `index_progress`)만 가정한다. 메모·AI 텍스트는 해당 테이블이
//! 생기는 단계(P2-7, P3)에서 [`load`] 에 추가한다.

pub mod load;
mod outbox;
mod rebuild;
pub mod user_values;
mod worker;

pub use outbox::SELECT_PENDING_SQL;
pub use rebuild::{
    RebuildProgress, RebuildReason, RebuildReport, WriteProgress, rebuild_request_path,
    request_rebuild, write_progress_on,
};
pub use worker::{BatchReport, OpenOutcome, SyncConfig, SyncWorker};

use crate::error::SearchError;

/// `index_progress.index_name` 에 쓰는 풀텍스트 인덱스 이름.
pub const FULLTEXT_INDEX: &str = "fulltext";

/// 사용자 편집 등 즉시 처리할 우선순위 (`search_outbox.priority`).
pub const PRIORITY_USER_EDIT: i64 = 0;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("sqlite error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("search index error: {0}")]
    Search(#[from] SearchError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// 재구축 교체가 실패해 열린 인덱스가 없다. `SyncWorker::open` 으로 다시 연다.
    #[error("index is unavailable after a failed rebuild; reopen with SyncWorker::open")]
    IndexUnavailable,
}

pub type Result<T> = std::result::Result<T, SyncError>;
