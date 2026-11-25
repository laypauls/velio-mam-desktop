//! 내보내기 보안 회귀 (L1 보호 경로 동적 계산, 입력 길이 상한). 실제 SQLite·실제 파일 시스템(임시 폴더).
//!
//! 분류: Business rule(실행 중 바뀐 백업 폴더 보호) / Boundary(자산 수 원본 길이 상한) / Validation.

mod common_jobs;

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use common_jobs::{EXPORTER, Env, exporter};
use mam_plugin_runtime::jobs::{ExportError, MAX_EXPORT_ASSETS};
use serde_json::json;

#[test]
fn t_pexpsec_01_backup_dir_changed_after_setup_is_protected_per_call() {
    // Arrange: 부팅 뒤에 백업 폴더가 바뀌는 상황 — source 가 현재 값을 돌려준다
    let env = Env::new();
    env.install(exporter());
    let assets = vec![env.add_asset("image", "photo.jpg")];
    let current: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let cur = Arc::clone(&current);
    env.jobs.set_protected_source(Arc::new(move || cur.lock().unwrap().iter().cloned().collect()));
    let new_backups = env.tmp.path().join("NewBackups");
    fs::create_dir(&new_backups).unwrap();
    let dest = new_backups.join("out.csv");
    env.invoker.script(vec![json!({"out": {"text": "a"}}), json!({"out": {"text": "b"}})]);

    // Act / Assert: 바꾸기 전에는 일반 폴더라 허용 (대조군)
    env.export(EXPORTER, "exp.step", &assets, &dest).expect("not yet protected");
    fs::remove_file(&dest).unwrap();
    // 설정이 바뀐 뒤: 같은 폴더가 거부되고 provider 는 호출되지 않는다
    *current.lock().unwrap() = Some(new_backups.clone());
    let calls = env.invoker.call_count();
    let err = env.export(EXPORTER, "exp.step", &assets, &dest).expect_err("protected after the setting change");
    assert!(matches!(err, ExportError::InvalidInput(m) if m.contains("app data folder")), "{err:?}");
    assert!(!dest.exists());
    assert_eq!(env.invoker.call_count(), calls);
}

#[test]
fn t_pexpsec_02_oversized_raw_asset_list_is_rejected_before_sorting() {
    // Arrange: 중복 제거 전 원본 길이가 상한(5,000 × 4)을 넘는 입력
    let env = Env::new();
    env.install(exporter());
    let dest = env.out_dir().join("n.csv");
    let huge: Vec<i64> = (0..(MAX_EXPORT_ASSETS * 4 + 1) as i64).map(|i| i % 3 + 1).collect();

    // Act
    let err = env.export(EXPORTER, "exp.echo", &huge, &dest).expect_err("too many raw ids");

    // Assert
    assert!(matches!(err, ExportError::InvalidInput(m) if m.contains("1..=5000")), "{err:?}");
    assert_eq!((env.invoker.call_count(), dest.exists()), (0, false));
}
