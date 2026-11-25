// 이어보기 command 계약 시험 (T-ipc-resume-01/03)
// Categories: Happy (01 시그니처·등록·응답 모양) / Validation (02 오류 코드 매핑, 내부 정보 비노출)
// tauri 의 `test` feature 는 이 크레이트의 시험 실행 파일을 띄우지 못하게 해 mock IPC 대신 소스 시그니처를 고정한다
use mam_db::{DbPool, SystemClock, migrate, open};
use serde_json::json;

use super::*;
use crate::error::ErrorCode;

fn fixture() -> (tempfile::TempDir, Arc<UserData>) {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
    conn.execute_batch(
        "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
           VALUES (1, 'video', 1, 0, 0, 0);
         INSERT INTO volumes (volume_id, kind, volume_key, last_seen_at) VALUES (1, 'fixed', 'v1', 0);
         INSERT INTO managed_folders (folder_id, volume_id, rel_root, path_key, display_path, created_at) VALUES (1, 1, 'a', 'a', 'a', 0);
         INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at) VALUES (1, 1, 'a/x.mp4', 'a/x.mp4', 0);",
    )
    .unwrap();
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    (dir, Arc::new(UserData::new(pool, Arc::new(SystemClock))))
}

/// command 본문과 같은 경로: `run_blocking` -> 서비스 호출 -> `CommandError` 변환
fn call<T: Send + 'static>(
    ud: &Arc<UserData>,
    f: impl FnOnce(&UserData) -> Result<T, IngestError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::block_on(run_blocking(Arc::clone(ud), f))
}

#[test]
fn t_ipc_resume_01_signatures_registration_and_response_shapes_match_the_contract() {
    // Arrange: 도구가 만든 파일은 커밋 전까지 CRLF 일 수 있다 - 즉시 정규화
    let src = include_str!("resume_commands.rs").replace("\r\n", "\n");
    let lib = include_str!("lib.rs").replace("\r\n", "\n");
    // Act / Assert: JS 인자 이름 = Rust 인자 이름의 camelCase 이므로 시그니처 변경은 프론트 invoke 를 조용히 깨뜨린다
    let squash = |s: &str| s.split_whitespace().collect::<String>().replace(",)", ")");
    for sig in [
        "pub async fn resume_get(svc: Shared<'_>, asset_id: i64) -> Result<Option<ResumePosition>, CommandError>",
        "pub async fn resume_save(\n    svc: Shared<'_>,\n    asset_id: i64,\n    position_ms: i64,\n    duration_ms: Option<i64>,\n) -> Result<ResumeSaveOutcome, CommandError>",
        "pub async fn resume_count_assets(svc: Shared<'_>, asset_ids: Vec<i64>) -> Result<i64, CommandError>",
        "pub async fn resume_clear_assets(svc: Shared<'_>, asset_ids: Vec<i64>) -> Result<i64, CommandError>",
        "pub async fn resume_clear_folder(svc: Shared<'_>, folder_id: i64) -> Result<i64, CommandError>",
        "pub async fn resume_clear_all(svc: Shared<'_>) -> Result<i64, CommandError>",
        "pub async fn resume_set_folder_enabled(svc: Shared<'_>, folder_id: i64, enabled: bool) -> Result<(), CommandError>",
        "pub async fn resume_stats(svc: Shared<'_>) -> Result<ResumeStats, CommandError>",
    ] {
        // rustfmt 이 긴 시그니처를 여러 줄로 접으므로 공백·후행 쉼표를 제거한 형태로 비교한다
        assert!(
            squash(&src).contains(&squash(sig)),
            "signature changed: {sig}"
        );
    }
    for name in [
        "resume_get",
        "resume_save",
        "resume_count_assets",
        "resume_clear_assets",
        "resume_clear_folder",
        "resume_clear_all",
        "resume_set_folder_enabled",
        "resume_stats",
    ] {
        assert!(
            lib.contains(&format!("resume_commands::{name},")),
            "not registered: {name}"
        );
    }
    // 서비스 왕복의 응답 모양
    let (_dir, ud) = fixture();
    assert_eq!(call(&ud, |u| u.resume_get(1)).unwrap(), None);
    let outcome = call(&ud, |u| u.resume_save(1, 60_000, Some(600_000))).unwrap();
    assert_eq!(serde_json::to_value(outcome).unwrap(), json!("saved"));
    let got = call(&ud, |u| u.resume_get(1)).unwrap();
    assert_eq!(
        serde_json::to_value(got).unwrap(),
        json!({"positionMs": 60_000, "durationMs": 600_000})
    );
    let stats = call(&ud, |u| u.resume_stats()).unwrap();
    assert_eq!(
        serde_json::to_value(stats).unwrap(),
        json!({"folders": [{"folderId": 1, "count": 1, "enabled": true}], "total": 1})
    );
    assert_eq!(call(&ud, |u| u.resume_count_assets(&[1, 2])).unwrap(), 1);
    assert_eq!(call(&ud, |u| u.resume_clear_all()).unwrap(), 1);
}

#[test]
fn t_ipc_resume_02_errors_map_to_contract_codes_without_internals() {
    // Arrange
    let (_dir, ud) = fixture();
    call(&ud, |u| u.resume_save(1, 60_000, Some(600_000))).unwrap();

    // Act
    let not_found = [
        call(&ud, |u| u.resume_save(99, 60_000, Some(600_000))).unwrap_err(),
        call(&ud, |u| u.resume_clear_folder(99)).unwrap_err(),
        call(&ud, |u| u.resume_set_folder_enabled(99, false)).unwrap_err(),
    ];
    let invalid = [call(&ud, |u| u.resume_save(1, -1, Some(600_000))).unwrap_err()];

    // Assert
    assert!(
        not_found.iter().all(|e| e.code() == ErrorCode::NotFound),
        "{not_found:?}"
    );
    assert!(
        invalid.iter().all(|e| e.code() == ErrorCode::InvalidInput),
        "{invalid:?}"
    );
    for e in not_found.iter().chain(invalid.iter()) {
        let body = serde_json::to_string(e).unwrap();
        assert!(
            !body.contains("SELECT") && !body.contains("mam.sqlite") && !body.contains("C:/"),
            "{body}"
        );
    }
    // 실패한 호출은 기존 기록을 바꾸지 않았다
    assert_eq!(
        call(&ud, |u| u.resume_get(1))
            .unwrap()
            .map(|p| p.position_ms),
        Some(60_000)
    );
}

#[test]
fn t_ipc_resume_03_set_folder_enabled_round_trip_keeps_the_record_and_reports_the_flag() {
    // Arrange
    let (_dir, ud) = fixture();
    call(&ud, |u| u.resume_save(1, 60_000, Some(600_000))).unwrap();

    // Act
    call(&ud, |u| u.resume_set_folder_enabled(1, false)).unwrap();

    // Assert
    assert_eq!(call(&ud, |u| u.resume_get(1)).unwrap(), None);
    let stats = serde_json::to_value(call(&ud, |u| u.resume_stats()).unwrap()).unwrap();
    assert_eq!(
        stats["folders"][0],
        json!({"folderId": 1, "count": 1, "enabled": false})
    );
    let skipped = call(&ud, |u| u.resume_save(1, 90_000, Some(600_000))).unwrap();
    assert_eq!(serde_json::to_value(skipped).unwrap(), json!("skipped"));
    call(&ud, |u| u.resume_set_folder_enabled(1, true)).unwrap();
    assert_eq!(
        call(&ud, |u| u.resume_get(1))
            .unwrap()
            .map(|p| p.position_ms),
        Some(60_000)
    );
}
