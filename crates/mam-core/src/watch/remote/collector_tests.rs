//! 수집기 시험 (T-remote-col-01 ~ 16). 가짜 접속·가짜 기록기·주입 시계 — 스레드·네트워크 없이 `step()` 을 직접 돌려 결정적으로 시험한다.
//!
//! 분류: Happy (01 기준선·02 라우팅·11 재시작 이어 읽기) / Boundary (05 깨우기 5초·13 폴더 루트 자신) / Validation (03 규칙 밖 경로·07 호스트 키) /
//! Business rule (04 재접속 후 재스캔 1회·09 늦게 붙은 폴더) / External (04 접속 실패·06 기록 실패) / Concurrency (12 중복 보고 합침).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{
    Attached, BatchCommitter, Collector, CommitError, CommitOutcome, Deps, FeedItem, Folders,
    Timings,
};
use crate::SharedClock;
use crate::secret::Secret;
use crate::volume::VolumeKind;
use crate::watch::detector::{
    ChangeDetector, DetectError, DetectorOptions, ProbeResult, WatchHandle, WatchTarget,
};
use crate::watch::remote::fake::{FakeConnector, FakeDdp, FakeSecretBox, line};
use crate::watch::remote::mapper::Mapping;
use crate::watch::remote::runner::RunError;
use crate::watch::remote::stats::Counts;
use crate::watch::remote::tail::MAX_FETCH;
use crate::watch::remote::{Auth, RemoteSource, SourceKind};
use crate::watch::{ChangeSink, DetectorRegistry, SourceHealth, WatchManager, WatchMsg};

const T0: i64 = 1_700_000_000_000;
const FP: &str = "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_FP: &str = "SHA256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

struct Clock(AtomicI64);

impl mam_db::Clock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// 시작할 때 받은 sink 를 폴더별로 보관하는 감지기 (`WatchManager` 가 sink 를 만들어 주므로 이것으로 얻는다)
struct Parking(Arc<Mutex<HashMap<i64, ChangeSink>>>);

struct Handle;
impl WatchHandle for Handle {}

impl ChangeDetector for Parking {
    fn id(&self) -> &'static str {
        "parking"
    }
    fn probe(&self, _t: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }
    fn start(
        &self,
        t: &WatchTarget,
        _o: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        self.0.lock().unwrap().insert(t.folder_id, sink);
        Ok(Box::new(Handle))
    }
}

type Commit = (Vec<(i64, PathBuf)>, String);

/// 기록 시도를 모으는 가짜 기록기 — 실패 주입·커서 보관
#[derive(Default)]
struct FakeCommitter {
    commits: Mutex<Vec<Commit>>,
    cursor: Mutex<Option<String>>,
    fail: AtomicBool,
    overflow: Mutex<Vec<i64>>,
}

impl BatchCommitter for FakeCommitter {
    fn commit_feed(
        &self,
        _source_id: i64,
        items: &[FeedItem],
        cursor: &str,
        _origin: &'static str,
    ) -> Result<CommitOutcome, CommitError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(CommitError("injected".into()));
        }
        self.commits.lock().unwrap().push((
            items
                .iter()
                .map(|i| (i.folder_id, i.path.clone()))
                .collect(),
            cursor.to_string(),
        ));
        *self.cursor.lock().unwrap() = Some(cursor.to_string());
        Ok(CommitOutcome {
            overflowed: std::mem::take(&mut *self.overflow.lock().unwrap()),
            dropped: 0,
        })
    }

    fn load_cursor(&self, _source_id: i64) -> Option<String> {
        self.cursor.lock().unwrap().clone()
    }
}

struct H {
    clock: Arc<Clock>,
    ddp: FakeDdp,
    conn: Arc<FakeConnector>,
    commit: Arc<FakeCommitter>,
    folders: Folders,
    m: WatchManager,
    sinks: Arc<Mutex<HashMap<i64, ChangeSink>>>,
    rx: Receiver<WatchMsg>,
    c: Collector,
}

fn source(host_key: Option<&str>, rules: &[(&str, &str)]) -> RemoteSource {
    RemoteSource {
        source_id: 1,
        kind: SourceKind::DdpAudit,
        name: "ddp-1".into(),
        host: "10.0.0.5".into(),
        port: 22,
        user: "audit".into(),
        auth: Auth::Password(Secret::new("pw-never-logged".into())),
        host_key: host_key.map(str::to_string),
        log_dir: Some("/tmp/audit".into()),
        db_path: None,
        poll_sec: 3,
        path_map: rules
            .iter()
            .map(|(f, t)| Mapping {
                from: (*f).into(),
                to: (*t).into(),
            })
            .collect(),
    }
}

fn timings() -> Timings {
    Timings::default()
}

/// `roots` = (폴더 ID, 루트 경로), `rules` = 경로 변환. 로그 파일 `audit-log.0` 은 비어 있게 시작한다.
fn harness(
    roots: &[(i64, &str)],
    rules: &[(&str, &str)],
    host_key: Option<&str>,
    resumed: Option<String>,
) -> H {
    let clock = Arc::new(Clock(AtomicI64::new(T0)));
    let shared: SharedClock = clock.clone();
    let ddp = FakeDdp::new("/tmp/audit");
    ddp.write("audit-log.0", "");
    let conn = Arc::new(FakeConnector::new(ddp.clone(), FP));
    let commit = Arc::new(FakeCommitter::default());
    *commit.cursor.lock().unwrap() = resumed;
    let sinks = Arc::new(Mutex::new(HashMap::new()));
    let (tx, rx) = channel();
    let all: Vec<Arc<dyn ChangeDetector>> = vec![Arc::new(Parking(Arc::clone(&sinks)))];
    let m = WatchManager::with_clock(
        tx,
        Arc::new(DetectorRegistry::new(all, &["parking"])),
        Arc::clone(&shared),
    );
    let folders: Folders = Arc::new(Mutex::new(Vec::new()));
    let mut h = H {
        c: Collector::new(
            source(host_key, rules),
            Deps {
                connector: conn.clone(),
                committer: commit.clone(),
                clock: shared,
                timings: timings(),
                folders: Arc::clone(&folders),
                loader: Arc::new(|_| None),
            },
        ),
        clock,
        ddp,
        conn,
        commit,
        folders,
        m,
        sinks,
        rx,
    };
    for (id, root) in roots {
        h.attach(*id, root, false);
    }
    h
}

impl H {
    fn attach(&mut self, id: i64, root: &str, late: bool) {
        let t = WatchTarget {
            folder_id: id,
            root: PathBuf::from(root),
            volume_kind: VolumeKind::Fixed,
        };
        self.m.start(&t, None, None);
        let sink = self.sinks.lock().unwrap().get(&id).cloned().expect("sink");
        self.folders.lock().unwrap().push(Attached {
            folder_id: id,
            root: root.into(),
            sink,
            needs_rescan: late,
        });
    }

    fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }

    /// 지금까지 도착한 재스캔 요청의 폴더 ID (정렬)
    fn rescans(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self
            .rx
            .try_iter()
            .filter_map(|m| matches!(m, WatchMsg::Rescan { .. }).then(|| m.folder_id()))
            .collect();
        v.sort_unstable();
        v
    }

    /// 첫 주기(기준선) 를 지나고 도착한 재스캔을 비운다
    fn primed(mut self) -> Self {
        self.c.step();
        self.rescans();
        self.advance(10_000);
        self
    }

    fn last_items(&self) -> Vec<(i64, PathBuf)> {
        self.commit
            .commits
            .lock()
            .unwrap()
            .last()
            .map(|c| c.0.clone())
            .unwrap_or_default()
    }

    fn n_commits(&self) -> usize {
        self.commit.commits.lock().unwrap().len()
    }
}

fn uhd() -> H {
    harness(&[(1, r"F:\UHD")], &[("/", r"F:\UHD")], Some(FP), None)
}

#[test]
fn t_remote_col_01_the_first_step_sets_a_baseline_stores_the_cursor_and_asks_each_folder_for_one_rescan()
 {
    let mut h = harness(
        &[(1, r"F:\UHD"), (2, r"F:\UHD\1TV")],
        &[("/", r"F:\UHD")],
        Some(FP),
        None,
    );
    h.ddp.write("audit-log.0", &line("CREATE", "/old.mxf")); // 과거 줄은 읽지 않는다
    let wait = h.c.step();
    assert_eq!(wait, Duration::from_secs(3));
    assert_eq!(h.n_commits(), 1);
    assert!(h.last_items().is_empty(), "history is not replayed");
    assert!(
        h.commit
            .cursor
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("audit-log.0")
    );
    assert_eq!(
        h.rescans(),
        vec![1, 2],
        "the unknown gap before the baseline is covered by one rescan per folder"
    );
    assert!(matches!(h.m.health(1), Some(SourceHealth::Ok { .. })));
}

#[test]
fn t_remote_col_02_a_change_is_reported_only_to_folders_whose_root_contains_the_mapped_path() {
    let mut h = harness(
        &[(1, r"F:\UHD"), (2, r"F:\UHD\1TV"), (3, r"G:\Other")],
        &[("/", r"F:\UHD")],
        Some(FP),
        None,
    )
    .primed();
    h.ddp.write(
        "audit-log.0",
        &(line("CREATE", "/1TV/a.mxf")
            + &line("WRITE", "/2TV/b.mxf")
            + &line("RENAME", "/tmp/c.mxf")),
    );
    h.ddp.write(
        "audit-log.0",
        "2026-10-23 09:30:00.1  RENAME  iqn.kbs-01  |/tmp/d.mxf|/1TV/d.mxf\n",
    );
    h.c.step();
    let mut items = h.last_items();
    items.sort();
    let p = |s: &str| PathBuf::from(s);
    assert_eq!(
        items,
        vec![
            (1, p(r"F:\UHD\1TV\a.mxf")),
            (1, p(r"F:\UHD\1TV\d.mxf")),
            (1, p(r"F:\UHD\2TV\b.mxf")),
            (1, p(r"F:\UHD\tmp\c.mxf")),
            (1, p(r"F:\UHD\tmp\d.mxf")),
            (2, p(r"F:\UHD\1TV\a.mxf")),
            (2, p(r"F:\UHD\1TV\d.mxf")),
        ],
        "folder 3 (G:\\Other) never receives them"
    );
    assert!(
        matches!(h.m.health(1), Some(SourceHealth::Ok { last_signal_at: Some(t) }) if t == T0 + 10_000)
    );
    assert!(matches!(
        h.m.health(3),
        Some(SourceHealth::Ok {
            last_signal_at: None
        })
    ));
}

#[test]
fn t_remote_col_03_unmapped_and_outside_paths_are_dropped_and_counted_but_the_cursor_still_advances()
 {
    let mut h = harness(
        &[(1, r"F:\UHD\1TV")],
        &[("/1TV", r"F:\UHD\1TV"), ("/other", r"F:\UHD\other")],
        Some(FP),
        None,
    )
    .primed();
    let before = h.n_commits();
    h.ddp.write(
        "audit-log.0",
        &(line("CREATE", "/2TV/x.mxf")
            + &line("CREATE", "/other/y.mxf")
            + &line("READ", "/1TV/z.mxf")
            + "garbage line\n"),
    );
    h.c.step();
    assert_eq!(h.n_commits(), before + 1, "cursor moved, so it is recorded");
    assert!(h.last_items().is_empty());
    let (win, _): (Counts, Counts) = h.c.counts();
    assert_eq!(
        (win.unmapped, win.outside, win.reported, win.bad),
        (1, 1, 0, 1)
    );
}

#[test]
fn t_remote_col_04_a_lost_connection_degrades_with_backoff_then_recovers_with_exactly_one_rescan() {
    let mut h = uhd().primed();
    h.ddp.set_fail(Some(RunError::Closed));
    h.conn.fail_connects(2, RunError::Connect("down".into()));
    // 살아 있던 접속이 끊겼다: 바로 한 번 다시 붙어 보고(실패) -> Degraded, 15초 뒤 재시도
    assert_eq!(h.c.step(), Duration::from_secs(15));
    assert_eq!(
        h.m.health(1),
        Some(SourceHealth::Degraded {
            since: T0 + 10_000,
            reason_key: "watch.health.remote_connect"
        })
    );
    assert!(h.rescans().is_empty(), "no rescan while still down");
    assert_eq!(h.c.step(), Duration::from_secs(30), "backoff doubles");
    // 서버가 돌아왔다
    h.ddp.set_fail(None);
    h.advance(60_000);
    assert_eq!(h.c.step(), Duration::from_secs(3));
    assert!(matches!(h.m.health(1), Some(SourceHealth::Ok { .. })));
    assert_eq!(h.rescans(), vec![1], "one rescan covers the gap");
    h.c.step();
    assert!(h.rescans().is_empty(), "and only one");
    assert!(!h.c.is_degraded());
}

#[test]
fn t_remote_col_05_rescans_are_gated_to_one_per_folder_per_5s_and_the_blocked_one_is_deferred_not_lost()
 {
    let mut h = uhd().primed(); // 마지막 재스캔: T0 (기준선), 지금 T0+10s
    // 로그가 갈아치워져(줄어듦) 공백 재스캔이 필요하다
    h.ddp.write("audit-log.0", &line("CREATE", "/a.mxf"));
    h.c.step();
    h.ddp.set("audit-log.0", "");
    h.c.step();
    assert_eq!(
        h.rescans(),
        vec![1],
        "first shrink: the gate is open (10s since the baseline rescan)"
    );
    h.ddp.write("audit-log.0", &line("CREATE", "/b.mxf"));
    h.c.step();
    h.advance(1_000);
    h.ddp.set("audit-log.0", "");
    h.c.step();
    assert!(h.rescans().is_empty(), "second shrink 1s later: held back");
    h.advance(4_000); // 첫 재스캔으로부터 정확히 5초 — 경계는 통과
    h.c.step();
    assert_eq!(
        h.rescans(),
        vec![1],
        "the deferred request goes out once the gate opens"
    );
    h.c.step();
    assert!(h.rescans().is_empty());
}

#[test]
fn t_remote_col_06_a_failed_record_keeps_the_cursor_and_the_same_lines_are_reported_after_the_next_success()
 {
    let mut h = uhd().primed();
    let saved = h.commit.cursor.lock().unwrap().clone();
    h.ddp.write("audit-log.0", &line("CREATE", "/a.mxf"));
    h.commit.fail.store(true, Ordering::SeqCst);
    h.c.step();
    assert_eq!(
        *h.commit.cursor.lock().unwrap(),
        saved,
        "nothing was stored"
    );
    assert!(
        !h.c.is_degraded(),
        "a record failure is not a connection failure"
    );
    h.commit.fail.store(false, Ordering::SeqCst);
    h.c.step();
    assert_eq!(h.last_items(), vec![(1, PathBuf::from(r"F:\UHD\a.mxf"))]);
    h.c.step();
    assert_eq!(
        h.n_commits(),
        2,
        "prime + the one successful batch; no duplicate"
    );
}

#[test]
fn t_remote_col_07_host_key_problems_are_distinct_hard_errors_that_back_off_for_5_minutes() {
    // 지문 미확인 (저장된 호스트 키 없음)
    let mut h = harness(&[(1, r"F:\UHD")], &[("/", r"F:\UHD")], None, None);
    assert_eq!(h.c.step(), Duration::from_secs(300));
    assert_eq!(
        h.m.health(1),
        Some(SourceHealth::Degraded {
            since: T0,
            reason_key: "watch.health.remote_host_key"
        })
    );
    assert_eq!(h.n_commits(), 0, "nothing is read from an unverified host");
    // 지문 불일치
    let mut h = harness(&[(1, r"F:\UHD")], &[("/", r"F:\UHD")], Some(OTHER_FP), None);
    assert_eq!(h.c.step(), Duration::from_secs(300));
    assert_eq!(
        h.ddp.calls(),
        0,
        "no command reaches a server whose key does not match"
    );
    // 인증 실패도 계정 잠금을 피하려고 길게 쉰다
    let mut h = uhd();
    h.conn.fail_connects(1, RunError::Auth);
    assert_eq!(h.c.step(), Duration::from_secs(300));
    assert_eq!(
        h.m.health(1),
        Some(SourceHealth::Degraded {
            since: T0,
            reason_key: "watch.health.remote_auth"
        })
    );
}

#[test]
fn t_remote_col_08_a_path_that_is_the_folder_root_itself_becomes_a_rescan_not_a_whole_tree_item() {
    let mut h = uhd().primed();
    h.ddp.write(
        "audit-log.0",
        &line("CREATE", "/UHD"), /* maps to F:\UHD\UHD */
    );
    h.c.step();
    assert_eq!(h.last_items().len(), 1);
    let mut h = harness(&[(1, r"F:\UHD")], &[("/media", r"F:\UHD")], Some(FP), None).primed();
    h.ddp.write("audit-log.0", &line("WRITE", "/media"));
    h.c.step();
    assert!(h.last_items().is_empty());
    assert_eq!(h.rescans(), vec![1]);
}

#[test]
fn t_remote_col_09_a_folder_attached_after_the_collector_started_gets_one_rescan_for_what_it_missed()
 {
    let mut h = uhd().primed();
    h.attach(2, r"F:\UHD\1TV", true);
    h.c.step();
    assert_eq!(h.rescans(), vec![2], "only the late folder");
    h.c.step();
    assert!(h.rescans().is_empty());
}

#[test]
fn t_remote_col_10_folders_that_hit_the_pending_cap_are_turned_into_a_rescan() {
    let mut h = uhd().primed();
    h.commit.overflow.lock().unwrap().push(1);
    h.ddp.write("audit-log.0", &line("CREATE", "/a.mxf"));
    h.c.step();
    assert_eq!(h.rescans(), vec![1]);
}

#[test]
fn t_remote_col_11_a_restart_resumes_from_the_stored_cursor_without_a_gap_rescan() {
    let mut h = uhd().primed();
    h.ddp.write("audit-log.0", &line("CREATE", "/seen.mxf"));
    h.c.step();
    let saved = h.commit.cursor.lock().unwrap().clone();
    h.ddp
        .write("audit-log.0", &line("CREATE", "/while-off.mxf")); // 앱이 꺼져 있던 동안 쌓인 줄
    let mut h2 = harness(&[(1, r"F:\UHD")], &[("/", r"F:\UHD")], Some(FP), saved);
    h2.ddp.set(
        "audit-log.0",
        &(line("CREATE", "/seen.mxf") + &line("CREATE", "/while-off.mxf")),
    );
    h2.c.step();
    assert_eq!(
        h2.last_items(),
        vec![(1, PathBuf::from(r"F:\UHD\while-off.mxf"))]
    );
    assert!(
        h2.rescans().is_empty(),
        "resumed from the cursor: nothing was missed"
    );
}

#[test]
fn t_remote_col_12_the_same_path_reported_many_times_in_one_batch_is_one_item() {
    let mut h = uhd().primed();
    let spam: String = (0..500).map(|_| line("WRITE", "/big.mxf")).collect();
    h.ddp.write("audit-log.0", &spam);
    h.c.step();
    assert_eq!(h.last_items().len(), 1);
}

#[test]
fn t_remote_col_13_more_than_1mib_in_one_poll_skips_to_the_end_and_requests_a_rescan() {
    let mut h = uhd().primed();
    h.ddp
        .write("audit-log.0", &("x".repeat(MAX_FETCH as usize + 10) + "\n"));
    h.c.step();
    assert_eq!(h.rescans(), vec![1]);
    assert!(h.c.counts().0.skipped_bytes > MAX_FETCH);
    assert!(h.last_items().is_empty());
}

#[test]
fn t_remote_col_14_reload_applies_the_new_path_map_and_closes_the_connection() {
    let mut h = uhd().primed();
    let connects = h.conn.connects();
    h.c.reload(source(Some(FP), &[("/", r"F:\Elsewhere")]));
    h.ddp.write("audit-log.0", &line("CREATE", "/a.mxf"));
    h.c.step();
    assert_eq!(
        h.conn.connects(),
        connects + 1,
        "reconnected with the new settings"
    );
    assert!(
        h.last_items().is_empty(),
        "F:\\Elsewhere\\a.mxf is outside F:\\UHD"
    );
}

#[test]
fn t_remote_col_15_secrets_never_reach_logs_or_debug_output_of_the_collector_inputs() {
    let s = source(Some(FP), &[("/", r"F:\UHD")]);
    assert!(!format!("{s:?}").contains("pw-never-logged"));
    let boxed = FakeSecretBox;
    assert!(format!("{boxed:?}").len() < 40);
    let e = RunError::Connect("refused".into());
    assert!(!format!("{e}{e:?}").contains("pw-never-logged"));
}

#[test]
fn t_remote_col_16_a_missing_log_dir_or_empty_audit_logs_degrade_with_a_log_dir_reason_and_keep_the_connection()
 {
    let mut h = uhd().primed();
    h.ddp.set_dir_missing(true);
    h.c.step();
    assert_eq!(
        h.m.health(1),
        Some(SourceHealth::Degraded {
            since: T0 + 10_000,
            reason_key: "watch.health.remote_log_dir"
        })
    );
    let connects = h.conn.connects();
    h.ddp.set_dir_missing(false);
    h.advance(20_000);
    h.c.step();
    assert!(matches!(h.m.health(1), Some(SourceHealth::Ok { .. })));
    assert_eq!(
        h.conn.connects(),
        connects,
        "a log directory problem does not drop the connection"
    );
}
