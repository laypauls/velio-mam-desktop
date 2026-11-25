//! P5 내보내기 검증 (T-pjob-24) — 입력·종류·권한·출력 계약 위반은 임시 파일·대상 파일을 남기지 않는다.
//!
//! 분류: Validation(경로 모양·자산 수·provider 종류) / Boundary(0·1·5,000·5,001 자산, 3 MiB 경계) / Business rule(`export.write`·활성 게이트) /
//! External(provider 출력이 계약 밖 -> `BadOutput` = 명령에서 `plugin_error`). 경로 검증 위반은 provider 호출 전에 거부된다(호출 수 0).
//! 실제 SQLite·실제 워커·임시 폴더.

mod common_jobs;

use std::fs;
use std::path::Path;

use common_jobs::{EXPORTER, Env, exporter};
use mam_plugin_runtime::jobs::{ExportError, MAX_EXPORT_ASSETS, MAX_EXPORT_BYTES};
use mam_plugin_runtime::{Permission, ProviderInvoker, RunError};
use serde_json::json;

fn world() -> (Env, Vec<i64>) {
    let env = Env::new();
    env.install(exporter());
    let assets = vec![env.add_asset("image", "a.jpg"), env.add_asset("image", "b.jpg")];
    (env, assets)
}

fn invalid(r: Result<impl std::fmt::Debug, ExportError>, part: &str) {
    match r {
        Err(ExportError::InvalidInput(m)) => assert!(m.contains(part), "expected '{part}' in '{m}'"),
        other => panic!("expected invalid_input({part}), got {other:?}"),
    }
}

fn no_files(dir: &Path) -> usize {
    fs::read_dir(dir).unwrap().count()
}

#[test]
fn t_pjob_24_bad_destination_paths_are_rejected_before_the_provider_runs() {
    // Arrange
    let (env, a) = world();
    let out = env.out_dir();
    fs::create_dir(out.join("dir.csv")).unwrap();
    let call = |dest: &str| env.jobs.export(EXPORTER, "exp.step", a.clone(), dest);

    // Act / Assert
    invalid(call(&out.join("report.txt").to_string_lossy()), "extension");
    invalid(call(&out.join("report").to_string_lossy()), "extension");
    invalid(call("report.csv"), "absolute");
    invalid(call(""), "empty");
    invalid(call(&format!("{}\\a\0b.csv", out.display())), "NUL");
    invalid(call(&out.join("missing").join("report.csv").to_string_lossy()), "parent");
    invalid(call(&out.join("dir.csv").to_string_lossy()), "directory");
    // 회귀: 예약 장치 이름은 확장자가 있어도 장치로 열린다 (NUL.csv 는 내용을 버린다)
    for name in ["NUL.csv", "con.csv", "Aux.csv", "COM1.csv", "lpt9.csv", "nul .csv"] {
        invalid(call(&out.join(name).to_string_lossy()), "reserved");
    }
    invalid(call(&format!("{}\\a:stream.csv", out.display())), "invalid characters");
    assert_eq!(env.invoker.call_count(), 0);
    assert_eq!(no_files(&out), 1, "only the pre-made directory exists");
}

#[test]
fn t_pjob_24_extension_match_is_case_insensitive() {
    // Arrange
    let (env, a) = world();
    let dest = env.out_dir().join("REPORT.CsV");
    env.invoker.script(vec![json!({"out": {"text": "ok"}})]);

    // Act
    let r = env.export(EXPORTER, "exp.step", &a, &dest);

    // Assert
    assert_eq!(r.unwrap().bytes_written, 2);
    assert_eq!(fs::read(&dest).unwrap(), b"ok");
}

#[test]
fn t_pjob_24_asset_count_bounds_apply_after_deduplication() {
    // Arrange
    let (env, _) = world();
    let dest = env.out_dir().join("n.csv");
    let ids = |n: i64| (1..=n).collect::<Vec<i64>>();
    let many_dups: Vec<i64> = (0..MAX_EXPORT_ASSETS as i64 + 1000).map(|i| 1 + i % 3).collect();

    // Act / Assert: 0 과 5,001 은 거부 (호출 없음)
    invalid(env.export(EXPORTER, "exp.echo", &[], &dest), "1..=5000");
    invalid(env.export(EXPORTER, "exp.echo", &ids(MAX_EXPORT_ASSETS as i64 + 1), &dest), "1..=5000");
    assert_eq!((env.invoker.call_count(), dest.exists()), (0, false));
    // 경계: 정확히 5,000 은 허용 (정렬된 id 가 그대로 전달된다)
    assert_eq!(env.export(EXPORTER, "exp.echo", &ids(MAX_EXPORT_ASSETS as i64), &dest).unwrap().bytes_written, fs::metadata(&dest).unwrap().len());
    assert!(fs::read_to_string(&dest).unwrap().starts_with("1,2,3,"));
    // 중복 제거 뒤 3 개면 6,000 개를 줘도 허용
    env.export(EXPORTER, "exp.echo", &many_dups, &dest).unwrap();
    assert_eq!(fs::read_to_string(&dest).unwrap(), "1,2,3|export");
}

#[test]
fn t_pjob_24_non_export_unknown_disabled_and_not_installed_providers_are_rejected() {
    // Arrange
    let (env, a) = world();
    let dest = env.out_dir().join("n.csv");

    // Act / Assert: 메타데이터 provider 는 내보내기에 쓸 수 없다
    invalid(env.export(EXPORTER, "meta.run", &a, &dest), "not an export provider");
    assert!(matches!(env.export(EXPORTER, "nope", &a, &dest), Err(ExportError::Run(RunError::UnknownTarget))));
    assert!(matches!(env.export("com.example.ghost", "x", &a, &dest), Err(ExportError::Run(RunError::NotInstalled))));
    env.disable(EXPORTER);
    assert!(matches!(env.export(EXPORTER, "exp.step", &a, &dest), Err(ExportError::Run(RunError::Disabled))));
    assert_eq!((env.invoker.call_count(), dest.exists()), (0, false));
}

#[test]
fn t_pjob_24_missing_export_write_permission_is_denied_without_invoking_the_provider() {
    // Arrange: 활성 플러그인이지만 `export.write` 가 철회됐다 (게이트의 닫힌 쪽)
    let (env, a) = world();
    let dest = env.out_dir().join("n.csv");
    assert!(env.invoker.allows(EXPORTER, Permission::ExportWrite));
    env.set_granted(EXPORTER, &[Permission::AssetReadMetadata, Permission::KvWrite]);

    // Act
    let r = env.export(EXPORTER, "exp.step", &a, &dest);

    // Assert
    assert!(matches!(r, Err(ExportError::Run(RunError::PermissionDenied(Permission::ExportWrite)))), "{r:?}");
    assert_eq!((env.invoker.call_count(), dest.exists()), (0, false));
}

#[test]
fn t_pjob_24_outputs_outside_the_contract_fail_as_bad_output_and_leave_no_file() {
    // Arrange: 둘 다·둘 다 아님·null·잘못된 형·잘못된 base64·3 MiB 초과(text, base64)
    let (env, a) = world();
    let dest = env.out_dir().join("n.csv");
    env.invoker.script(vec![
        json!({"out": {"text": "a", "dataBase64": "AA=="}}),
        json!({"out": {}}),
        json!({}),
        json!({"out": {"text": 1}}),
        json!({"out": {"dataBase64": "!!!"}}),
        json!({"out": {"other": "x"}}),
        json!({"bigText": MAX_EXPORT_BYTES + 1}),
        json!({"bigBytes": MAX_EXPORT_BYTES + 3}),
    ]);

    // Act / Assert
    for i in 0..8 {
        let r = env.export(EXPORTER, "exp.step", &a, &dest);
        match r {
            Err(ExportError::BadOutput(_)) => {}
            other => panic!("case {i}: expected bad output, got {other:?}"),
        }
        assert!(!dest.exists(), "case {i}: no destination file");
    }
    assert_eq!(no_files(&env.out_dir()), 0, "no temp file left behind");
}

#[test]
fn t_pjob_24_text_at_the_size_limit_is_accepted() {
    // Arrange
    let (env, a) = world();
    let dest = env.out_dir().join("big.csv");
    env.invoker.script(vec![json!({"bigText": MAX_EXPORT_BYTES})]);

    // Act
    let r = env.export(EXPORTER, "exp.step", &a, &dest).unwrap();

    // Assert
    assert_eq!((r.bytes_written, fs::metadata(&dest).unwrap().len()), (MAX_EXPORT_BYTES as u64, MAX_EXPORT_BYTES as u64));
}


#[test]
fn t_pjob_24_regression_base64_at_the_documented_limit_fits_one_worker_line() {
    // Arrange: 예전 상한(정확히 3 MiB)의 base64 는 4 MiB + 봉투라 워커 줄 한도(4 MiB)를 넘어 `too_large` 로 실패했다
    let (env, a) = world();
    let dest = env.out_dir().join("b64.csv");
    env.invoker.script(vec![json!({"bigBytes": MAX_EXPORT_BYTES - MAX_EXPORT_BYTES % 3}), json!({"bigBytes": MAX_EXPORT_BYTES + 3})]);

    // Act
    let at_limit = env.export(EXPORTER, "exp.step", &a, &dest);
    let over = env.export(EXPORTER, "exp.step", &a, &env.out_dir().join("over.csv"));

    // Assert: 상한(3 바이트 단위로 내린 값)까지는 성공, 넘으면 계약 위반 오류 — 어느 쪽도 부분 파일 없음
    let written = at_limit.expect("base64 at the limit is accepted").bytes_written as usize;
    assert_eq!((written, fs::metadata(&dest).unwrap().len() as usize), (MAX_EXPORT_BYTES - MAX_EXPORT_BYTES % 3, MAX_EXPORT_BYTES - MAX_EXPORT_BYTES % 3));
    assert!(matches!(over, Err(ExportError::BadOutput(_))), "{over:?}");
    assert_eq!(no_files(&env.out_dir()), 1);
}
