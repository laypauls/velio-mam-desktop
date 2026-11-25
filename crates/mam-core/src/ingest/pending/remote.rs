//! 원격 피드의 기록기 — 수집기가 읽은 보고(`watch_pending` UPSERT)와 커서(`watch_cursors`)를 **한 SQLite 트랜잭션**에 쓴다.
//!
//! 둘을 따로 커밋하면 사이에서 죽었을 때 (a) 커서만 올라가 그 줄을 영영 못 보거나 (b) 보고만 남고 같은 줄을 다시 읽는다.
//! 한 트랜잭션이면 둘 다 반영되거나 둘 다 롤백되어, 다음 실행은 같은 오프셋부터 이어 읽는다 (중복 보고는 경로당 1행 UPSERT 가 합친다).

use std::sync::Arc;

use mam_db::{DbPool, Priority};

use super::store::{self, Incoming, PendingKind};
use crate::SharedClock;
use crate::watch::remote::collector::{BatchCommitter, CommitError, CommitOutcome, FeedItem};
use crate::watch::remote::store as remote_store;

pub struct DbCommitter {
    pool: Arc<DbPool>,
    clock: SharedClock,
    /// 첫 확인까지의 대기 (`IngestConfig::debounce_ms`)
    due_ms: i64,
    /// 폴더당 대기 행 상한 (`PendingConfig::max_rows_per_folder`)
    cap: usize,
}

impl DbCommitter {
    pub fn new(pool: Arc<DbPool>, clock: SharedClock, due_ms: i64, cap: usize) -> Self {
        Self { pool, clock, due_ms, cap }
    }
}

impl BatchCommitter for DbCommitter {
    fn commit_feed(&self, source_id: i64, items: &[FeedItem], cursor: &str, origin: &'static str) -> Result<CommitOutcome, CommitError> {
        let incoming: Vec<Incoming> = items
            .iter()
            .filter_map(|i| Incoming::new(i.folder_id, &i.path, if i.subtree { PendingKind::Subtree } else { PendingKind::Path }, origin))
            .collect();
        let (now, due, cap) = (self.clock.now_ms(), self.clock.now_ms() + self.due_ms, self.cap);
        // 청크마다 별도 트랜잭션 — 커서는 **마지막 청크의 같은 트랜잭션**에서만 올린다 (중간 실패 시 커서 불변)
        let report = store::upsert_chunked(&incoming, store::MAX_ROWS_PER_TX, |part, last| {
            let part = part.to_vec();
            let cursor = last.then(|| cursor.to_string());
            self.pool
                .write(Priority::Low, move |c| {
                    Ok(store::upsert_batch_with(c, &part, now, due, cap, |tx| match &cursor {
                        Some(cur) => remote_store::save_cursor(tx, source_id, cur, now),
                        None => Ok(()),
                    }))
                })
                .map_err(crate::scan::IngestError::from)?
        })
        .map_err(|e| CommitError(e.to_string()))?;
        Ok(CommitOutcome { overflowed: report.overflowed, dropped: report.dropped })
    }

    fn load_cursor(&self, source_id: i64) -> Option<String> {
        self.pool.read(|c| Ok(remote_store::load_cursor(c, source_id))).ok().flatten()
    }
}
