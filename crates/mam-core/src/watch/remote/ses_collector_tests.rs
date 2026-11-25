//! SES 수집기 시험 (T-ses-20 ~ 30). 진짜 SQLite(SES 스키마) + 가짜 접속·가짜 기록기·주입 시계 — 스레드·네트워크 없이 `step()` 을 직접 돌린다.
//!
//! 분류: Happy (20 기준선·21 라우팅·23 재시작) / Boundary (27 폴더 루트 자신) / Validation (21 규칙 밖 경로·30 호스트 키) /
//! Business rule (24 큐 초기화·25 밀림 건너뜀·26 재접속 재스캔 1회·29 설정 변경) / External (26 사유 키·22 기록 실패) / 비노출 (28).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use tempfile::TempDir;

use super::SesCollector;
use crate::SharedClock;
use crate::secret::Secret;
use crate::volume::VolumeKind;
use crate::watch::detector::{
    ChangeDetector, DetectError, DetectorOptions, ProbeResult, WatchHandle, WatchTarget,
};
use crate::watch::remote::contract::{
    Attached, BatchCommitter, CommitError, CommitOutcome, Deps, FeedItem, Folders, Timings,
};
use crate::watch::remote::fake::FakeConnector;
use crate::watch::remote::fake_ses::{FakeSes, SES_SCHEMA, insert_event};
use crate::watch::remote::mapper::Mapping;
use crate::watch::remote::runner::RunError;
use crate::watch::remote::ses::{BACKLOG_MAX, Cursor};
use crate::watch::remote::{Auth, RemoteSource, SourceKind};
use crate::watch::{ChangeSink, DetectorRegistry, SourceHealth, WatchManager, WatchMsg};

const T0: i64 = 1_700_000_000_000;
const FP: &str = "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const LOGICAL: &str = "/srv/ses/events.db";
const IN: &str = "/data/incoming";

struct Clock(AtomicI64);

impl mam_db::Clock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

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

type Commit = (Vec<FeedItem>, String, &'static str);

#[derive(Default)]
struct FakeCommitter {
    commits: Mutex<Vec<Commit>>,
    cursor: Mutex<Option<String>>,
    fail: AtomicBool,
}

impl BatchCommitter for FakeCommitter {
    fn commit_feed(
        &self,
        _source_id: i64,
        items: &[FeedItem],
        cursor: &str,
        origin: &'static str,
    ) -> Result<CommitOutcome, CommitError> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(CommitError("injected".into()));
        }
        self.commits
            .lock()
            .unwrap()
            .push((items.to_vec(), cursor.to_string(), origin));
        *self.cursor.lock().unwrap() = Some(cursor.to_string());
        Ok(CommitOutcome::default())
    }

    fn load_cursor(&self, _source_id: i64) -> Option<String> {
        self.cursor.lock().unwrap().clone()
    }
}

struct H {
    _dir: TempDir,
    db: Connection,
    clock: Arc<Clock>,
    ses: FakeSes,
    conn: Arc<FakeConnector>,
    commit: Arc<FakeCommitter>,
    folders: Folders,
    m: WatchManager,
    sinks: Arc<Mutex<HashMap<i64, ChangeSink>>>,
    rx: Receiver<WatchMsg>,
    c: SesCollector,
    shared: SharedClock,
}

fn source(host_key: Option<&str>, rules: &[(&str, &str)]) -> RemoteSource {
    RemoteSource {
        source_id: 1,
        kind: SourceKind::SesQueue,
        name: "ses-1".into(),
        host: "10.0.0.9".into(),
        port: 22,
        user: "events".into(),
        auth: Auth::Password(Secret::new("pw-never-logged".into())),
        host_key: host_key.map(str::to_string),
        log_dir: None,
        db_path: Some(LOGICAL.into()),
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

fn collector(h: &Fixture, host_key: Option<&str>, rules: &[(&str, &str)]) -> SesCollector {
    SesCollector::new(
        source(host_key, rules),
        Deps {
            connector: h.conn.clone(),
            committer: h.commit.clone(),
            clock: Arc::clone(&h.shared),
            timings: Timings::default(),
            folders: Arc::clone(&h.folders),
            loader: Arc::new(|_| None),
        },
    )
}

/// 수집기를 만들기 전의 뼈대 (재시작 시험이 같은 DB·기록기로 수집기를 다시 만든다)
struct Fixture {
    conn: Arc<FakeConnector>,
    commit: Arc<FakeCommitter>,
    folders: Folders,
    shared: SharedClock,
}

fn harness(roots: &[(i64, &str)], rules: &[(&str, &str)], host_key: Option<&str>) -> H {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("events.db");
    let db = Connection::open(&file).unwrap();
    db.execute_batch(SES_SCHEMA).unwrap();
    let ses = FakeSes::new(&file, LOGICAL);
    let clock = Arc::new(Clock(AtomicI64::new(T0)));
    let shared: SharedClock = clock.clone();
    let fx = Fixture {
        conn: Arc::new(FakeConnector::ses(ses.clone(), FP)),
        commit: Arc::new(FakeCommitter::default()),
        folders: Arc::new(Mutex::new(Vec::new())),
        shared: Arc::clone(&shared),
    };
    let sinks = Arc::new(Mutex::new(HashMap::new()));
    let (tx, rx) = channel();
    let all: Vec<Arc<dyn ChangeDetector>> = vec![Arc::new(Parking(Arc::clone(&sinks)))];
    let m = WatchManager::with_clock(
        tx,
        Arc::new(DetectorRegistry::new(all, &["parking"])),
        Arc::clone(&shared),
    );
    let c = collector(&fx, host_key, rules);
    let mut h = H {
        _dir: dir,
        db,
        clock,
        ses,
        conn: fx.conn,
        commit: fx.commit,
        folders: fx.folders,
        m,
        sinks,
        rx,
        c,
        shared,
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

    fn rescans(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self
            .rx
            .try_iter()
            .filter_map(|m| matches!(m, WatchMsg::Rescan { .. }).then(|| m.folder_id()))
            .collect();
        v.sort_unstable();
        v
    }

    fn add(&self, types: &str, path: &str, is_dir: bool) -> i64 {
        insert_event(&self.db, types, path, is_dir).unwrap()
    }

    /// 첫 주기(기준선)를 지나고 도착한 재스캔을 비운다
    fn primed(mut self) -> Self {
        self.c.step();
        self.rescans();
        self.advance(10_000);
        self
    }

    fn last(&self) -> Commit {
        self.commit
            .commits
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a commit")
    }

    fn n_commits(&self) -> usize {
        self.commit.commits.lock().unwrap().len()
    }

    fn stored_cursor(&self) -> Option<i64> {
        self.commit
            .cursor
            .lock()
            .unwrap()
            .as_deref()
            .and_then(Cursor::from_json)
            .map(|c| c.id)
    }
}

fn uhd() -> H {
    harness(&[(1, r"F:\UHD")], &[(IN, r"F:\UHD")], Some(FP))
}

fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
}

#[test]
fn t_ses_20_the_first_step_baselines_at_the_current_max_id_without_replaying_history_and_asks_each_folder_for_one_rescan()
 {
    // Arrange
    let mut h = harness(
        &[(1, r"F:\UHD"), (2, r"F:\UHD\1TV")],
        &[(IN, r"F:\UHD")],
        Some(FP),
    );
    h.add("create", &format!("{IN}/old.mov"), false);
    h.add("create", &format!("{IN}/older.mov"), false);
    // Act
    let wait = h.c.step();
    // Assert
    assert_eq!(wait, Duration::from_secs(3));
    assert_eq!(h.n_commits(), 1);
    assert!(h.last().0.is_empty(), "history is not replayed");
    assert_eq!(h.stored_cursor(), Some(2));
    assert_eq!(h.rescans(), vec![1, 2]);
    assert!(matches!(h.m.health(1), Some(SourceHealth::Ok { .. })));
    // 같은 상태에서 다시 돌면 아무것도 기록하지 않는다
    h.advance(10_000);
    h.c.step();
    assert_eq!(h.n_commits(), 1, "nothing new -> no commit");
}

#[test]
fn t_ses_21_events_map_to_path_or_subtree_reports_only_for_folders_that_contain_them_and_outside_paths_are_counted()
 {
    // Arrange
    let mut h = harness(
        &[(1, r"F:\UHD"), (2, r"F:\UHD\1TV"), (3, r"G:\Other")],
        &[(IN, r"F:\UHD")],
        Some(FP),
    )
    .primed();
    h.add("create,close_write", &format!("{IN}/1TV/a.mov"), false);
    h.add("create", &format!("{IN}/1TV/새 폴더"), true);
    h.add("delete", &format!("{IN}/1TV/gone.mov"), false);
    h.add("moved_from", &format!("{IN}/2TV/old.mov"), false);
    h.add("moved_to", &format!("{IN}/2TV/new.mov"), false);
    h.add("modify", &format!("{IN}/1TV/ignored.mov"), false);
    h.add("create", "/elsewhere/x.mov", false); // 변환 규칙 밖
    // Act
    h.c.step();
    // Assert
    let (mut items, cursor, origin) = h.last();
    items.sort_by(|a, b| (a.folder_id, &a.path).cmp(&(b.folder_id, &b.path)));
    let item = |f: i64, path: &str, subtree: bool| FeedItem {
        folder_id: f,
        path: p(path),
        subtree,
    };
    assert_eq!(
        items,
        vec![
            item(1, r"F:\UHD\1TV\a.mov", false),
            item(1, r"F:\UHD\1TV\gone.mov", false),
            item(1, r"F:\UHD\1TV\새 폴더", true),
            item(1, r"F:\UHD\2TV\new.mov", false),
            item(1, r"F:\UHD\2TV\old.mov", false),
            item(2, r"F:\UHD\1TV\a.mov", false),
            item(2, r"F:\UHD\1TV\gone.mov", false),
            item(2, r"F:\UHD\1TV\새 폴더", true),
        ],
        "folder 3 (G:\\Other) never receives them; modify is ignored"
    );
    assert_eq!((origin, cursor.as_str()), ("fanotify_feed", r#"{"id":7}"#));
    let (win, _) = h.c.counts();
    assert_eq!(
        (win.unmapped, win.reported, win.outside),
        (1, 5, 0),
        "the unmapped path is dropped and counted"
    );
    assert!(
        matches!(h.m.health(1), Some(SourceHealth::Ok { last_signal_at: Some(t) }) if t == T0 + 10_000)
    );
}

#[test]
fn t_ses_22_a_failed_record_keeps_the_cursor_and_the_same_rows_are_reported_after_the_next_success_and_the_cursor_never_goes_back()
 {
    // Arrange
    let mut h = uhd().primed();
    let id1 = h.add("close_write", &format!("{IN}/a.mov"), false);
    h.commit.fail.store(true, Ordering::SeqCst);
    // Act
    h.c.step();
    // Assert: 기록이 실패하면 커서는 그대로, 연결 문제로 보지 않는다
    assert_eq!(h.stored_cursor(), Some(0));
    assert!(!h.c.is_degraded());
    h.commit.fail.store(false, Ordering::SeqCst);
    h.c.step();
    assert_eq!(
        h.last().0,
        vec![FeedItem {
            folder_id: 1,
            path: p(r"F:\UHD\a.mov"),
            subtree: false
        }]
    );
    assert_eq!(h.stored_cursor(), Some(id1));
    let id2 = h.add("close_write", &format!("{IN}/b.mov"), false);
    h.c.step();
    let all: Vec<i64> = h
        .commit
        .commits
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| Cursor::from_json(&c.1))
        .map(|c| c.id)
        .collect();
    assert!(
        all.windows(2).all(|w| w[0] <= w[1]),
        "cursor is monotonic: {all:?}"
    );
    assert_eq!(all.last(), Some(&id2));
}

#[test]
fn t_ses_23_a_restarted_collector_resumes_after_the_stored_id_without_a_rescan() {
    // Arrange: 첫 수집기가 3 까지 반영했다
    let mut h = uhd().primed();
    for n in ["a", "b", "c"] {
        h.add("close_write", &format!("{IN}/{n}.mov"), false);
    }
    h.c.step();
    assert_eq!(h.stored_cursor(), Some(3));
    // 앱이 꺼진 사이에 2건이 쌓였다
    h.add("close_write", &format!("{IN}/d.mov"), false);
    h.add("create", &format!("{IN}/e.mov"), false);
    // Act: 같은 기록기(저장된 커서)로 새 수집기
    let fx = Fixture {
        conn: h.conn.clone(),
        commit: h.commit.clone(),
        folders: Arc::clone(&h.folders),
        shared: Arc::clone(&h.shared),
    };
    h.c = collector(&fx, Some(FP), &[(IN, r"F:\UHD")]);
    h.advance(10_000);
    h.c.step();
    // Assert
    let paths: Vec<PathBuf> = h.last().0.iter().map(|i| i.path.clone()).collect();
    assert_eq!(
        paths,
        vec![p(r"F:\UHD\d.mov"), p(r"F:\UHD\e.mov")],
        "only the rows after the stored id"
    );
    assert_eq!(h.stored_cursor(), Some(5));
    assert!(
        h.rescans().is_empty(),
        "continuity from the cursor needs no rescan"
    );
}

#[test]
fn t_ses_24_a_reset_queue_rescans_once_and_the_cursor_goes_back_to_the_new_end() {
    // Arrange: 커서 5 까지 따라갔다
    let mut h = uhd().primed();
    for _ in 0..5 {
        h.add("create", &format!("{IN}/x.mov"), false);
    }
    h.c.step();
    assert_eq!(h.stored_cursor(), Some(5));
    // SES DB 가 새로 만들어졌다 (id 가 1 부터 다시)
    h.db.execute_batch("DELETE FROM events; DELETE FROM sqlite_sequence WHERE name = 'events';")
        .unwrap();
    h.add("create", &format!("{IN}/new.mov"), false);
    // Act
    h.advance(10_000);
    h.c.step();
    // Assert
    assert_eq!(h.stored_cursor(), Some(1));
    assert!(
        h.last().0.is_empty(),
        "the reset row is covered by the rescan, not reported"
    );
    assert_eq!(h.rescans(), vec![1]);
    h.add("close_write", &format!("{IN}/after.mov"), false);
    h.advance(10_000);
    h.c.step();
    assert_eq!(h.last().0.len(), 1, "reading continues after the reset");
    assert!(h.rescans().is_empty());
}

#[test]
fn t_ses_25_a_backlog_over_50_000_rows_skips_to_the_end_reports_nothing_and_rescans_once() {
    // Arrange
    let mut h = uhd().primed();
    h.db
        .execute_batch(&format!(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {})
             INSERT INTO events (captured_at, event_mask, event_types, path, name, dir, is_dir) SELECT 't', 0, 'create', '{IN}/f' || i, 'f', '{IN}', 0 FROM n;",
            BACKLOG_MAX + 1
        ))
        .unwrap();
    // Act
    h.advance(10_000);
    h.c.step();
    // Assert
    assert!(h.last().0.is_empty());
    assert_eq!(h.stored_cursor(), Some(BACKLOG_MAX + 1));
    assert_eq!(h.rescans(), vec![1]);
    let (win, _) = h.c.counts();
    assert_eq!(win.skipped_rows, (BACKLOG_MAX + 1) as u64);
}

#[test]
fn t_ses_26_an_unreadable_queue_degrades_with_a_specific_reason_then_recovers_with_exactly_one_rescan()
 {
    // Arrange
    let mut h = uhd().primed();
    // Act / Assert: sqlite3 없음 -> 사유 키, 접속은 닫지 않는다 (재접속 시도 없음)
    let connects = h.conn.connects();
    h.ses.set_fail(Some(RunError::NoSqlite));
    h.c.step();
    assert_eq!(
        h.m.health(1),
        Some(SourceHealth::Degraded {
            since: T0 + 10_000,
            reason_key: "watch.health.remote_sqlite_missing"
        })
    );
    assert_eq!(
        h.conn.connects(),
        connects,
        "a missing tool is not a link failure"
    );
    h.ses.set_fail(Some(RunError::DbUnreadable(
        "unable to open database file".into(),
    )));
    h.c.step();
    assert_eq!(
        h.m.health(1),
        Some(SourceHealth::Degraded {
            since: T0 + 10_000,
            reason_key: "watch.health.remote_db_unreadable"
        })
    );
    assert!(h.rescans().is_empty(), "no rescan while still down");
    // 접속 끊김: 사유가 바뀐다
    h.ses.set_fail(Some(RunError::Closed));
    h.conn.fail_connects(1, RunError::Connect("down".into()));
    h.c.step();
    assert!(matches!(
        h.m.health(1),
        Some(SourceHealth::Degraded {
            reason_key: "watch.health.remote_connect",
            ..
        })
    ));
    // 돌아왔다
    h.ses.set_fail(None);
    h.advance(60_000);
    assert_eq!(h.c.step(), Duration::from_secs(3));
    assert!(matches!(h.m.health(1), Some(SourceHealth::Ok { .. })));
    assert_eq!(h.rescans(), vec![1], "one rescan covers the gap");
    h.c.step();
    assert!(h.rescans().is_empty(), "and only one");
    assert!(!h.c.is_degraded());
}

#[test]
fn t_ses_27_a_directory_event_for_the_folder_root_itself_becomes_a_rescan_not_a_full_subtree_walk()
{
    // Arrange
    let mut h = uhd().primed();
    h.add("create", IN, true);
    h.add("close_write", &format!("{IN}/a.mov"), false);
    // Act
    h.c.step();
    // Assert
    assert_eq!(
        h.last().0,
        vec![FeedItem {
            folder_id: 1,
            path: p(r"F:\UHD\a.mov"),
            subtree: false
        }]
    );
    assert_eq!(h.rescans(), vec![1]);
}

#[test]
fn t_ses_28_secrets_never_reach_debug_output_or_error_texts() {
    // Arrange
    let h = uhd();
    let src = source(Some(FP), &[(IN, r"F:\UHD")]);
    // Act
    let texts = [
        format!("{src:?}"),
        format!(
            "{:?}",
            RunError::DbUnreadable("unable to open database file".into())
        ),
        RunError::NoSqlite.to_string(),
        RunError::DbUnreadable("x".into()).to_string(),
    ];
    // Assert
    for t in &texts {
        assert!(!t.contains("pw-never-logged"), "{t}");
    }
    assert!(h.ses.commands().is_empty());
    let seen = h.conn.seen();
    assert!(seen.iter().all(|(_, u)| !u.contains("pw-never-logged")));
}

#[test]
fn t_ses_29_a_reload_with_the_same_db_keeps_the_cursor_and_a_different_db_starts_a_new_baseline() {
    // Arrange
    let mut h = uhd().primed();
    h.add("create", &format!("{IN}/a.mov"), false);
    h.c.step();
    assert_eq!(h.c.cursor(), Some(Cursor { id: 1 }));
    // Act / Assert
    h.c.reload(source(Some(FP), &[(IN, r"F:\UHD")]));
    assert_eq!(
        h.c.cursor(),
        Some(Cursor { id: 1 }),
        "same db path -> same cursor"
    );
    let mut other = source(Some(FP), &[(IN, r"F:\UHD")]);
    other.db_path = Some("/srv/other/events.db".into());
    h.c.reload(other);
    assert_eq!(h.c.cursor(), None, "another database is a new queue");
}

#[test]
fn t_ses_30_an_unconfirmed_or_mismatching_host_key_reads_nothing() {
    // Arrange / Act
    let mut h = harness(&[(1, r"F:\UHD")], &[(IN, r"F:\UHD")], None);
    let wait = h.c.step();
    // Assert
    assert_eq!(wait, Duration::from_secs(300));
    assert_eq!(
        h.m.health(1),
        Some(SourceHealth::Degraded {
            since: T0,
            reason_key: "watch.health.remote_host_key"
        })
    );
    assert_eq!(
        (h.n_commits(), h.ses.commands().len()),
        (0, 0),
        "nothing is read from an unverified host"
    );
}
