//! 스캔 엔진: 순회(pre-order, cursor 재개) + 배치 반영. 스캔 실행(`run.rs`)과 감시 이벤트(`single.rs`)가 함께 쓴다.
//!
//! 쓰기 규칙
//! - 디렉터리들의 항목을 버퍼에 모았다가 500건 단위 트랜잭션 하나로 반영한다 (`apply_batch`, 파일마다 SAVEPOINT)
//! - 해시가 필요한 파일은 쓰기 스레드 밖에서 해시한 뒤 두 번째 트랜잭션에서 반영한다
//! - 재개 지점(cursor)은 **그 디렉터리의 마지막 항목과 같은 트랜잭션**에서만 전진한다 — 해시 대기 항목이 남아 있으면
//!   두 번째 트랜잭션에서 함께 쓴다

use std::cmp::Ordering;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_db::{DbPool, Priority};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::IngestError;
use super::errors::{core_code, io_code, record_error};
use super::folders::FolderRow;
use super::media_types::MediaTypeSet;
use super::guarded::{Fail, UNRESPONSIVE_CODE, UNRESPONSIVE_MESSAGE};
use super::io_guard::{Guarded, IoGuard};
use super::progress::ScanProgressCell;
use super::walk::{Listing, WalkRules, preorder_cmp, subtree_before};
use crate::SharedClock;
use crate::error::CoreError;
use crate::hash::{ContentHasher, EdgeFile, HashedFile};
use crate::identity::{FileIdentityProvider, FileMeta};
use crate::keyrange::join_key;
use crate::processing::schedule_for_outcome;
use crate::reconcile::{
    BatchItem, BatchOutcome, MediaType, Outcome, Reconciler, ScanEntry, ScanSession, touch_excluded_paths,
};

/// 한 트랜잭션에 반영하는 최대 파일 수 (D3)
pub const BATCH_FILES: usize = 500;
/// 디렉터리 항목을 모아두는 최대 개수·시간 — 작은 디렉터리가 많아도 커밋 횟수를 줄인다
const FLUSH_DIRS: usize = 256;
const FLUSH_EVERY: Duration = Duration::from_secs(1);

/// 스캔에 필요한 외부 의존성 묶음.
pub struct ScanEnv {
    pub pool: Arc<DbPool>,
    pub clock: SharedClock,
    pub identity: Arc<dyn FileIdentityProvider + Send + Sync>,
    pub hasher: Arc<dyn ContentHasher + Send + Sync>,
    /// 항상 제외할 절대 경로 (MAMData, 백업 폴더 — PL8)
    pub auto_excludes: Vec<PathBuf>,
    pub progress: ScanProgressCell,
    /// 막힐 수 있는 볼륨(네트워크·이동식)의 파일 호출에 거는 제한 시간·볼륨별 검사 상한 (`io_guard`)
    pub io: Arc<IoGuard>,
    /// true 면 스캔이 자산을 만들거나 바꾸는 트랜잭션에서 메타·썸네일·해시 작업도 함께 등록한다 (P2-3, `Ingest::enable_processing`)
    pub schedule_processing: AtomicBool,
    /// 순회가 이 창(ms) 안에 수정된(또는 미래 시각인) 파일을 반영하지 않고 변경 대기열(안정화 게이트)에 넘긴다. 0 = 끔 (결정 I)
    pub recent_guard_ms: i64,
    /// 변경 대기열 폴더당 상한·첫 확인까지의 대기 (최근 수정 파일을 넘길 때 쓴다)
    pub pending_cap: usize,
    pub pending_due_ms: i64,
}

impl ScanEnv {
    pub fn write<T, F>(&self, priority: Priority, f: F) -> Result<T, IngestError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, IngestError> + Send + 'static,
    {
        self.pool.write(priority, move |c| Ok(f(c)))?
    }

    /// 폴더 하나에 적용할 순회 규칙: 자동 제외 + 사용자 제외
    pub(super) fn rules_for(&self, folder: &FolderRow) -> WalkRules {
        let root = folder.root_path();
        WalkRules::new(
            self.auto_excludes
                .iter()
                .cloned()
                .chain(folder.excluded.iter().map(|rel| {
                    rel.split(['/', '\\'])
                        .filter(|s| !s.is_empty())
                        .fold(root.clone(), |p, seg| p.join(seg))
                })),
        )
        .with_types(folder.media_types)
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanCounts {
    pub processed: u64,
    pub created: u64,
    pub updated: u64,
    pub moved: u64,
    pub unchanged: u64,
    pub missing: u64,
    pub errors: u64,
}

impl ScanCounts {
    fn add(&mut self, o: Outcome) {
        self.processed += 1;
        match o {
            Outcome::Created { .. } => self.created += 1,
            Outcome::Unchanged { .. } | Outcome::HashDeferred => self.unchanged += 1,
            Outcome::Moved { .. } => self.moved += 1,
            Outcome::Reappeared { .. }
            | Outcome::Touched { .. }
            | Outcome::ContentChanged { .. }
            | Outcome::PendingUpdated { .. }
            | Outcome::Reattached { .. } => self.updated += 1,
        }
    }
}

/// 반영할 파일 1건.
#[derive(Debug, Clone)]
pub(super) struct Work {
    /// 볼륨 기준 상대 경로(표시 표기, `\` 구분) — `file_locations.rel_path`
    pub rel_vol: String,
    /// 폴더 루트 기준 상대 경로(`/` 구분) — 오류 기록용
    pub rel_root: String,
    pub abs: PathBuf,
    pub media: MediaType,
    pub meta: FileMeta,
}

#[derive(Debug, Clone)]
pub(super) struct DirRec {
    pub key: String,
    pub parent_key: String,
    pub name: String,
    pub rel: String,
}

#[derive(Debug, Clone)]
pub(super) struct ErrRec {
    pub rel: String,
    pub code: String,
    pub message: String,
}

/// 파일 반영과 같은 트랜잭션에서 쓰는 꼬리: 재개 지점·폴더 트리·오류
#[derive(Debug, Default, Clone)]
pub(super) struct Tail {
    pub cursor: Option<String>,
    pub dirs: Vec<DirRec>,
    pub errors: Vec<ErrRec>,
    /// 대상 유형 밖이라 등록하지 않은 파일의 볼륨 기준 상대 경로 — 이미 있는 활성 위치만 "봤다" 로 표시한다
    pub seen: Vec<String>,
}

impl Tail {
    fn is_empty(&self) -> bool {
        self.cursor.is_none()
            && self.dirs.is_empty()
            && self.errors.is_empty()
            && self.seen.is_empty()
    }
}

#[derive(Debug, Clone)]
pub(super) struct Task {
    pub abs: PathBuf,
    /// 폴더 루트 기준 디렉터리 키 ('' = 루트)
    pub key: String,
    /// 폴더 루트 기준 표시 경로 ('/' 구분, '' = 루트)
    pub rel: String,
    pub name: String,
    pub parent_key: String,
}

impl Task {
    pub fn root(abs: PathBuf) -> Self {
        Self {
            abs,
            key: String::new(),
            rel: String::new(),
            name: String::new(),
            parent_key: String::new(),
        }
    }
}

pub(super) enum WalkEnd {
    Completed,
    /// 일시정지·종료로 멈췄다 — cursor 까지 저장됨
    Stopped,
    /// 폴더가 제거됐다
    FolderGone,
    /// 볼륨이 응답 없음 상태가 됐다 — 읽지 못한 곳을 건너뛰며 계속하지 않고 멈춘다 (재개 지점은 마지막으로 끝난 디렉터리)
    Unresponsive,
}

enum Flow {
    Continue,
    Gone,
}

pub(super) fn join_rel(dir_rel: &str, name: &str) -> String {
    if dir_rel.is_empty() {
        name.to_string()
    } else {
        format!("{dir_rel}/{name}")
    }
}

struct Buffer {
    work: Vec<Work>,
    /// 최근 수정이라 반영을 미룬 파일 (flush 에서 변경 대기열로 넘긴다)
    deferred: Vec<PathBuf>,
    tail: Tail,
    dirs_since_flush: usize,
    last_flush: Instant,
}

impl Buffer {
    fn new() -> Self {
        Self {
            work: Vec::new(),
            deferred: Vec::new(),
            tail: Tail::default(),
            dirs_since_flush: 0,
            last_flush: Instant::now(),
        }
    }
}

pub(super) struct Walk<'a> {
    pub env: &'a ScanEnv,
    pub folder: &'a FolderRow,
    pub rules: WalkRules,
    pub session: Arc<Mutex<ScanSession>>,
    pub scan_at: i64,
    /// Some = 스캔 실행(재개 지점 기록), None = 감시 이벤트 등 단발 반영
    pub run_id: Option<i64>,
    pub counts: ScanCounts,
    pub cancel: &'a AtomicBool,
    pub stop: &'a dyn Fn() -> bool,
    /// Some(감지기 ID) = 감시 이벤트의 폴더 순회 — 찾은 미디어 파일을 바로 반영하지 않고 모두 변경 대기열(안정화 게이트)에 `path` 행으로
    /// 넘긴다. 수정 시각을 보존하는 복사(탐색기·robocopy)로 들어온 덜 복사된 파일이 60초 창을 피해 등록되는 것을 막는다. None = 일반 스캔.
    pub defer_source: Option<String>,
    /// 대기열에 넣지 못한 파일이 있다(폴더 행 상한·기록 실패) — 호출자가 재스캔 1회로 바꿔야 한다
    pub deferred_overflow: bool,
    buf: Buffer,
}

/// 트랜잭션 밖에서 미리 계산해 두는 값 (전체 해시 / 앞뒤 4K 체크섬)
#[derive(Clone, Copy, Default)]
struct Pre {
    hash: Option<HashedFile>,
    edge: Option<EdgeFile>,
}

#[derive(Clone, Copy)]
enum Need {
    Hash,
    Edge,
}

struct ChunkArgs {
    folder_id: i64,
    /// 이 청크를 만든 스캔이 본 폴더 위치 — 폴더 이전(relocate)으로 바뀌었으면 쓰지 않는다
    volume_id: i64,
    path_key: String,
    run_id: Option<i64>,
    session: Arc<Mutex<ScanSession>>,
    clock: SharedClock,
    scan_at: i64,
    counts: ScanCounts,
    items: Vec<(Work, Pre)>,
    tail: Option<Tail>,
    /// 자산을 만들거나 바꾼 판정마다 처리 작업을 같은 트랜잭션에서 등록한다
    schedule_processing: bool,
    /// 이 청크를 만든 스캔이 적용한 대상 유형 — 그 사이 바뀌었으면 쓰지 않는다 (제외된 유형 자산이 되살아나지 않게)
    types: MediaTypeSet,
}

struct ChunkResult {
    gone: bool,
    need_hash: Vec<(usize, Need)>,
    counts: ScanCounts,
    items: Vec<(Work, Pre)>,
    /// 해시 대기 항목 때문에 반영하지 못한 꼬리
    tail: Option<Tail>,
}

fn upsert_dir(
    conn: &Connection,
    folder_id: i64,
    d: &DirRec,
    seen_at: i64,
) -> Result<(), IngestError> {
    conn.execute(
        "INSERT INTO folder_dirs (folder_id, dir_key, parent_key, name, rel_path, seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (folder_id, dir_key) DO UPDATE SET
           parent_key = excluded.parent_key, name = excluded.name,
           rel_path = excluded.rel_path, seen_at = excluded.seen_at",
        params![folder_id, d.key, d.parent_key, d.name, d.rel, seen_at],
    )?;
    Ok(())
}

fn apply_chunk(c: &mut Connection, mut a: ChunkArgs) -> Result<ChunkResult, IngestError> {
    let mut tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // 폴더가 제거 중이거나 사라졌거나 다른 위치로 이전됐으면 아무것도 쓰지 않는다
    // (제거 후 위치가 되살아나거나, 이전 전 경로로 위치가 다시 생기는 것을 막는다)
    let current: Option<(i64, i64, String, MediaTypeSet)> = tx
        .query_row(
            "SELECT enabled, volume_id, path_key, scan_image, scan_video, scan_audio
             FROM managed_folders WHERE folder_id = ?1",
            [a.folder_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    MediaTypeSet {
                        image: r.get::<_, i64>(3)? != 0,
                        video: r.get::<_, i64>(4)? != 0,
                        audio: r.get::<_, i64>(5)? != 0,
                    },
                ))
            },
        )
        .optional()?;
    if current != Some((1, a.volume_id, a.path_key.clone(), a.types)) {
        return Ok(ChunkResult {
            gone: true,
            need_hash: Vec::new(),
            counts: a.counts,
            items: a.items,
            tail: a.tail,
        });
    }
    let never = AtomicBool::new(false);
    let hasher = crate::hash::Blake3Hasher;
    let reconciler = Reconciler {
        clock: &*a.clock,
        hasher: &hasher,
        cancel: &never,
    };
    let results = {
        let batch: Vec<BatchItem<'_>> = a
            .items
            .iter()
            .map(|(w, h)| BatchItem {
                entry: ScanEntry {
                    rel_path: &w.rel_vol,
                    abs_path: &w.abs,
                    media_type: w.media,
                    meta: w.meta.clone(),
                },
                hash: h.hash,
                edge: h.edge,
            })
            .collect();
        let mut session = a.session.lock().unwrap_or_else(|p| p.into_inner());
        reconciler.apply_batch(&mut tx, &mut session, &batch)?
    };
    let now = a.clock.now_ms();
    let mut need_hash = Vec::new();
    for (i, (r, (w, _))) in results.iter().zip(&a.items).enumerate() {
        match r {
            BatchOutcome::Done(o) => {
                a.counts.add(*o);
                if a.schedule_processing {
                    schedule_for_outcome(&tx, now, *o, w.media)?;
                }
            }
            BatchOutcome::NeedHash => need_hash.push((i, Need::Hash)),
            BatchOutcome::NeedEdge => need_hash.push((i, Need::Edge)),
            BatchOutcome::Failed(e) => {
                a.counts.errors += 1;
                record_error(
                    &tx,
                    a.folder_id,
                    &w.rel_root,
                    core_code(e),
                    &e.to_string(),
                    now,
                )?;
            }
        }
    }
    let mut leftover = None;
    if need_hash.is_empty() {
        if let Some(t) = a.tail.take() {
            if !t.seen.is_empty() {
                touch_excluded_paths(&tx, now, a.volume_id, &t.seen, a.scan_at)?;
            }
            for d in &t.dirs {
                upsert_dir(&tx, a.folder_id, d, a.scan_at)?;
            }
            for e in &t.errors {
                a.counts.errors += 1;
                record_error(&tx, a.folder_id, &e.rel, &e.code, &e.message, now)?;
            }
            if let (Some(run), Some(cursor)) = (a.run_id, &t.cursor) {
                let counts = serde_json::to_string(&a.counts).unwrap_or_else(|_| "{}".into());
                tx.execute(
                    "UPDATE scan_runs SET cursor = ?2, counts = ?3 WHERE run_id = ?1",
                    params![run, cursor, counts],
                )?;
            }
        }
    } else {
        leftover = a.tail.take();
    }
    tx.commit()?;
    Ok(ChunkResult {
        gone: false,
        need_hash,
        counts: a.counts,
        items: a.items,
        tail: leftover,
    })
}

impl<'a> Walk<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        env: &'a ScanEnv,
        folder: &'a FolderRow,
        session: Arc<Mutex<ScanSession>>,
        scan_at: i64,
        run_id: Option<i64>,
        counts: ScanCounts,
        cancel: &'a AtomicBool,
        stop: &'a dyn Fn() -> bool,
    ) -> Self {
        Self {
            env,
            folder,
            rules: env.rules_for(folder),
            session,
            scan_at,
            run_id,
            counts,
            cancel,
            stop,
            defer_source: None,
            deferred_overflow: false,
            buf: Buffer::new(),
        }
    }

    pub fn counts(&self) -> ScanCounts {
        self.counts
    }

    /// 수정 시각이 PC 시계 기준 `recent_guard_ms` 이내이거나 미래인가 (NAS 시계가 달라도 보수적으로 미룬다).
    fn is_recent(&self, modified_ms: i64) -> bool {
        let g = self.env.recent_guard_ms;
        g > 0 && self.env.clock.now_ms().saturating_sub(modified_ms) < g
    }

    /// 지금 붙들고 있는 경로를 진행 상태에 기록한다 (멈춤 감시용)
    fn hold(&self, path: &std::path::Path) {
        self.env.progress.hold(
            self.folder.folder_id,
            &super::folders::path_text(path),
            self.env.clock.now_ms(),
        );
    }

    /// 볼륨이 응답 없음이라 순회를 멈춘다 — 모아 둔 것을 반영하고 재개 지점만 남긴다 (끝난 것으로 치지 않으므로 missing 처리도 없다)
    fn stop_unresponsive(&mut self) -> Result<WalkEnd, IngestError> {
        Ok(match self.flush()? {
            Flow::Continue => WalkEnd::Unresponsive,
            Flow::Gone => WalkEnd::FolderGone,
        })
    }

    fn vol_rel(&self, rel_root: &str) -> String {
        let tail = rel_root.replace('/', "\\");
        match (self.folder.rel_root.is_empty(), tail.is_empty()) {
            (true, _) => tail,
            (false, true) => self.folder.rel_root.clone(),
            (false, false) => format!("{}\\{}", self.folder.rel_root, tail),
        }
    }

    /// `start` 부터 하위를 pre-order 로 순회해 반영한다. `cursor` 가 있으면 그 지점까지 끝난 디렉터리는 건너뛴다.
    pub fn walk(&mut self, start: Task, cursor: Option<&str>) -> Result<WalkEnd, IngestError> {
        let mut stack = vec![start];
        while let Some(task) = stack.pop() {
            if (self.stop)() {
                return Ok(match self.flush()? {
                    Flow::Continue => WalkEnd::Stopped,
                    Flow::Gone => WalkEnd::FolderGone,
                });
            }
            let files_done =
                cursor.is_some_and(|c| preorder_cmp(&task.key, c) != Ordering::Greater);
            self.hold(&task.abs);
            let listing = match self.env.list_dir_guarded(self.folder, &task.abs, &self.rules) {
                Guarded::Done(Ok(l)) => l,
                Guarded::Done(Err(f)) => {
                    self.buf.tail.errors.push(ErrRec {
                        rel: task.rel.clone(),
                        code: f.code.into(),
                        message: f.message,
                    });
                    Listing::default()
                }
                // 읽지 못한 디렉터리는 missing 이 아니다 — 오류로 남기면 완료 단계가 그 하위를 "봤다" 로 보호한다
                Guarded::TimedOut => {
                    if self.env.io.is_unresponsive(self.folder.volume_id) {
                        return self.stop_unresponsive();
                    }
                    self.buf.tail.errors.push(ErrRec {
                        rel: task.rel.clone(),
                        code: UNRESPONSIVE_CODE.into(),
                        message: UNRESPONSIVE_MESSAGE.into(),
                    });
                    Listing::default()
                }
            };
            for ee in &listing.entry_errors {
                self.buf.tail.errors.push(ErrRec {
                    rel: join_rel(&task.rel, &ee.name),
                    code: io_code(&ee.error).into(),
                    message: ee.error.to_string(),
                });
            }
            if !files_done {
                for f in &listing.files {
                    let rel_root = join_rel(&task.rel, &f.name);
                    self.hold(&f.abs);
                    if self.defer_source.is_some() {
                        // 이미 있는 위치는 "봤다" 로 표시해 missing 으로 오판되지 않게 한다 (새 파일이면 아직 위치가 없어 무해하다)
                        self.buf.tail.seen.push(self.vol_rel(&rel_root));
                        self.buf.deferred.push(f.abs.clone());
                        continue;
                    }
                    match self.env.stat_guarded(self.folder, &f.abs) {
                        // 결정 I: 방금 바뀌었거나 미래 시각인 파일은 복사 중일 수 있다 — 반영하지 않고 안정화 게이트에 넘긴다.
                        // 이미 있는 위치는 "봤다" 로 표시해 missing 으로 오판되지 않게 한다 (새 파일이면 아직 위치가 없어 무해하다)
                        Guarded::Done(Ok(meta)) if self.is_recent(meta.modified_ms) => {
                            self.buf.tail.seen.push(self.vol_rel(&rel_root));
                            self.buf.deferred.push(f.abs.clone());
                        }
                        Guarded::Done(Ok(meta)) => self.buf.work.push(Work {
                            rel_vol: self.vol_rel(&rel_root),
                            rel_root,
                            abs: f.abs.clone(),
                            media: f.media,
                            meta,
                        }),
                        // 목록화 직후 사라진 파일은 오류가 아니다 (마지막에 missing 처리)
                        Guarded::Done(Err(Fail { not_found: true, .. })) => {}
                        Guarded::Done(Err(f)) => self.buf.tail.errors.push(ErrRec {
                            rel: rel_root,
                            code: f.code.into(),
                            message: f.message,
                        }),
                        // 응답 없음 — 이 파일은 이번에 건너뛴다 (오류가 위치를 "봤다" 로 보호해 missing 이 되지 않는다)
                        Guarded::TimedOut => {
                            if self.env.io.is_unresponsive(self.folder.volume_id) {
                                return self.stop_unresponsive();
                            }
                            self.buf.tail.errors.push(ErrRec {
                                rel: rel_root,
                                code: UNRESPONSIVE_CODE.into(),
                                message: UNRESPONSIVE_MESSAGE.into(),
                            });
                        }
                    }
                }
                for f in &listing.skipped {
                    let rel_root = join_rel(&task.rel, &f.name);
                    self.buf.tail.seen.push(self.vol_rel(&rel_root));
                }
                if !task.key.is_empty() {
                    self.buf.tail.dirs.push(DirRec {
                        key: task.key.clone(),
                        parent_key: task.parent_key.clone(),
                        name: task.name.clone(),
                        rel: task.rel.clone(),
                    });
                }
                self.buf.tail.cursor = Some(task.key.clone());
                self.buf.dirs_since_flush += 1;
            }
            let mut children = Vec::new();
            for d in listing.dirs {
                let key = join_key(&task.key, &d.key);
                if cursor.is_some_and(|c| subtree_before(&key, c)) {
                    continue;
                }
                children.push(Task {
                    rel: join_rel(&task.rel, &d.name),
                    abs: d.abs,
                    key,
                    name: d.name,
                    parent_key: task.key.clone(),
                });
            }
            stack.extend(children.into_iter().rev());
            let due = self.buf.work.len() + self.buf.tail.seen.len() >= BATCH_FILES
                || self.buf.dirs_since_flush >= FLUSH_DIRS
                || self.buf.last_flush.elapsed() >= FLUSH_EVERY;
            if due && matches!(self.flush()?, Flow::Gone) {
                return Ok(WalkEnd::FolderGone);
            }
        }
        Ok(match self.flush()? {
            Flow::Continue => WalkEnd::Completed,
            Flow::Gone => WalkEnd::FolderGone,
        })
    }

    /// 단발 반영(감시 이벤트): 파일들과 폴더 트리 기록을 한 번에 쓴다.
    /// `seen` = 대상 유형 밖이라 등록하지 않는 파일의 볼륨 기준 상대 경로 (존재 확인만).
    pub fn apply_once(
        &mut self,
        work: Vec<Work>,
        dirs: Vec<DirRec>,
        seen: Vec<String>,
    ) -> Result<bool, IngestError> {
        self.buf.work.extend(work);
        self.buf.tail.dirs.extend(dirs);
        self.buf.tail.seen.extend(seen);
        Ok(matches!(self.flush()?, Flow::Continue))
    }

    fn flush(&mut self) -> Result<Flow, IngestError> {
        let buf = std::mem::replace(&mut self.buf, Buffer::new());
        self.defer_to_pending(&buf.deferred);
        let mut tail = (!buf.tail.is_empty()).then_some(buf.tail);
        let mut chunks: Vec<Vec<Work>> = Vec::new();
        let mut it = buf.work.into_iter().peekable();
        while it.peek().is_some() {
            chunks.push(it.by_ref().take(BATCH_FILES).collect());
        }
        if chunks.is_empty() && tail.is_some() {
            chunks.push(Vec::new());
        }
        let n = chunks.len();
        for (i, chunk) in chunks.into_iter().enumerate() {
            let t = if i + 1 == n { tail.take() } else { None };
            if let Flow::Gone = self.write_chunk(chunk, t)? {
                return Ok(Flow::Gone);
            }
        }
        Ok(Flow::Continue)
    }

    /// 반영을 미룬 파일을 변경 대기열에 넣는다 (source = 감시 폴더 순회면 그 감지기 ID, 아니면 `scan`).
    /// 실패해도 순회는 계속한다 — 일반 스캔은 다음 스캔이 다시 보고, 감시 폴더 순회는 `deferred_overflow` 로 재스캔을 요청한다.
    fn defer_to_pending(&mut self, files: &[PathBuf]) {
        use crate::ingest::pending::store::{Incoming, PendingKind, upsert_batch};
        if files.is_empty() {
            return;
        }
        let folder_id = self.folder.folder_id;
        let source = self.defer_source.as_deref().unwrap_or("scan");
        let items: Vec<Incoming> = files
            .iter()
            .filter_map(|p| Incoming::new(folder_id, p, PendingKind::Path, source))
            .collect();
        let (now, cap) = (self.env.clock.now_ms(), self.env.pending_cap);
        let due = now + self.env.pending_due_ms;
        match self.env.write(Priority::Low, move |c| upsert_batch(c, &items, now, due, cap)) {
            Ok(report) if report.overflowed.is_empty() => {}
            Ok(_) => self.deferred_overflow = true,
            Err(e) => {
                tracing::warn!(%e, folder_id, "cannot queue recently modified files for settling");
                self.deferred_overflow = true;
            }
        }
    }

    fn run_chunk(
        &self,
        items: Vec<(Work, Pre)>,
        tail: Option<Tail>,
    ) -> Result<ChunkResult, IngestError> {
        let args = ChunkArgs {
            folder_id: self.folder.folder_id,
            volume_id: self.folder.volume_id,
            path_key: self.folder.path_key.clone(),
            run_id: self.run_id,
            session: Arc::clone(&self.session),
            clock: Arc::clone(&self.env.clock),
            scan_at: self.scan_at,
            counts: self.counts,
            items,
            tail,
            schedule_processing: self.env.schedule_processing.load(AtomicOrdering::Relaxed),
            types: self.folder.media_types,
        };
        self.env.write(Priority::Low, move |c| apply_chunk(c, args))
    }

    fn write_chunk(&mut self, chunk: Vec<Work>, tail: Option<Tail>) -> Result<Flow, IngestError> {
        let before = self.counts.processed;
        let r = self.run_chunk(chunk.into_iter().map(|w| (w, Pre::default())).collect(), tail)?;
        self.counts = r.counts;
        if r.gone {
            return Ok(Flow::Gone);
        }
        if !r.need_hash.is_empty() {
            let mut second = Vec::new();
            let mut tail = r.tail.unwrap_or_default();
            for (idx, need) in r.need_hash {
                let work = r.items[idx].0.clone();
                self.hold(&work.abs);
                let computed = match need {
                    Need::Hash => self
                        .env
                        .hasher
                        .hash_file_progress(&work.abs, self.cancel, &mut |_| {
                            self.env
                                .progress
                                .bytes_read(self.folder.folder_id, self.env.clock.now_ms());
                        })
                        .map(|h| Pre {
                            hash: Some(h),
                            edge: None,
                        }),
                    Need::Edge => self
                        .env
                        .hasher
                        .edge_signature(&work.abs, self.cancel)
                        .map(|e| Pre {
                            hash: None,
                            edge: Some(e),
                        }),
                };
                match computed {
                    Ok(pre) => second.push((work, pre)),
                    Err(CoreError::Cancelled) => return Err(CoreError::Cancelled.into()),
                    // 사라진 파일은 오류가 아니다 (missing 처리). 그 밖에는 기록하고 이번엔 보류한다
                    Err(CoreError::Io { source, .. })
                        if source.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => tail.errors.push(ErrRec {
                        rel: work.rel_root.clone(),
                        code: core_code(&e).into(),
                        message: e.to_string(),
                    }),
                }
            }
            let r2 = self.run_chunk(second, Some(tail))?;
            self.counts = r2.counts;
            if r2.gone {
                return Ok(Flow::Gone);
            }
            if !r2.need_hash.is_empty() {
                return Err(CoreError::Internal("hash requested twice".into()).into());
            }
        }
        self.env.progress.add(
            self.folder.folder_id,
            self.counts.processed.saturating_sub(before),
            self.env.clock.now_ms(),
        );
        Ok(Flow::Continue)
    }
}
