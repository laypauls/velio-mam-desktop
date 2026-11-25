//! 설치 전 점검(`inspect_dir`) 시험 — 분류: Happy / Validation / Business rule(변경 종류·추가 권한) / Boundary(크기 한도).
mod common_rt;

use std::fs;
use std::path::Path;

use common_rt::{EN, Rt, snapshot};
use mam_plugin_runtime::{AssetKind, InstallChange, InstallError, Permission, ProviderKind};

fn mklink_junction(link: &Path, target: &Path) {
    let st = std::process::Command::new("cmd").args(["/D", "/C", "mklink", "/J"]).arg(link).arg(target).stdout(std::process::Stdio::null()).status().expect("mklink");
    assert!(st.success(), "junction creation failed");
}

/// 오류 메시지에 호스트(임시) 경로가 없다
fn assert_no_host_path(rt: &Rt, e: &InstallError) {
    let msg = e.to_string();
    let base = rt.tmp.path().to_string_lossy().to_string();
    assert!(!msg.contains(&base), "message leaks host path: {msg}");
    assert!(!msg.contains("\\\\"), "message leaks path separators: {msg}");
}

#[test]
fn t_pinst_01_inspect_happy_translates_current_then_en_then_id() {
    // Arrange
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");

    // Act
    let ko = rt.rt.inspect_dir(&dir, "ko").unwrap();
    let fr = rt.rt.inspect_dir(&dir, "fr-FR").unwrap();

    // Assert — 현재 로케일 -> en -> id
    assert_eq!((ko.id.as_str(), ko.version.as_str()), ("com.example.basic", "1.0.0"));
    assert_eq!(ko.display_name, "기본 플러그인");
    assert_eq!(ko.description.as_deref(), Some("Basic description")); // ko 에 없어 en
    let titles: Vec<_> = ko.commands.iter().map(|c| (c.id.as_str(), c.title.as_str())).collect();
    assert_eq!(&titles[..3], &[("echo", "에코"), ("assets", "Assets"), ("asset", "asset")]);
    assert_eq!(ko.providers.len(), 3);
    assert_eq!((ko.providers[0].id.as_str(), ko.providers[0].kind, ko.providers[0].asset_kinds.clone()), ("meta.extract", ProviderKind::Metadata, vec![AssetKind::Image]));
    assert_eq!(ko.providers[1].title, "ana.scan"); // 어디에도 번역이 없다 -> provider id
    assert_eq!((ko.providers[2].kind, ko.providers[2].file_extension.as_deref()), (ProviderKind::Export, Some("csv")));
    assert!(ko.unsigned);
    assert_eq!(fr.display_name, "Basic Plugin"); // fr-FR -> fr -> en
    // 처음 설치: 비교 대상 없음
    assert_eq!(ko.change, InstallChange::Install);
    assert!(ko.installed.is_none() && ko.added_permissions.is_empty());
    assert_eq!(ko.permissions.len(), 5);
}

#[test]
fn t_pinst_01b_inspect_reports_change_and_added_permissions_against_installed() {
    // Arrange — 일부 권한만 허용해 설치
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.rt.install(&dir, "com.example.basic", "1.0.0", vec!["kv.read".into()], EN).unwrap();

    // Act
    let same = rt.rt.inspect_dir(&dir, EN).unwrap();
    rt.patch(&dir, "plugin.json", |s| s.replace("\"version\": \"1.0.0\"", "\"version\": \"1.2.0\""));
    let newer = rt.rt.inspect_dir(&dir, EN).unwrap();
    rt.patch(&dir, "plugin.json", |s| s.replace("\"version\": \"1.2.0\"", "\"version\": \"0.9.0\""));
    let older = rt.rt.inspect_dir(&dir, EN).unwrap();

    // Assert
    assert_eq!(same.change, InstallChange::Reinstall);
    assert_eq!(newer.change, InstallChange::Upgrade);
    assert_eq!(older.change, InstallChange::Downgrade);
    let installed = same.installed.unwrap();
    assert_eq!((installed.version.as_str(), installed.granted.clone()), ("1.0.0", vec![Permission::KvRead]));
    assert_eq!(same.added_permissions, vec![Permission::AssetReadMetadata, Permission::KvWrite, Permission::TagWrite, Permission::ExportWrite]);
}

#[test]
fn t_pinst_02_inspect_rejects_bad_paths_without_reading() {
    // Arrange
    let rt = Rt::new();
    let file = rt.src_root.join("a-file.txt");
    fs::write(&file, "x").unwrap();
    // MAMData 안 폴더에는 plugin.json 도 없다 — 겹침이 먼저 걸려야 한다(읽기 전 검사)
    let inside = rt.plugins_dir.join("some").join("dir");
    fs::create_dir_all(&inside).unwrap();

    // Act / Assert
    assert!(matches!(rt.rt.inspect_dir(Path::new("relative/dir"), EN), Err(InstallError::NotAbsolute)));
    assert!(matches!(rt.rt.inspect_dir(&rt.src_root.join("missing"), EN), Err(InstallError::NotADirectory)));
    assert!(matches!(rt.rt.inspect_dir(&file, EN), Err(InstallError::NotADirectory)));
    assert!(matches!(rt.rt.inspect_dir(&inside, EN), Err(InstallError::OverlapsDataDir)));
    assert!(matches!(rt.rt.inspect_dir(&rt.data_root, EN), Err(InstallError::OverlapsDataDir)));
    // MAMData 를 품은 상위 폴더도 겹침
    assert!(matches!(rt.rt.inspect_dir(rt.tmp.path(), EN), Err(InstallError::OverlapsDataDir)));
}

#[test]
fn t_pinst_02b_inspect_rejects_bad_manifest_without_leaking_host_paths() {
    // Arrange
    let rt = Rt::new();
    let new_dir = |n: &str| rt.fixture("basic", n);
    let (no_manifest, big, bad_id, engine, api2) = (new_dir("a"), new_dir("b"), new_dir("c"), new_dir("d"), new_dir("e"));
    fs::remove_file(no_manifest.join("plugin.json")).unwrap();
    fs::write(big.join("plugin.json"), " ".repeat(65 * 1024)).unwrap();
    rt.patch(&bad_id, "plugin.json", |s| s.replace("com.example.basic", "Com.Example.BASIC"));
    rt.patch(&engine, "plugin.json", |s| s.replace(">=0.1.0", ">=9.0.0"));
    rt.patch(&api2, "plugin.json", |s| s.replace("\"apiVersion\": 1", "\"apiVersion\": 2"));

    // Act
    let errs: Vec<_> = [&no_manifest, &big, &bad_id, &engine, &api2].iter().map(|d| rt.rt.inspect_dir(d, EN).unwrap_err()).collect();

    // Assert
    assert!(matches!(errs[0], InstallError::ManifestMissing));
    assert!(matches!(errs[1], InstallError::Manifest(mam_plugin_runtime::ManifestError::TooLarge { .. })), "{:?}", errs[1]);
    assert!(matches!(errs[2], InstallError::Manifest(mam_plugin_runtime::ManifestError::InvalidId { .. })));
    assert!(matches!(errs[3], InstallError::Engine(_)), "{:?}", errs[3]);
    assert!(matches!(errs[4], InstallError::Manifest(mam_plugin_runtime::ManifestError::UnsupportedApiVersion { .. })));
    errs.iter().for_each(|e| assert_no_host_path(&rt, e));
}

#[test]
fn t_pinst_02c_inspect_rejects_bad_locale_files() {
    // Arrange
    let rt = Rt::new();
    let (bad_json, bad_code, too_big, too_many) = (rt.fixture("basic", "a"), rt.fixture("basic", "b"), rt.fixture("basic", "c"), rt.fixture("basic", "d"));
    fs::write(bad_json.join("l10n").join("xx.json"), r#"{"a.b": 1}"#).unwrap();
    fs::write(bad_code.join("l10n").join("BAD_CODE.json"), "{}").unwrap();
    fs::write(too_big.join("l10n").join("yy.json"), format!("{{\"k\": \"{}\"}}", "v".repeat(256 * 1024))).unwrap();
    (0..65).for_each(|i| fs::write(too_many.join("l10n").join(format!("f{i}.json")), "{}").unwrap());

    // Act
    let errs: Vec<_> = [&bad_json, &bad_code, &too_big, &too_many].iter().map(|d| rt.rt.inspect_dir(d, EN).unwrap_err()).collect();

    // Assert
    assert!(matches!(&errs[0], InstallError::Locale { file_stem, .. } if file_stem == "xx"), "{:?}", errs[0]);
    assert!(matches!(&errs[1], InstallError::Locale { file_stem, .. } if file_stem == "BAD_CODE"), "{:?}", errs[1]);
    assert!(matches!(&errs[2], InstallError::Locale { file_stem, source: mam_plugin_runtime::LocaleError::TooLarge } if file_stem == "yy"), "{:?}", errs[2]);
    assert!(matches!(errs[3], InstallError::TooManyLocaleFiles), "{:?}", errs[3]);
    errs.iter().for_each(|e| assert_no_host_path(&rt, e));
}

#[test]
fn t_pinst_02d_inspect_rejects_bad_main_files() {
    // Arrange
    let rt = Rt::new();
    let (missing, huge, not_utf8, boundary) = (rt.fixture("basic", "a"), rt.fixture("basic", "b"), rt.fixture("basic", "c"), rt.fixture("basic", "d"));
    fs::remove_file(missing.join("dist").join("main.js")).unwrap();
    fs::write(huge.join("dist").join("main.js"), vec![b'/'; 2 * 1024 * 1024 + 1]).unwrap();
    fs::write(not_utf8.join("dist").join("main.js"), [0xffu8, 0xfe, 0xfd]).unwrap();
    fs::write(boundary.join("dist").join("main.js"), vec![b'/'; 2 * 1024 * 1024]).unwrap(); // 정확히 2 MiB 는 허용

    // Act / Assert
    assert!(matches!(rt.rt.inspect_dir(&missing, EN), Err(InstallError::MainMissing)));
    assert!(matches!(rt.rt.inspect_dir(&huge, EN), Err(InstallError::MainTooLarge)));
    assert!(matches!(rt.rt.inspect_dir(&not_utf8, EN), Err(InstallError::MainNotUtf8)));
    assert!(rt.rt.inspect_dir(&boundary, EN).is_ok());
}

#[test]
fn t_pinst_02e_inspect_rejects_main_that_escapes_through_a_junction() {
    // Arrange — main.js 가 폴더 밖에 있고 `dist` 가 거기로 가는 정션이다
    let rt = Rt::new();
    let dir = rt.fixture("basic", "esc");
    let outside = rt.src_root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("main.js"), "mam.commands.register('x', () => 1);").unwrap();
    fs::remove_dir_all(dir.join("dist")).unwrap();
    mklink_junction(&dir.join("dist"), &outside);
    let before = snapshot(&outside);

    // Act
    let err = rt.rt.inspect_dir(&dir, EN).unwrap_err();

    // Assert
    assert!(matches!(err, InstallError::MainEscapes), "{err:?}");
    assert_no_host_path(&rt, &err);
    assert_eq!(snapshot(&outside), before);
}
