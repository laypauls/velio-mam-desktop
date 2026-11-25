//! 실행 경로 시험 — 분류: Happy / Validation(범위·대상) / Business rule(비활성·권한 철회·자동 비활성) / External(워커 결함) / Time(호출 타임아웃).
mod common_rt;

use std::sync::Arc;
use std::time::Duration;

use common_rt::{EN, Rt, pid_alive};
use mam_plugin_host::HostState;
use mam_plugin_runtime::runtime::TestKnobs;
use mam_plugin_runtime::{ChangeReason, InvokeError, Permission, ProviderInvoker, ProviderKind, RunError};
use serde_json::json;

const ID: &str = "com.example.basic";

fn installed() -> (Rt, std::path::PathBuf) {
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.install_basic(&dir);
    (rt, dir)
}

fn msg(rt: &Rt, cmd: &str, assets: Vec<i64>) -> Option<String> {
    rt.rt.run_command(ID, cmd, assets).unwrap().message
}

#[test]
fn t_pinst_11_disable_closes_gateway_at_once_and_kills_worker_enable_resets_breaker() {
    // Arrange — 호출로 워커를 띄우고(게이트 ON: 활성 + kv.write 허용) PID 를 잡는다
    let (rt, _dir) = installed();
    assert_eq!(msg(&rt, "setkv", vec![]).as_deref(), Some("written"));
    let host = rt.rt.host_for_test(ID).unwrap();
    let HostState::Running { pid } = host.state() else { panic!("worker should be running") };

    // Act
    let info = rt.rt.set_enabled(ID, false, EN).unwrap();

    // Assert — 즉시 권한 닫힘, 워커 PID 없음, 호출은 Disabled, 저장됨
    assert!(!info.enabled);
    assert!(!rt.rt.allows(ID, Permission::KvWrite));
    assert!(!pid_alive(pid));
    assert!(matches!(rt.rt.run_command(ID, "setkv", vec![]), Err(RunError::Disabled)));
    assert_eq!(rt.setting(&format!("plugin_enabled:{ID}")).as_deref(), Some("false"));
    assert!(rt.rt.contributions(EN).menu_items.is_empty() && rt.rt.auto_providers().is_empty());
    assert!(rt.hooks.enabled.lock().unwrap().is_empty());

    // Act — 켜면 훅이 불리고 호출이 다시 된다
    let on = rt.rt.set_enabled(ID, true, EN).unwrap();

    // Assert
    assert!(on.enabled && rt.rt.allows(ID, Permission::KvWrite));
    assert_eq!(*rt.hooks.enabled.lock().unwrap(), vec![ID.to_owned()]);
    assert_eq!(msg(&rt, "setkv", vec![]).as_deref(), Some("written"));
    assert_eq!(rt.events().iter().map(|e| e.1).collect::<Vec<_>>(), vec![ChangeReason::Installed, ChangeReason::Disabled, ChangeReason::Enabled]);
    assert!(matches!(rt.rt.set_enabled("com.example.missing", true, EN), Err(mam_plugin_runtime::InstallError::NotInstalled)));
    assert!(matches!(rt.rt.set_enabled("a:b", true, EN), Err(mam_plugin_runtime::InstallError::NotInstalled)));
}

#[test]
fn t_pinst_12_grant_revoke_is_effective_on_the_next_call() {
    // Arrange
    let (rt, _dir) = installed();
    assert_eq!(msg(&rt, "setkv", vec![]).as_deref(), Some("written"));

    // Act — kv.write 를 거둔다 (나머지 권한과 활성은 그대로 — 게이트 ON)
    let info = rt.rt.grant_permissions(ID, vec!["asset.read.metadata".into()], EN).unwrap();

    // Assert
    assert_eq!(info.granted, vec![Permission::AssetReadMetadata]);
    assert_eq!(msg(&rt, "setkv", vec![]).as_deref(), Some("permission_denied"));
    assert_eq!(rt.setting(&format!("plugin_granted:{ID}")).as_deref(), Some(r#"["asset.read.metadata"]"#));
    assert!(rt.events().iter().any(|e| e.1 == ChangeReason::PermissionsChanged));
    // 다시 허용하면 열린다
    rt.rt.grant_permissions(ID, vec!["kv.write".into(), "asset.read.metadata".into()], EN).unwrap();
    assert_eq!(msg(&rt, "setkv", vec![]).as_deref(), Some("written"));
}

#[test]
fn t_pinst_12b_grant_validation_rejects_unknown_undeclared_and_unknown_plugin() {
    // Arrange
    let (rt, _dir) = installed();
    let before = rt.setting(&format!("plugin_granted:{ID}"));
    let granted = |p: Vec<&str>| rt.rt.grant_permissions(ID, p.into_iter().map(String::from).collect(), EN);

    // Act / Assert
    assert!(matches!(granted(vec!["bogus"]), Err(mam_plugin_runtime::InstallError::UnknownPermission)));
    assert!(matches!(granted(vec!["kv.read", "asset.read.derived"]), Err(mam_plugin_runtime::InstallError::UnknownPermission))); // 선언하지 않음
    assert!(matches!(rt.rt.grant_permissions("com.example.nope", vec![], EN), Err(mam_plugin_runtime::InstallError::NotInstalled)));
    assert!(matches!(rt.rt.grant_permissions("x:y", vec![], EN), Err(mam_plugin_runtime::InstallError::NotInstalled)));
    assert_eq!(rt.setting(&format!("plugin_granted:{ID}")), before);
    assert!(granted(vec![]).unwrap().granted.is_empty()); // 모두 거두기
}

#[test]
fn t_pinst_16_run_command_happy_and_scope_is_the_selection() {
    // Arrange
    let (rt, _dir) = installed();
    let (a, b) = (rt.add_asset("pics/a.jpg"), rt.add_asset("pics/b.jpg"));

    // Act / Assert — 선택한 자산은 읽힌다
    assert_eq!(msg(&rt, "asset", vec![a]).as_deref(), Some("a.jpg"));
    // 선택하지 않은 자산(존재하는 b)은 범위 밖 — 없는 자산과 같은 not_found
    assert_eq!(msg(&rt, "neighbor", vec![a]).as_deref(), Some("not_found"));
    assert_eq!(msg(&rt, "neighbor", vec![a, b]).as_deref(), Some("visible"));
    // 중복 제거, 제어 문자 제거
    assert_eq!(msg(&rt, "echo", vec![b, a, b]).as_deref(), Some("done 2 assets"));
    // 문자열이 아닌 결과는 메시지 없음
    assert_eq!(msg(&rt, "assets", vec![a]), None);
}

#[test]
fn t_pinst_17_run_command_validation() {
    // Arrange
    let (rt, _dir) = installed();
    let run = |cmd: &str, n: usize| rt.rt.run_command(ID, cmd, (1..=n as i64).collect());

    // Act / Assert — 경계: 10,000 은 되고 10,001 은 안 된다. 중복은 제거한 뒤 센다
    assert!(run("assets", 10_000).is_ok());
    assert!(matches!(run("assets", 10_001), Err(RunError::Scope(_))));
    assert!(rt.rt.run_command(ID, "assets", (0..20_000).map(|i| i % 3).collect()).is_ok());
    assert!(matches!(run("nope", 1), Err(RunError::UnknownTarget)));
    assert!(matches!(rt.rt.run_command("com.example.none", "echo", vec![]), Err(RunError::NotInstalled)));
    assert!(matches!(rt.rt.run_command("a:b", "echo", vec![]), Err(RunError::NotInstalled)));
    // 꺼져 있으면 대상 검사보다 먼저 Disabled
    rt.rt.set_enabled(ID, false, EN).unwrap();
    assert!(matches!(run("echo", 1), Err(RunError::Disabled)));
    assert!(matches!(run("nope", 1), Err(RunError::Disabled)));
}

#[test]
fn t_pinst_18_error_mapping_and_three_faults_auto_disable_persisted() {
    // Arrange — 호출 타임아웃을 줄이고 장기 대기를 피한다
    let (rt, _dir) = installed();
    rt.rt.set_test_knobs(TestKnobs { host_tweak: Some(Arc::new(|c| c.call_timeout = Duration::from_millis(400))), ..TestKnobs::default() });
    rt.install_basic(&rt.src_root.join("basic")); // 새 설정을 가진 호스트로 교체
    let events_before = rt.events().len();

    // Act / Assert — 스크립트 예외는 결함이 아니다(차단기에 안 쌓인다)
    for _ in 0..5 {
        assert!(matches!(rt.rt.run_command(ID, "throw", vec![]), Err(RunError::Invoke(InvokeError::Script(m))) if m == "boom"));
    }
    assert!(rt.rt.list(EN)[0].enabled);
    // 타임아웃 = 결함 1 (워커 종료 후 다음 호출에서 재시작)
    assert!(matches!(rt.rt.run_command(ID, "slow", vec![]), Err(RunError::Invoke(InvokeError::Timeout))));
    // 결함 2·3 — 3번째에서 차단기가 열려 자동 비활성
    assert!(matches!(rt.rt.run_command(ID, "flood", vec![]), Err(RunError::Invoke(InvokeError::Flood | InvokeError::Timeout))));
    assert!(rt.rt.list(EN)[0].enabled);
    assert!(matches!(rt.rt.run_command(ID, "flood", vec![]), Err(RunError::Invoke(_))));

    // Assert — 저장·권한 닫힘·알림
    assert_eq!(rt.setting(&format!("plugin_enabled:{ID}")).as_deref(), Some("false"));
    assert!(!rt.rt.list(EN)[0].enabled && !rt.rt.allows(ID, Permission::KvRead));
    let new_events: Vec<_> = rt.events()[events_before..].iter().map(|e| e.1).collect();
    assert_eq!(new_events.iter().filter(|r| **r == ChangeReason::AutoDisabled).count(), 1, "{new_events:?}");
    assert!(matches!(rt.rt.run_command(ID, "echo", vec![]), Err(RunError::Disabled)));
    // 부팅해도 꺼져 있다 (저장됐다)
    let mut rt = rt;
    rt.reboot(Some(common_rt::worker_exe()), "0.2.0");
    assert!(!rt.rt.list(EN)[0].enabled);
    // 사용자가 다시 켜면 차단기 이력이 지워져 두 번 더 실패해도 꺼지지 않는다
    rt.rt.set_enabled(ID, true, EN).unwrap();
    assert!(matches!(rt.rt.run_command(ID, "flood", vec![]), Err(RunError::Invoke(_))));
    assert!(matches!(rt.rt.run_command(ID, "flood", vec![]), Err(RunError::Invoke(_))));
    assert!(rt.rt.list(EN)[0].enabled);
}

#[test]
fn t_pinst_19_contributions_filter_order_locale_and_raw_when() {
    // Arrange
    let (rt, _dir) = installed();
    let other = rt.fixture("other", "other");
    rt.rt.install(&other, "com.example.other", "0.5.0", vec![], EN).unwrap();

    // Act
    let c = rt.rt.contributions("ko");

    // Assert — 플러그인 id 순 -> manifest 순서, 번역(현재 -> en -> id), `when` 원문
    let items: Vec<_> = c.menu_items.iter().map(|m| (m.plugin_id.as_str(), m.command_id.as_str(), m.title.as_str(), m.when.as_deref())).collect();
    assert_eq!(
        items,
        vec![
            (ID, "echo", "에코", Some("selection.count > 0 && asset.kind == 'image'")),
            (ID, "assets", "Assets", None),
            (ID, "throw", "throw", None),
            ("com.example.other", "ping", "ping", None),
        ]
    );
    assert_eq!(c.export_providers.len(), 1);
    let e = &c.export_providers[0];
    assert_eq!((e.plugin_id.as_str(), e.provider_id.as_str(), e.title.as_str(), e.file_extension.as_str()), (ID, "exp.csv", "Export CSV", "csv"));
    assert_eq!(e.asset_kinds, vec![mam_plugin_runtime::AssetKind::Image, mam_plugin_runtime::AssetKind::Video]);
    // 꺼진 플러그인은 빠진다
    rt.rt.set_enabled(ID, false, EN).unwrap();
    let off = rt.rt.contributions(EN);
    assert!(off.menu_items.iter().all(|m| m.plugin_id == "com.example.other") && off.export_providers.is_empty());
}

#[test]
fn t_pinst_24_one_host_per_plugin_shared_by_commands_and_providers() {
    // Arrange
    let (rt, dir) = installed();
    let host = rt.rt.host_for_test(ID).unwrap();

    // Act
    assert_eq!(msg(&rt, "echo", vec![1]).as_deref(), Some("done 1 assets"));
    let got = rt.rt.invoke_provider(ID, "meta.extract", vec![], json!({"x": 1})).unwrap();
    assert_eq!(msg(&rt, "echo", vec![1]).as_deref(), Some("done 1 assets"));

    // Assert — 워커는 한 번만 시작됐고 같은 호스트다
    assert_eq!(got["got"]["x"], 1, "{got}");
    assert_eq!(host.starts(), 1);
    assert!(Arc::ptr_eq(&host, &rt.rt.host_for_test(ID).unwrap()));
    // 재설치하면 새 호스트 1개로 교체되고 옛 호스트는 정리된다
    rt.install_basic(&dir);
    let fresh = rt.rt.host_for_test(ID).unwrap();
    assert!(!Arc::ptr_eq(&host, &fresh));
    assert_eq!((host.state(), fresh.starts()), (HostState::Idle, 0));
}

#[test]
fn t_pinst_25_provider_invoker_contract() {
    // Arrange
    let (rt, _dir) = installed();

    // Act
    let auto = rt.rt.auto_providers();
    let export = rt.rt.provider(ID, "exp.csv").unwrap();
    let out = rt.rt.invoke_provider(ID, "exp.csv", vec![], json!(null)).unwrap();

    // Assert — 자동 실행은 metadata·analysis 만
    let names: Vec<_> = auto.iter().map(|s| (s.provider.as_str(), s.kind)).collect();
    assert_eq!(names, vec![("meta.extract", ProviderKind::Metadata), ("ana.scan", ProviderKind::Analysis)]);
    assert_eq!(auto[0].asset_kinds, vec![mam_plugin_runtime::AssetKind::Image]);
    assert_eq!((export.kind, export.file_extension.as_deref()), (ProviderKind::Export, Some("csv")));
    assert_eq!(out["text"], "a,b");
    assert!(rt.rt.allows(ID, Permission::ExportWrite));
    assert!(matches!(rt.rt.provider(ID, "nope"), Err(RunError::UnknownTarget)));
    assert!(matches!(rt.rt.invoke_provider(ID, "nope", vec![], json!(null)), Err(RunError::UnknownTarget)));
    assert!(matches!(rt.rt.provider("com.example.none", "x"), Err(RunError::NotInstalled)));
    assert!(matches!(rt.rt.invoke_provider(ID, "meta.extract", (1..=10_001).collect(), json!(null)), Err(RunError::Scope(_))));
    // 꺼지면 provider 존재 여부와 상관없이 Disabled (대기 중 작업을 지우지 않는다)
    rt.rt.set_enabled(ID, false, EN).unwrap();
    assert!(matches!(rt.rt.provider(ID, "meta.extract"), Err(RunError::Disabled)));
    assert!(matches!(rt.rt.provider(ID, "nope"), Err(RunError::Disabled)));
    assert!(matches!(rt.rt.invoke_provider(ID, "meta.extract", vec![], json!(null)), Err(RunError::Disabled)));
    assert!(!rt.rt.allows(ID, Permission::ExportWrite));
}
