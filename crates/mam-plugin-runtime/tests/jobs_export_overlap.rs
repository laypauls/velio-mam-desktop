//! P5 내보내기 겹침 거부 (T-pjob-23) — **게이트 ON**(플러그인 활성 + `export.write` 허용 + 내보내기 provider)에서도
//! 관리 폴더(원본)·MAMData·백업 폴더와 겹치는 대상은 provider 호출 **전에** `invalid_input` 으로 거부한다.
//!
//! 분류: Business rule(원본 보호) / Boundary(루트와 같은 경로·하위·정션·대소문자·8.3 짧은 이름 변형).
//! 대조군: 같은 플러그인이 겹치지 않는 폴더로는 내보낸다(거부 이유가 권한·비활성이 아님을 보인다).
//! 단언: `invalid_input`, 파일이 생기지 않음, provider 호출 수 불변(TestInvoker 카운터), 원본 폴더 내용·수정 시각 불변.

mod common_jobs;

use std::fs;
use std::path::{Path, PathBuf};

use common_jobs::{EXPORTER, Env, exporter, junction, short_path};
use mam_plugin_runtime::jobs::ExportError;
use serde_json::json;

/// 게이트 ON 환경 + 대조군 성공 1회. `lib` 는 관리 폴더로 등록돼 있다
fn gate_on() -> (Env, Vec<i64>) {
    let env = Env::new();
    env.install(exporter());
    env.add_managed_root(&env.lib);
    let assets = vec![env.add_asset("image", "photo.jpg")];
    env.invoker.script(vec![json!({"out": {"text": "control"}})]);
    let free = env.out_dir().join("control.csv");
    env.export(EXPORTER, "exp.step", &assets, &free).expect("a free folder is accepted with the gate on");
    assert_eq!((fs::read(&free).unwrap(), env.invoker.call_count()), (b"control".to_vec(), 1));
    (env, assets)
}

/// 거부돼야 하는 대상들을 시도한다: invalid_input + 파일 없음 + provider 미호출 + 원본 불변
fn assert_rejected(env: &Env, assets: &[i64], dests: &[(&str, PathBuf)], expect: &str) {
    let before = lib_state(&env.lib);
    let calls = env.invoker.call_count();
    for (label, dest) in dests {
        let existed = dest.exists();
        let err = env.export(EXPORTER, "exp.step", assets, dest).expect_err(label);
        match err {
            ExportError::InvalidInput(m) => assert!(m.contains(expect), "{label}: {m}"),
            other => panic!("{label}: expected invalid_input, got {other:?}"),
        }
        assert_eq!(dest.exists(), existed, "{label}: no file created");
    }
    assert_eq!(env.invoker.call_count(), calls, "the provider is never invoked for a rejected destination");
    assert_eq!(lib_state(&env.lib), before, "original media untouched");
}

#[test]
fn t_pjob_23_destination_inside_or_equal_to_a_managed_folder_is_rejected_before_the_provider_runs() {
    // Arrange: 관리 폴더 하위 폴더, 그리고 "관리 폴더 루트가 파일 경로"인 변형(루트와 같은 경로가 모양 검사를 통과)
    let (env, assets) = gate_on();
    fs::create_dir(env.lib.join("sub")).unwrap();
    let weird = env.tmp.path().join("WeirdRoot.csv");
    fs::write(&weird, b"x").unwrap();
    env.add_managed_root(&weird);
    let equal_dir = env.tmp.path().join("EqualRoot.csv");
    fs::create_dir(&equal_dir).unwrap();
    env.add_managed_root(&equal_dir);

    // Act / Assert
    assert_rejected(&env, &assets, &[("in root", env.lib.join("out.csv")), ("in subfolder", env.lib.join("sub").join("out.csv")), ("equal to a root (file)", weird)], "managed folder");
    assert_rejected(&env, &assets, &[("equal to a root (dir)", equal_dir)], "directory");
}

#[test]
fn t_pjob_23_destination_inside_mamdata_or_the_backup_folder_is_rejected() {
    // Arrange
    let (env, assets) = gate_on();
    fs::create_dir_all(env.data.join("cache")).unwrap();

    // Act / Assert
    let dests = [("MAMData", env.data.join("out.csv")), ("MAMData subfolder", env.data.join("cache").join("out.csv")), ("backups", env.backups.join("out.csv"))];
    assert_rejected(&env, &assets, &dests, "app data folder");
}

#[test]
fn t_pjob_23_destination_through_a_directory_junction_into_the_library_or_mamdata_is_rejected() {
    // Arrange: 허용 폴더 안의 정션이 관리 폴더·MAMData 를 가리킨다
    let (env, assets) = gate_on();
    let (to_lib, to_data) = (env.out_dir().join("to_lib"), env.out_dir().join("to_data"));
    let (Some(_j1), Some(_j2)) = (junction(&to_lib, &env.lib), junction(&to_data, &env.data)) else {
        eprintln!("SKIPPED: `cmd /c mklink /J` could not create a junction in the temp dir");
        return;
    };

    // Act / Assert
    assert_rejected(&env, &assets, &[("junction to library", to_lib.join("out.csv"))], "managed folder");
    assert_rejected(&env, &assets, &[("junction to MAMData", to_data.join("out.csv"))], "app data folder");
    assert!(env.lib.join("photo.jpg").is_file() && !env.lib.join("out.csv").exists());
}

#[test]
fn t_pjob_23_destination_spelled_with_different_case_is_rejected() {
    // Arrange
    let (env, assets) = gate_on();
    let upper = |p: &Path| PathBuf::from(p.to_string_lossy().to_uppercase());
    let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());

    // Act / Assert
    let dests = [("upper case", upper(&env.lib).join("OUT.CSV")), ("lower case", lower(&env.lib).join("out.csv"))];
    assert_rejected(&env, &assets, &dests, "managed folder");
    assert_rejected(&env, &assets, &[("MAMData upper case", upper(&env.data).join("out.csv"))], "app data folder");
}

#[test]
fn t_pjob_23_destination_spelled_with_the_8_3_short_name_is_rejected() {
    // Arrange: `SecretLibrary` 는 8자를 넘어 짧은 이름(SECRET~1)이 있을 수 있다 — 볼륨이 8.3 생성을 끄면 짧은 이름이 같아 변형이 없다
    let (env, assets) = gate_on();
    let Some(short) = short_path(&env.lib) else {
        eprintln!("SKIPPED: cannot derive the 8.3 name (`cmd /c for %~sI` failed)");
        return;
    };
    let variant = short != env.lib;
    if !variant {
        eprintln!("NOTE: this volume has no 8.3 name for the library folder; asserting the plain path (no short-name variant exists here)");
    }

    // Act / Assert
    assert_rejected(&env, &assets, &[("8.3 name", short.join("out.csv"))], "managed folder");
    if variant {
        assert_ne!(short.to_string_lossy().to_lowercase(), env.lib.to_string_lossy().to_lowercase(), "a real short-name spelling was tested");
    }
}

/// 관리 폴더의 항목 이름과 (파일이면) 내용·수정 시각 — 하위 폴더가 있어도 쓸 수 있다
type FileState = Option<(Vec<u8>, std::time::SystemTime)>;

fn lib_state(root: &Path) -> Vec<(String, FileState)> {
    let mut out: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            let data = p.is_file().then(|| (fs::read(&p).unwrap(), fs::metadata(&p).unwrap().modified().unwrap()));
            (p.file_name().unwrap().to_string_lossy().into_owned(), data)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
