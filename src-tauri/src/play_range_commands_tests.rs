// 재생 구간 command 계약 시험 (T-ipc-ranges-01/02)
// Categories: Happy (01 인자 이름·응답 camelCase) / Validation (01 잘못된 모양 거부) / Business rule (02 오류 코드 매핑, 내부 정보 비노출)
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
           VALUES (1, 'video', 1, 0, 0, 0), (2, 'image', 1, 0, 0, 0);
         INSERT INTO media_metadata (asset_id, duration_ms, fps) VALUES (1, 9000, 25.0);",
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
fn t_ipc_ranges_01_arguments_and_responses_use_camel_case_contract_shapes() {
    // Arrange + Act: command 인자(Vec<PlayRange>) 역직렬화
    let ok: Vec<PlayRange> = serde_json::from_value(
        json!([{"startMs": 100, "endMs": 900}, {"startMs": 2000, "endMs": 3000}]),
    )
    .unwrap();

    // Assert: 계약 모양만 받는다 (snake_case·누락·소수·문자열은 command 진입 전에 거부)
    assert_eq!(ok.len(), 2);
    for bad in [
        json!({"start_ms": 1, "end_ms": 2}),
        json!({"startMs": 1}),
        json!({"startMs": 1.5, "endMs": 900}),
        json!({"startMs": "1", "endMs": 900}),
        json!(null),
    ] {
        assert!(
            serde_json::from_value::<PlayRange>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    // 응답 키
    assert_eq!(
        serde_json::to_value(ok[0]).unwrap(),
        json!({"startMs": 100, "endMs": 900})
    );
    let s = serde_json::to_value(PlayRangeSummary {
        asset_id: 1,
        range_count: 2,
        total_ms: 1500,
    })
    .unwrap();
    assert_eq!(s, json!({"assetId": 1, "rangeCount": 2, "totalMs": 1500}));

    // 서비스 왕복 (command 본문과 같은 호출)
    let (_dir, ud) = fixture();
    let saved = call(&ud, move |u| u.play_ranges_set(1, &ok)).unwrap();
    assert_eq!(
        serde_json::to_value(&saved).unwrap(),
        json!([{"startMs": 100, "endMs": 900}, {"startMs": 2000, "endMs": 3000}])
    );
    let cid = ud.create_collection("c").unwrap().collection_id;
    ud.add_to_collection(cid, &[1]).unwrap();
    let sum = call(&ud, move |u| u.collection_play_range_summary(cid)).unwrap();
    assert_eq!(
        serde_json::to_value(&sum).unwrap(),
        json!([{"assetId": 1, "rangeCount": 2, "totalMs": 1800}])
    );
}

#[test]
fn t_ipc_ranges_02_errors_map_to_contract_codes_without_internals() {
    // Arrange
    let (_dir, ud) = fixture();
    call(&ud, |u| {
        u.play_ranges_set(
            1,
            &[PlayRange {
                start_ms: 0,
                end_ms: 1000,
            }],
        )
    })
    .unwrap();

    // Act
    let not_found = [
        call(&ud, |u| u.play_ranges_get(99)).unwrap_err(),
        call(&ud, |u| u.play_ranges_set(99, &[])).unwrap_err(),
        call(&ud, |u| u.collection_play_range_summary(99)).unwrap_err(),
    ];
    let invalid = [
        call(&ud, |u| {
            u.play_ranges_set(
                2,
                &[PlayRange {
                    start_ms: 0,
                    end_ms: 1000,
                }],
            )
        })
        .unwrap_err(),
        call(&ud, |u| {
            u.play_ranges_set(
                1,
                &[PlayRange {
                    start_ms: 10,
                    end_ms: 5,
                }],
            )
        })
        .unwrap_err(),
        call(&ud, |u| {
            u.play_ranges_set(
                1,
                &[PlayRange {
                    start_ms: -1,
                    end_ms: 5,
                }],
            )
        })
        .unwrap_err(),
    ];

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
    // 실패한 호출은 기존 구간을 바꾸지 않았다
    let kept = call(&ud, |u| u.play_ranges_get(1)).unwrap();
    assert_eq!(
        kept,
        [PlayRange {
            start_ms: 0,
            end_ms: 1000
        }]
    );
}
