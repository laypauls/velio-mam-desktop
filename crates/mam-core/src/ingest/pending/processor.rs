//! 처리기 — `due_at <= now` 인 행을 기한 순으로 최대 N개 꺼내 안정화 게이트를 거쳐 `apply_path` 로 반영한다.
//!
//! 파일 상태 조회(`stat`)·배타적 열기 검사·`apply_path` 는 모두 DB 쓰기 스레드 밖에서 하고, 표본 갱신·삭제·실패 기록은
//! 한 번의 Low 우선순위 트랜잭션으로 모아 쓴다. 꺼내기 준비 중인 폴더의 행은 건드리지 않는다(행 유지).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use mam_db::Priority;

use super::MAX_ATTEMPTS;
use super::settle::{ExclusiveOutcome, Obs, Step, probe_exclusive};
use super::store::{self, Mutation, PendingRow};
use crate::identity::modified_ms;
use crate::ingest::Ingest;
use crate::ingest::stop::Tick;
use crate::scan::{
    Fail, Guarded, IngestError, PathAction, ScanEnv, UNRESPONSIVE_CODE, UNRESPONSIVE_MESSAGE,
    apply_path_settled, load_folder, record_error,
};
use crate::volume::{VolumeKind, strip_verbatim};
use crate::watch::exclusive_open_check_of;

/// 응답 없는 볼륨의 행을 다시 확인하기까지의 대기 (시도 횟수는 올리지 않는다 — 연결 끊김은 포기 사유가 아니다)
const UNRESPONSIVE_RETRY_MS: i64 = 30_000;

/// 한 번의 점검 결과 (시험·로그용).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PendingPassReport {
    /// 꺼낸 행 수
    pub examined: usize,
    /// `apply_path` 로 반영하고 지운 행 (삭제 반영 포함)
    pub applied: usize,
    /// 아직 안정이 아니라 표본만 갱신한 행
    pub waiting: usize,
    /// `apply_path` 가 실패해 백오프로 미룬 행
    pub retried: usize,
    /// 볼륨이 응답하지 않아 표본을 못 읽고 미룬 행 (시도 횟수 변화 없음, `scan_errors` 에 "응답 없음" 기록)
    pub unresponsive: usize,
    /// 재시도 상한에 닿아 `scan_errors` 에 남기고 지운 행
    pub gave_up: usize,
}

/// 재시도 백오프: 5초, 10초, 20초, 40초 … 최대 5분
fn backoff_ms(attempts: i64) -> i64 {
    let exp = u32::try_from((attempts - 1).clamp(0, 16)).unwrap_or(16);
    (5_000_i64 << exp).min(300_000)
}

fn error_code(e: &IngestError) -> &'static str {
    match e {
        IngestError::Io(_) | IngestError::Core(crate::CoreError::Io { .. }) => "io",
        _ => "internal",
    }
}

/// 폴더 루트 기준 상대 경로('/' 구분) — 루트 밖이면 절대 경로 그대로.
fn rel_for_error(root: &Path, abs: &Path) -> String {
    let parts = |p: &Path| -> Vec<String> {
        PathBuf::from(strip_verbatim(&p.to_string_lossy()))
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect()
    };
    let (r, a) = (parts(root), parts(abs));
    let inside = a.len() > r.len()
        && r.iter()
            .zip(&a)
            .all(|(x, y)| x.to_lowercase() == y.to_lowercase());
    if inside {
        a[r.len()..].join("/")
    } else {
        strip_verbatim(&abs.to_string_lossy())
    }
}

#[derive(Clone)]
struct Seen {
    obs: Obs,
    is_file: bool,
}

/// 폴더 하나의 처리기용 정보 (한 점검에서 한 번만 읽는다)
#[derive(Clone, Copy)]
struct FolderInfo {
    volume_id: i64,
    kind: VolumeKind,
    exclusive: bool,
}

/// 읽기 전용 상태 조회 — 심볼릭 링크를 따라가지 않는다. 없으면 `Gone`.
fn observe(path: &Path) -> Result<Seen, Fail> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => Ok(Seen {
            obs: Obs::Present {
                size: i64::try_from(m.len()).unwrap_or(i64::MAX),
                mtime_ms: m.modified().map_or(0, modified_ms),
            },
            is_file: m.is_file(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Seen {
            obs: Obs::Gone,
            is_file: false,
        }),
        Err(e) => Err(Fail::io(&e)),
    }
}

impl Ingest {
    /// 점검 1회. 스레드가 주기적으로 부르며, 시험에서는 주입 시계를 전진시키며 직접 부른다.
    pub fn process_pending_once(&self) -> Result<PendingPassReport, IngestError> {
        // 폴더는 하위 파일을 바로 반영하지 않고 같은 게이트로 되돌려 보낸다 (출처는 지금 감시 중인 감지기 ID)
        self.process_pending_with(&|env, folder_id, path| {
            apply_path_settled(env, folder_id, path, self.source_of(folder_id))
        })
    }

    /// [`Self::process_pending_once`] 의 반영 함수를 바꿀 수 있는 형태 (시험용 — 실패·경합을 결정적으로 만든다).
    pub fn process_pending_with(
        &self,
        apply: &dyn Fn(&ScanEnv, i64, &Path) -> Result<PathAction, IngestError>,
    ) -> Result<PendingPassReport, IngestError> {
        let now = self.env.clock.now_ms();
        let held = self.held_folder_ids();
        let limit = self.config.pending.batch;
        let rows = self
            .env
            .pool
            .read(|c| Ok(store::due_rows(c, now, limit, &held)))??;
        let mut report = PendingPassReport {
            examined: rows.len(),
            ..PendingPassReport::default()
        };
        let mut muts: Vec<Mutation> = Vec::with_capacity(rows.len());
        let mut infos: HashMap<i64, FolderInfo> = HashMap::new();
        // 응답 없는 볼륨에서 못 읽은 행 — 점검 끝에 한 번에 `scan_errors` 로 남긴다
        let mut unresponsive: Vec<(i64, String)> = Vec::new();
        let mut applied_any = false;
        for row in rows {
            if self.stop.is_set() {
                break;
            }
            // 점검을 시작한 뒤 꺼내기 준비가 시작됐을 수 있다 — 파일을 열지 않고 행을 그대로 둔다 (재연결 뒤 다시 확인)
            if self.folder_held(row.folder_id) {
                continue;
            }
            let path = PathBuf::from(&row.abs_path);
            let del = |m: &mut Vec<Mutation>| {
                m.push(Mutation::Delete {
                    folder_id: row.folder_id,
                    path_key: row.path_key.clone(),
                    seen: row.last_seen_at,
                });
            };
            // 원본 이동 중인 폴더 아래의 이벤트는 버린다 (이동 작업의 재연결이 맡는다)
            if self.suppressed.covers(&path) {
                del(&mut muts);
                continue;
            }
            let info = *infos
                .entry(row.folder_id)
                .or_insert_with(|| self.folder_info(row.folder_id));
            // 막힐 수 있는 볼륨은 제한 시간을 건다 — 응답이 없으면 이 행은 건너뛰고 나중에 다시 본다 (사라진 것으로 치지 않는다)
            let probe_path = path.clone();
            let observed = self.env.guarded_io(
                info.kind,
                info.volume_id,
                format!("pe:{}", path.display()),
                move || observe(&probe_path),
            );
            let seen = match observed {
                Guarded::Done(Ok(s)) => s,
                Guarded::Done(Err(f)) => {
                    self.fail(&row, &f.message, f.code, now, true, &mut muts, &mut report);
                    continue;
                }
                Guarded::TimedOut => {
                    muts.push(Mutation::Resample {
                        folder_id: row.folder_id,
                        path_key: row.path_key.clone(),
                        seen: row.last_seen_at,
                        size: row.sample_size.unwrap_or(0),
                        mtime: row.sample_mtime.unwrap_or(0),
                        stable: 0,
                        due_at: now + UNRESPONSIVE_RETRY_MS,
                    });
                    unresponsive.push((row.folder_id, row.abs_path.clone()));
                    report.unresponsive += 1;
                    continue;
                }
            };
            let check = info.exclusive;
            // 배타적 열기도 응답 없는 공유에서 영구히 막힐 수 있다 — 같은 제한 시간 안에서만 연다.
            // 제한 시간을 넘기면 "열려 있음"으로 보고 이번에는 반영하지 않는다(나중에 다시 확인, 삭제 아님).
            let mut probe = || {
                let p = path.clone();
                let outcome = self.env.guarded_io(
                    info.kind,
                    info.volume_id,
                    format!("px:{}", path.display()),
                    move || probe_exclusive(&p),
                );
                match outcome {
                    Guarded::Done(ExclusiveOutcome::Unknown) => {
                        tracing::debug!(
                            folder_id = row.folder_id,
                            "exclusive open check unavailable; using size/mtime only"
                        );
                        ExclusiveOutcome::Unknown
                    }
                    Guarded::Done(other) => other,
                    Guarded::TimedOut => ExclusiveOutcome::Busy,
                }
            };
            let exclusive = (check && seen.is_file)
                .then_some(&mut probe as &mut dyn FnMut() -> ExclusiveOutcome);
            match self
                .config
                .pending
                .gate
                .step(&row, seen.obs, now, exclusive)
            {
                Step::Resample {
                    size,
                    mtime,
                    stable,
                    due_at,
                    overdue,
                } => {
                    if overdue
                        && crate::ingest::lock(&self.pending.warned_overdue)
                            .insert((row.folder_id, row.path_key.clone()))
                    {
                        tracing::warn!(
                            folder_id = row.folder_id,
                            "file keeps changing; copy may not be finished"
                        );
                    }
                    muts.push(Mutation::Resample {
                        folder_id: row.folder_id,
                        path_key: row.path_key.clone(),
                        seen: row.last_seen_at,
                        size,
                        mtime,
                        stable,
                        due_at,
                    });
                    report.waiting += 1;
                }
                Step::Apply => match apply(&self.env, row.folder_id, &path) {
                    // 읽지 못해 scan_errors 에 이미 기록됐다 — 일시 오류일 수 있으니 백오프로 다시 본다 (포기할 때 중복 기록은 하지 않는다)
                    Ok(PathAction::Failed) => self.fail(
                        &row,
                        "read failed",
                        "io",
                        now,
                        false,
                        &mut muts,
                        &mut report,
                    ),
                    Ok(done) => {
                        // 폴더 하위 파일을 행으로 다 넘기지 못했다 — 재스캔 1회가 대신 맞춘다 (기존 폴더 상한 초과와 같은 처리)
                        if done == PathAction::SubtreeOverflow {
                            self.on_overflow(&[row.folder_id]);
                        }
                        applied_any = true;
                        del(&mut muts);
                        report.applied += 1;
                        crate::ingest::lock(&self.pending.warned_overdue)
                            .remove(&(row.folder_id, row.path_key.clone()));
                    }
                    Err(e) => self.fail(
                        &row,
                        &e.to_string(),
                        error_code(&e),
                        now,
                        true,
                        &mut muts,
                        &mut report,
                    ),
                },
            }
        }
        if !muts.is_empty() {
            self.env
                .write(Priority::Low, move |c| store::apply_mutations(c, &muts))?;
        }
        self.record_unresponsive(&unresponsive, now);
        if applied_any {
            self.jobs.notify();
        }
        self.release_overflow_marks();
        Ok(report)
    }

    fn folder_info(&self, folder_id: i64) -> FolderInfo {
        let loaded = self
            .env
            .pool
            .read(|c| Ok(load_folder(c, folder_id)))
            .ok()
            .and_then(Result::ok);
        match loaded {
            Some(f) => FolderInfo {
                volume_id: f.volume_id,
                kind: f.volume_kind,
                exclusive: exclusive_open_check_of(f.watch_options.as_deref()),
            },
            // 폴더가 사라졌다 — 검사 없이 `apply_path` 가 무시하고 행이 지워진다
            None => FolderInfo {
                volume_id: 0,
                kind: VolumeKind::Fixed,
                exclusive: false,
            },
        }
    }

    /// 응답 없음으로 읽지 못한 경로를 `scan_errors`(코드 `unresponsive`)에 남긴다. 실패해도 점검은 계속한다.
    fn record_unresponsive(&self, items: &[(i64, String)], now: i64) {
        if items.is_empty() {
            return;
        }
        let mut rels: Vec<(i64, String)> = Vec::with_capacity(items.len());
        for (folder_id, abs) in items {
            let rel = self
                .env
                .pool
                .read(|c| Ok(load_folder(c, *folder_id)))
                .ok()
                .and_then(Result::ok)
                .map_or_else(
                    || abs.clone(),
                    |f| rel_for_error(&f.root_path(), Path::new(abs)),
                );
            rels.push((*folder_id, rel));
        }
        if let Err(e) = self.env.write(Priority::Low, move |c| {
            for (folder_id, rel) in &rels {
                record_error(
                    c,
                    *folder_id,
                    rel,
                    UNRESPONSIVE_CODE,
                    UNRESPONSIVE_MESSAGE,
                    now,
                )?;
            }
            Ok(())
        }) {
            tracing::warn!(%e, "cannot record unresponsive paths");
        }
    }

    /// 실패를 백오프로 미루고, 상한에 닿으면 (`record` 이면 `scan_errors` 에 남기고) 지운다.
    #[allow(clippy::too_many_arguments)]
    fn fail(
        &self,
        row: &PendingRow,
        message: &str,
        code: &'static str,
        now: i64,
        record: bool,
        muts: &mut Vec<Mutation>,
        report: &mut PendingPassReport,
    ) {
        let attempts = row.attempts + 1;
        if attempts < MAX_ATTEMPTS {
            muts.push(Mutation::Fail {
                folder_id: row.folder_id,
                path_key: row.path_key.clone(),
                seen: row.last_seen_at,
                attempts,
                error: message.to_string(),
                due_at: now + backoff_ms(attempts),
            });
            report.retried += 1;
            return;
        }
        tracing::warn!(
            folder_id = row.folder_id,
            attempts,
            "giving up applying a watch change; a rescan will reconcile it"
        );
        if record {
            let rel = self
                .env
                .pool
                .read(|c| Ok(load_folder(c, row.folder_id)))
                .ok()
                .and_then(Result::ok)
                .map_or_else(
                    || row.abs_path.clone(),
                    |f| rel_for_error(&f.root_path(), Path::new(&row.abs_path)),
                );
            let (folder_id, message) = (row.folder_id, message.to_string());
            if let Err(e) = self.env.write(Priority::Low, move |c| {
                record_error(c, folder_id, &rel, code, &message, now)
            }) {
                tracing::warn!(%e, "cannot record a watch apply failure");
            }
        }
        muts.push(Mutation::Delete {
            folder_id: row.folder_id,
            path_key: row.path_key.clone(),
            seen: row.last_seen_at,
        });
        report.gave_up += 1;
    }

    /// 상한에 막혀 재스캔으로 바꾼 폴더가 상한 아래로 내려오면 표시를 푼다.
    fn release_overflow_marks(&self) {
        let marked: Vec<i64> = crate::ingest::lock(&self.pending.overflowed)
            .iter()
            .copied()
            .collect();
        for f in marked {
            let below = self
                .env
                .pool
                .read(|c| Ok(store::count_for_folder(c, f)))
                .ok()
                .and_then(Result::ok)
                .is_some_and(|n| (n as usize) < self.config.pending.max_rows_per_folder);
            if below {
                crate::ingest::lock(&self.pending.overflowed).remove(&f);
            }
        }
    }

    pub(in crate::ingest) fn processor_loop(&self) {
        loop {
            match self.process_pending_once() {
                // 꽉 찬 묶음이면 바로 이어서 처리한다 (대량 복사 뒤 밀린 행)
                Ok(r) if r.examined >= self.config.pending.batch && !self.stop.is_set() => continue,
                Ok(_) => {}
                Err(e) => tracing::warn!(%e, "cannot process pending watch changes"),
            }
            // `wait_tick` 이 아니다 — 장치 깨우기 요청은 점검 스레드의 것이라 여기서 소비하면 점검이 깨지 않는다
            if self.stop.wait_stop(self.config.pending.process_every) == Tick::Stopped {
                break;
            }
        }
    }
}
