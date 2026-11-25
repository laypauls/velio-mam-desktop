// Categories: Validation (없는 폴더 -> not_found 코드) / Happy (토글이 list_folders 계약에 반영) / Concurrency (멱등)
// command 는 `blocking` 에서 `IngestError` 를 `CommandError` 로 바꾸므로 같은 변환으로 계약을 확인한다.
use super::*;

fn add_media_folder(fx: &Fx) -> i64 {
    fx.ingest.add_folder(fx.media.to_str().unwrap()).unwrap().folder_id
}

#[test]
fn t_hide_cmd_01_unknown_folder_maps_to_the_not_found_code() {
    // Arrange
    let fx = fixture();

    // Act
    let err: CommandError = fx.ingest.set_folder_hidden(9999, true).map_err(CommandError::from).unwrap_err();

    // Assert
    assert_eq!(err.code(), ErrorCode::NotFound);
    assert_eq!(serde_json::to_value(&err).unwrap()["code"], "not_found");
}

#[test]
fn t_hide_cmd_02_toggle_is_idempotent_and_reflected_as_hidden_in_list_folders() {
    // Arrange
    let fx = fixture();
    let id = add_media_folder(&fx);
    let hidden = |fx: &Fx| serde_json::to_value(fx.ingest.list_folders().unwrap()).unwrap()[0]["hidden"].clone();
    assert_eq!(hidden(&fx), false);

    // Act + Assert
    fx.ingest.set_folder_hidden(id, true).unwrap();
    fx.ingest.set_folder_hidden(id, true).unwrap();
    assert_eq!(hidden(&fx), true);
    fx.ingest.set_folder_hidden(id, false).unwrap();
    assert_eq!(hidden(&fx), false);
}

#[test]
fn t_hide_cmd_03_removed_folder_maps_to_the_not_found_code() {
    // Arrange
    let fx = fixture();
    let id = add_media_folder(&fx);
    fx.ingest.remove_folder(id).unwrap();

    // Act
    let err = fx.ingest.set_folder_hidden(id, true).map_err(CommandError::from).unwrap_err();

    // Assert
    assert_eq!(err.code(), ErrorCode::NotFound);
}
