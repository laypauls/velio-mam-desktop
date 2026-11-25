//! 콘텐츠 제목 `UserData::set_title` (T-title-*) — 실제 SQLite + 스캔으로 만든 자산.
//!
//! 분류: Happy (01 저장·조회, 02 지우기) / Boundary (03 공백·200자·201자·유니코드) /
//! Validation (04 설정 꺼짐, 05 없는 자산, 06 제어 문자) / Concurrency·멱등 (07 같은 값 no-op) /
//! Business rule (08 상세 반영, 09 원본 보호, 10 꺼진 뒤에도 저장값 보존)

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::env::{Env, snapshot};
use mam_core::query::QueryService;
use mam_core::scan::{IngestError, scan_folder};
use mam_core::userdata::UserData;
use mam_db::{Priority, settings};
use serde_json::json;

struct Fixture {
    env: Env,
    ud: UserData,
    ids: Vec<i64>,
}

fn fixture(n: u8) -> Fixture {
    let env = Env::new();
    for i in 0..n {
        env.write(&format!("lib/img{i}.jpg"), i + 1);
    }
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false)).unwrap();
    let ids = env.read(|c| {
        let mut s = c
            .prepare("SELECT asset_id FROM media_assets ORDER BY asset_id")
            .unwrap();
        s.query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());
    Fixture { env, ud, ids }
}

fn set_title_edit(f: &Fixture, on: bool) {
    f.env
        .pool
        .write(Priority::High, move |c| {
            Ok(settings::update(c, json!({"titleEditEnabled": on}).as_object().unwrap()).is_ok())
        })
        .unwrap()
        .then_some(())
        .unwrap();
}

fn enabled_fixture(n: u8) -> Fixture {
    let f = fixture(n);
    set_title_edit(&f, true);
    f
}

fn stored_title(f: &Fixture, id: i64) -> Option<String> {
    f.env.read(|c| {
        c.query_row(
            "SELECT title FROM asset_user_data WHERE asset_id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap_or(None)
    })
}

fn revision(f: &Fixture, id: i64) -> i64 {
    f.env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {id}"
    ))
}

fn outbox_count(f: &Fixture) -> i64 {
    f.env.count("SELECT count(*) FROM search_outbox")
}

fn last_event(f: &Fixture, id: i64) -> (i64, i64) {
    f.env.read(|c| {
        c.query_row(
            "SELECT revision, priority FROM search_outbox WHERE asset_id = ?1 ORDER BY event_id DESC LIMIT 1",
            [id],
            |r| Ok((r.get(0).unwrap(), r.get(1).unwrap())),
        )
        .unwrap()
    })
}

#[test]
fn t_title_01_sets_the_title_with_revision_and_priority_zero_outbox_in_one_commit() {
    // Arrange
    let f = enabled_fixture(2);
    let a = f.ids[0];
    let (ra, events) = (revision(&f, a), outbox_count(&f));

    // Act
    f.ud.set_title(a, Some("  제주 일출 2024  ")).unwrap();

    // Assert: trim 된 값, revision +1, 같은 revision 의 high-priority upsert 이벤트 1건
    assert_eq!(stored_title(&f, a).as_deref(), Some("제주 일출 2024"));
    assert_eq!(revision(&f, a), ra + 1);
    assert_eq!(outbox_count(&f), events + 1);
    assert_eq!(last_event(&f, a), (ra + 1, 0));
    assert_eq!(stored_title(&f, f.ids[1]), None, "다른 자산은 그대로");
}

#[test]
fn t_title_02_null_empty_and_whitespace_clear_the_title_to_null() {
    for clear in [None, Some(""), Some("   \t ")] {
        // Arrange
        let f = enabled_fixture(1);
        let a = f.ids[0];
        f.ud.set_title(a, Some("제목")).unwrap();
        let r = revision(&f, a);

        // Act
        f.ud.set_title(a, clear).unwrap();

        // Assert: NULL (빈 문자열 저장 아님), revision 증가
        assert_eq!(stored_title(&f, a), None, "{clear:?}");
        assert_eq!(
            f.env.count(&format!(
                "SELECT count(*) FROM asset_user_data WHERE asset_id = {a} AND title IS NULL"
            )),
            1
        );
        assert_eq!(revision(&f, a), r + 1);
    }
}

#[test]
fn t_title_03_length_limit_is_200_unicode_chars_after_trim() {
    let f = enabled_fixture(1);
    let a = f.ids[0];

    // 200자(한글, 바이트로는 600) 허용 — 앞뒤 공백은 세지 않는다
    let ok = format!("  {}  ", "가".repeat(200));
    f.ud.set_title(a, Some(&ok)).unwrap();
    assert_eq!(stored_title(&f, a).unwrap().chars().count(), 200);

    // 201자 거부 — 기존 값 유지, revision/outbox 변화 없음
    let (r, events) = (revision(&f, a), outbox_count(&f));
    let err = f.ud.set_title(a, Some(&"가".repeat(201))).unwrap_err();
    assert!(matches!(err, IngestError::InvalidInput("title")), "{err:?}");
    assert_eq!(stored_title(&f, a).unwrap().chars().count(), 200);
    assert_eq!((revision(&f, a), outbox_count(&f)), (r, events));
}

#[test]
fn t_title_04_rejected_with_invalid_input_when_title_editing_is_disabled() {
    // Arrange: 기본값 = 꺼짐
    let f = fixture(1);
    let a = f.ids[0];
    let (r, events) = (revision(&f, a), outbox_count(&f));

    // Act
    let err = f.ud.set_title(a, Some("x")).unwrap_err();
    let err_clear = f.ud.set_title(a, None).unwrap_err();

    // Assert: 지우기도 거부, 아무것도 쓰지 않는다
    assert!(matches!(err, IngestError::InvalidInput(_)), "{err:?}");
    assert!(matches!(err_clear, IngestError::InvalidInput(_)));
    assert_eq!(stored_title(&f, a), None);
    assert_eq!((revision(&f, a), outbox_count(&f)), (r, events));
}

#[test]
fn t_title_05_unknown_asset_is_not_found_and_writes_nothing() {
    let f = enabled_fixture(1);
    let events = outbox_count(&f);

    let err = f.ud.set_title(999_999, Some("x")).unwrap_err();

    assert!(matches!(err, IngestError::NotFound), "{err:?}");
    assert_eq!(outbox_count(&f), events);
    assert_eq!(f.env.count("SELECT count(*) FROM asset_user_data"), 0);
}

#[test]
fn t_title_06_control_characters_are_invalid() {
    let f = enabled_fixture(1);
    let a = f.ids[0];

    for bad in ["a\0b", "line1\nline2", "tab\there"] {
        let err = f.ud.set_title(a, Some(bad)).unwrap_err();
        assert!(matches!(err, IngestError::InvalidInput("title")), "{bad:?}");
    }
    assert_eq!(stored_title(&f, a), None);
}

#[test]
fn t_title_07_same_value_and_clearing_an_absent_title_are_noops() {
    // Arrange
    let f = enabled_fixture(2);
    let (a, b) = (f.ids[0], f.ids[1]);
    f.ud.set_title(a, Some("같은 제목")).unwrap();
    let (r, events) = (revision(&f, a), outbox_count(&f));

    // Act
    f.ud.set_title(a, Some("같은 제목")).unwrap();
    f.ud.set_title(a, Some(" 같은 제목 ")).unwrap();
    f.ud.set_title(b, None).unwrap(); // 행이 없는 자산을 지우기

    // Assert
    assert_eq!((revision(&f, a), outbox_count(&f)), (r, events));
    assert_eq!(f.env.count("SELECT count(*) FROM asset_user_data"), 1);
}

#[test]
fn t_title_08_list_and_detail_expose_the_raw_stored_title() {
    // Arrange
    let f = enabled_fixture(2);
    let (a, b) = (f.ids[0], f.ids[1]);
    f.ud.set_title(a, Some("내 제목")).unwrap();
    let q = QueryService::new(Arc::clone(&f.env.pool), f.env.clock.clone());

    // Act
    let page = q.hydrate(&[a, b]).unwrap();
    let detail = q.detail(a).unwrap();

    // Assert: 파일명은 유지, 제목은 별도 필드 (없으면 null)
    assert_eq!(page[0].title.as_deref(), Some("내 제목"));
    assert_eq!(page[0].file_name, "img0.jpg");
    assert_eq!(page[1].title, None);
    assert_eq!(detail.summary.title.as_deref(), Some("내 제목"));
    let json = serde_json::to_value(&detail).unwrap();
    assert_eq!(json["title"], "내 제목");
    assert_eq!(json["fileName"], "img0.jpg");
    assert!(
        serde_json::to_value(&page[1]).unwrap()["title"].is_null(),
        "title 키는 항상 존재하고 null"
    );
}

#[test]
fn t_title_09_setting_a_title_never_touches_original_files() {
    // Arrange
    let f = enabled_fixture(2);
    let before = snapshot(&f.env.path("lib"));

    // Act
    f.ud.set_title(f.ids[0], Some("바뀐 제목")).unwrap();
    f.ud.set_title(f.ids[1], Some("another")).unwrap();
    f.ud.set_title(f.ids[1], None).unwrap();

    // Assert: 이름·내용·수정 시각 불변
    assert_eq!(snapshot(&f.env.path("lib")), before);
}

#[test]
fn t_title_10_stored_title_survives_turning_the_setting_off() {
    // Arrange
    let f = enabled_fixture(1);
    let a = f.ids[0];
    f.ud.set_title(a, Some("보존")).unwrap();

    // Act
    set_title_edit(&f, false);
    let q = QueryService::new(Arc::clone(&f.env.pool), f.env.clock.clone());

    // Assert: 값은 지우지 않는다 (표시 여부는 프론트가 설정으로 결정)
    assert_eq!(stored_title(&f, a).as_deref(), Some("보존"));
    assert_eq!(q.hydrate(&[a]).unwrap()[0].title.as_deref(), Some("보존"));
    assert!(f.ud.set_title(a, Some("수정")).is_err());
}
