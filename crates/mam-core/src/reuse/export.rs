//! 내보내기 — 선택한 자산의 원본을 읽어 대상 폴더에 복사한다 (D5).
//!
//! 준비(`prepare_export`)에서 입력을 검증하고 자산을 해석하며, 실행(`ExportJob::run`)은 백그라운드 스레드에서
//! 한 건씩 복사하고 진행 이벤트를 낸다(최대 4Hz + 마지막 1회). 대상은 관리 폴더·MAM 데이터 안이면 거부한다.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::manifest::{self, MANIFEST_NAME};
use super::resolve::{self, Access, Resolved};
use super::{
    ConflictPolicy, ExportFs, ExportInput, ExportProgress, ProgressSink, Reuse, TaskStarted,
};
use crate::SharedClock;
use crate::scan::{IngestError, load_all};
use crate::userdata::clean_ids;

/// 진행 이벤트 최소 간격 (4Hz)
const EMIT_EVERY_MS: i64 = 250;
/// 이름 충돌 시 `이름 (n).확장자` 를 시도하는 최대 n
const MAX_RENAME_TRIES: u32 = 9_999;

/// 내보내기를 중간에 멈춘 이유
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    /// 대상 디스크 공간 부족 — 남은 항목은 실패로 센다
    DiskFull,
}

/// 실행 결과 (로그·시험용; 사용자에게는 진행 이벤트의 집계가 간다)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    pub total: u64,
    pub copied: u64,
    pub failed: u64,
    pub skipped: u64,
    pub manifest_written: bool,
    pub abort: Option<AbortReason>,
}

pub struct ExportJob {
    task_id: String,
    dest: PathBuf,
    policy: ConflictPolicy,
    manifest: bool,
    items: Vec<Resolved>,
    fs: Arc<dyn ExportFs>,
    clock: SharedClock,
}

impl Reuse {
    /// 입력을 검증하고 자산을 해석한다. 아직 아무 파일도 쓰지 않는다.
    pub fn prepare_export(&self, input: ExportInput) -> Result<ExportJob, IngestError> {
        let ids = clean_ids(&input.asset_ids)?;
        if ids.is_empty() {
            return Err(IngestError::InvalidInput("assetIds"));
        }
        let dest = self.checked_dest(&input.dest_dir)?;
        let items = self.pool.read(|c| Ok(resolve::resolve(c, &ids)))??;
        let n = self.seq.fetch_add(1, Ordering::Relaxed);
        Ok(ExportJob {
            task_id: format!("export-{}-{n}", self.clock.now_ms()),
            dest,
            policy: input.conflict,
            manifest: input.include_manifest,
            items,
            fs: Arc::clone(&self.fs),
            clock: Arc::clone(&self.clock),
        })
    }

    /// 준비 후 백그라운드 스레드에서 실행한다. 반환 시점에 이벤트가 이미 나가기 시작했을 수 있다
    /// (이벤트는 `taskId` 를 담는다).
    pub fn start_export(
        &self,
        input: ExportInput,
        sink: ProgressSink,
    ) -> Result<TaskStarted, IngestError> {
        let job = self.prepare_export(input)?;
        let task_id = job.task_id.clone();
        std::thread::Builder::new()
            .name("mam-export".into())
            .spawn(move || {
                let report = job.run(&*sink);
                tracing::info!(
                    total = report.total,
                    copied = report.copied,
                    failed = report.failed,
                    skipped = report.skipped,
                    manifest = report.manifest_written,
                    aborted = report.abort.is_some(),
                    "export finished"
                );
            })?;
        Ok(TaskStarted { task_id })
    }

    /// 대상 폴더: 존재하는 디렉터리여야 하고 관리 폴더·MAM 데이터 안이면 안 된다 (원본·관리 데이터 오염 방지).
    fn checked_dest(&self, raw: &str) -> Result<PathBuf, IngestError> {
        if raw.is_empty() || raw.contains('\0') || !Path::new(raw).is_absolute() {
            return Err(IngestError::InvalidInput("destDir"));
        }
        let dest = match std::fs::canonicalize(raw) {
            Ok(p) => p,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(IngestError::InvalidInput("destDir"));
            }
            Err(e) => return Err(e.into()),
        };
        if !dest.is_dir() {
            return Err(IngestError::InvalidInput("destDir is not a directory"));
        }
        let managed = self.pool.read(|c| Ok(load_all(c)))??;
        let roots = managed
            .iter()
            .map(|f| f.root_path())
            .chain(self.protected.iter().cloned());
        let dest_key = comparable(&dest);
        for root in roots {
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            if dest_key.starts_with(comparable(&root)) {
                return Err(IngestError::InvalidInput(
                    "destDir is inside a managed folder",
                ));
            }
        }
        Ok(dest)
    }
}

/// 대소문자 무시·확장 접두어 제거 비교용 경로 (`Path::starts_with` 는 구성 요소 단위라 `C:\ab` 가 `C:\a` 에 속하지 않는다)
fn comparable(p: &Path) -> PathBuf {
    PathBuf::from(crate::volume::strip_verbatim(&p.to_string_lossy()).to_lowercase())
}

struct Emitter<'a> {
    sink: &'a dyn Fn(ExportProgress),
    clock: &'a SharedClock,
    last: Option<i64>,
    task_id: &'a str,
    total: u64,
}

impl Emitter<'_> {
    fn emit(&mut self, done: u64, failed: u64, skipped: u64, finished: bool) {
        let now = self.clock.now_ms();
        if !finished && self.last.is_some_and(|l| now - l < EMIT_EVERY_MS) {
            return;
        }
        self.last = Some(now);
        (self.sink)(ExportProgress {
            task_id: self.task_id.to_string(),
            done,
            total: self.total,
            failed,
            skipped,
            finished,
        });
    }
}

enum Placed {
    Written(String),
    Skipped,
}

/// 대상 폴더에 이름을 정해 쓴다 (충돌 정책 적용). 이번 실행에서 이미 쓴 이름은 `used` 로 추적한다.
struct Placer<'a> {
    dest: &'a Path,
    policy: ConflictPolicy,
    used: HashSet<String>,
}

impl Placer<'_> {
    fn place(
        &mut self,
        name: &str,
        write: &dyn Fn(&Path, bool) -> io::Result<()>,
    ) -> io::Result<Placed> {
        // overwrite 는 "이전 내보내기·기존 파일" 만 덮어쓴다. 이번 실행에서 방금 쓴 같은 이름은 지우지 않는다.
        let overwrite =
            self.policy == ConflictPolicy::Overwrite && !self.used.contains(&name.to_lowercase());
        let result: io::Result<String> = if overwrite {
            write(&self.dest.join(name), true).map(|()| name.to_string())
        } else if self.policy == ConflictPolicy::Skip {
            match write(&self.dest.join(name), false) {
                Ok(()) => Ok(name.to_string()),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(Placed::Skipped),
                Err(e) => Err(e),
            }
        } else {
            self.first_free(name, write)
        };
        let final_name = result?;
        self.used.insert(final_name.to_lowercase());
        Ok(Placed::Written(final_name))
    }

    /// `이름`, `이름 (1)`, `이름 (2)` ... 중 처음으로 만들 수 있는 이름 (create_new 라 경합에도 덮어쓰지 않는다)
    fn first_free(
        &self,
        name: &str,
        write: &dyn Fn(&Path, bool) -> io::Result<()>,
    ) -> io::Result<String> {
        let path = Path::new(name);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        for n in 0..=MAX_RENAME_TRIES {
            let candidate = if n == 0 {
                name.to_string()
            } else {
                format!("{stem} ({n}){ext}")
            };
            if self.used.contains(&candidate.to_lowercase()) {
                continue;
            }
            match write(&self.dest.join(&candidate), false) {
                Ok(()) => return Ok(candidate),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no free file name",
        ))
    }
}

impl ExportJob {
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// 한 건씩 복사한다. 오프라인·파일 없음·개별 복사 실패는 실패로 집계하고 계속한다.
    /// 디스크 가득 참은 남은 항목이 모두 실패할 것이므로 중단한다. 마지막에 `finished: true` 이벤트를 반드시 낸다.
    pub fn run(self, sink: &dyn Fn(ExportProgress)) -> ExportReport {
        let total = self.items.len() as u64;
        let mut emitter = Emitter {
            sink,
            clock: &self.clock,
            last: None,
            task_id: &self.task_id,
            total,
        };
        let mut placer = Placer {
            dest: &self.dest,
            policy: self.policy,
            used: HashSet::new(),
        };
        let (mut copied, mut failed, mut skipped) = (0u64, 0u64, 0u64);
        let mut abort = None;
        let mut exported: Vec<(String, &Resolved, String)> = Vec::new();

        for item in &self.items {
            if abort.is_some() {
                failed += 1;
                continue;
            }
            let outcome = match &item.access {
                Access::Ready(src) if src.is_file() => {
                    let name = src
                        .file_name()
                        .and_then(|n| n.to_str())
                        .filter(|n| !n.is_empty())
                        .map(str::to_string);
                    match name {
                        Some(name) => {
                            placer.place(&name, &|dst, replace| self.fs.copy(src, dst, replace))
                        }
                        None => Err(io::Error::new(io::ErrorKind::InvalidInput, "no file name")),
                    }
                }
                _ => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "source unavailable",
                )),
            };
            match outcome {
                Ok(Placed::Written(name)) => {
                    copied += 1;
                    if let Access::Ready(src) = &item.access {
                        exported.push((name, item, src.to_string_lossy().into_owned()));
                    }
                }
                Ok(Placed::Skipped) => skipped += 1,
                Err(e) => {
                    failed += 1;
                    if e.kind() == io::ErrorKind::StorageFull {
                        abort = Some(AbortReason::DiskFull);
                    }
                    // 경로는 로그에 남기지 않는다 (자산 id 와 오류 종류만)
                    tracing::warn!(asset_id = item.asset_id, kind = ?e.kind(), "export item failed");
                }
            }
            emitter.emit(copied + failed + skipped, failed, skipped, false);
        }

        let manifest_written = self.manifest
            && abort.is_none()
            && !exported.is_empty()
            && self.write_manifest(&mut placer, &exported);
        emitter.emit(copied + failed + skipped, failed, skipped, true);
        ExportReport {
            total,
            copied,
            failed,
            skipped,
            manifest_written,
            abort,
        }
    }

    fn write_manifest(
        &self,
        placer: &mut Placer<'_>,
        exported: &[(String, &Resolved, String)],
    ) -> bool {
        let rows: Vec<manifest::Row<'_>> = exported
            .iter()
            .map(|(name, item, original)| manifest::Row {
                file_name: name,
                original_path: original,
                tags: &item.tags,
                memo: &item.memo,
                captured_at: item.captured_at,
            })
            .collect();
        let data = manifest::render(&rows);
        match placer.place(MANIFEST_NAME, &|dst, replace| {
            self.fs.write(dst, &data, replace)
        }) {
            Ok(Placed::Written(_)) => true,
            Ok(Placed::Skipped) => false,
            Err(e) => {
                tracing::warn!(kind = ?e.kind(), "cannot write export manifest");
                false
            }
        }
    }
}
