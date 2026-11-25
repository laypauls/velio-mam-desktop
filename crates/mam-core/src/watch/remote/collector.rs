//! 수집기 — 원격 접속 하나를 여러 폴더가 공유한다. 감사 로그를 이어 읽어 **경로가 속한 폴더에만** 보고한다.
//!
//! 한 번의 [`Collector::step`] = (필요하면 접속) -> 로그 읽기 -> 보고 기록(대기열 + 커서, **한 트랜잭션**) -> 재스캔 요청.
//! 스레드 루프(`hub.rs`)는 `step` 이 돌려준 시간만큼 기다린 뒤 다시 부른다 — 시험은 `step` 을 직접 불러 결정적으로 돌린다(시계·가짜 주입).
//!
//! 안전 장치
//! - 접속·읽기 실패 -> 해당 폴더 `Degraded`(사유 키), 15초부터 두 배씩(최대 60초) 다시 붙는다. 인증·호스트 키 오류는 서버 계정 잠금을 피하려고 길게(5분) 쉰다
//! - 다시 붙으면 `Ok` 로 돌아오고 **재스캔 1회**로 끊긴 구간을 덮는다. 첫 기준선·1 MiB 건너뜀·로그 교체도 재스캔
//! - 재스캔은 폴더마다 5초에 1번(`KickGate`) — 막힌 요청은 버리지 않고 다음 `step` 으로 미룬다
//! - 기록이 실패하면 커서를 올리지 않는다 -> 다음 주기에 같은 오프셋부터 다시 읽는다

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::RemoteSource;
use super::mapper::{Mapper, same_dir, within_dir};
use super::parse::{is_noise, parse_line};
use super::runner::{RemoteRunner, RunError};
use super::stats::{Counts, KickGate};
use super::tail::{Cursor, LogTail, Polled};

pub use super::contract::{
    Attached, BatchCommitter, CommitError, CommitOutcome, Deps, Feed, FeedItem, Folders,
    SourceLoader, Timings,
};

pub struct Collector {
    src: RemoteSource,
    mapper: Mapper,
    tail: LogTail,
    run: Option<Box<dyn RemoteRunner>>,
    d: Deps,
    gate: KickGate,
    /// 모든 폴더에 재스캔을 요청해야 한다 (다시 붙음·첫 기준선·건너뜀) / 특정 폴더만
    want_all: bool,
    want: BTreeSet<i64>,
    degraded: bool,
    down_since: i64,
    fail_streak: u32,
    window: Counts,
    total: Counts,
    last_report: i64,
    outside_noted: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// `Degraded` 사유 번역 키 (`watch.health.*`)
pub(super) fn health_key(e: &RunError) -> &'static str {
    match e {
        RunError::Connect(_) => "watch.health.remote_connect",
        RunError::Auth | RunError::BadKey => "watch.health.remote_auth",
        RunError::HostKeyMismatch { .. } | RunError::HostKeyUnconfirmed { .. } => {
            "watch.health.remote_host_key"
        }
        RunError::Timeout => "watch.health.remote_timeout",
        RunError::NoLogDir | RunError::NoAuditFiles => "watch.health.remote_log_dir",
        RunError::NoSqlite => "watch.health.remote_sqlite_missing",
        RunError::DbUnreadable(_) => "watch.health.remote_db_unreadable",
        RunError::Exec(_) | RunError::Closed => "watch.health.remote_read",
    }
}

/// 설정(계정·호스트 키)이 바뀌어야 풀리는 오류 — 같은 접속을 자주 다시 시도하지 않는다
pub(super) fn is_hard(e: &RunError) -> bool {
    matches!(
        e,
        RunError::Auth
            | RunError::BadKey
            | RunError::HostKeyMismatch { .. }
            | RunError::HostKeyUnconfirmed { .. }
    )
}

/// 접속 자체가 끊겼을 가능성이 있는 오류 (로그 폴더 문제는 접속을 닫을 이유가 아니다)
pub(super) fn is_link_error(e: &RunError) -> bool {
    !matches!(
        e,
        RunError::NoLogDir
            | RunError::NoAuditFiles
            | RunError::NoSqlite
            | RunError::DbUnreadable(_)
    )
}

impl Collector {
    pub fn new(src: RemoteSource, d: Deps) -> Self {
        let resumed = d
            .committer
            .load_cursor(src.source_id)
            .and_then(|c| Cursor::from_json(&c));
        let now = d.clock.now_ms();
        Self {
            mapper: Mapper::new(&src.path_map),
            tail: LogTail::new(src.log_dir(), resumed),
            gate: KickGate::new(d.timings.kick_gate_ms),
            src,
            run: None,
            d,
            want_all: false,
            want: BTreeSet::new(),
            degraded: false,
            down_since: 0,
            fail_streak: 0,
            window: Counts::default(),
            total: Counts::default(),
            last_report: now,
            outside_noted: false,
        }
    }

    pub fn is_degraded(&self) -> bool {
        self.degraded
    }

    pub fn counts(&self) -> (Counts, Counts) {
        (self.window, self.total)
    }

    pub fn cursor(&self) -> &Cursor {
        self.tail.cursor()
    }

    /// 접속 설정이 바뀌었다 — 접속을 닫고 새 설정으로 다시 붙는다 (로그 폴더가 같으면 커서를 이어 쓴다).
    pub fn reload(&mut self, src: RemoteSource) {
        if src.log_dir() != self.src.log_dir() {
            self.tail = LogTail::new(src.log_dir(), None);
        }
        self.mapper = Mapper::new(&src.path_map);
        self.src = src;
        self.run = None;
        self.fail_streak = 0;
    }

    /// 한 주기. 다음 주기까지 기다릴 시간을 돌려준다.
    pub fn step(&mut self) -> Duration {
        let now = self.d.clock.now_ms();
        self.absorb_late_attached();
        let wait = match self.poll_once() {
            Ok(()) => {
                self.fail_streak = 0;
                self.d
                    .timings
                    .poll_override
                    .unwrap_or_else(|| Duration::from_secs(u64::from(self.src.poll_sec.max(1))))
            }
            Err(e) => self.on_fail(&e, now),
        };
        self.flush_rescans();
        self.report_if_due(now);
        wait
    }

    fn poll_once(&mut self) -> Result<(), RunError> {
        let mut fresh = false;
        for attempt in 0..2 {
            if self.run.is_none() {
                self.run = Some(self.d.connector.connect(&self.src)?);
                fresh = true;
            }
            let Some(run) = self.run.as_mut() else {
                return Err(RunError::Closed);
            };
            match self.tail.poll(run.as_mut()) {
                Ok(p) => {
                    self.recover_if_degraded();
                    return self.apply(p);
                }
                Err(e) => {
                    if is_link_error(&e) {
                        self.run = None;
                        // 살아 있던 접속이 끊긴 것이면 한 번은 바로 다시 붙어 본다 (유휴 끊김에 `Degraded`·재스캔을 내지 않는다)
                        if !fresh && attempt == 0 && !is_hard(&e) {
                            continue;
                        }
                    }
                    return Err(e);
                }
            }
        }
        Err(RunError::Closed)
    }

    /// 읽기에 성공했다 — 저하 상태였다면 `Ok` 로 돌아오고 재스캔 1회로 끊긴 구간을 덮는다 (접속만 되고 읽기가 실패하면 아직 복구가 아니다).
    fn recover_if_degraded(&mut self) {
        if !self.degraded {
            return;
        }
        self.degraded = false;
        let secs = (self.d.clock.now_ms() - self.down_since) / 1000;
        tracing::info!(source = %self.src.name, down_secs = secs, "remote source reconnected; scheduling a rescan to cover the gap");
        for f in lock(&self.d.folders).iter() {
            f.sink.recovered();
        }
        self.want_all = true;
    }

    fn on_fail(&mut self, e: &RunError, now: i64) -> Duration {
        self.window.fails += 1;
        self.total.fails += 1;
        self.fail_streak += 1;
        if self.degraded {
            tracing::debug!(source = %self.src.name, key = e.key(), "remote source still unreachable");
        } else {
            self.degraded = true;
            self.down_since = now;
            // 오류 문구에는 비밀이 없다 (RunError 는 대상·종류만 담는다)
            tracing::warn!(source = %self.src.name, error = %e, "remote source unreachable; periodic rescans keep the index correct meanwhile");
        }
        for f in lock(&self.d.folders).iter() {
            f.sink.degraded(health_key(e));
        }
        let t = &self.d.timings;
        if is_hard(e) {
            return t.hard_retry;
        }
        let shift = (self.fail_streak - 1).min(10);
        t.reconnect_base
            .saturating_mul(1 << shift)
            .min(t.reconnect_max)
    }

    /// 읽은 줄을 폴더별로 나눠 기록하고(대기열 + 커서), 성공하면 커서를 올린다.
    fn apply(&mut self, p: Polled) -> Result<(), RunError> {
        self.window.polls += 1;
        self.total.polls += 1;
        let (items, root_hits) = self.route(&p.lines);
        let unchanged = p.next == *self.tail.cursor();
        if items.is_empty() && unchanged && root_hits.is_empty() && !p.gap {
            return Ok(());
        }
        let cursor_json = p.next.to_json();
        match self
            .d
            .committer
            .commit(self.src.source_id, &items, &cursor_json)
        {
            Ok(out) => {
                self.tail.commit(p.next);
                self.want.extend(out.overflowed);
                self.want.extend(root_hits);
                if p.gap {
                    self.want_all = true;
                }
                self.window.skipped_bytes += p.skipped_bytes;
                self.total.skipped_bytes += p.skipped_bytes;
                let folders = lock(&self.d.folders);
                let reported: HashSet<i64> = items.iter().map(|(f, _)| *f).collect();
                for f in folders.iter().filter(|f| reported.contains(&f.folder_id)) {
                    f.sink.signal();
                }
            }
            // 연결 문제가 아니다 — 커서를 올리지 않았으니 다음 주기에 같은 오프셋부터 다시 읽는다
            Err(e) => {
                tracing::warn!(source = %self.src.name, error = %e, "cannot record remote changes; will retry from the same offsets")
            }
        }
        Ok(())
    }

    /// 줄 -> (폴더 ID, 이 PC 경로) 목록. 루트 폴더 자신을 가리키는 경로는 전체 순회가 되지 않게 재스캔 요청으로 바꾼다.
    fn route(&mut self, lines: &[String]) -> (Vec<(i64, PathBuf)>, BTreeSet<i64>) {
        let attached = Arc::clone(&self.d.folders);
        let folders = lock(&attached);
        let mut items = Vec::new();
        let mut seen: HashSet<(i64, String)> = HashSet::new();
        let mut root_hits = BTreeSet::new();
        for line in lines {
            self.window.lines += 1;
            self.total.lines += 1;
            let Some(ev) = parse_line(line) else {
                self.window.bad += 1;
                self.total.bad += 1;
                continue;
            };
            if !ev.is_change() {
                continue;
            }
            for t in ev.targets().into_iter().filter(|t| !is_noise(t)) {
                let Some(full) = self.mapper.map(t) else {
                    self.window.unmapped += 1;
                    self.total.unmapped += 1;
                    continue;
                };
                self.window.changes += 1;
                self.total.changes += 1;
                let mut hit = false;
                for f in folders.iter().filter(|f| within_dir(&full, &f.root)) {
                    hit = true;
                    if same_dir(&full, &f.root) {
                        root_hits.insert(f.folder_id);
                    } else if seen.insert((f.folder_id, full.clone())) {
                        items.push((f.folder_id, PathBuf::from(&full)));
                    }
                }
                if hit {
                    self.window.reported += 1;
                    self.total.reported += 1;
                } else {
                    self.window.outside += 1;
                    self.total.outside += 1;
                    self.note_outside();
                }
            }
        }
        (items, root_hits)
    }

    /// 변환은 됐는데 어느 폴더 루트에도 들지 않는 경로 — 공용 볼륨의 다른 폴더면 정상이지만 `to` 가 한 칸 어긋나도 똑같이 조용하다.
    /// 둘을 구분할 수 있게 한 번 알린다 (2026-10-23 실기에서 로그 수준을 올리고서야 알게 된 자리).
    fn note_outside(&mut self) {
        if !std::mem::replace(&mut self.outside_noted, true) {
            tracing::info!(
                source = %self.src.name,
                "a remote change maps outside every attached folder; normal when other folders of the volume change — check the path map if nothing ever wakes a folder"
            );
        }
    }

    fn absorb_late_attached(&mut self) {
        for f in lock(&self.d.folders).iter_mut().filter(|f| f.needs_rescan) {
            f.needs_rescan = false;
            self.want.insert(f.folder_id);
        }
    }

    /// 요청된 재스캔을 깨우기 제한(폴더당 5초)을 지켜 보낸다. 막힌 것은 남겨 다음 주기에 다시 시도한다.
    fn flush_rescans(&mut self) {
        if !self.want_all && self.want.is_empty() {
            return;
        }
        let now = self.d.clock.now_ms();
        let folders = lock(&self.d.folders);
        if self.want_all {
            self.want.extend(folders.iter().map(|f| f.folder_id));
            self.want_all = false;
        }
        let gate = &mut self.gate;
        self.want.retain(|id| {
            let Some(f) = folders.iter().find(|f| f.folder_id == *id) else {
                return false; // 떼어 낸 폴더
            };
            if gate.allow(*id, now) {
                f.sink.rescan();
                false
            } else {
                true
            }
        });
    }

    fn report_if_due(&mut self, now: i64) {
        if now - self.last_report < self.d.timings.report_every_ms || self.degraded {
            return;
        }
        let w = std::mem::take(&mut self.window);
        self.last_report = now;
        tracing::info!(
            source = %self.src.name, polls = w.polls, lines = w.lines, reported = w.reported,
            outside = w.outside, unmapped = w.unmapped, bad = w.bad, "remote audit source healthy"
        );
    }
}

impl Feed for Collector {
    fn step(&mut self) -> Duration {
        Collector::step(self)
    }

    fn reload(&mut self, src: RemoteSource) {
        Collector::reload(self, src);
    }
}

#[cfg(test)]
#[path = "collector_tests.rs"]
mod tests;
