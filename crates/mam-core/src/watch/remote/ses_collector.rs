//! `ses_queue` 수집기 — SES(storage-event-service) 큐를 id 커서로 이어 읽어 **경로가 속한 폴더에만** 보고한다 (`collector.rs` 의 짝).
//!
//! 한 번의 [`SesCollector::step`] = (필요하면 접속) -> `events` 읽기 -> 보고 기록(대기열 + 커서, **한 트랜잭션**) -> 재스캔 요청.
//! 접속·재접속·깨우기 제한·`Degraded`/`Ok` 규칙은 감사 로그 수집기와 같다. 다른 점은 놓친 구간의 판정이다(`ses.rs`):
//! 첫 기준선·큐 초기화·커서 행 사라짐·밀림 초과는 `Rescan`. 앱이 꺼져 있던 동안의 행은 저장된 커서부터 이어 읽으므로 재스캔이 필요 없다.
//! (SES 자체가 꺼져 있던 구간은 큐에 흔적이 없다 — 주기 재스캔이 맞춘다.)

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::RemoteSource;
use super::collector::{health_key, is_hard, is_link_error};
use super::contract::{Deps, Feed, FeedItem};
use super::mapper::{Mapper, same_dir, within_dir};
use super::runner::{RemoteRunner, RunError};
use super::ses::{Cursor, Polled, SesFeed, classify};
use super::stats::{Counts, KickGate};

/// 대기열 행의 출처 (`watch_pending.source`)
pub const ORIGIN: &str = "fanotify_feed";

pub struct SesCollector {
    src: RemoteSource,
    mapper: Mapper,
    feed: SesFeed,
    run: Option<Box<dyn RemoteRunner>>,
    d: Deps,
    gate: KickGate,
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

impl SesCollector {
    pub fn new(src: RemoteSource, d: Deps) -> Self {
        let resumed = d
            .committer
            .load_cursor(src.source_id)
            .and_then(|c| Cursor::from_json(&c));
        let now = d.clock.now_ms();
        Self {
            mapper: Mapper::new(&src.path_map),
            feed: SesFeed::new(src.db_path.as_deref().unwrap_or_default(), resumed),
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

    pub fn cursor(&self) -> Option<Cursor> {
        self.feed.cursor()
    }

    /// 접속 설정이 바뀌었다 — 접속을 닫고 새 설정으로 다시 붙는다 (같은 DB 면 커서를 이어 쓴다).
    pub fn reload(&mut self, src: RemoteSource) {
        let db = src.db_path.clone().unwrap_or_default();
        if db != self.feed.db() {
            self.feed = SesFeed::new(&db, None); // 다른 DB — 새 기준선
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
            match self.feed.poll(run.as_mut()) {
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

    /// 읽기에 성공했다 — 저하 상태였다면 `Ok` 로 돌아오고 재스캔 1회로 끊긴 구간을 덮는다.
    fn recover_if_degraded(&mut self) {
        if !self.degraded {
            return;
        }
        self.degraded = false;
        let secs = (self.d.clock.now_ms() - self.down_since) / 1000;
        tracing::info!(source = %self.src.name, down_secs = secs, "remote event queue reconnected; scheduling a rescan to cover the gap");
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
            tracing::debug!(source = %self.src.name, key = e.key(), "remote event queue still unreadable");
        } else {
            self.degraded = true;
            self.down_since = now;
            // 오류 문구에는 비밀이 없다 (RunError 는 대상·종류만 담는다)
            tracing::warn!(source = %self.src.name, error = %e, "remote event queue unreadable; periodic rescans keep the index correct meanwhile");
        }
        for f in lock(&self.d.folders).iter() {
            f.sink.degraded(health_key(e));
        }
        let t = &self.d.timings;
        if is_hard(e) {
            return t.hard_retry;
        }
        t.reconnect_base
            .saturating_mul(1 << (self.fail_streak - 1).min(10))
            .min(t.reconnect_max)
    }

    /// 읽은 행을 폴더별로 나눠 기록하고(대기열 + 커서), 성공하면 커서를 올린다.
    fn apply(&mut self, p: Polled) -> Result<(), RunError> {
        self.window.polls += 1;
        self.total.polls += 1;
        let (items, root_hits) = self.route(&p);
        let Some(next) = p.next else { return Ok(()) };
        if items.is_empty() && Some(next) == self.feed.cursor() && root_hits.is_empty() && !p.gap {
            return Ok(());
        }
        match self
            .d
            .committer
            .commit_feed(self.src.source_id, &items, &next.to_json(), ORIGIN)
        {
            Ok(out) => {
                self.feed.commit(next);
                self.want.extend(out.overflowed);
                self.want.extend(root_hits);
                self.want_all |= p.gap;
                if p.reset {
                    tracing::warn!(source = %self.src.name, "remote event queue was reset (max id < cursor); cursor reset and rescan requested");
                }
                if p.skipped_rows > 0 {
                    tracing::warn!(source = %self.src.name, rows = p.skipped_rows, "remote event backlog too large; skipped to the end and rescan requested");
                }
                self.window.skipped_rows += p.skipped_rows;
                self.total.skipped_rows += p.skipped_rows;
                let folders = lock(&self.d.folders);
                let reported: HashSet<i64> = items.iter().map(|i| i.folder_id).collect();
                for f in folders.iter().filter(|f| reported.contains(&f.folder_id)) {
                    f.sink.signal();
                }
            }
            // 연결 문제가 아니다 — 커서를 올리지 않았으니 다음 주기에 같은 id 부터 다시 읽는다
            Err(e) => {
                tracing::warn!(source = %self.src.name, error = %e, "cannot record remote changes; will retry from the same cursor")
            }
        }
        Ok(())
    }

    /// 행 -> (폴더 ID, 이 PC 경로, 하위 순회 여부). 루트 폴더 자신을 가리키는 경로는 전체 순회가 되지 않게 재스캔 요청으로 바꾼다.
    fn route(&mut self, p: &Polled) -> (Vec<FeedItem>, BTreeSet<i64>) {
        let attached = Arc::clone(&self.d.folders);
        let folders = lock(&attached);
        let (mut items, mut root_hits) = (Vec::new(), BTreeSet::new());
        let mut seen: HashSet<(i64, String, bool)> = HashSet::new();
        for c in [&mut self.window, &mut self.total] {
            c.lines += p.rows.len() as u64;
            c.bad += p.bad;
        }
        for row in &p.rows {
            let Some(subtree) = classify(row) else {
                continue;
            };
            let Some(full) = self.mapper.map(&row.path) else {
                // 변환 규칙 밖 (또는 위험한 경로) — 버리고 센다
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
                } else if seen.insert((f.folder_id, full.clone(), subtree)) {
                    items.push(FeedItem {
                        folder_id: f.folder_id,
                        path: PathBuf::from(&full),
                        subtree,
                    });
                }
            }
            let c = if hit { (1, 0) } else { (0, 1) };
            for k in [&mut self.window, &mut self.total] {
                k.reported += c.0;
                k.outside += c.1;
            }
            if !hit && !std::mem::replace(&mut self.outside_noted, true) {
                tracing::info!(
                    source = %self.src.name,
                    "a remote event maps outside every attached folder; normal when other folders of the volume change — check the path map if nothing ever wakes a folder"
                );
            }
        }
        (items, root_hits)
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
                return false;
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
            source = %self.src.name, polls = w.polls, rows = w.lines, reported = w.reported,
            outside = w.outside, unmapped = w.unmapped, bad = w.bad, "remote event queue healthy"
        );
    }
}

impl Feed for SesCollector {
    fn step(&mut self) -> Duration {
        SesCollector::step(self)
    }

    fn reload(&mut self, src: RemoteSource) {
        SesCollector::reload(self, src);
    }
}

#[cfg(test)]
#[path = "ses_collector_tests.rs"]
mod tests;
