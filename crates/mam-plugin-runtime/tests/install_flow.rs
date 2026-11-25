//! 설치·업그레이드·재설치·다운그레이드 시험 — 분류: Happy / Validation / Business rule(fail-closed 권한) / External(디스크 실패).
mod common_rt;

use std::fs;

use common_rt::{EN, Rt, file_list, pid_alive, snapshot};
use mam_plugin_host::HostState;
use mam_plugin_runtime::{ChangeReason, InstallError, Permission, PluginStatus, ProviderInvoker};

const ID: &str = "com.example.basic";

fn all_perms() -> Vec<Permission> {
    vec![
        Permission::AssetReadMetadata,
        Permission::KvRead,
        Permission::KvWrite,
        Permission::TagWrite,
        Permission::ExportWrite,
    ]
}

#[test]
fn t_pinst_03_install_happy_copies_only_three_kinds_and_leaves_source_untouched() {
    // Arrange — 복사 대상이 아닌 파일을 원본에 섞어 둔다
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    fs::write(dir.join("README.txt"), "readme").unwrap();
    fs::write(dir.join("dist").join("extra.js"), "// not main").unwrap();
    fs::write(dir.join("l10n").join("notes.txt"), "notes").unwrap();
    let before = snapshot(&dir);

    // Act
    let info = rt.install_basic(&dir);

    // Assert — 응답
    assert_eq!(
        (
            info.id.as_str(),
            info.version.as_str(),
            info.status,
            info.enabled
        ),
        (ID, "1.0.0", PluginStatus::Ok, true)
    );
    assert_eq!(info.granted, all_perms());
    assert_eq!(info.installed_at, common_rt::T0);
    assert!(info.worker_available && info.unsigned);
    // 복사된 파일은 3종류뿐
    let installed = rt.plugins_dir.join(ID).join("1.0.0");
    assert_eq!(
        file_list(&installed),
        vec![
            "dist/main.js",
            "l10n/en.json",
            "l10n/ko.json",
            "plugin.json"
        ]
    );
    // 원본 보호 — 내용·수정 시각 불변
    assert_eq!(snapshot(&dir), before);
    // 설정 3행
    let installed_json: serde_json::Value =
        serde_json::from_str(&rt.setting(&format!("plugin_installed:{ID}")).unwrap()).unwrap();
    assert_eq!(
        (
            installed_json["version"].as_str(),
            installed_json["installedAt"].as_i64()
        ),
        (Some("1.0.0"), Some(common_rt::T0))
    );
    assert_eq!(
        rt.setting(&format!("plugin_enabled:{ID}")).as_deref(),
        Some("true")
    );
    assert_eq!(
        rt.setting(&format!("plugin_granted:{ID}")).as_deref(),
        Some(r#"["asset.read.metadata","export.write","kv.read","kv.write","tag.write"]"#)
    );
    // 유효 권한 = manifest ∩ granted, 선언하지 않은 권한은 없다. 호스트는 첫 호출 전까지 Idle
    assert!(
        rt.rt.allows(ID, Permission::KvWrite) && !rt.rt.allows(ID, Permission::AssetReadDerived)
    );
    let host = rt.rt.host_for_test(ID).unwrap();
    assert_eq!((host.state(), host.starts()), (HostState::Idle, 0));
    // 훅·알림
    assert_eq!(
        *rt.hooks.installed.lock().unwrap(),
        vec![(ID.to_owned(), true)]
    );
    assert_eq!(rt.events(), vec![(ID.to_owned(), ChangeReason::Installed)]);
    // 스테이징은 남지 않는다
    assert!(file_list(&rt.plugins_dir.join(".staging")).is_empty());
}

#[test]
fn t_pinst_04_toctou_mismatch_changes_nothing() {
    // Arrange — 미리보기 뒤에 폴더 내용이 바뀐 상황
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    let preview = rt.rt.inspect_dir(&dir, EN).unwrap();
    rt.patch(&dir, "plugin.json", |s| {
        s.replace("\"version\": \"1.0.0\"", "\"version\": \"1.0.1\"")
    });

    // Act
    let by_version = rt
        .rt
        .install(&dir, &preview.id, &preview.version, vec![], EN);
    let by_id = rt
        .rt
        .install(&dir, "com.example.other", "1.0.1", vec![], EN);

    // Assert
    assert!(matches!(by_version, Err(InstallError::PreviewMismatch)));
    assert!(matches!(by_id, Err(InstallError::PreviewMismatch)));
    assert!(rt.rt.list(EN).is_empty());
    assert_eq!(
        rt.count("SELECT COUNT(*) FROM settings WHERE key LIKE 'plugin_%'"),
        0
    );
    assert!(file_list(&rt.plugins_dir).is_empty());
    assert!(rt.hooks.installed.lock().unwrap().is_empty() && rt.events().is_empty());
}

#[test]
fn t_pinst_05_granted_must_be_known_and_declared() {
    // Arrange — `other` 는 kv.read 만 선언한다
    let rt = Rt::new();
    let dir = rt.fixture("other", "other");
    let try_install = |granted: Vec<&str>| {
        rt.rt.install(
            &dir,
            "com.example.other",
            "0.5.0",
            granted.into_iter().map(String::from).collect(),
            EN,
        )
    };

    // Act / Assert — 선언하지 않은 권한, 모르는 문자열, 대소문자 다른 문자열은 모두 거부하고 아무것도 남기지 않는다
    for bad in [
        vec!["kv.write"],
        vec!["bogus.perm"],
        vec!["kv.read", "export.write"],
        vec!["KV.READ"],
        vec![""],
    ] {
        assert!(
            matches!(
                try_install(bad.clone()),
                Err(InstallError::UnknownPermission)
            ),
            "{bad:?}"
        );
    }
    assert!(rt.rt.list(EN).is_empty());
    assert_eq!(
        rt.count("SELECT COUNT(*) FROM settings WHERE key LIKE 'plugin_%'"),
        0
    );
    assert!(file_list(&rt.plugins_dir).is_empty());
    // 중복은 집합으로 합쳐 저장하고, 빈 목록도 허용한다(아무 권한도 주지 않음)
    let info = try_install(vec!["kv.read", "kv.read"]).unwrap();
    assert_eq!(info.granted, vec![Permission::KvRead]);
    let none = rt
        .rt
        .install(&dir, "com.example.other", "0.5.0", vec![], EN)
        .unwrap();
    assert!(none.granted.is_empty() && !rt.rt.allows("com.example.other", Permission::KvRead));
}

#[test]
fn t_pinst_06_upgrade_with_same_perms_shuts_old_host_and_removes_old_version() {
    // Arrange — v1 설치 후 워커를 띄운다
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.install_basic(&dir);
    let msg = |rt: &Rt| {
        rt.rt
            .run_command(ID, "echo", vec![1, 2, 3])
            .unwrap()
            .message
    };
    assert_eq!(msg(&rt).as_deref(), Some("done 3 assets"));
    let old_host = rt.rt.host_for_test(ID).unwrap();
    let HostState::Running { pid } = old_host.state() else {
        panic!("worker should be running")
    };
    rt.patch(&dir, "plugin.json", |s| {
        s.replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
    });
    rt.patch(&dir, "dist/main.js", |s| {
        s.replace("'done\\u0007 '", "'v2\\u0007 '")
    });

    // Act
    let info = rt
        .rt
        .install(&dir, ID, "1.1.0", common_rt::basic_perms(), EN)
        .unwrap();

    // Assert
    assert!(!pid_alive(pid), "old worker must be gone");
    assert_eq!(old_host.state(), HostState::Idle);
    assert!(
        !rt.plugins_dir.join(ID).join("1.0.0").exists()
            && rt.plugins_dir.join(ID).join("1.1.0").exists()
    );
    assert_eq!(
        (info.version.as_str(), info.granted),
        ("1.1.0", all_perms())
    );
    assert_eq!(rt.rt.host_for_test(ID).unwrap().starts(), 0); // 새 호스트 — 첫 호출 전
    assert_eq!(msg(&rt).as_deref(), Some("v2 3 assets"));
    assert_eq!(
        *rt.hooks.installed.lock().unwrap(),
        vec![(ID.to_owned(), true), (ID.to_owned(), false)]
    );
    assert_eq!(
        rt.events(),
        vec![
            (ID.to_owned(), ChangeReason::Installed),
            (ID.to_owned(), ChangeReason::Upgraded)
        ]
    );
    assert_eq!(
        rt.count("SELECT COUNT(*) FROM settings WHERE key LIKE 'plugin_installed:%'"),
        1
    );
}

#[test]
fn t_pinst_07_upgrade_with_added_permission_stays_denied_unless_granted() {
    // Arrange — v0.5.0 은 kv.read 만 선언·허용
    let rt = Rt::new();
    let id = "com.example.other";
    let dir = rt.fixture("other", "other");
    rt.rt
        .install(&dir, id, "0.5.0", vec!["kv.read".into()], EN)
        .unwrap();
    rt.patch(&dir, "plugin.json", |s| {
        s.replace("0.5.0", "0.6.0")
            .replace("[\"kv.read\"]", "[\"kv.read\", \"kv.write\"]")
    });
    let preview = rt.rt.inspect_dir(&dir, EN).unwrap();

    // Act — UI 가 다시 묻지 않고 기존 허용 그대로 업그레이드
    let kept = rt
        .rt
        .install(&dir, id, "0.6.0", vec!["kv.read".into()], EN)
        .unwrap();

    // Assert — 추가된 권한은 fail-closed
    assert_eq!(preview.added_permissions, vec![Permission::KvWrite]);
    assert_eq!(
        kept.permissions,
        vec![Permission::KvRead, Permission::KvWrite]
    );
    assert_eq!(kept.granted, vec![Permission::KvRead]);
    assert!(rt.rt.allows(id, Permission::KvRead) && !rt.rt.allows(id, Permission::KvWrite));
    // 사용자가 허용하면 다음부터 열린다
    let granted = rt
        .rt
        .install(
            &dir,
            id,
            "0.6.0",
            vec!["kv.read".into(), "kv.write".into()],
            EN,
        )
        .unwrap();
    assert_eq!(
        granted.granted,
        vec![Permission::KvRead, Permission::KvWrite]
    );
    assert!(rt.rt.allows(id, Permission::KvWrite));
}

#[test]
fn t_pinst_08_reinstall_replaces_content_and_downgrade_removes_newer_version() {
    // Arrange
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.install_basic(&dir);
    rt.patch(&dir, "dist/main.js", |s| {
        s.replace("'done\\u0007 '", "'again\\u0007 '")
    });

    // Act — 같은 버전 재설치
    let again = rt
        .rt
        .install(&dir, ID, "1.0.0", common_rt::basic_perms(), EN)
        .unwrap();

    // Assert — 새 내용이 적용되고 옛 복사본·휴지통은 남지 않는다
    assert_eq!(again.version, "1.0.0");
    assert_eq!(
        rt.rt
            .run_command(ID, "echo", vec![1])
            .unwrap()
            .message
            .as_deref(),
        Some("again 1 assets")
    );
    assert!(file_list(&rt.plugins_dir.join(".trash")).is_empty());
    assert_eq!(file_list(&rt.plugins_dir.join(ID)).len(), 4);
    assert_eq!(
        rt.hooks.installed.lock().unwrap().last(),
        Some(&(ID.to_owned(), false))
    );

    // Act — 낮은 버전으로
    rt.patch(&dir, "plugin.json", |s| {
        s.replace("\"version\": \"1.0.0\"", "\"version\": \"0.9.0\"")
    });
    let down = rt
        .rt
        .install(&dir, ID, "0.9.0", common_rt::basic_perms(), EN)
        .unwrap();

    // Assert
    assert_eq!(down.version, "0.9.0");
    assert!(
        rt.plugins_dir.join(ID).join("0.9.0").exists()
            && !rt.plugins_dir.join(ID).join("1.0.0").exists()
    );
}

#[test]
fn t_pinst_09_failed_staging_keeps_previous_version_and_settings() {
    // Arrange — v1 설치 뒤 `.staging` 자리를 파일로 막아 스테이징을 실패시킨다
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.install_basic(&dir);
    fs::remove_dir_all(rt.plugins_dir.join(".staging")).ok();
    fs::write(rt.plugins_dir.join(".staging"), "blocker").unwrap();
    rt.patch(&dir, "plugin.json", |s| {
        s.replace("\"version\": \"1.0.0\"", "\"version\": \"1.1.0\"")
    });

    // Act
    let r = rt
        .rt
        .install(&dir, ID, "1.1.0", common_rt::basic_perms(), EN);

    // Assert — 이전 버전 그대로 동작
    assert!(matches!(r, Err(InstallError::Io(_))), "{r:?}");
    assert!(
        rt.plugins_dir
            .join(ID)
            .join("1.0.0")
            .join("plugin.json")
            .exists()
    );
    assert!(!rt.plugins_dir.join(ID).join("1.1.0").exists());
    assert!(
        rt.setting(&format!("plugin_installed:{ID}"))
            .unwrap()
            .contains("1.0.0")
    );
    assert_eq!(rt.rt.list(EN)[0].version, "1.0.0");
    assert_eq!(
        rt.rt
            .run_command(ID, "echo", vec![1])
            .unwrap()
            .message
            .as_deref(),
        Some("done 1 assets")
    );
    // 막은 것을 치우면 다시 설치된다
    fs::remove_file(rt.plugins_dir.join(".staging")).unwrap();
    assert_eq!(
        rt.rt
            .install(&dir, ID, "1.1.0", common_rt::basic_perms(), EN)
            .unwrap()
            .version,
        "1.1.0"
    );
}
