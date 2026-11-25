//! 원격 접속 + `ddp_audit` 감지기 통합 시험 (T-ddp-10 ~ 19). 실제 SQLite + 임시 폴더 + 주입 시계 + **가짜 접속기**(네트워크 없음).
//!
//! 분류: Happy (12 e2e·13 접속 공유) / Validation (11 저장 거부·15 접속 시험) / Business rule (16 CRUD·삭제 거부·18 폴백 사유) /
//! External (14 재시작 이어 읽기·18 호스트 키 불일치) / Concurrency (13 공유·해제) / 원자성 (17 대기열 + 커서 한 트랜잭션) / 비노출 (15·19).
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
use mam_core::watch::remote::fake::{FakeConnector, FakeDdp, FakeSecretBox, line};
use mam_core::watch::remote::hub::HubRefs;
use mam_core::watch::remote::{Mapping, RemoteHub, Timings};
use mam_core::watch::{DdpAudit, DetectorRegistry, SourceHealth};
use mam_core::{Blake3Hasher, SharedClock, WindowsFileIdentity};
use serde_json::json;

const FP: &str = "SHA256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_FP: &str = "SHA256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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
        DetectorRegistry::platform_default_with(vec![Arc::new(DdpAudit::new(Arc::clone(hub)))])
            .with_ref_resolver(Arc::new(HubRefs(Arc::clone(hub)))),
    );
}

struct Fx {
    env: Env,
    ddp: FakeDdp,
    conn: Arc<FakeConnector>,
    hub: Arc<RemoteHub>,
    lib: PathBuf,
}

fn fx() -> Fx {
    let ddp = FakeDdp::new("/audit");
    ddp.write("audit-log.0", "");
    let conn = Arc::new(FakeConnector::new(ddp.clone(), FP));
    let hub = RemoteHub::with_timings(conn.clone(), fast());
    let h2 = Arc::clone(&hub);
    let env = Env::with_config(move |c, _| config_with(&h2, c));
    let lib = env.path("lib");
    std::fs::create_dir_all(&lib).unwrap();
    Fx {
        env,
        ddp,
        conn,
        hub,
        lib,
    }
}

fn input(map: Vec<(&str, PathBuf)>, host_key: Option<&str>) -> RemoteSourceInput {
    RemoteSourceInput {
        kind: "ddp_audit".into(),
        name: "ddp-1".into(),
        host: "10.0.0.5".into(),
        port: 22,
        user: "audit".into(),
        auth_kind: "password".into(),
        secret: Some(Secret::new("pw-never-leaks".into())),
        host_key: host_key.map(str::to_string),
        log_dir: Some("/audit".into()),
        db_path: None,
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
            .create_remote_source(input(vec![("/", self.lib.clone())], Some(FP)))
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
                Some("ddp_audit".into()),
                Some(json!({"source": source})),
            )
            .map(|_| ())
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

    fn cursor_rows(&self) -> i64 {
        self.env.count("SELECT COUNT(*) FROM watch_cursors")
    }

    fn pending(&self) -> i64 {
        self.env.count("SELECT COUNT(*) FROM watch_pending")
    }

    /// 수집기가 첫 기준선(커서)을 저장할 때까지 기다린다
    fn wait_baseline(&self) {
        assert!(
            wait_until(Duration::from_secs(20), || self.cursor_rows() == 1),
            "collector never stored its baseline cursor"
        );
    }
}

#[test]
fn t_ddp_10_the_detector_is_listed_as_experimental_and_available_but_never_auto_selected() {
    let f = fx();
    let folder = f.folder();
    let list = f.env.ingest.list_change_detectors(folder).unwrap();
    let d = list
        .iter()
        .find(|d| d.id == "ddp_audit")
        .expect("registered");
    assert!(d.experimental && d.available);
    assert_eq!(
        serde_json::to_value(&d.option_fields).unwrap()[0]["kind"],
        "ref"
    );
    // 자동 선택에는 들어가지 않는다: 방식을 고르지 않은 폴더는 ddp_audit 로 동작하지 않는다
    assert_ne!(f.view(folder)["watchActive"], "ddp_audit");
    assert_eq!(
        f.conn.connects(),
        0,
        "nothing connects unless the user picked the detector for a folder"
    );
}

#[test]
fn t_ddp_11_saving_is_rejected_for_missing_unknown_wrong_kind_and_unreachable_sources_and_nothing_is_stored()
 {
    let f = fx();
    let folder = f.folder();
    let save = |opts: Option<serde_json::Value>| {
        f.env
            .ingest
            .set_folder_watch_mode(folder, Some("ddp_audit".into()), opts)
            .map(|_| ())
    };
    assert!(
        matches!(save(None), Err(IngestError::InvalidInput("watchOptions"))),
        "source is required"
    );
    assert!(matches!(
        save(Some(json!({}))),
        Err(IngestError::InvalidInput("watchOptions"))
    ));
    assert!(
        matches!(
            save(Some(json!({"source": 4242}))),
            Err(IngestError::InvalidInput("watchOptions"))
        ),
        "unknown id"
    );
    assert!(matches!(
        save(Some(json!({"source": "1"}))),
        Err(IngestError::InvalidInput("watchOptions"))
    ));
    // 종류가 다른 접속 (후속 단계 값 — SQL 로만 만든다)
    f.env.pool.write(mam_db::Priority::High, |c| {
        c.execute(
            "INSERT INTO remote_sources (kind, name, host, user, auth_kind, auth_blob, path_map_json, created_at, updated_at)
             VALUES ('ses_queue', 'ses', 'h', 'u', 'password', x'01', '[]', 1, 1)",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    let ses = f
        .env
        .count("SELECT source_id FROM remote_sources WHERE kind = 'ses_queue'");
    assert!(
        matches!(
            save(Some(json!({"source": ses}))),
            Err(IngestError::InvalidInput("watchOptions"))
        ),
        "wrong kind"
    );
    // 경로 변환이 폴더 루트에 닿지 않는다
    let nowhere = f.env.path("nowhere");
    let bad = f
        .env
        .ingest
        .create_remote_source(RemoteSourceInput {
            name: "far".into(),
            ..input(vec![("/", nowhere)], Some(FP))
        })
        .unwrap();
    assert!(matches!(
        save(Some(json!({"source": bad.source_id}))),
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
fn t_ddp_12_an_audit_line_becomes_a_pending_row_then_one_asset_and_the_original_is_untouched() {
    let f = fx();
    let (folder, src) = (f.folder(), f.source());
    f.watch(folder, src).unwrap();
    f.wait_baseline();
    assert_eq!(f.view(folder)["watchActive"], "ddp_audit");
    let before = snapshot(&f.env.vol);
    f.env.write("lib/clip.mov", 3);
    let snap = snapshot(&f.env.vol);
    f.ddp.write(
        "audit-log.0",
        &(line("CREATE", "/clip.mov") + &line("WRITE", "/clip.mov") + &line("READ", "/clip.mov")),
    );
    assert!(
        wait_until(Duration::from_secs(20), || f.pending() == 1),
        "one row per path no matter how many lines"
    );
    let row_source = f
        .env
        .count("SELECT COUNT(*) FROM watch_pending WHERE source = 'ddp_audit'");
    assert_eq!(row_source, 1);
    // 안정화 게이트를 지나 반영된다 (표본 3회 — 주입 시계를 전진)
    for _ in 0..8 {
        f.env.clock.advance(500);
        f.env.ingest.process_pending_once().unwrap();
    }
    assert_eq!(f.env.count("SELECT COUNT(*) FROM media_assets"), 1);
    assert_eq!(
        f.env
            .count("SELECT COUNT(*) FROM media_assets WHERE status = 'superseded'"),
        0
    );
    assert_eq!(
        snapshot(&f.env.vol),
        snap,
        "the original file is read-only for MAM"
    );
    assert_ne!(before, snap);
    assert!(matches!(
        f.view(folder)["watchHealth"]["state"].as_str(),
        Some("ok")
    ));
}

#[test]
fn t_ddp_13_folders_share_one_connection_and_each_gets_only_its_own_paths_and_the_collector_stops_with_the_last_folder()
 {
    let f = fx();
    let (a, b) = (f.env.path("lib_a"), f.env.path("lib_b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let src = f
        .env
        .ingest
        .create_remote_source(input(vec![("/a", a.clone()), ("/b", b.clone())], Some(FP)))
        .unwrap()
        .source_id;
    let fa = f
        .env
        .ingest
        .add_folder(a.to_str().unwrap())
        .unwrap()
        .folder_id;
    let fb = f
        .env
        .ingest
        .add_folder(b.to_str().unwrap())
        .unwrap()
        .folder_id;
    f.watch(fa, src).unwrap();
    f.watch(fb, src).unwrap();
    f.wait_baseline();
    assert_eq!(f.hub.running_sources(), 1);
    f.ddp.write(
        "audit-log.0",
        &(line("CREATE", "/a/x.mov") + &line("CREATE", "/b/y.mov") + &line("CREATE", "/c/z.mov")),
    );
    assert!(wait_until(Duration::from_secs(20), || f.pending() == 2));
    assert_eq!(
        f.env.count(&format!(
            "SELECT COUNT(*) FROM watch_pending WHERE folder_id = {fa} AND abs_path LIKE '%x.mov'"
        )),
        1
    );
    assert_eq!(
        f.env.count(&format!(
            "SELECT COUNT(*) FROM watch_pending WHERE folder_id = {fb} AND abs_path LIKE '%y.mov'"
        )),
        1
    );
    assert_eq!(f.conn.connects(), 1, "one connection for both folders");
    f.env.ingest.set_folder_watch_mode(fa, None, None).unwrap();
    assert_eq!(f.hub.running_sources(), 1, "still used by the other folder");
    f.env.ingest.set_folder_watch_mode(fb, None, None).unwrap();
    assert_eq!(f.hub.running_sources(), 0);
}

#[test]
fn t_ddp_14_after_an_app_restart_the_collector_resumes_from_the_stored_cursor() {
    let f = fx();
    let (folder, src) = (f.folder(), f.source());
    f.watch(folder, src).unwrap();
    f.wait_baseline();
    f.ddp.write("audit-log.0", &line("CREATE", "/before.mov"));
    assert!(wait_until(Duration::from_secs(20), || f.pending() == 1));
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
    // 꺼져 있는 동안 쌓인 줄
    f.ddp
        .write("audit-log.0", &line("CREATE", "/while-off.mov"));
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
            Some("ddp_audit".into()),
            Some(json!({"source": src})),
        )
        .unwrap();
    assert!(
        wait_until(Duration::from_secs(20), || f.pending() == 1),
        "the line written while the app was off is picked up"
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
    ingest2.request_shutdown();
}

#[test]
fn t_ddp_15_the_connection_test_never_sends_credentials_before_the_host_key_is_confirmed_and_returns_only_keys_and_counts()
 {
    let f = fx();
    // 지문 미확인: 접속하지 않고 지문만 알려 준다
    let r = f
        .env
        .ingest
        .test_remote_source(None, input(vec![("/", f.lib.clone())], None))
        .unwrap();
    assert!(!r.ok);
    assert_eq!(
        (r.error_key, r.fingerprint.as_deref()),
        (Some("host_key_unconfirmed"), Some(FP))
    );
    assert_eq!(f.conn.connects(), 0, "no authentication attempt");
    // 확인한 지문으로: 성공 + 로그 파일 수
    let r = f
        .env
        .ingest
        .test_remote_source(None, input(vec![("/", f.lib.clone())], Some(FP)))
        .unwrap();
    assert!(r.ok && r.file_count == Some(1) && r.error_key.is_none());
    // 지문이 다르면: 거부 + 서버 지문
    let r = f
        .env
        .ingest
        .test_remote_source(None, input(vec![("/", f.lib.clone())], Some(OTHER_FP)))
        .unwrap();
    assert_eq!(
        (r.ok, r.error_key, r.fingerprint.as_deref()),
        (false, Some("host_key_mismatch"), Some(FP))
    );
    // 저장된 비밀 재사용 / 비밀이 어디에도 없으면 입력 오류
    let id = f.source();
    let mut no_secret = input(vec![("/", f.lib.clone())], Some(FP));
    no_secret.secret = None;
    assert!(
        f.env
            .ingest
            .test_remote_source(Some(id), no_secret)
            .unwrap()
            .ok
    );
    let mut none_at_all = input(vec![("/", f.lib.clone())], Some(FP));
    none_at_all.secret = None;
    assert!(matches!(
        f.env.ingest.test_remote_source(None, none_at_all),
        Err(IngestError::InvalidInput("secret"))
    ));
    // 결과·입력 어디에도 비밀이 없다
    let json = serde_json::to_string(&r).unwrap();
    assert!(!json.contains("pw-never-leaks"));
    assert!(!format!("{:?}", input(vec![], None)).contains("pw-never-leaks"));
}

#[test]
fn t_ddp_16_crud_keeps_secrets_out_rejects_duplicates_and_refuses_changes_that_orphan_a_folder() {
    let f = fx();
    let view = f
        .env
        .ingest
        .create_remote_source(input(vec![("/", f.lib.clone())], Some(FP)))
        .unwrap();
    let json = serde_json::to_string(&view).unwrap();
    assert!(
        json.contains("\"hasSecret\":true")
            && !json.contains("pw-never-leaks")
            && !json.contains("authBlob")
    );
    assert!(matches!(
        f.env
            .ingest
            .create_remote_source(input(vec![("/", f.lib.clone())], Some(FP))),
        Err(IngestError::AlreadyRegistered)
    ));
    let mut no_secret = input(vec![("/", f.lib.clone())], Some(FP));
    no_secret.secret = None;
    no_secret.name = "n2".into();
    assert!(matches!(
        f.env.ingest.create_remote_source(no_secret),
        Err(IngestError::InvalidInput("secret"))
    ));
    let mut bad_kind = input(vec![("/", f.lib.clone())], Some(FP));
    bad_kind.kind = "ftp".into();
    assert!(matches!(
        f.env.ingest.create_remote_source(bad_kind),
        Err(IngestError::InvalidInput("kind"))
    ));
    // ses_queue 는 DB 경로가 필수이고 로그 폴더는 쓰지 않는다 (입력은 ddp 용 그대로 — 거부)
    let mut ses_as_ddp = input(vec![("/", f.lib.clone())], Some(FP));
    ses_as_ddp.kind = "ses_queue".into();
    assert!(matches!(
        f.env.ingest.create_remote_source(ses_as_ddp),
        Err(IngestError::InvalidInput("logDir"))
    ));
    // 폴더가 쓰는 접속: 경로 변환을 폴더에서 벗어나게 바꾸거나 삭제하는 것은 거부
    let folder = f.folder();
    f.watch(folder, view.source_id).unwrap();
    let far = input(vec![("/", f.env.path("nowhere"))], Some(FP));
    assert!(matches!(
        f.env.ingest.update_remote_source(view.source_id, far),
        Err(IngestError::InvalidInput("pathMap"))
    ));
    assert_eq!(
        f.env.ingest.list_remote_sources().unwrap()[0].path_map[0].to,
        f.lib.to_string_lossy()
    );
    assert!(matches!(
        f.env.ingest.delete_remote_source(view.source_id),
        Err(IngestError::Busy)
    ));
    // 비밀을 비워 두고 수정하면 저장된 비밀이 유지되어 접속 시험이 계속 된다
    let mut keep = input(vec![("/", f.lib.clone())], Some(FP));
    keep.secret = None;
    keep.port = 2200;
    assert_eq!(
        f.env
            .ingest
            .update_remote_source(view.source_id, keep)
            .unwrap()
            .port,
        2200
    );
    // 수정은 도는 수집기를 재접속시킨다
    f.wait_baseline();
    let n = f.conn.connects();
    let mut renamed = input(vec![("/", f.lib.clone())], Some(FP));
    renamed.secret = None;
    renamed.poll_sec = 2;
    f.env
        .ingest
        .update_remote_source(view.source_id, renamed)
        .unwrap();
    assert!(
        wait_until(Duration::from_secs(20), || f.conn.connects() > n),
        "reload reconnects with the new settings"
    );
    // 폴더가 떠나면 삭제된다
    f.env
        .ingest
        .set_folder_watch_mode(folder, None, None)
        .unwrap();
    f.env.ingest.delete_remote_source(view.source_id).unwrap();
    assert!(f.env.ingest.list_remote_sources().unwrap().is_empty());
    assert!(matches!(
        f.env.ingest.delete_remote_source(view.source_id),
        Err(IngestError::NotFound)
    ));
}

#[test]
fn t_ddp_17_pending_rows_and_the_cursor_are_one_transaction_a_failure_between_rolls_both_back() {
    let f = fx();
    let (folder, src) = (f.folder(), f.source());
    let committer = DbCommitter::new(Arc::clone(&f.env.pool), f.env.clock.clone(), 2_000, 50_000);
    let item = (folder, f.lib.join("a.mov"));
    // 성공: 둘 다 반영
    committer
        .commit(
            src,
            std::slice::from_ref(&item),
            r#"{"offsets":{"audit-log.0":10}}"#,
        )
        .unwrap();
    assert_eq!((f.pending(), f.cursor_rows()), (1, 1));
    assert_eq!(
        committer.load_cursor(src).as_deref(),
        Some(r#"{"offsets":{"audit-log.0":10}}"#)
    );
    // 실패 주입: 대기열 UPSERT 뒤에 커서 저장이 외래 키 위반으로 실패한다 (없는 접속 ID) -> 새 행도 롤백
    let other = (folder, f.lib.join("b.mov"));
    assert!(
        committer
            .commit(4242, std::slice::from_ref(&other), r#"{"offsets":{}}"#)
            .is_err()
    );
    assert_eq!(
        f.pending(),
        1,
        "the new pending row was rolled back with the cursor"
    );
    assert_eq!(
        f.env
            .count("SELECT COUNT(*) FROM watch_pending WHERE abs_path LIKE '%b.mov'"),
        0
    );
    assert_eq!(
        committer.load_cursor(src).as_deref(),
        Some(r#"{"offsets":{"audit-log.0":10}}"#),
        "cursor unchanged"
    );
    assert_eq!(committer.load_cursor(4242), None);
}

#[test]
fn t_ddp_18_an_unconfirmed_host_key_falls_back_with_a_reason_and_a_mismatching_one_degrades_without_reading_anything()
 {
    let f = fx();
    let folder = f.folder();
    // 지문 미확인 접속: 저장은 되지만 감시는 폴백 + 사유 키
    let unconfirmed = f
        .env
        .ingest
        .create_remote_source(input(vec![("/", f.lib.clone())], None))
        .unwrap()
        .source_id;
    f.watch(folder, unconfirmed).unwrap();
    let v = f.view(folder);
    assert_ne!(
        v["watchActive"], "ddp_audit",
        "falls back to the automatic detector"
    );
    assert_eq!(v["watchFallbackKey"], "remote_host_key_unconfirmed");
    assert_eq!(f.conn.connects(), 0);
    // 지문 불일치: 감시는 시작되고 `Degraded`(호스트 키 사유), 서버에는 명령이 가지 않는다
    f.env
        .ingest
        .update_remote_source(
            unconfirmed,
            RemoteSourceInput {
                secret: None,
                ..input(vec![("/", f.lib.clone())], Some(OTHER_FP))
            },
        )
        .unwrap();
    f.env
        .ingest
        .set_folder_watch_mode(
            folder,
            Some("ddp_audit".into()),
            Some(json!({"source": unconfirmed})),
        )
        .unwrap();
    assert!(wait_until(Duration::from_secs(20), || {
        let v = f.view(folder);
        v["watchActive"] == "ddp_audit"
            && v["watchHealth"]["reasonKey"] == "watch.health.remote_host_key"
    }));
    assert_eq!(f.ddp.calls(), 0);
    assert_eq!(f.pending(), 0);
    assert!(matches!(
        f.env.ingest.list_folders().unwrap()[0].watch_health,
        Some(SourceHealth::Degraded { .. })
    ));
}

#[test]
fn t_ddp_19_secrets_are_not_in_the_database_in_plain_form_nor_in_any_view() {
    let f = fx();
    f.source();
    let blob: Vec<u8> = f.env.read(|c| {
        c.query_row("SELECT auth_blob FROM remote_sources", [], |r| r.get(0))
            .unwrap()
    });
    assert!(!blob.windows(14).any(|w| w == b"pw-never-leaks"));
    let all = serde_json::to_string(&f.env.ingest.list_remote_sources().unwrap()).unwrap();
    assert!(!all.contains("pw-never-leaks"));
}

#[test]
fn t_ddp_20_a_5000_path_batch_is_chunked_all_rows_land_and_the_cursor_is_written_only_with_the_last_chunk()
 {
    let f = fx();
    let (folder, src) = (f.folder(), f.source());
    let committer = DbCommitter::new(Arc::clone(&f.env.pool), f.env.clock.clone(), 2_000, 50_000);
    let items: Vec<_> = (0..5_000)
        .map(|i| (folder, f.lib.join(format!("f{i}.mov"))))
        .collect();
    committer
        .commit(src, &items, r#"{"offsets":{"audit-log.0":1}}"#)
        .unwrap();
    assert_eq!(f.pending(), 5_000);
    assert_eq!(
        committer.load_cursor(src).as_deref(),
        Some(r#"{"offsets":{"audit-log.0":1}}"#)
    );
    // 마지막 청크(커서 저장)가 외래 키 위반으로 실패한다 — 앞 청크 행은 남아도 되지만 커서는 생기지 않는다
    let more: Vec<_> = (0..5_000)
        .map(|i| (folder, f.lib.join(format!("g{i}.mov"))))
        .collect();
    assert!(committer.commit(4242, &more, r#"{"offsets":{}}"#).is_err());
    assert_eq!(committer.load_cursor(4242), None);
    assert_eq!(
        committer.load_cursor(src).as_deref(),
        Some(r#"{"offsets":{"audit-log.0":1}}"#),
        "existing cursor untouched"
    );
    // 같은 묶음을 다시 쓰면(원격이 같은 줄을 다시 읽는다) 행은 합쳐지고 커서가 오른다
    committer
        .commit(src, &more, r#"{"offsets":{"audit-log.0":2}}"#)
        .unwrap();
    assert_eq!(f.pending(), 10_000);
    assert_eq!(
        committer.load_cursor(src).as_deref(),
        Some(r#"{"offsets":{"audit-log.0":2}}"#)
    );
}
