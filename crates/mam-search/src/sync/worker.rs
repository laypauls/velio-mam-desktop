//! Outbox 배치 처리 — 역전 방지·실패 이력.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mam_db::Clock;
use rusqlite::Connection;

use super::rebuild::{
    RebuildReason, RebuildReport, WriteProgress, clear_rebuild_request, rebuild_requested,
};
use super::{FULLTEXT_INDEX, PRIORITY_USER_EDIT, Result, SyncError, load, outbox};
use crate::coordinator::SearchHandle;
use crate::error::SearchError;
use crate::index::{IndexOptions, SearchIndex, UpsertOutcome};

/// 워커 설정.
#[derive(Clone, Debug)]
pub struct SyncConfig {
    /// 한 배치에서 읽는 최대 이벤트 수.
    pub batch_size: usize,
    /// 저우선(`priority > 0`) 이벤트를 모으는 최대 시간(ms). 사용자 편집(priority 0)은 즉시 처리한다.
    pub bulk_delay_ms: i64,
    /// 이 횟수만큼 실패한 이벤트는 `failed` 가 된다.
    pub max_attempts: u32,
    /// 재구축 진행 콜백을 부르는 문서 수 간격.
    pub rebuild_progress_every: u64,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            batch_size: 500,
            bulk_delay_ms: 2_000,
            max_attempts: 5,
            rebuild_progress_every: 10_000,
        }
    }
}

/// `process_batch` 결과. 자산 단위 수치는 "배치 안의 서로 다른 자산" 기준이다.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchReport {
    /// 이번 배치로 읽은 이벤트 수 (0 이면 처리할 것이 없다).
    pub events_selected: usize,
    pub upserted: usize,
    pub deleted: usize,
    /// 이미 반영된 revision 이라 건너뛴 자산 수 (멱등·역전 방지).
    pub skipped_stale: usize,
    pub events_done: usize,
    /// 실패해 `pending` 으로 남은 이벤트 수 (attempts 증가).
    pub events_retry: usize,
    /// 이번 실패로 `failed` 가 된 이벤트 수.
    pub events_failed: usize,
    /// 저우선 이벤트를 더 모으는 중이면 처리 가능해지는 시각(UTC ms). 이때까지 호출해도 아무 일도 없다.
    pub deferred_until_ms: Option<i64>,
    /// 인덱스 반영·commit 이 실패했다면 그 사유 (이벤트는 위 규칙대로 기록됨).
    pub error: Option<String>,
}

impl BatchReport {
    fn merge(&mut self, o: &BatchReport) {
        self.events_selected += o.events_selected;
        self.upserted += o.upserted;
        self.deleted += o.deleted;
        self.skipped_stale += o.skipped_stale;
        self.events_done += o.events_done;
        self.events_retry += o.events_retry;
        self.events_failed += o.events_failed;
        self.deferred_until_ms = o.deferred_until_ms;
        self.error = o.error.clone().or(self.error.take());
    }
}

/// `SyncWorker::open` 결과.
#[derive(Debug)]
pub struct OpenOutcome {
    /// 열면서 전체 재구축을 했다면 그 보고.
    pub rebuilt: Option<RebuildReport>,
}

/// Outbox -> Tantivy 색인 워커. 연결은 호출마다 받는다(`&rusqlite::Connection`) —
/// 어떤 스레드·풀 핸들에서 호출해도 되며, 인덱스 쓰기는 `SearchIndex` 내부 잠금으로 직렬화된다.
pub struct SyncWorker {
    pub(super) index: Option<SearchIndex>,
    pub(super) dir: PathBuf,
    pub(super) options: IndexOptions,
    pub(super) config: SyncConfig,
    pub(super) handle: SearchHandle,
}

impl SyncWorker {
    /// 이미 연 인덱스로 워커를 만든다 (재구축 필요 여부는 확인하지 않는다 — `open` 사용 권장).
    pub fn new(index: SearchIndex, dir: &Path, options: IndexOptions, config: SyncConfig) -> Self {
        Self::new_with_handle(index, dir, options, config, SearchHandle::new())
    }

    /// `new` 와 같지만 검색 핸들을 호출자가 미리 만들어 둔 것으로 쓴다 (앱은 워커가 열리기 전에도 핸들을 나눠준다).
    pub fn new_with_handle(
        index: SearchIndex,
        dir: &Path,
        options: IndexOptions,
        config: SyncConfig,
        handle: SearchHandle,
    ) -> Self {
        handle.publish(Some(&index));
        Self {
            index: Some(index),
            dir: dir.to_path_buf(),
            options,
            config,
            handle,
        }
    }

    /// 인덱스를 열고, 형식·분석기·옵션 불일치나 손상이면 SQLite 에서 재구축한다. 열린 뒤 문서 수가
    /// SQLite 와 크게 어긋나도(`needs_rebuild`) 재구축한다. 다른 프로세스가 쓰기 잠금을 쥐고 있으면
    /// (`LockFailure`) 재구축하지 않고 오류를 반환한다.
    pub fn open(
        conn: &Connection,
        dir: &Path,
        options: IndexOptions,
        config: SyncConfig,
    ) -> Result<(Self, OpenOutcome)> {
        Self::open_split(conn, dir, options, config, &mut |p| {
            super::rebuild::write_progress_on(conn, p)
        })
    }

    /// `open` 과 같지만 재구축의 긴 읽기 스냅샷(`read`)과 마지막 `index_progress` 쓰기(`write_progress`)를
    /// 나눈다 — 앱은 쓰기 스레드를 오래 점유하지 않도록 읽기 연결로 스냅샷을 읽는다.
    pub fn open_split(
        read: &Connection,
        dir: &Path,
        options: IndexOptions,
        config: SyncConfig,
        write_progress: &mut WriteProgress,
    ) -> Result<(Self, OpenOutcome)> {
        Self::open_with_handle(
            read,
            dir,
            options,
            config,
            SearchHandle::new(),
            write_progress,
        )
    }

    /// `open_split` 와 같지만 검색 핸들을 호출자가 정한다. 재구축이 끝나기 전에는 핸들이 `NotReady` 다.
    pub fn open_with_handle(
        read: &Connection,
        dir: &Path,
        options: IndexOptions,
        config: SyncConfig,
        handle: SearchHandle,
        write_progress: &mut WriteProgress,
    ) -> Result<(Self, OpenOutcome)> {
        let conn = read;
        // 명시적 재구축 요청(복원·복구)이 있으면 문서 수 비교를 건너뛰고 재구축한다. 표지는 재구축이 끝난 뒤에만 지운다.
        let forced = rebuild_requested(dir);
        match SearchIndex::open_or_create(dir, options.clone()) {
            Ok(index) => {
                // 빈 인덱스는 재구축 여부가 정해지고 교체될 때까지 게시하지 않는다 — 게시한 채로 두면 최초 구축 중 UI 가
                // "결과 0건" 성공 응답을 받아 목록이 비어 보인다 (NotReady 가 맞다). 문서가 있는 인덱스는 바로 게시한다
                // (100만 건에서 문서 수 점검만 약 1.5초라 그동안도 검색이 돼야 한다).
                let usable_now = index.num_docs() > 0;
                let mut w = Self {
                    index: Some(index),
                    dir: dir.to_path_buf(),
                    options,
                    config,
                    handle,
                };
                if usable_now {
                    w.handle.publish(w.index.as_ref());
                }
                let reason = if forced {
                    Some(RebuildReason::Requested)
                } else {
                    w.needs_rebuild(conn)?
                };
                let rebuilt = match reason {
                    // 재구축은 교체 시점에 새 인덱스를 게시한다
                    Some(reason) => {
                        let report = w.rebuild_split(conn, reason, &mut |_| {}, write_progress)?;
                        clear_rebuild_request(dir);
                        Some(report)
                    }
                    None => {
                        if !usable_now {
                            w.handle.publish(w.index.as_ref());
                        }
                        None
                    }
                };
                Ok((w, OpenOutcome { rebuilt }))
            }
            Err(e) => {
                let reason = match e {
                    SearchError::IndexMismatch(m) => RebuildReason::Mismatch(m),
                    SearchError::Tantivy(tantivy::TantivyError::LockFailure(..)) => {
                        return Err(e.into());
                    }
                    SearchError::Tantivy(_) | SearchError::OpenDirectory(_) => {
                        RebuildReason::OpenFailed(e.to_string())
                    }
                    other => return Err(other.into()),
                };
                let mut w = Self {
                    index: None,
                    dir: dir.to_path_buf(),
                    options,
                    config,
                    handle,
                };
                let report = w.rebuild_split(conn, reason, &mut |_| {}, write_progress)?;
                clear_rebuild_request(dir);
                Ok((
                    w,
                    OpenOutcome {
                        rebuilt: Some(report),
                    },
                ))
            }
        }
    }

    /// 검색에 쓰는 인덱스 핸들. 재구축 교체가 실패한 뒤에는 `IndexUnavailable`.
    pub fn index(&self) -> Result<&SearchIndex> {
        self.index.as_ref().ok_or(SyncError::IndexUnavailable)
    }

    /// 검색용 공유 핸들 — 워커(재구축 포함)와 독립적으로 복제해 여러 스레드에서 검색한다.
    pub fn search_handle(&self) -> SearchHandle {
        self.handle.clone()
    }

    pub fn config(&self) -> &SyncConfig {
        &self.config
    }

    /// `pending` 이벤트 수.
    pub fn pending_events(&self, conn: &Connection) -> Result<u64> {
        Ok(outbox::pending_count(conn)?)
    }

    /// 배치 1회를 처리한다.
    ///
    /// 1. 읽기 스냅샷 안에서 이벤트 선택 + `applied_revision` 조회 + 자산 문서 로드 (질의는 배치당 고정 횟수)
    /// 2. 인덱스에 upsert/delete 후 commit
    /// 3. 성공하면 한 트랜잭션으로 `index_progress` 갱신 + 이벤트 `done`
    ///
    /// 인덱스 반영·commit 실패는 `Ok(report)` 의 `error` 로 돌려주고 이벤트에 실패를 기록한다.
    /// SQLite 오류나 writer poisoned(인덱스를 다시 열어야 함)는 `Err` — 이때 이벤트는 건드리지 않는다.
    /// 우선순위 0 이벤트가 있으면 그것만 먼저 처리하고, 저우선 이벤트는 `bulk_delay_ms` 동안 모은다.
    pub fn process_batch(&self, conn: &Connection, clock: &dyn Clock) -> Result<BatchReport> {
        let index = self.index()?;
        let mut report = BatchReport::default();

        let snapshot = conn.unchecked_transaction()?;
        let mut events = outbox::select_pending(&snapshot, self.config.batch_size)?;
        let Some(first) = events.first() else {
            return Ok(report);
        };
        let first_priority = first.priority;
        events.retain(|e| e.priority == first_priority);
        if first_priority > PRIORITY_USER_EDIT && events.len() < self.config.batch_size {
            let oldest = events.iter().map(|e| e.created_at).min().unwrap_or(0);
            let due = oldest.saturating_add(self.config.bulk_delay_ms);
            if clock.now_ms() < due {
                report.deferred_until_ms = Some(due);
                return Ok(report);
            }
        }
        report.events_selected = events.len();

        // 자산별 최신 revision
        let mut order: Vec<u64> = Vec::new();
        let mut max_rev: HashMap<u64, u64> = HashMap::new();
        for e in &events {
            let slot = max_rev.entry(e.asset_id).or_insert_with(|| {
                order.push(e.asset_id);
                0
            });
            *slot = (*slot).max(e.revision);
        }
        let applied = outbox::applied_revisions(&snapshot, FULLTEXT_INDEX, &order)?;
        let candidates: Vec<u64> = order
            .iter()
            .copied()
            .filter(|id| max_rev[id] > applied.get(id).copied().unwrap_or(0))
            .collect();
        let docs = load::load_docs(&snapshot, &candidates)?;
        let revisions = load::load_revisions(&snapshot, &candidates)?;
        snapshot.commit()?;
        report.skipped_stale = order.len() - candidates.len();

        // 인덱스 반영 — 이벤트 종류가 아니라 읽은 시점의 자산 상태가 기준
        let mut progress: Vec<(u64, u64)> = Vec::new();
        let mut failure: Option<String> = None;
        for id in &candidates {
            let applied_rev = applied.get(id).copied().unwrap_or(0);
            let result = match docs.get(id) {
                Some(doc) if doc.revision <= applied_rev => Ok(None),
                Some(doc) => index.upsert(doc).map(|outcome| match outcome {
                    UpsertOutcome::Applied => {
                        report.upserted += 1;
                        Some(doc.revision)
                    }
                    UpsertOutcome::Stale { indexed_revision } => {
                        report.skipped_stale += 1;
                        Some(indexed_revision)
                    }
                }),
                None => {
                    // 삭제됐거나 활성 위치가 없다 -> 인덱스에서 제거. 삭제 revision 도 기록한다.
                    let target = max_rev[id].max(revisions.get(id).copied().unwrap_or(0));
                    if target <= applied_rev {
                        Ok(None)
                    } else {
                        index.delete(*id).map(|()| {
                            report.deleted += 1;
                            Some(target)
                        })
                    }
                }
            };
            match result {
                Ok(Some(rev)) => progress.push((*id, rev)),
                Ok(None) => report.skipped_stale += 1,
                Err(SearchError::WriterPoisoned) => return Err(SearchError::WriterPoisoned.into()),
                Err(e) => {
                    failure = Some(e.to_string());
                    break;
                }
            }
        }
        if failure.is_none() && report.upserted + report.deleted > 0 {
            match index.commit() {
                Ok(()) => {}
                Err(SearchError::WriterPoisoned) => return Err(SearchError::WriterPoisoned.into()),
                Err(e) => failure = Some(e.to_string()),
            }
        }

        let tx = conn.unchecked_transaction()?;
        if let Some(message) = failure {
            // 인덱스 쪽 배치는 버려졌다 -> 반영 수치를 되돌리고 이벤트에 실패를 기록
            report.upserted = 0;
            report.deleted = 0;
            for e in &events {
                if outbox::record_failure(&tx, e.event_id, &message, self.config.max_attempts)? {
                    report.events_failed += 1;
                } else {
                    report.events_retry += 1;
                }
            }
            report.error = Some(message);
        } else {
            for (id, rev) in &progress {
                outbox::raise_progress(&tx, FULLTEXT_INDEX, *id, *rev)?;
            }
            for e in &events {
                outbox::mark_done(&tx, e.event_id)?;
            }
            report.events_done = events.len();
        }
        tx.commit()?;
        Ok(report)
    }

    /// 처리할 이벤트가 없거나, 저우선 이벤트를 모으는 중이거나, 배치가 실패할 때까지 반복한다.
    /// 합산 보고를 반환한다.
    pub fn drain(&self, conn: &Connection, clock: &dyn Clock) -> Result<BatchReport> {
        let mut total = BatchReport::default();
        loop {
            let r = self.process_batch(conn, clock)?;
            total.merge(&r);
            if r.events_selected == 0 || r.error.is_some() {
                return Ok(total);
            }
        }
    }
}
