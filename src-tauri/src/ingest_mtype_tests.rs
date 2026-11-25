// Categories: Validation (모두 꺼짐 -> invalid_input, 없는 폴더 -> not_found) / Happy (계약 직렬화·역직렬화) / Business rule (keep -> detached 0, purge -> 개수)
// command 는 `blocking` 에서 `IngestError` 를 `CommandError` 로 바꾸므로 같은 변환으로 계약을 확인한다.
use super::*;

const VIDEO_ONLY: MediaTypeSet = MediaTypeSet {
    image: false,
    video: true,
    audio: false,
};

#[test]
fn t_mtype_cmd_01_media_types_arg_and_mode_deserialize_from_the_contract_json() {
    // Arrange / Act: 프론트가 보내는 camelCase JSON
    let types: MediaTypeSet =
        serde_json::from_value(serde_json::json!({ "image": false, "video": true, "audio": true })).unwrap();
    let keep: MediaTypeChangeMode = serde_json::from_value(serde_json::json!("keep")).unwrap();
    let purge: MediaTypeChangeMode = serde_json::from_value(serde_json::json!("purge")).unwrap();
    let bad = serde_json::from_value::<MediaTypeChangeMode>(serde_json::json!("delete"));
    let missing_field = serde_json::from_value::<MediaTypeSet>(serde_json::json!({ "image": true }));

    // Assert
    assert_eq!((types.image, types.video, types.audio), (false, true, true));
    assert_eq!((keep, purge), (MediaTypeChangeMode::Keep, MediaTypeChangeMode::Purge));
    assert!(bad.is_err(), "알 수 없는 mode 는 거부");
    assert!(missing_field.is_err(), "누락 필드는 거부");
}

#[test]
fn t_mtype_cmd_02_all_off_maps_to_invalid_input_for_add_and_set() {
    // Arrange
    let fx = fixture();
    let none = MediaTypeSet { image: false, video: false, audio: false };
    let id = fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap().folder_id;

    // Act
    let add = fx.ingest.add_folder_with_types(fx.media.to_str().unwrap(), none).map_err(CommandError::from).unwrap_err();
    let set = fx.ingest.set_folder_media_types(id, none, true).map_err(CommandError::from).unwrap_err();

    // Assert
    assert_eq!(add.code(), ErrorCode::InvalidInput);
    assert_eq!(set.code(), ErrorCode::InvalidInput);
    assert_eq!(serde_json::to_value(&set).unwrap()["code"], "invalid_input");
}

#[test]
fn t_mtype_cmd_03_unknown_folder_maps_to_not_found_for_set_and_count() {
    let fx = fixture();

    let set = fx.ingest.set_folder_media_types(9999, VIDEO_ONLY, false).map_err(CommandError::from).unwrap_err();
    let count = fx.ingest.count_folder_assets_by_type(9999).map_err(CommandError::from).unwrap_err();

    assert_eq!(set.code(), ErrorCode::NotFound);
    assert_eq!(count.code(), ErrorCode::NotFound);
}

#[test]
fn t_mtype_cmd_04_result_and_counts_serialize_with_contract_keys_and_purge_reports_detached() {
    // Arrange: a.jpg(이미지) + sub/b.mp4(동영상) 를 실제 스캔
    let fx = fixture();
    let id = fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap().folder_id;
    scan_folder(fx.ingest.env(), id, &|| false, &AtomicBool::new(false)).unwrap();
    let view = serde_json::to_value(fx.ingest.list_folders().unwrap()).unwrap();

    // Act
    let counts = serde_json::to_value(fx.ingest.count_folder_assets_by_type(id).unwrap()).unwrap();
    let keep = fx.ingest.set_folder_media_types(id, VIDEO_ONLY, false).unwrap();
    let purge = fx.ingest.set_folder_media_types(id, VIDEO_ONLY, true).unwrap();
    let result = serde_json::to_value(SetMediaTypesResult { detached: purge }).unwrap();

    // Assert
    assert_eq!(view[0]["mediaTypes"], serde_json::json!({ "image": true, "video": true, "audio": true }));
    assert_eq!(counts, serde_json::json!({ "image": 1, "video": 1, "audio": 0 }));
    assert_eq!(keep, 0);
    assert_eq!(result, serde_json::json!({ "detached": 1 }));
}
