//! SQLite 에서 인덱스를 전체 재구축하고 디렉터리를 교체한다.
//!
//! 순서: 새 디렉터리(`<dir>.rebuild`)에 읽기 스냅샷 기준으로 전부 색인 -> commit·병합 대기 ->
//! 기존 인덱스 해제 -> `<dir>` 를 `<dir>.old` 로 옮기고 새 디렉터리를 `<dir>` 로 -> 다시 열기 ->
//! `index_progress` 를 스냅샷 revision 으로 덮어쓰기 -> `.old` 삭제(실패해도 무시, 보고에 남김).
//! 재구축 중·직후에 들어온 변경의 이벤트는 `pending` 으로 남아 있고, 스냅샷 revision 보다 높으므로
//! 재구축 뒤 `process_batch` 가 처리한다(스냅샷에 이미 담긴 이벤트는 건너뛰어 `done`).
//! 교체 실패 시 가능한 한 원래 디렉터리를 복원한다.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::worker::SyncWorker;
use super::{FULLTEXT_INDEX, Result, load, outbox};
use super::user_values::{ALL_VALUES_SQL, UserValueStream};
use crate::index::SearchIndex;
use crate::retry_dir::{RetryPolicy, retry_io};

/// 재구축 사유.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebuildReason {
    /// 호출자가 직접 요청.
    Requested,
    /// 형식·분석기 버전·전략 옵션 불일치 (`IndexMismatch`).
    Mismatch(String),
    /// 열 수 없음 — 메타·세그먼트 손상 등.
    OpenFailed(String),
    /// 인덱스 문서 수가 SQLite 와 허용 오차(pending 이벤트 수 + max(10, 1%)) 이상 어긋남.
    CountDrift {
        sqlite_docs: u64,
        index_docs: u64,
        pending_events: u64,
    },
}

#[derive(Clone, Debug)]
pub struct RebuildReport {
    pub reason: RebuildReason,
    /// 색인한 문서 수.
    pub docs: u64,
    /// `.old` 디렉터리 삭제 실패 사유 (재구축 자체는 성공).
    pub cleanup_error: Option<String>,
}

/// 재구축 진행 콜백 인자 (`SyncConfig::rebuild_progress_every` 문서마다).
#[derive(Clone, Copy, Debug)]
pub struct RebuildProgress {
    pub docs_written: u64,
}

fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "index".into());
    dir.with_file_name(format!("{name}.{suffix}"))
}

fn remove_dir(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    retry_io(RetryPolicy::default(), || std::fs::remove_dir_all(dir))
}

fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    retry_io(RetryPolicy::default(), || std::fs::rename(from, to))
}

impl SyncWorker {
    /// 인덱스가 SQLite 와 어긋났는가. 문서 수만 비교한다(싸다 — 100만 건 `count(*)` 1회).
    /// 열기 실패·불일치는 `open` 이 먼저 처리한다.
    pub fn needs_rebuild(&self, conn: &Connection) -> Result<Option<RebuildReason>> {
        let index = self.index()?;
        let sqlite_docs = load::count_docs(conn)?;
        let index_docs = index.num_docs();
        let pending_events = outbox::pending_count(conn)?;
        let tolerance = pending_events + (sqlite_docs / 100).max(10);
        Ok(
            (sqlite_docs.abs_diff(index_docs) > tolerance).then_some(RebuildReason::CountDrift {
                sqlite_docs,
                index_docs,
                pending_events,
            }),
        )
    }

    /// 전체 재구축 (`RebuildReason::Requested`).
    pub fn rebuild(&mut self, conn: &Connection) -> Result<RebuildReport> {
        self.rebuild_with(conn, RebuildReason::Requested, &mut |_| {})
    }

    /// 사유와 진행 콜백을 지정한 재구축. 실패하면 기존 인덱스(있었다면)를 유지·복원한다.
    pub fn rebuild_with(
        &mut self,
        conn: &Connection,
        reason: RebuildReason,
        on_progress: &mut dyn FnMut(RebuildProgress),
    ) -> Result<RebuildReport> {
        self.rebuild_split(conn, reason, on_progress, &mut |p| {
            write_progress_on(conn, p)
        })
    }

    /// [`Self::rebuild_with`] 와 같지만 읽기 스냅샷(`read`, 읽기 전용 연결이어도 된다)과 마지막
    /// `index_progress` 덮어쓰기(`write_progress`)를 나눈다. 앱은 쓰기 스레드를 재구축 내내 점유하지 않는다.
    pub fn rebuild_split(
        &mut self,
        read: &Connection,
        reason: RebuildReason,
        on_progress: &mut dyn FnMut(RebuildProgress),
        write_progress: &mut WriteProgress,
    ) -> Result<RebuildReport> {
        let tmp = sibling(&self.dir, "rebuild");
        let old = sibling(&self.dir, "old");
        remove_dir(&tmp)?;
        remove_dir(&old)?;

        let built = self.build_into(read, &tmp, on_progress);
        let (docs, progress) = match built {
            Ok(v) => v,
            Err(e) => {
                let _ = remove_dir(&tmp);
                return Err(e);
            }
        };

        // 검색을 잠깐 막고(리더 해제) 디렉터리를 교체한 뒤 새 리더를 게시한다 (세대 증가)
        let handle = self.handle.clone();
        let mut swap = handle.begin_swap();
        let swapped = self.swap_dirs(&tmp, &old);
        swap.publish(self.index.as_ref());
        drop(swap);
        swapped?;

        // 스냅샷 revision 으로 덮어쓴다. 실패해도 낮은 값이라 다음 처리에서 멱등하게 보정된다.
        write_progress(&progress)?;

        let cleanup_error = remove_dir(&old).err().map(|e| e.to_string());
        Ok(RebuildReport {
            reason,
            docs,
            cleanup_error,
        })
    }

    /// 기존 인덱스의 mmap·잠금을 풀고 `<dir>` 를 `<dir>.old` 로, 새 디렉터리를 `<dir>` 로 옮긴 뒤 다시 연다.
    fn swap_dirs(&mut self, tmp: &Path, old: &Path) -> Result<()> {
        self.index = None;
        if self.dir.exists()
            && let Err(e) = rename(&self.dir, old)
        {
            self.reopen_best_effort();
            let _ = remove_dir(tmp);
            return Err(e.into());
        }
        if let Err(e) = rename(tmp, &self.dir) {
            if old.exists() {
                let _ = rename(old, &self.dir);
            }
            self.reopen_best_effort();
            let _ = remove_dir(tmp);
            return Err(e.into());
        }
        self.index = Some(SearchIndex::open_or_create(
            &self.dir,
            self.options.clone(),
        )?);
        Ok(())
    }

    fn reopen_best_effort(&mut self) {
        self.index = SearchIndex::open_or_create(&self.dir, self.options.clone()).ok();
    }

    /// 읽기 스냅샷 하나로 전부 색인하고 commit·병합 대기 후 닫는다. (색인 문서 수, (asset_id, revision) 목록)
    fn build_into(
        &self,
        conn: &Connection,
        dir: &Path,
        on_progress: &mut dyn FnMut(RebuildProgress),
    ) -> Result<(u64, Vec<(u64, u64)>)> {
        let index = SearchIndex::open_or_create(dir, self.options.clone())?;
        let every = self.config.rebuild_progress_every.max(1);
        let snapshot = conn.unchecked_transaction()?;
        let mut progress = Vec::new();
        let mut docs = 0u64;
        {
            let mut stmt = snapshot.prepare(load::ALL_DOCS_SQL)?;
            let mut rows = stmt.query([])?;
            // 사용자 필드 값은 자산 id 순으로 따로 흘려 병합한다 (자산 100만 건당 질의 100만 번 방지)
            let mut values_stmt = snapshot.prepare(ALL_VALUES_SQL)?;
            let mut values = UserValueStream::new(values_stmt.query([])?);
            while let Some(row) = rows.next()? {
                let mut doc = load::doc_from_row(row)?;
                doc.user_fields = values.take_for(doc.asset_id)?;
                index.upsert_unguarded(&doc)?;
                progress.push((doc.asset_id, doc.revision));
                docs += 1;
                if docs.is_multiple_of(every) {
                    on_progress(RebuildProgress { docs_written: docs });
                }
            }
        }
        snapshot.commit()?;
        index.commit()?;
        index.close_waiting_for_merges()?;
        Ok((docs, progress))
    }
}

/// 재구축 마지막 단계의 `index_progress` 덮어쓰기. 인자: (asset_id, revision) 목록.
pub type WriteProgress<'a> = dyn FnMut(&[(u64, u64)]) -> Result<()> + 'a;

/// 같은 연결(쓰기 가능)에 한 트랜잭션으로 덮어쓴다.
pub fn write_progress_on(conn: &Connection, progress: &[(u64, u64)]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for (asset_id, revision) in progress {
        outbox::set_progress(&tx, FULLTEXT_INDEX, *asset_id, *revision)?;
    }
    tx.commit()?;
    Ok(())
}

/// 명시적 재구축 요청 표지 경로 — 인덱스 디렉터리 **옆**의 `<이름>.rebuild-requested` (재구축의 디렉터리 교체에 휩쓸리지 않는다).
/// 백업 복원·DB 자동 복구처럼 문서 수가 비슷해도 내용이 달라진 경우에 쓴다 (문서 수 허용 오차로는 못 잡는다).
pub fn rebuild_request_path(dir: &Path) -> PathBuf {
    sibling(dir, "rebuild-requested")
}

/// 다음 `SyncWorker::open*` 이 문서 수와 무관하게 전체 재구축하도록 표지를 남긴다 (멱등).
pub fn request_rebuild(dir: &Path) -> std::io::Result<()> {
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(rebuild_request_path(dir), b"")
}

pub(super) fn rebuild_requested(dir: &Path) -> bool {
    rebuild_request_path(dir).is_file()
}

/// 재구축이 끝난 뒤에만 지운다 (실패하면 남겨 다음 기동에서 다시 시도한다).
pub(super) fn clear_rebuild_request(dir: &Path) {
    let _ = std::fs::remove_file(rebuild_request_path(dir));
}
