//! 제거·동시성 시험 — 분류: Happy / Business rule(데이터 보존·삭제 순서) / Concurrency(진행 중 호출·Busy) / Boundary(배치 크기).
mod common_rt;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common_rt::{EN, Rt, file_list, pid_alive};
use mam_plugin_host::HostState;
use mam_plugin_runtime::runtime::TestKnobs;
use mam_plugin_runtime::{ChangeReason, InstallError, Permission, ProviderInvoker, RunError};

const ID: &str = "com.example.basic";
const OTHER: &str = "com.example.other";

fn two_plugins() -> (Rt, std::path::PathBuf, std::path::PathBuf) {
    let rt = Rt::new();
    let (basic, other) = (rt.fixture("basic", "basic"), rt.fixture("other", "other"));
    rt.install_basic(&basic);
    rt.rt
        .install(&other, OTHER, "0.5.0", vec!["kv.read".into()], EN)
        .unwrap();
    (rt, basic, other)
}

fn count_for(rt: &Rt, table: &str, plugin: &str) -> i64 {
    rt.count(&format!(
        "SELECT COUNT(*) FROM {table} JOIN plugins USING (plugin_ref) WHERE plugin_id = '{plugin}'"
    ))
}

#[test]
fn t_pinst_13_uninstall_without_keep_data_removes_everything_of_that_plugin_only() {
    // Arrange — 배치 크기 5 로 반복 경로를 탄다
    let (rt, _b, _o) = two_plugins();
    rt.rt.set_test_knobs(TestKnobs {
        delete_batch: Some(5),
        ..TestKnobs::default()
    });
    let (a1, a2) = (rt.add_asset("p/1.jpg"), rt.add_asset("p/2.jpg"));
    rt.seed_data(ID, 12, &[a1, a2]);
    rt.seed_data(OTHER, 3, &[a1]);
    rt.exec(&format!(
        "INSERT INTO settings (key, value) VALUES ('plugin_feed_cursor:{ID}', '42')"
    ));
    let host = rt.rt.host_for_test(ID).unwrap();

    // Act
    rt.rt.uninstall(ID, false).unwrap();

    // Assert — 설정 4종·패키지·KV·작업 행이 모두 사라지고 다른 플러그인은 그대로
    assert_eq!(
        rt.count(&format!(
            "SELECT COUNT(*) FROM settings WHERE key LIKE 'plugin\\_%:{ID}' ESCAPE '\\'"
        )),
        0
    );
    assert!(!rt.plugins_dir.join(ID).exists());
    assert_eq!(
        (
            count_for(&rt, "plugin_kv", ID),
            count_for(&rt, "plugin_tasks", ID)
        ),
        (0, 0)
    );
    assert_eq!(
        (
            count_for(&rt, "plugin_kv", OTHER),
            count_for(&rt, "plugin_tasks", OTHER)
        ),
        (3, 1)
    );
    assert_eq!(
        rt.rt
            .list(EN)
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        vec![OTHER]
    );
    assert_eq!(
        rt.rt
            .run_command(OTHER, "ping", vec![])
            .unwrap()
            .message
            .as_deref(),
        Some("pong")
    );
    assert!(rt.plugins_dir.join(OTHER).join("0.5.0").exists());
    assert!(matches!(
        rt.rt.run_command(ID, "echo", vec![]),
        Err(RunError::NotInstalled)
    ));
    assert!(!rt.rt.allows(ID, Permission::KvRead) && host.state() == HostState::Idle);
    assert_eq!(
        rt.events().last(),
        Some(&(ID.to_owned(), ChangeReason::Uninstalled))
    );
    // 다시 제거하거나 잘못된 id 면 NotInstalled
    assert!(matches!(
        rt.rt.uninstall(ID, false),
        Err(InstallError::NotInstalled)
    ));
    assert!(matches!(
        rt.rt.uninstall("a:b", false),
        Err(InstallError::NotInstalled)
    ));
    assert!(matches!(
        rt.rt.uninstall("", true),
        Err(InstallError::NotInstalled)
    ));
}

#[test]
fn t_pinst_14_keep_data_leaves_rows_and_reinstall_starts_with_a_fresh_cursor() {
    // Arrange
    let (rt, basic, _o) = two_plugins();
    let a1 = rt.add_asset("p/1.jpg");
    rt.seed_data(ID, 4, &[a1]);
    rt.exec(&format!(
        "INSERT INTO settings (key, value) VALUES ('plugin_feed_cursor:{ID}', '42')"
    ));

    // Act
    rt.rt.uninstall(ID, true).unwrap();

    // Assert — 행은 남고 설정(커서 포함)은 지워진다
    assert_eq!(
        (
            count_for(&rt, "plugin_kv", ID),
            count_for(&rt, "plugin_tasks", ID)
        ),
        (4, 1)
    );
    assert_eq!(
        rt.count(&format!(
            "SELECT COUNT(*) FROM settings WHERE key LIKE 'plugin\\_%:{ID}' ESCAPE '\\'"
        )),
        0
    );
    assert!(!rt.plugins_dir.join(ID).exists());

    // Act — 재설치
    rt.install_basic(&basic);

    // Assert — fresh 로 알려지고(커서 없음) 데이터가 다시 보인다
    assert_eq!(
        rt.hooks.installed.lock().unwrap().last(),
        Some(&(ID.to_owned(), true))
    );
    assert_eq!(rt.setting(&format!("plugin_feed_cursor:{ID}")), None);
    assert_eq!(count_for(&rt, "plugin_kv", ID), 4);
}

#[test]
fn t_pref_04_keep_data_reinstall_reuses_the_registry_row_and_the_string_api_sees_the_kept_kv() {
    // Arrange
    let (rt, basic, _o) = two_plugins();
    let a1 = rt.add_asset("p/1.jpg");
    rt.seed_data(ID, 4, &[a1]);
    let ref_of = |rt: &Rt| {
        rt.count(&format!(
            "SELECT plugin_ref FROM plugins WHERE plugin_id = '{ID}'"
        ))
    };
    let before = ref_of(&rt);
    let read = |rt: &Rt| {
        rt.pool
            .read(|c| Ok(mam_db::plugin_kv::get(c, ID, None, "k2")?))
            .unwrap()
    };
    assert_eq!(read(&rt).as_deref(), Some("1"));

    // Act
    rt.rt.uninstall(ID, true).unwrap();
    let kept_ref = ref_of(&rt);
    rt.install_basic(&basic);

    // Assert — 레지스트리 행은 지워지지 않고 같은 ref 가 재사용되며, 문자열 API 로 보존된 KV 가 다시 보인다
    assert_eq!((kept_ref, ref_of(&rt)), (before, before));
    assert_eq!(
        rt.count(&format!(
            "SELECT COUNT(*) FROM plugins WHERE plugin_id = '{ID}'"
        )),
        1
    );
    assert_eq!(read(&rt).as_deref(), Some("1"));
}

#[test]
fn t_pinst_15_uninstall_waits_for_in_flight_call_and_leaves_no_worker() {
    // Arrange — 워커를 먼저 띄워 PID 를 잡고, 1.2초 걸리는 명령을 다른 스레드에서 시작한다
    let (rt, _b, _o) = two_plugins();
    rt.rt.run_command(ID, "assets", vec![]).unwrap();
    let HostState::Running { pid } = rt.rt.host_for_test(ID).unwrap().state() else {
        panic!("worker should be running")
    };
    let host = rt.rt.host_for_test(ID).unwrap();
    let before = host.activity().invokes_started;
    let runtime = Arc::clone(&rt.rt);
    let call = std::thread::spawn(move || runtime.run_command(ID, "slow", vec![]));
    // 호출이 호스트 감독자 락을 쥘 때까지 조건 변수로 기다린다 (고정 sleep 없음)
    assert!(
        host.wait_activity(Duration::from_secs(30), |a| a.invokes_started > before),
        "slow call never entered the host"
    );

    // Act
    let t = Instant::now();
    rt.rt.uninstall(ID, false).unwrap();
    let waited = t.elapsed();

    // Assert — 진행 중 호출은 끝까지 성공하고, 제거는 그 호출을 기다렸으며, 워커 프로세스는 남지 않는다
    assert_eq!(
        call.join().unwrap().unwrap().message.as_deref(),
        Some("slow done")
    );
    assert!(
        waited >= Duration::from_millis(300),
        "uninstall returned too early: {waited:?}"
    );
    assert!(!pid_alive(pid));
    assert!(matches!(
        rt.rt.run_command(ID, "echo", vec![]),
        Err(RunError::NotInstalled)
    ));
}

#[test]
fn t_pinst_21_concurrent_lifecycle_operations_report_busy_without_torn_state() {
    // Arrange — A: 느린 호출(1.2초) 진행 중, B: 그 플러그인 제거(호출을 기다리며 lifecycle 락을 쥔다)
    let (rt, _b, _o) = two_plugins();
    let third = rt.fixture("other", "third");
    rt.patch(&third, "plugin.json", |s| {
        s.replace("com.example.other", "com.example.third")
    });
    rt.rt.run_command(ID, "assets", vec![]).unwrap();
    let results = Arc::new(Mutex::new(Vec::new()));
    let (r1, r2) = (Arc::clone(&rt.rt), Arc::clone(&rt.rt));
    let host = rt.rt.host_for_test(ID).unwrap();
    let before = host.activity();
    let call = std::thread::spawn(move || r1.run_command(ID, "slow", vec![]));
    // A 가 호스트 락을 쥘 때까지, 그리고 B(제거)가 lifecycle 락을 쥔 채 호스트 종료를 요청할 때까지 조건 변수로 기다린다 (고정 sleep 없음)
    assert!(
        host.wait_activity(Duration::from_secs(30), |a| a.invokes_started
            > before.invokes_started),
        "slow call never entered the host"
    );
    let res = Arc::clone(&results);
    let uninstall = std::thread::spawn(move || res.lock().unwrap().push(r2.uninstall(ID, false)));
    assert!(
        host.wait_activity(Duration::from_secs(30), |a| a.shutdowns_requested
            > before.shutdowns_requested),
        "uninstall never reached host shutdown"
    );

    // Act — B 가 lifecycle 락을 쥐고 A 의 호출 종료를 기다리는 동안 다른 모든 수명주기 작업은 Busy
    let install = rt
        .rt
        .install(&third, "com.example.third", "0.5.0", vec![], EN);
    let enable = rt.rt.set_enabled(OTHER, false, EN);
    let grant = rt.rt.grant_permissions(OTHER, vec![], EN);
    let other_uninstall = rt.rt.uninstall(OTHER, true);

    // Assert
    assert!(matches!(install, Err(InstallError::Busy)), "{install:?}");
    assert!(matches!(enable, Err(InstallError::Busy)));
    assert!(matches!(grant, Err(InstallError::Busy)));
    assert!(matches!(other_uninstall, Err(InstallError::Busy)));
    call.join().unwrap().unwrap();
    uninstall.join().unwrap();
    assert!(results.lock().unwrap().iter().all(Result::is_ok));
    // 찢어진 상태 없음 — Busy 였던 작업은 아무것도 쓰지 않았고, 제거는 완전히 끝났다
    assert!(!rt.plugins_dir.join("com.example.third").exists());
    assert_eq!(rt.setting("plugin_installed:com.example.third"), None);
    assert!(rt.rt.list(EN).iter().all(|i| i.id == OTHER && i.enabled));
    assert_eq!(rt.setting(&format!("plugin_installed:{ID}")), None);
    assert_eq!(
        file_list(&rt.plugins_dir.join(".staging")),
        Vec::<String>::new()
    );
}

#[test]
fn t_pinst_21b_racing_installs_end_in_a_consistent_state() {
    // Arrange
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    let runtime = &rt.rt;

    // Act — 4 스레드가 같은 플러그인을 동시에 설치한다
    let results: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                s.spawn(|| {
                    runtime
                        .install(&dir, ID, "1.0.0", common_rt::basic_perms(), EN)
                        .map(|i| i.version)
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // Assert — 성공 또는 Busy 뿐, 적어도 하나는 성공, 최종 상태는 온전하다
    assert!(
        results
            .iter()
            .all(|r| matches!(r, Ok(_) | Err(InstallError::Busy))),
        "{results:?}"
    );
    assert!(results.iter().any(Result::is_ok));
    assert_eq!(rt.rt.list(EN).len(), 1);
    assert_eq!(file_list(&rt.plugins_dir.join(ID)).len(), 4);
    assert!(
        file_list(&rt.plugins_dir.join(".staging")).is_empty()
            && file_list(&rt.plugins_dir.join(".trash")).is_empty()
    );
    assert_eq!(
        rt.rt
            .run_command(ID, "echo", vec![1])
            .unwrap()
            .message
            .as_deref(),
        Some("done 1 assets")
    );
}
