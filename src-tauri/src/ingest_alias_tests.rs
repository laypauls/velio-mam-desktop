// Categories: Validation (65자 -> invalid_input, 없는 폴더 -> not_found) / Happy (list_folders 계약에 camelCase `alias`) / Boundary (빈 값 해제)
// command 는 `blocking` 에서 `IngestError` 를 `CommandError` 로 바꾸므로 같은 변환으로 계약을 확인한다.
use super::*;

#[test]
fn t_alias_cmd_01_too_long_maps_to_invalid_input_and_unknown_to_not_found() {
    // Arrange
    let fx = fixture();
    let id = fx
        .ingest
        .add_folder(fx.media.to_str().unwrap())
        .unwrap()
        .folder_id;

    // Act
    let long = fx
        .ingest
        .set_folder_alias(id, Some("가".repeat(65)))
        .map_err(CommandError::from)
        .unwrap_err();
    let missing = fx
        .ingest
        .set_folder_alias(9999, None)
        .map_err(CommandError::from)
        .unwrap_err();

    // Assert
    assert_eq!(long.code(), ErrorCode::InvalidInput);
    assert_eq!(
        serde_json::to_value(&long).unwrap()["code"],
        "invalid_input"
    );
    assert_eq!(missing.code(), ErrorCode::NotFound);
}

#[test]
fn t_alias_cmd_02_alias_is_exposed_in_list_folders_contract_and_blank_clears_it() {
    // Arrange
    let fx = fixture();
    let id = fx
        .ingest
        .add_folder(fx.media.to_str().unwrap())
        .unwrap()
        .folder_id;
    let alias = |fx: &Fx| {
        serde_json::to_value(fx.ingest.list_folders().unwrap()).unwrap()[0]["alias"].clone()
    };
    assert!(alias(&fx).is_null());

    // Act + Assert
    fx.ingest
        .set_folder_alias(id, Some(" 내 폴더 ".into()))
        .unwrap();
    assert_eq!(alias(&fx), "내 폴더");
    fx.ingest.set_folder_alias(id, Some("  ".into())).unwrap();
    assert!(alias(&fx).is_null());
}
