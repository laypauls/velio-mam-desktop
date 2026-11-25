//! 워커 격리 시험 — 분류: Business rule(워커 없음 = 절대 띄우지 않음) / External(MAMData 에 심은 실행 파일).
mod common_rt;

use std::fs;

use common_rt::{Rt, worker_exe};
use mam_plugin_runtime::{InvokeError, RunError};

const ID: &str = "com.example.basic";

/// T-prt-iso-01: 워커를 못 찾았을 때(`worker_exe: None`) MAMData 안의 옛 대체 경로(`plugins/.no-worker/`)에 실행 파일이 심겨 있어도 실행되지 않는다.
/// 대조군: 같은 설치에서 실제 워커를 주면 호출이 성공한다(시험 설정이 유효함을 증명)
#[test]
fn t_prt_iso_01_no_worker_never_falls_back_to_a_planted_exe_in_mamdata() {
    // Arrange: 실제 워커 복사본을 옛 폴백 위치에 심는다
    let rt = Rt::with(None, "0.2.0");
    let planted_dir = rt.plugins_dir.join(".no-worker");
    fs::create_dir_all(&planted_dir).unwrap();
    fs::copy(worker_exe(), planted_dir.join("mam-plugin-worker.exe")).unwrap();
    let dir = rt.fixture("basic", "basic");
    let info = rt.install_basic(&dir);

    // Act
    let err = rt.rt.run_command(ID, "echo", vec![1]).unwrap_err();

    // Assert: 시작 오류 — 심은 파일이 돌았다면 성공했을 호출이다
    assert!(!info.worker_available);
    assert!(
        matches!(&err, RunError::Invoke(InvokeError::Start(_))),
        "{err:?}"
    );
}

/// T-prt-iso-02 (대조군): 실제 워커 경로를 주면 같은 호출이 성공한다
#[test]
fn t_prt_iso_02_control_real_worker_runs_the_same_call() {
    let rt = Rt::new();
    let dir = rt.fixture("basic", "basic");
    rt.install_basic(&dir);
    assert!(rt.rt.run_command(ID, "echo", vec![1]).is_ok());
}
