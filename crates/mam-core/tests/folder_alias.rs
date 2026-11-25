//! 관리 폴더 별칭 저장·조회 (T-alias-01 ~ 10).
//!
//! 분류: Happy (01 설정·목록 반영, 02 직렬화) / Boundary (03 trim, 06 64자 OK·65자 거부, 07 한글 문자 수) /
//! Business rule (04 None 해제, 05 공백 해제, 10 표시 전용 - 자산·outbox 불변) / Validation (08 없는 폴더) /
//! Concurrency (09 멱등). 실제 SQLite + 임시 디렉터리 볼륨.

mod common;

use common::env::Env;
use mam_core::scan::IngestError;

fn add(env: &Env, rel: &str) -> i64 {
    env.ingest
        .add_folder(env.path(rel).to_str().unwrap())
        .unwrap()
        .folder_id
}

fn alias_of(env: &Env, id: i64) -> Option<String> {
    env.ingest
        .list_folders()
        .unwrap()
        .into_iter()
        .find(|f| f.folder_id == id)
        .unwrap()
        .alias
}

fn set(env: &Env, id: i64, alias: &str) -> Result<(), IngestError> {
    env.ingest.set_folder_alias(id, Some(alias.to_string()))
}

fn lib(env: &Env) -> i64 {
    env.write("lib/a.jpg", 1);
    add(env, "lib")
}

#[test]
fn t_alias_01_set_alias_is_reflected_in_list_folders() {
    // Arrange
    let env = Env::new();
    let id = lib(&env);
    assert_eq!(alias_of(&env, id), None, "기본값은 별칭 없음");

    // Act
    set(&env, id, "여행 사진").unwrap();

    // Assert
    assert_eq!(alias_of(&env, id).as_deref(), Some("여행 사진"));
}

#[test]
fn t_alias_02_alias_serializes_as_camel_case_key() {
    // Arrange
    let env = Env::new();
    let id = lib(&env);
    set(&env, id, "A").unwrap();

    // Act
    let view = env
        .ingest
        .list_folders()
        .unwrap()
        .into_iter()
        .find(|f| f.folder_id == id)
        .unwrap();
    let json = serde_json::to_value(&view).unwrap();

    // Assert
    assert_eq!(json["alias"], "A");
}

#[test]
fn t_alias_03_alias_is_trimmed() {
    let env = Env::new();
    let id = lib(&env);

    set(&env, id, "  \t이름\n ").unwrap();

    assert_eq!(alias_of(&env, id).as_deref(), Some("이름"));
}

#[test]
fn t_alias_04_none_clears_alias() {
    let env = Env::new();
    let id = lib(&env);
    set(&env, id, "이름").unwrap();

    env.ingest.set_folder_alias(id, None).unwrap();

    assert_eq!(alias_of(&env, id), None);
}

#[test]
fn t_alias_05_blank_string_clears_alias_to_null() {
    let env = Env::new();
    let id = lib(&env);
    set(&env, id, "이름").unwrap();

    set(&env, id, "   ").unwrap();

    assert_eq!(alias_of(&env, id), None);
    // 빈 문자열이 아니라 실제 NULL 로 저장된다
    let n: i64 = env
        .pool
        .read(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM managed_folders WHERE folder_id = ?1 AND alias IS NULL",
                [id],
                |r| r.get(0),
            ))
        })
        .unwrap()
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn t_alias_06_64_chars_ok_65_rejected_and_keeps_previous_value() {
    let env = Env::new();
    let id = lib(&env);
    set(&env, id, "이전").unwrap();

    set(&env, id, &"a".repeat(64)).unwrap();
    assert_eq!(alias_of(&env, id).map(|s| s.len()), Some(64));

    let err = set(&env, id, &"a".repeat(65)).unwrap_err();
    assert!(matches!(err, IngestError::InvalidInput("alias")), "{err:?}");
    assert_eq!(
        alias_of(&env, id).map(|s| s.len()),
        Some(64),
        "거부 시 값 유지"
    );
}

#[test]
fn t_alias_07_length_counts_chars_not_bytes() {
    let env = Env::new();
    let id = lib(&env);
    let ok = "가".repeat(64); // 192 바이트
    let too_long = "가".repeat(65);

    set(&env, id, &ok).unwrap();
    let err = set(&env, id, &too_long).unwrap_err();

    assert_eq!(alias_of(&env, id).as_deref(), Some(ok.as_str()));
    assert!(matches!(err, IngestError::InvalidInput("alias")), "{err:?}");
    // trim 후 길이로 판정: 64자 + 앞뒤 공백은 허용
    set(&env, id, &format!("  {ok}  ")).unwrap();
}

#[test]
fn t_alias_08_unknown_folder_is_not_found() {
    let env = Env::new();
    let id = lib(&env);

    assert!(matches!(set(&env, 9999, "x"), Err(IngestError::NotFound)));
    assert!(matches!(
        env.ingest.set_folder_alias(-1, None),
        Err(IngestError::NotFound)
    ));
    // 검증은 존재 확인보다 먼저: 없는 폴더라도 길이 위반은 InvalidInput
    assert!(matches!(
        set(&env, 9999, &"a".repeat(65)),
        Err(IngestError::InvalidInput("alias"))
    ));
    assert_eq!(alias_of(&env, id), None);
}

#[test]
fn t_alias_09_resetting_same_value_is_idempotent() {
    let env = Env::new();
    let id = lib(&env);

    set(&env, id, "같은 이름").unwrap();
    set(&env, id, "같은 이름").unwrap();
    assert_eq!(alias_of(&env, id).as_deref(), Some("같은 이름"));

    env.ingest.set_folder_alias(id, None).unwrap();
    env.ingest.set_folder_alias(id, None).unwrap();
    assert_eq!(alias_of(&env, id), None);
}

#[test]
fn t_alias_10_duplicates_allowed_and_display_only_does_not_touch_outbox_or_revision() {
    let env = Env::new();
    env.write("lib/a.jpg", 1);
    env.write("other/b.jpg", 2);
    let a = add(&env, "lib");
    let b = add(&env, "other");
    let counts = |env: &Env| -> (i64, i64) {
        env.pool
            .read(|c| {
                Ok(c.query_row(
                    "SELECT (SELECT COUNT(*) FROM search_outbox), COALESCE((SELECT SUM(revision) FROM media_assets), 0)",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                ))
            })
            .unwrap()
            .unwrap()
    };
    let before = counts(&env);

    set(&env, a, "같음").unwrap();
    set(&env, b, "같음").unwrap();

    assert_eq!(alias_of(&env, a), alias_of(&env, b), "중복 허용");
    assert_eq!(counts(&env), before, "outbox·revision 불변");
}
