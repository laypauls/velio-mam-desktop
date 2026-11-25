//! 원격 접속 + `fanotify_feed` 감지기 통합 시험 (T-ses-40 ~ 46). 실제 SQLite(앱 DB) + 진짜 SQLite 파일(SES `events` 스키마) + 주입 시계 +
//! **가짜 접속기**(네트워크 없음). 가짜 호스트는 수집기가 보낸 같은 SQL 을 읽기 전용으로 실행한다.
//!
//! 분류: Happy (41 e2e) / Validation (40 저장 거부·44 입력 검증) / Business rule (44 CRUD·접속 시험) / External (42 재시작 이어 읽기·45 sqlite3 없음) /
//! 원자성 (43 대기열 + 커서 한 트랜잭션) / 비노출 (44) / 자동 선택 제외 (40).
#![cfg(windows)]

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::env::{Env, snapshot, wait_until};
use mam_core::ingest::{DbCommitter, Ingest, IngestConfig, IngestDeps, RemoteSourceInput};
use mam_core::scan::IngestError;
use mam_core::secret::Secret;
use mam_core::watch::remote::collector::BatchCommitter;
use mam_core::watch::remote::collector::FeedItem;
use mam_core::watch::remote::fake::{FakeConnector, FakeSecretBox};
use mam_core::watch::remote::fake_ses::{FakeSes, SES_SCHEMA, insert_event};
use mam_core::watch::remote::hub::HubRefs;
use mam_core::watch::remote::runner::RunError;
use mam_core::watch::remote::{Mapping, RemoteHub, Timings};
use mam_core::watch::{DdpAudit, DetectorRegistry, FanotifyFeed};
use mam_core::{Blake3Hasher, SharedClock, WindowsFileIdentity};
use rusqlite::Connection;
use serde_json::json;

const FP: &str = "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DB: &str = "/srv/ses/events.db";
const IN: &str = "/data/incoming";

fn fast() -> Timings {
    Timings {
        reconnect_base: Duration::from_millis(40),
        reconnect_max: Duration::from_millis(80),
        hard_retry: Duration::from_millis(200),
        poll_override: Some(Duration::from_millis(40)),
        ..Timings::default()
    }
}

fn config_with(hub: &Arc<RemoteHub>, c: &mut IngestConfig) {
    c.remote = Arc::clone(hub);
    c.secrets = Arc::new(FakeSecretBox);
    c.detectors = Arc::new(
        DetectorRegistry::platform_default_with(vec![
            Arc::new(DdpAudit::new(Arc::clone(hub))),
            Arc::new(FanotifyFeed::new(Arc::clone(hub))),
        ])
        .with_ref_resolver(Arc::new(HubRefs(Arc::clone(hub)))),
    );
}

struct Fx {
    env: Env,
    _ses_dir: tempfile::TempDir,
    ses_file: PathBuf,
    ses: FakeSes,
    conn: Arc<FakeConnector>,
    hub: Arc<RemoteHub>,
    lib: PathBuf,
}

fn fx() -> Fx {
    let ses_dir = tempfile::TempDir::new().unwrap();
    let ses_file = ses_dir.path().join("events.db");
    Connection::open(&ses_file)
        .unwrap()
        .execute_batch(SES_SCHEMA)
        .unwrap();
    let ses = FakeSes::new(&ses_file, DB);
    let conn = Arc::new(FakeConnector::ses(ses.clone(), FP));
    let hub = RemoteHub::with_timings(conn.clone(), fast());
    let h2 = Arc::clone(&hub);
    let env = Env::with_config(move |c, _| config_with(&h2, c));
    let lib = env.path("lib");
    std::fs::create_dir_all(&lib).unwrap();
    Fx {
        env,
        _ses_dir: ses_dir,
        ses_file,
        ses,
        conn,
        hub,
        lib,
    }
}

fn input(map: Vec<(&str, PathBuf)>, host_key: Option<&str>) -> RemoteSourceInput {
    RemoteSourceInput {
        kind: "ses_queue".into(),
        name: "ses-1".into(),
        host: "10.0.0.9".into(),
        port: 22,
        user: "events".into(),
        auth_kind: "password".into(),
        secret: Some(Secret::new("pw-never-leaks".into())),
        host_key: host_key.map(str::to_string),
        log_dir: None,
        db_path: Some(DB.into()),
        poll_sec: 1,
        path_map: map
            .into_iter()
            .map(|(f, t)| Mapping {
                from: f.into(),
                to: t.to_string_lossy().into_owned(),
            })
            .collect(),
    }
}

impl Fx {
    fn source(&self) -> i64 {
        self.env
            .ingest
            .create_remote_source(input(vec![(IN, self.lib.clone())], Some(FP)))
            .unwrap()
            .source_id
    }

    fn folder(&self) -> i64 {
        self.env
            .ingest
            .add_folder(self.lib.to_str().unwrap())
            .unwrap()
            .folder_id
    }

    fn watch(&self, folder: i64, source: i64) -> Result<(), IngestError> {
        self.env
            .ingest
            .set_folder_watch_mode(
                folder,
                Some("fanotify_feed".into()),
                Some(json!({"source": source})),
            )
            .map(|_| ())
    }

    fn events(&self) -> Connection {
        Connection::open(&self.ses_file).unwrap()
    }

    fn view(&self, folder: i64) -> serde_json::Value {
        let v = self
            .env
            .ingest
            .list_folders()
            .unwrap()
            .into_iter()
            .find(|f| f.folder_id == folder)
            .unwrap();
        serde_json::to_value(v).unwrap()
    }

    fn pending(&self) -> i64 {
        self.env.count("SELECT COUNT(*) FROM watch_pending")
    }

    fn cursor(&self) -> Option<String> {
        self.env.read(|c| {
            c.query_row("SELECT cursor FROM watch_cursors", [], |r| r.get(0))
                .ok()
        })
    }

    /// 수집기가 첫 기준선(커서)을 저장할 때까지 기다린다
    fn wait_baseline(&self) {
        assert!(
            wait_until(Duration::from_secs(20), || self.cursor().is_some()),
            "collector never stored its baseline cursor"
        );
    }

    /// 주입 시계를 전진시키며 대기열 처리기를 돌린다 (안정화 표본 3회)
    fn settle(&self) {
        for _ in 0..12 {
            self.env.clock.advance(500);
            self.env.ingest.process_pending_once().unwrap();
        }
    }
}

#[test]
fn t_ses_40_the_detector_is_experimental_never_auto_selected_and_saving_rejects_bad_references_and_unreachable_maps()
 {
    let f = fx();
    let folder = f.folder();
    let list = f.env.ingest.list_change_detectors(folder).unwrap();
    let d = list
        .iter()
        .find(|d| d.id == "fanotify_feed")
        .expect("registered");
    assert!(d.experimental && d.available);
    assert_eq!(
        serde_json::to_value(&d.option_fields).unwrap()[0]["kind"],
        "ref"
    );
    assert_ne!(f.view(folder)["watchActive"], "fanotify_feed");
    assert_eq!(
        f.conn.connects(),
        0,
        "nothing connects unless the user picked the detector"
    );
    let save = |opts: Option<serde_json::Value>| {
        f.env
            .ingest
            .set_folder_watch_mode(folder, Some("fanotify_feed".into()), opts)
            .map(|_| ())
    };
    assert!(matches!(
        save(None),
        Err(IngestError::InvalidInput("watchOptions"))
    ));
    assert!(matches!(
        save(Some(json!({"source": 4242}))),
        Err(IngestError::InvalidInput("watchOptions"))
    ));
    // ddp_audit 종류 접속 (SQL 로만 만든다)은 이 감지기에 쓸 수 없다
    f.env.pool.write(mam_db::Priority::High, |c| {
        c.execute(
            "INSERT INTO remote_sources (kind, name, host, user, auth_kind, auth_blob, path_map_json, created_at, updated_at)
             VALUES ('ddp_audit', 'ddp', 'h', 'u', 'password', x'01', '[]', 1, 1)",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let ddp = f
        .env
        .count("SELECT source_id FROM remote_sources WHERE kind = 'ddp_audit'");
    assert!(
        matches!(
            save(Some(json!({"source": ddp}))),
            Err(IngestError::InvalidInput("watchOptions"))
        ),
        "wrong kind"
    );
    // 경로 변환이 폴더 루트에 닿지 않는다
    let far = f
        .env
        .ingest
        .create_remote_source(RemoteSourceInput {
            name: "far".into(),
            ..input(vec![(IN, f.env.path("nowhere"))], Some(FP))
        })
        .unwrap();
    assert!(matches!(
        save(Some(json!({"source": far.source_id}))),
        Err(IngestError::InvalidInput("pathMap"))
    ));
    assert_eq!(
        f.env
            .count("SELECT COUNT(*) FROM managed_folders WHERE watch_mode IS NOT NULL"),
        0,
        "DB unchanged"
    );
    assert_eq!(f.conn.connects(), 0);
}

#[test]
fn t_ses_41_queue_rows_become_path_and_subtree_pending_rows_then_assets_and_the_original_is_untouched()
 {
    let f = fx();
    let (folder, src) = (f.folder(), f.source());
    f.watch(folder, src).unwrap();
    f.wait_baseline();
    assert_eq!(f.view(folder)["watchActive"], "fanotify_feed");
    f.env.write("lib/clip.mov", 3);
    std::fs::create_dir_all(f.lib.join("inner")).unwrap();
    f.env.write("lib/inner/deep.mov", 4);
    let snap = snapshot(&f.env.vol);
    let ev = f.events();
    insert_event(&ev, "create", &format!("{IN}/clip.mov"), false).unwrap();
    insert_event(&ev, "close_write", &format!("{IN}/clip.mov"), false).unwrap();
    insert_event(&ev, "create", &format!("{IN}/inner"), true).unwrap(); // 새 하위 폴더 -> Subtree
    assert!(
        wait_until(Duration::from_secs(20), || f.pending() == 2),
        "one row per path"
    );
    assert_eq!(
        f.env
            .count("SELECT COUNT(*) FROM watch_pending WHERE source = 'fanotify_feed'"),
        2
    );
    assert_eq!(
        f.env.count(
            "SELECT COUNT(*) FROM watch_pending WHERE kind = 'subtree' AND abs_path LIKE '%inner'"
        ),
        1
    );
    assert_eq!(
        f.env.count(
            "SELECT COUNT(*) FROM watch_pending WHERE kind = 'path' AND abs_path LIKE '%clip.mov'"
        ),
        1
    );
    f.settle(); // close_write 도 안정화 게이트를 거친다 (결정 O)
    assert_eq!(
        f.env
            .count("SELECT COUNT(*) FROM media_assets WHERE status = 'superseded'"),
        0
    );
    assert_eq!(
        f.env.count("SELECT COUNT(*) FROM media_assets"),
        2,
        "the file and the file found by walking the new subfolder"
    );
    assert_eq!(
        snapshot(&f.env.vol),
        snap,
        "the original files are read-only for MAM"
    );
    assert!(matches!(
        f.view(folder)["watchHealth"]["state"].as_str(),
        Some("ok")
    ));
}

#[test]
fn t_ses_42_after_an_app_restart_the_collector_resumes_after_the_stored_id() {
    let f = fx();
    let (folder, src) = (f.folder(), f.source());
    f.watch(folder, src).unwrap();
    f.wait_baseline();
    insert_event(&f.events(), "create", &format!("{IN}/before.mov"), false).unwrap();
    assert!(wait_until(Duration::from_secs(20), || f.pending() == 1));
    assert_eq!(f.cursor().as_deref(), Some(r#"{"id":1}"#));
    // "앱 종료": 감시를 멈추고 대기열을 비운다 (이미 처리됐다고 본다)
    f.env.ingest.request_shutdown();
    assert!(wait_until(Duration::from_secs(10), || f
        .hub
        .running_sources()
        == 0));
    f.env
        .pool
        .write(mam_db::Priority::High, |c| {
            Ok(c.execute("DELETE FROM watch_pending", [])?)
        })
        .unwrap();
    insert_event(&f.events(), "create", &format!("{IN}/while-off.mov"), false).unwrap();
    // "재시작": 같은 DB 위에 새 Ingest + 새 허브
    let hub2 = RemoteHub::with_timings(f.conn.clone(), fast());
    let mut cfg = IngestConfig {
        auto_excludes: vec![f.env.data.clone()],
        ..IngestConfig::default()
    };
    config_with(&hub2, &mut cfg);
    let clock: SharedClock = f.env.clock.clone();
    let ingest2 = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&f.env.pool),
            clock,
            volumes: f.env.volumes.clone(),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        cfg,
    );
    ingest2
        .set_folder_watch_mode(
            folder,
            Some("fanotify_feed".into()),
            Some(json!({"source": src})),
        )
        .unwrap();
    assert!(
        wait_until(Duration::from_secs(20), || f.pending() == 1),
        "the row written while the app was off is picked up"
    );
    assert_eq!(
        f.env
            .count("SELECT COUNT(*) FROM watch_pending WHERE abs_path LIKE '%while-off.mov'"),
        1
    );
    assert_eq!(
        f.env
            .count("SELECT COUNT(*) FROM watch_pending WHERE abs_path LIKE '%before.mov'"),
        0,
        "not read twice"
    );
    assert_eq!(f.cursor().as_deref(), Some(r#"{"id":2}"#));
    ingest2.request_shutdown();
}

#[test]
fn t_ses_43_pending_rows_subtree_kind_and_the_cursor_are_one_transaction_a_failure_between_rolls_both_back()
 {
    let f = fx();
    let (folder, src) = (f.folder(), f.source());
    let committer = DbCommitter::new(Arc::clone(&f.env.pool), f.env.clock.clone(), 2_000, 50_000);
    let item = |name: &str, subtree: bool| FeedItem {
        folder_id: folder,
        path: f.lib.join(name),
        subtree,
    };
    // 성공: 둘 다 반영 (종류·출처 포함)
    committer
        .commit_feed(
            src,
            &[item("a.mov", false), item("dir", true)],
            r#"{"id":7}"#,
            "fanotify_feed",
        )
        .unwrap();
    assert_eq!(f.pending(), 2);
    assert_eq!(
        f.env.count(
            "SELECT COUNT(*) FROM watch_pending WHERE kind = 'subtree' AND source = 'fanotify_feed'"
        ),
        1
    );
    assert_eq!(committer.load_cursor(src).as_deref(), Some(r#"{"id":7}"#));
    // 실패 주입: 커서 저장이 외래 키 위반으로 실패한다 (없는 접속 ID) -> 새 행도 롤백
    assert!(
        committer
            .commit_feed(
                4242,
                &[item("b.mov", false)],
                r#"{"id":99}"#,
                "fanotify_feed"
            )
            .is_err()
    );
    assert_eq!(
        f.pending(),
        2,
        "the new pending row was rolled back with the cursor"
    );
    assert_eq!(
        committer.load_cursor(src).as_deref(),
        Some(r#"{"id":7}"#),
        "cursor unchanged"
    );
    // 같은 묶음 재기록은 합쳐지고 커서가 오른다
    committer
        .commit_feed(
            src,
            &[item("b.mov", false), item("a.mov", false)],
            r#"{"id":9}"#,
            "fanotify_feed",
        )
        .unwrap();
    assert_eq!(
        (f.pending(), committer.load_cursor(src).as_deref()),
        (3, Some(r#"{"id":9}"#))
    );
}

#[test]
fn t_ses_44_saving_requires_a_safe_db_path_and_the_connection_test_reports_row_count_and_max_id_without_leaking_secrets()
 {
    let f = fx();
    // 저장 검증: db_path 필수·안전한 경로만·log_dir 금지
    for (db, log) in [
        (None, None),
        (Some("relative.db"), None),
        (Some("/a/b;rm -rf /"), None),
        (Some("/a/'b"), None),
        (Some("/a/$(id)"), None),
        (Some("/a/b\nc"), None),
        (Some("/a/../b"), None),
    ] {
        let mut i = input(vec![(IN, f.lib.clone())], Some(FP));
        (i.db_path, i.log_dir) = (db.map(str::to_string), log);
        assert!(
            matches!(
                f.env.ingest.create_remote_source(i),
                Err(IngestError::InvalidInput("dbPath"))
            ),
            "{db:?}"
        );
    }
    let mut with_log = input(vec![(IN, f.lib.clone())], Some(FP));
    with_log.log_dir = Some("/tmp/audit".into());
    assert!(matches!(
        f.env.ingest.create_remote_source(with_log),
        Err(IngestError::InvalidInput("logDir"))
    ));
    assert_eq!(
        f.env.count("SELECT COUNT(*) FROM remote_sources"),
        0,
        "nothing was stored"
    );
    // 접속 시험: 행 수·최대 id
    let ev = f.events();
    for n in ["a", "b", "c"] {
        insert_event(&ev, "create", &format!("{IN}/{n}.mov"), false).unwrap();
    }
    let r = f
        .env
        .ingest
        .test_remote_source(None, input(vec![(IN, f.lib.clone())], Some(FP)))
        .unwrap();
    assert!(r.ok && r.error_key.is_none());
    assert_eq!(
        (r.row_count, r.max_id, r.file_count),
        (Some(3), Some(3), None)
    );
    // 지문 미확인: 접속하지 않는다
    let r = f
        .env
        .ingest
        .test_remote_source(None, input(vec![(IN, f.lib.clone())], None))
        .unwrap();
    assert_eq!((r.ok, r.error_key), (false, Some("host_key_unconfirmed")));
    assert_eq!(
        f.conn.connects(),
        1,
        "only the confirmed-key test authenticated"
    );
    // 설정한 경로에 DB 가 없다
    let mut wrong = input(vec![(IN, f.lib.clone())], Some(FP));
    wrong.db_path = Some("/srv/nowhere/events.db".into());
    let r = f.env.ingest.test_remote_source(None, wrong).unwrap();
    assert_eq!(
        (r.ok, r.error_key, r.row_count),
        (false, Some("db_unreadable"), None)
    );
    // CRUD: 읽기 모델에 db_path, 비밀 없음, 종류 변경 거부, 안전하지 않은 경로로 수정 거부
    let view = f
        .env
        .ingest
        .create_remote_source(input(vec![(IN, f.lib.clone())], Some(FP)))
        .unwrap();
    let v = serde_json::to_value(&view).unwrap();
    assert_eq!(
        (
            v["kind"].as_str(),
            v["dbPath"].as_str(),
            v["logDir"].is_null()
        ),
        (Some("ses_queue"), Some(DB), true)
    );
    assert!(!v.to_string().contains("pw-never-leaks"));
    let mut bad = input(vec![(IN, f.lib.clone())], Some(FP));
    bad.secret = None;
    bad.db_path = Some("/a/b|c".into());
    assert!(matches!(
        f.env.ingest.update_remote_source(view.source_id, bad),
        Err(IngestError::InvalidInput("dbPath"))
    ));
    let mut as_ddp = input(vec![(IN, f.lib.clone())], Some(FP));
    (as_ddp.secret, as_ddp.kind, as_ddp.db_path, as_ddp.log_dir) =
        (None, "ddp_audit".into(), None, Some("/audit".into()));
    assert!(
        matches!(
            f.env.ingest.update_remote_source(view.source_id, as_ddp),
            Err(IngestError::InvalidInput("kind"))
        ),
        "the kind cannot change"
    );
    assert_eq!(
        f.env.ingest.list_remote_sources().unwrap()[0]
            .db_path
            .as_deref(),
        Some(DB)
    );
    // 쓰는 폴더가 있으면 삭제 거부 (fanotify_feed 도 참조로 센다)
    let folder = f.folder();
    f.watch(folder, view.source_id).unwrap();
    assert!(matches!(
        f.env.ingest.delete_remote_source(view.source_id),
        Err(IngestError::Busy)
    ));
    f.env
        .ingest
        .set_folder_watch_mode(folder, None, None)
        .unwrap();
    f.env.ingest.delete_remote_source(view.source_id).unwrap();
}

#[test]
fn t_ses_45_a_host_without_sqlite3_degrades_with_its_own_reason_and_reads_nothing() {
    let f = fx();
    let (folder, src) = (f.folder(), f.source());
    f.ses.set_fail(Some(RunError::NoSqlite));
    f.watch(folder, src).unwrap();
    assert!(wait_until(
        Duration::from_secs(20),
        || f.view(folder)["watchHealth"]["reasonKey"] == "watch.health.remote_sqlite_missing"
    ));
    assert_eq!((f.pending(), f.cursor()), (0, None));
    let r = f
        .env
        .ingest
        .test_remote_source(None, input(vec![(IN, f.lib.clone())], Some(FP)))
        .unwrap();
    assert_eq!((r.ok, r.error_key), (false, Some("no_sqlite")));
    // 설치되면 `Ok` 로 돌아온다
    f.ses.set_fail(None);
    // (신호가 아직 없으면 `watchHealth` 자체가 없다 — 사유 키가 사라지면 복구된 것)
    assert!(wait_until(Duration::from_secs(20), || {
        f.view(folder)["watchHealth"]["reasonKey"].is_null()
    }));
    f.wait_baseline();
}

#[test]
fn t_ses_46_a_reset_queue_resets_the_stored_cursor_to_the_new_end() {
    let f = fx();
    let (folder, src) = (f.folder(), f.source());
    let ev = f.events();
    for _ in 0..5 {
        insert_event(&ev, "create", &format!("{IN}/x.mov"), false).unwrap();
    }
    f.watch(folder, src).unwrap();
    assert!(
        wait_until(Duration::from_secs(20), || f.cursor().as_deref()
            == Some(r#"{"id":5}"#)),
        "baseline at the current end, no replay"
    );
    assert_eq!(f.pending(), 0);
    // SES DB 가 새로 만들어졌다
    ev.execute_batch("DELETE FROM events; DELETE FROM sqlite_sequence WHERE name = 'events';")
        .unwrap();
    insert_event(&ev, "create", &format!("{IN}/new.mov"), false).unwrap();
    assert!(
        wait_until(Duration::from_secs(20), || f.cursor().as_deref()
            == Some(r#"{"id":1}"#)),
        "cursor went back to the new end"
    );
}
