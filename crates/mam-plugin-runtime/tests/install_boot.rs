//! 부팅 시험 — 분류: Happy / Validation(손상 설정은 닫힘) / Business rule(Broken·Incompatible 목록 유지) / External(크래시 잔여물·워커 없음).
mod common_rt;

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

use common_rt::{EN, Rt, file_list, worker_exe};
use mam_plugin_runtime::{InstallError, InvokeError, Permission, PluginStatus, ProviderInvoker, RunError, locate_worker};

const ID: &str = "com.example.basic";

fn mklink_junction(link: &std::path::Path, target: &std::path::Path) {
    let st = std::process::Command::new("cmd").args(["/D", "/C", "mklink", "/J"]).arg(link).arg(target).stdout(std::process::Stdio::null()).status().expect("mklink");
    assert!(st.success());
}

#[test]
fn t_pinst_09b_boot_cleans_crash_leftovers_but_never_leaves_plugins_dir() {
    // Arrange — 설치 후 크래시 잔여물을 만든다
    let mut rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.install_basic(&dir);
    let p = rt.plugins_dir.clone();
    fs::create_dir_all(p.join(".staging").join("x-1")).unwrap();
    fs::write(p.join(".staging").join("x-1").join("junk"), "j").unwrap();
    fs::create_dir_all(p.join(".trash").join("y")).unwrap();
    fs::create_dir_all(p.join("com.example.orphan").join("1.0.0")).unwrap();
    fs::write(p.join("com.example.orphan").join("1.0.0").join("plugin.json"), "{}").unwrap();
    fs::create_dir_all(p.join(ID).join("0.1.0")).unwrap(); // 업그레이드 도중 크래시로 남은 옛 버전
    fs::write(p.join("stray.txt"), "s").unwrap();
    // `plugins` 안의 정션이 밖을 가리켜도 링크만 지운다
    let outside = rt.src_root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("keep.txt"), "keep").unwrap();
    mklink_junction(&p.join("com.example.link"), &outside);

    // Act
    rt.reboot(Some(worker_exe()), "0.2.0");

    // Assert
    assert_eq!(file_list(&p), vec![format!("{ID}/1.0.0/dist/main.js"), format!("{ID}/1.0.0/l10n/en.json"), format!("{ID}/1.0.0/l10n/ko.json"), format!("{ID}/1.0.0/plugin.json")]);
    assert!(!p.join("com.example.link").exists() && !p.join(".staging").exists() && !p.join(".trash").exists());
    assert_eq!(fs::read_to_string(outside.join("keep.txt")).unwrap(), "keep");
    assert_eq!(rt.rt.list(EN)[0].status, PluginStatus::Ok);
}

#[test]
fn t_pinst_10_boot_lists_missing_package_as_broken() {
    // Arrange
    let mut rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.install_basic(&dir);
    rt.rt.shutdown_all();
    fs::remove_dir_all(rt.plugins_dir.join(ID).join("1.0.0")).unwrap();

    // Act
    rt.reboot(Some(worker_exe()), "0.2.0");

    // Assert — 목록에는 남고 실행·기여는 없다
    let list = rt.rt.list(EN);
    assert_eq!(list.len(), 1);
    let info = &list[0];
    assert_eq!((info.id.as_str(), info.version.as_str(), info.status), (ID, "1.0.0", PluginStatus::Broken));
    assert_eq!(info.display_name, ID);
    assert!(info.commands.is_empty() && info.providers.is_empty() && !info.worker_available && !info.enabled);
    assert!(matches!(rt.rt.run_command(ID, "echo", vec![]), Err(RunError::Disabled)));
    assert!(rt.rt.contributions(EN).menu_items.is_empty());
    assert!(!rt.rt.allows(ID, Permission::KvRead));
    // 깨진 항목도 제거할 수 있고, 같은 폴더로 다시 설치하면 복구된다
    assert!(matches!(rt.rt.set_enabled(ID, true, EN), Err(InstallError::ManifestMissing)));
    assert_eq!(rt.rt.install(&dir, ID, "1.0.0", common_rt::basic_perms(), EN).unwrap().status, PluginStatus::Ok);
}

#[test]
fn t_pinst_10b_boot_marks_engine_mismatch_incompatible_and_recovers_with_app_version() {
    // Arrange
    let mut rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.install_basic(&dir);

    // Act — 앱 버전이 낮아졌다
    rt.reboot(Some(worker_exe()), "0.0.5");

    // Assert
    let info = rt.rt.list(EN).remove(0);
    assert_eq!(info.status, PluginStatus::Incompatible);
    assert_eq!(info.display_name, "Basic Plugin"); // 패키지는 읽혔으니 이름은 보인다
    assert!(!info.worker_available);
    assert!(rt.rt.host_for_test(ID).is_none() && !rt.rt.allows(ID, Permission::KvRead));
    assert!(matches!(rt.rt.run_command(ID, "echo", vec![]), Err(RunError::Disabled)));
    assert!(matches!(rt.rt.set_enabled(ID, true, EN), Err(InstallError::Engine(_))));
    assert!(rt.rt.contributions(EN).menu_items.is_empty());
    assert!(rt.rt.auto_providers().is_empty());
    // 앱 버전이 다시 맞으면 저장된 상태(활성·권한)로 복구
    rt.reboot(Some(worker_exe()), "0.2.0");
    let back = rt.rt.list(EN).remove(0);
    assert_eq!((back.status, back.enabled, back.granted.len()), (PluginStatus::Ok, true, 5));
}

#[test]
fn t_pinst_10c_boot_treats_corrupt_or_missing_enabled_as_disabled() {
    // Arrange
    let mut rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.install_basic(&dir);
    let key = format!("plugin_enabled:{ID}");

    for raw in [Some("garbage"), Some("\"true\""), Some("1"), None] {
        // Act
        match raw {
            Some(v) => rt.exec(&format!("UPDATE settings SET value = '{v}' WHERE key = '{key}'")),
            None => rt.exec(&format!("DELETE FROM settings WHERE key = '{key}'")),
        }
        rt.reboot(Some(worker_exe()), "0.2.0");

        // Assert — 권한도 닫힌다 (게이트 ON 이 아닌 상태)
        let info = rt.rt.list(EN).remove(0);
        assert_eq!((info.status, info.enabled), (PluginStatus::Ok, false), "{raw:?}");
        assert!(!rt.rt.allows(ID, Permission::KvRead), "{raw:?}");
        assert!(matches!(rt.rt.run_command(ID, "echo", vec![]), Err(RunError::Disabled)), "{raw:?}");
        rt.exec(&format!("INSERT OR REPLACE INTO settings (key, value) VALUES ('{key}', 'true')"));
    }
    // 정상 값 `true` 면 켜진다
    rt.reboot(Some(worker_exe()), "0.2.0");
    assert!(rt.rt.list(EN)[0].enabled && rt.rt.allows(ID, Permission::KvRead));
}

#[test]
fn t_pinst_10d_boot_survives_corrupt_rows_and_mismatched_packages() {
    // Arrange — 손상된 installed 문서, 버전이 다른 폴더, 잘못된 id 행
    let mut rt = Rt::new();
    let (basic, other) = (rt.fixture("basic", "basic"), rt.fixture("other", "other"));
    rt.install_basic(&basic);
    rt.rt.install(&other, "com.example.other", "0.5.0", vec![], EN).unwrap();
    rt.rt.shutdown_all();
    rt.exec("UPDATE settings SET value = 'not json' WHERE key = 'plugin_installed:com.example.basic'");
    fs::rename(rt.plugins_dir.join("com.example.other").join("0.5.0"), rt.plugins_dir.join("com.example.other").join("0.7.0")).unwrap();
    rt.exec("UPDATE settings SET value = '{\"version\":\"0.7.0\",\"installedAt\":1}' WHERE key = 'plugin_installed:com.example.other'");
    rt.exec("INSERT INTO settings (key, value) VALUES ('plugin_installed:Not A Valid Id', '{\"version\":\"1.0.0\",\"installedAt\":1}')");

    // Act — 패닉 없이 부팅한다
    rt.reboot(Some(worker_exe()), "0.2.0");

    // Assert — 셋 모두 Broken 으로 목록에 남는다
    let list = rt.rt.list(EN);
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|i| i.status == PluginStatus::Broken), "{list:?}");
    // 손상 행의 폴더는 지우지 않는다 (설정 키가 있으므로)
    assert!(rt.plugins_dir.join(ID).join("1.0.0").exists());
}

#[test]
fn t_pinst_20_list_includes_broken_next_to_healthy_plugins_in_id_order() {
    // Arrange
    let mut rt = Rt::new();
    let (basic, other) = (rt.fixture("basic", "basic"), rt.fixture("other", "other"));
    rt.install_basic(&basic);
    rt.rt.install(&other, "com.example.other", "0.5.0", vec!["kv.read".into()], EN).unwrap();
    rt.rt.shutdown_all();
    fs::remove_file(rt.plugins_dir.join(ID).join("1.0.0").join("plugin.json")).unwrap();

    // Act
    rt.reboot(Some(worker_exe()), "0.2.0");

    // Assert
    let list = rt.rt.list("ko");
    let summary: Vec<_> = list.iter().map(|i| (i.id.as_str(), i.status)).collect();
    assert_eq!(summary, vec![(ID, PluginStatus::Broken), ("com.example.other", PluginStatus::Ok)]);
    assert_eq!(rt.rt.run_command("com.example.other", "ping", vec![]).unwrap().message.as_deref(), Some("pong"));
}

#[test]
fn t_pinst_23_worker_locate_order() {
    // Arrange
    let tmp = tempfile::tempdir().unwrap();
    let (env_dir, exe_dir) = (tmp.path().join("env"), tmp.path().join("exe"));
    let deps = exe_dir.join("deps");
    for d in [&env_dir, &exe_dir, &deps] {
        fs::create_dir_all(d).unwrap();
    }
    let (env_file, exe_file) = (env_dir.join("custom.exe"), exe_dir.join("mam-plugin-worker.exe"));
    fs::write(&env_file, "x").unwrap();
    fs::write(&exe_file, "x").unwrap();
    let env = |p: &PathBuf| Some(OsString::from(p));

    // Act / Assert — 환경변수(파일) > 실행 파일 폴더 > `deps` 의 부모
    assert_eq!(locate_worker(env(&env_file), Some(exe_dir.as_path())), Some(env_file.clone()));
    assert_eq!(locate_worker(env(&env_dir.join("missing.exe")), Some(exe_dir.as_path())), Some(exe_file.clone())); // 잘못된 환경변수는 무시
    assert_eq!(locate_worker(env(&env_dir), Some(exe_dir.as_path())), Some(exe_file.clone())); // 폴더는 파일이 아니다
    assert_eq!(locate_worker(Some(OsString::new()), Some(exe_dir.as_path())), Some(exe_file.clone()));
    assert_eq!(locate_worker(None, Some(deps.as_path())), Some(exe_file.clone()));
    assert_eq!(locate_worker(None, Some(env_dir.as_path())), None);
    assert_eq!(locate_worker(None, None), None);
}

#[test]
fn t_pinst_23b_missing_worker_reports_unavailable_and_start_error() {
    // Arrange — 워커 없음(None) 과 존재하지 않는 경로
    for worker in [None, Some(PathBuf::from("Z:\\definitely\\missing\\mam-plugin-worker.exe"))] {
        let rt = Rt::with(worker, "0.2.0");
        let dir = rt.fixture("basic", "basic");

        // Act
        let info = rt.install_basic(&dir);
        let err = rt.rt.run_command(ID, "echo", vec![1]).unwrap_err();

        // Assert — 설치·목록은 되고, 호출만 Start 오류. Start 는 결함이 아니라 자동 비활성이 되지 않는다
        assert!(!info.worker_available);
        assert!(matches!(&err, RunError::Invoke(InvokeError::Start(_))), "{err:?}");
        for _ in 0..4 {
            assert!(matches!(rt.rt.run_command(ID, "echo", vec![]), Err(RunError::Invoke(InvokeError::Start(_)))));
        }
        assert!(rt.rt.list(EN)[0].enabled);
        assert_eq!(rt.setting(&format!("plugin_enabled:{ID}")).as_deref(), Some("true"));
    }
}
