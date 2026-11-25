// Categories: Validation (t_ipc_46 인자·응답 모양) / Business rule (t_ipc_47 오류 코드 매핑, 내부 정보 비노출)
use mam_core::query::{Availability, MediaKind};
use mam_core::userdata::{CollectionMembership, PlayQueueItem};
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
        "INSERT INTO volumes (volume_id, kind, volume_key, label, last_mount_point, availability, last_seen_at)
           VALUES (1, 'fixed', 'v1', 'Main', 'C:/Media', 'online', 0);
         INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
           VALUES (1, 'video', 1, 0, 0, 0), (2, 'image', 1, 0, 0, 0);
         INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
           VALUES (1, 1, 'a/clip.mp4', 'a/clip.mp4', 0), (2, 1, 'a/pic.jpg', 'a/pic.jpg', 0);
         INSERT INTO media_metadata (asset_id, duration_ms) VALUES (1, 9000);",
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
fn t_ipc_46_reorder_target_deserializes_three_forms_and_responses_use_camel_case() {
    // Arrange + Act: 계약 `ReorderTarget` 세 모양
    let before: ReorderTarget = serde_json::from_value(json!({"kind": "before", "assetId": 7})).unwrap();
    let start: ReorderTarget = serde_json::from_value(json!({"kind": "start"})).unwrap();
    let end: ReorderTarget = serde_json::from_value(json!({"kind": "end"})).unwrap();

    // Assert
    assert_eq!(before, ReorderTarget::Before { asset_id: 7 });
    assert_eq!((start, end), (ReorderTarget::Start, ReorderTarget::End));
    // 잘못된 모양은 command 인자 변환 단계에서 거부된다 (snake_case 키·모르는 kind·assetId 누락)
    for bad in [
        json!({"kind": "before", "asset_id": 7}),
        json!({"kind": "before"}),
        json!({"kind": "middle"}),
        json!({"assetId": 7}),
        json!(null),
        json!("end"),
    ] {
        assert!(serde_json::from_value::<ReorderTarget>(bad.clone()).is_err(), "{bad}");
    }
    // 응답 키
    let m = serde_json::to_value(CollectionMembership { collection_id: 3, count: 2 }).unwrap();
    assert_eq!(m, json!({"collectionId": 3, "count": 2}));
    let q = serde_json::to_value(PlayQueueItem {
        asset_id: 1,
        media_type: MediaKind::Video,
        availability: Availability::Offline,
        file_name: "clip.mp4".into(),
        title: None,
        duration_ms: Some(9000),
    })
    .unwrap();
    assert_eq!(
        q,
        json!({"assetId": 1, "mediaType": "video", "availability": "offline",
               "fileName": "clip.mp4", "title": null, "durationMs": 9000})
    );
}

#[test]
fn t_ipc_47_collection_command_errors_map_to_contract_codes_without_internals() {
    // Arrange
    let (_dir, ud) = fixture();
    let cid = ud.create_collection("c").unwrap().collection_id;
    ud.add_to_collection(cid, &[1, 2]).unwrap();

    // Act + Assert: 정상 경로
    call(&ud, move |u| u.reorder_collection(cid, &[2], ReorderTarget::Start)).unwrap();
    let queue = call(&ud, move |u| u.collection_play_queue(cid)).unwrap();
    assert_eq!(queue.iter().map(|i| i.asset_id).collect::<Vec<_>>(), [2, 1]);
    assert_eq!(queue[1].file_name, "clip.mp4");
    let membership = call(&ud, |u| u.collection_membership(&[1, 2])).unwrap();
    assert_eq!(membership, [CollectionMembership { collection_id: cid, count: 2 }]);

    // 오류 코드: 없는 컬렉션 -> not_found, 잘못된 인자 -> invalid_input
    let not_found = [
        call(&ud, |u| u.reorder_collection(999, &[1], ReorderTarget::End)).unwrap_err(),
        call(&ud, |u| u.collection_play_queue(999)).unwrap_err(),
    ];
    let invalid = [
        call(&ud, move |u| u.reorder_collection(cid, &[], ReorderTarget::End)).unwrap_err(),
        call(&ud, move |u| u.reorder_collection(cid, &[1], ReorderTarget::Before { asset_id: 1 })).unwrap_err(),
        call(&ud, move |u| u.reorder_collection(cid, &[42], ReorderTarget::End)).unwrap_err(),
        call(&ud, |u| u.collection_membership(&(1..=10_001).collect::<Vec<_>>())).unwrap_err(),
    ];
    assert!(not_found.iter().all(|e| e.code() == ErrorCode::NotFound), "{not_found:?}");
    assert!(invalid.iter().all(|e| e.code() == ErrorCode::InvalidInput), "{invalid:?}");
    for e in not_found.iter().chain(invalid.iter()) {
        let body = serde_json::to_string(e).unwrap();
        assert!(!body.contains("SELECT") && !body.contains("mam.sqlite") && !body.contains("C:/"), "{body}");
    }
    // 실패한 호출은 순서를 바꾸지 않았다
    let queue = call(&ud, move |u| u.collection_play_queue(cid)).unwrap();
    assert_eq!(queue.iter().map(|i| i.asset_id).collect::<Vec<_>>(), [2, 1]);
}
