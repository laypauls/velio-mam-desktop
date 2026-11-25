// Categories: Happy (T-ipc-umeta-01/02/04/07 계약 모양·종단 검색) / Validation (03 조건 대조, 05 대량 한도) /
// Business rule (06 삭제 뒤 값 정리) / External (오류 코드 매핑)
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_core::query::{
    QueryService, SearchFilters, SearchPage, SearchRequest, SortDirection, SortField, SortSpec,
    UserFieldFilter,
};
use mam_core::userdata::{FieldPatch, FieldValue};
use mam_db::{DbPool, SystemClock, migrate, open};
use serde_json::json;

use super::*;
use crate::error::ErrorCode;
use crate::metadata_types::FieldValueDto;
use crate::search_service::SearchFailure;

struct Fx {
    _dir: tempfile::TempDir,
    pool: Arc<DbPool>,
    user: Arc<UserData>,
    search: Arc<SearchService>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        self.search.shutdown();
    }
}

fn fixture(asset_count: i64) -> Fx {
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
    conn.execute_batch(
        "INSERT INTO volumes (volume_id, kind, volume_key, label, last_mount_point, availability, last_seen_at)
           VALUES (1, 'fixed', 'v1', 'Main', 'C:\\Media', 'online', 0);",
    )
    .unwrap();
    for id in 1..=asset_count {
        conn.execute(
            "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             VALUES (?1, 'image', 10, ?1, 0, 0)",
            [id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
             VALUES (?1, 1, ?2, ?2, 0)",
            rusqlite::params![id, format!("photos/p{id}.jpg")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
             VALUES (?1, 1, 'upsert', 0, 0)",
            [id],
        )
        .unwrap();
    }
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    let clock: SharedClock = Arc::new(SystemClock);
    let query = Arc::new(QueryService::new(Arc::clone(&pool), Arc::clone(&clock)));
    let search = SearchService::start(
        Arc::clone(&pool),
        Arc::clone(&clock),
        dir.path().join("indexes").join("tantivy"),
        query,
    );
    let user = Arc::new(UserData::new(Arc::clone(&pool), clock));
    let fx = Fx {
        _dir: dir,
        pool,
        user,
        search,
    };
    wait_total(&fx, &request(vec![]), asset_count);
    fx
}

fn request(user_fields: Vec<UserFieldFilter>) -> SearchRequest {
    SearchRequest {
        query: String::new(),
        filters: SearchFilters {
            media_types: vec![],
            include_offline: true,
            include_missing: false,
            captured_from: None,
            captured_to: None,
            size_min: None,
            size_max: None,
            duration_min_ms: None,
            duration_max_ms: None,
            folder: None,
            favorites_only: false,
            collection_id: None,
            tag_ids: vec![],
            rating_min: None,
            rating_max: None,
            user_fields,
        },
        sort: SortSpec {
            field: SortField::Name,
            direction: SortDirection::Asc,
        },
        limit: 50,
        cursor: None,
        mode: None,
        direction: None,
        at: None,
    }
}

/// 색인이 편집을 반영해 `total` 이 기대값이 될 때까지 기다린다 (최대 30초).
fn wait_total(fx: &Fx, req: &SearchRequest, expected: i64) -> SearchPage {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(p) = fx.search.search(req)
            && p.total == expected
        {
            return p;
        }
        assert!(
            Instant::now() < deadline,
            "index did not reach total {expected} in time"
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn ids(p: &SearchPage) -> Vec<i64> {
    p.items.iter().map(|i| i.asset_id).collect()
}

/// command 본문과 같은 경로: `run_blocking` -> 서비스 호출 -> `CommandError` 변환
fn call<T: Send + 'static>(
    fx: &Fx,
    f: impl FnOnce(&UserData) -> Result<T, IngestError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::block_on(run_blocking(Arc::clone(&fx.user), f))
}

fn search_code(fx: &Fx, conds: Vec<UserFieldFilter>) -> ErrorCode {
    let e: SearchFailure = fx
        .search
        .search(&request(conds))
        .expect_err("must be rejected");
    CommandError::from(e).code()
}

fn text_field(fx: &Fx, name: &str) -> i64 {
    fx.user
        .create_metadata_field(name, ValueType::Keyword, &[])
        .unwrap()
        .field_id
}

fn set_keyword(fx: &Fx, asset: i64, field: i64, v: &str) {
    fx.user
        .set_metadata_values(
            asset,
            &[(field, FieldPatch::Set(FieldValue::Keyword(v.into())))],
        )
        .unwrap();
    fx.search.wake();
}

#[test]
fn t_ipc_umeta_01_search_filters_user_fields_json_round_trips_and_old_json_still_parses() {
    // Arrange: 모든 종류의 조건 (작업 문서 D3 에 기록된 JSON 모양)
    let src = json!([
        { "kind": "keywordAnyOf", "fieldId": 1, "values": ["alpha"] },
        { "kind": "numberRange", "fieldId": 2, "min": 1.5, "max": null },
        { "kind": "dateRange", "fieldId": 3, "min": 20261101, "max": 20261130 },
        { "kind": "bool", "fieldId": 4, "value": true },
        { "kind": "enumAnyOf", "fieldId": 5, "optionIds": [1, 2] },
        { "kind": "multiEnum", "fieldId": 6, "optionIds": [3, 4], "all": true },
        { "kind": "has", "fieldId": 1 },
        { "kind": "missing", "fieldId": 1 }
    ]);
    let mut filters = serde_json::to_value(request(vec![]).filters).unwrap();
    // Act: 키가 비어 있으면 직렬화되지 않는다 (기존 최근 검색 JSON 과 동일 -> 중복 기록 방지)
    assert!(filters.get("userFields").is_none());
    let old_json = filters.to_string();
    filters["userFields"] = src.clone();
    let parsed: SearchFilters = serde_json::from_value(filters).unwrap();
    let old: SearchFilters = serde_json::from_str(&old_json).unwrap();

    // Assert
    assert_eq!(parsed.user_fields.len(), 8);
    assert_eq!(serde_json::to_value(&parsed.user_fields).unwrap(), src);
    assert!(old.user_fields.is_empty() && old.is_default());
    assert!(!parsed.is_default());
    // 모르는 kind·snake_case 키는 인자 변환 단계에서 거부된다
    for bad in [
        json!({ "kind": "contains", "fieldId": 1 }),
        json!({ "kind": "has", "field_id": 1 }),
        json!({ "kind": "has" }),
    ] {
        assert!(
            serde_json::from_value::<UserFieldFilter>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn t_ipc_umeta_02_command_wire_shapes_match_the_ts_contract() {
    // Arrange
    let fx = fixture(1);
    let created = call(&fx, |u| {
        u.create_metadata_field(
            "프로젝트",
            ValueType::MultiEnum,
            &["봄".to_string(), "여름".to_string()],
        )
        .map(MetadataFieldView::from)
    })
    .unwrap();
    let opt = created.options[0].option_id;

    // Act + Assert: 필드 보기
    let v = serde_json::to_value(&created).unwrap();
    assert_eq!(v["valueType"], "multiEnum");
    assert_eq!(v["fieldId"], created.field_id);
    assert_eq!(
        v["options"][0],
        json!({ "optionId": opt, "label": "봄", "position": 0 })
    );
    for k in ["name", "position", "createdAt", "updatedAt"] {
        assert!(v.get(k).is_some(), "{k}");
    }
    // 값: { kind, value } 로 타입을 함께 싣는다
    assert_eq!(
        serde_json::to_value(FieldValueDto::Date(20261120)).unwrap(),
        json!({ "kind": "date", "value": 20261120 })
    );
    assert_eq!(
        serde_json::to_value(FieldValueDto::MultiEnum(vec![1, 2])).unwrap(),
        json!({ "kind": "multiEnum", "value": [1, 2] })
    );
    // 패치 입력: set / clear
    let set: FieldPatchDto = serde_json::from_value(
        json!({ "op": "set", "fieldId": 3, "value": { "kind": "number", "value": 1.5 } }),
    )
    .unwrap();
    let clear: FieldPatchDto =
        serde_json::from_value(json!({ "op": "clear", "fieldId": 3 })).unwrap();
    assert_eq!(
        set,
        FieldPatchDto::Set {
            field_id: 3,
            value: FieldValueDto::Number(1.5)
        }
    );
    assert_eq!(clear, FieldPatchDto::Clear { field_id: 3 });
    for bad in [
        json!({ "op": "set", "fieldId": 3 }),
        json!({ "op": "toggle", "fieldId": 3 }),
        json!({ "op": "clear", "field_id": 3 }),
        json!({ "op": "set", "fieldId": 3, "value": { "kind": "text", "value": "x" } }),
    ] {
        assert!(
            serde_json::from_value::<FieldPatchDto>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    // 자산 값 보기 + 타입 문자열 입력
    call(&fx, move |u| {
        u.set_metadata_values(
            1,
            &[(
                created.field_id,
                FieldPatch::Set(FieldValue::MultiEnum(vec![opt])),
            )],
        )
        .map(|_| ())
    })
    .unwrap();
    let got = call(&fx, |u| {
        Ok(u.get_metadata_values(1)?
            .into_iter()
            .map(AssetFieldValueView::from)
            .collect::<Vec<_>>())
    })
    .unwrap();
    assert_eq!(
        serde_json::to_value(&got).unwrap(),
        json!([{ "fieldId": v["fieldId"], "value": { "kind": "multiEnum", "value": [opt] } }])
    );
    assert_eq!(
        serde_json::from_value::<ValueType>(json!("multiEnum")).unwrap(),
        ValueType::MultiEnum
    );
    assert!(serde_json::from_value::<ValueType>(json!("multi_enum")).is_err());
}

#[test]
fn t_ipc_umeta_03_conditions_are_checked_against_current_definitions() {
    // Arrange: keyword 필드, enum 필드(선택지 2), 삭제 요청한 필드
    let fx = fixture(2);
    let kw = text_field(&fx, "감독");
    let en = fx
        .user
        .create_metadata_field(
            "프로젝트",
            ValueType::Enum,
            &["봄".to_string(), "여름".to_string()],
        )
        .unwrap();
    let (o1, o2) = (en.options[0].option_id, en.options[1].option_id);
    let gone = text_field(&fx, "임시");
    fx.user.delete_metadata_field(gone).unwrap(); // 값 정리 전이어도 삭제 중 = 없는 필드
    fx.user.delete_metadata_option(o2).unwrap();

    // Act + Assert: 정상 조건은 통과
    for ok in [
        vec![UserFieldFilter::KeywordAnyOf {
            field_id: kw,
            values: vec!["x".into()],
        }],
        vec![UserFieldFilter::EnumAnyOf {
            field_id: en.field_id,
            option_ids: vec![o1],
        }],
        vec![
            UserFieldFilter::Has {
                field_id: en.field_id,
            },
            UserFieldFilter::Missing { field_id: kw },
        ],
    ] {
        assert!(fx.search.search(&request(ok)).is_ok());
    }
    // 거부: 없는 필드, 삭제 중인 필드, 음수 id, 삭제 중인 선택지, 모르는 선택지, 타입 불일치, 조건 과다
    let rejected: Vec<Vec<UserFieldFilter>> = vec![
        vec![UserFieldFilter::Has { field_id: 9_999 }],
        vec![UserFieldFilter::Missing { field_id: gone }],
        vec![UserFieldFilter::Has { field_id: -1 }],
        vec![UserFieldFilter::EnumAnyOf {
            field_id: en.field_id,
            option_ids: vec![o2],
        }],
        vec![UserFieldFilter::EnumAnyOf {
            field_id: en.field_id,
            option_ids: vec![o1, 9_999],
        }],
        vec![UserFieldFilter::NumberRange {
            field_id: kw,
            min: Some(1.0),
            max: None,
        }],
        vec![UserFieldFilter::KeywordAnyOf {
            field_id: en.field_id,
            values: vec!["x".into()],
        }],
        vec![UserFieldFilter::MultiEnum {
            field_id: en.field_id,
            option_ids: vec![o1],
            all: false,
        }],
        vec![UserFieldFilter::Has { field_id: kw }; mam_core::query::MAX_USER_FIELD_FILTERS + 1],
        // 구조 오류(mam-search 검증)도 invalid_input
        vec![UserFieldFilter::NumberRange {
            field_id: kw,
            min: None,
            max: None,
        }],
    ];
    for (i, conds) in rejected.into_iter().enumerate() {
        assert_eq!(search_code(&fx, conds), ErrorCode::InvalidInput, "case {i}");
    }
    // search_index 도 같은 필터 검증을 거친다
    let idx = mam_core::query::SearchIndexRequest {
        query: String::new(),
        filters: request(vec![UserFieldFilter::Has { field_id: 9_999 }]).filters,
        sort: SortSpec {
            field: SortField::Name,
            direction: SortDirection::Asc,
        },
        mode: None,
    };
    let e = fx.search.search_index(&idx).expect_err("must be rejected");
    assert_eq!(CommandError::from(e).code(), ErrorCode::InvalidInput);
}

#[test]
fn t_ipc_umeta_04_search_filtered_by_a_user_field_returns_the_right_asset_end_to_end() {
    // Arrange: 자산 1 에만 keyword 값, 자산 2 에는 enum 값
    let fx = fixture(3);
    let kw = text_field(&fx, "감독");
    let en = fx
        .user
        .create_metadata_field("프로젝트", ValueType::Enum, &["봄".to_string()])
        .unwrap();
    let opt = en.options[0].option_id;
    set_keyword(&fx, 1, kw, "  Kim Minsu ");
    fx.user
        .set_metadata_values(2, &[(en.field_id, FieldPatch::Set(FieldValue::Enum(opt)))])
        .unwrap();
    fx.search.wake();

    // Act + Assert: 대소문자·공백이 달라도 일치
    let by_kw = request(vec![UserFieldFilter::KeywordAnyOf {
        field_id: kw,
        values: vec!["kim minsu".into()],
    }]);
    assert_eq!(ids(&wait_total(&fx, &by_kw, 1)), vec![1]);
    let by_enum = request(vec![UserFieldFilter::EnumAnyOf {
        field_id: en.field_id,
        option_ids: vec![opt],
    }]);
    assert_eq!(ids(&wait_total(&fx, &by_enum, 1)), vec![2]);
    let missing = request(vec![UserFieldFilter::Missing { field_id: kw }]);
    assert_eq!(ids(&wait_total(&fx, &missing, 2)), vec![2, 3]);
    // 조건 없는 검색은 영향 없음 + AND 결합
    assert_eq!(fx.search.search(&request(vec![])).unwrap().total, 3);
    let both = request(vec![
        UserFieldFilter::Has { field_id: kw },
        UserFieldFilter::Has {
            field_id: en.field_id,
        },
    ]);
    assert_eq!(fx.search.search(&both).unwrap().total, 0);
}

#[test]
fn t_ipc_umeta_05_bulk_set_rejects_more_than_10000_assets_and_reports_changed_count() {
    // Arrange
    let fx = fixture(3);
    let kw = text_field(&fx, "감독");
    let patch = |v: &str| {
        vec![FieldPatchDto::Set {
            field_id: kw,
            value: FieldValueDto::Keyword(v.into()),
        }]
    };

    // Act + Assert: 10,001 건은 아무것도 쓰기 전에 invalid_input
    let too_many: Vec<i64> = (1..=10_001).collect();
    let p = to_patches(patch("x"));
    let err = call(&fx, move |u| u.bulk_set_metadata_values(&too_many, &p)).unwrap_err();
    assert_eq!(err.code(), ErrorCode::InvalidInput);
    assert!(call(&fx, |u| u.get_metadata_values(1)).unwrap().is_empty());
    // 정상: 바뀐 자산 수, 같은 값 재적용은 0
    let p = to_patches(patch("x"));
    let q = p.clone();
    assert_eq!(
        call(&fx, move |u| u.bulk_set_metadata_values(&[1, 2], &p)).unwrap(),
        2
    );
    assert_eq!(
        call(&fx, move |u| u.bulk_set_metadata_values(&[1, 2], &q)).unwrap(),
        0
    );
    // 없는 자산이 섞이면 not_found, 값 타입 불일치는 invalid_input
    let r = to_patches(patch("y"));
    assert_eq!(
        call(&fx, move |u| u.bulk_set_metadata_values(&[1, 99], &r))
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
    let bad = to_patches(vec![FieldPatchDto::Set {
        field_id: kw,
        value: FieldValueDto::Date(20261120),
    }]);
    assert_eq!(
        call(&fx, move |u| u.bulk_set_metadata_values(&[1], &bad))
            .unwrap_err()
            .code(),
        ErrorCode::InvalidInput
    );
    // 지우기
    let clear = to_patches(vec![FieldPatchDto::Clear { field_id: kw }]);
    assert_eq!(
        call(&fx, move |u| u.bulk_set_metadata_values(&[1, 2, 3], &clear)).unwrap(),
        2
    );
}

#[test]
fn t_ipc_umeta_06_deleting_a_field_hides_it_at_once_and_the_kicked_purge_removes_values() {
    // Arrange
    let fx = fixture(2);
    let kw = text_field(&fx, "감독");
    set_keyword(&fx, 1, kw, "Kim");
    set_keyword(&fx, 2, kw, "Lee");
    let has = request(vec![UserFieldFilter::Has { field_id: kw }]);
    wait_total(&fx, &has, 2);

    // Act: command 와 같은 순서 — 삭제 요청 -> 정리 스레드 시작(기다리지 않음)
    let affected = call(&fx, move |u| u.delete_metadata_field(kw)).unwrap();
    // 요청 직후: 목록·필터에서 이미 없는 필드
    assert!(call(&fx, |u| u.list_metadata_fields()).unwrap().is_empty());
    assert_eq!(
        search_code(&fx, vec![UserFieldFilter::Has { field_id: kw }]),
        ErrorCode::InvalidInput
    );
    let handle =
        kick_purge(Arc::clone(&fx.user), Some(Arc::clone(&fx.search))).expect("thread started");
    handle.join().unwrap();

    // Assert: 값·필드 행이 지워지고, 같은 이름을 다시 만들 수 있으며, 재호출은 멱등
    assert_eq!(affected, 2);
    let left: i64 = fx
        .pool
        .read(|c| Ok(c.query_row("SELECT (SELECT count(*) FROM asset_metadata_values) + (SELECT count(*) FROM metadata_fields)", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(left, 0);
    assert!(
        call(&fx, |u| u.create_metadata_field(
            "감독",
            ValueType::Keyword,
            &[]
        ))
        .is_ok()
    );
    assert_eq!(fx.user.purge_pending_metadata_deletions().unwrap(), 0);
    // 삭제한 필드의 값 조회는 없음
    assert!(call(&fx, |u| u.get_metadata_values(1)).unwrap().is_empty());
}

#[test]
fn t_ipc_umeta_07_definition_errors_map_to_contract_codes_and_option_delete_cleans_values() {
    // Arrange
    let fx = fixture(2);
    let en = fx
        .user
        .create_metadata_field(
            "프로젝트",
            ValueType::Enum,
            &["봄".to_string(), "여름".to_string()],
        )
        .unwrap();
    let (o1, o2) = (en.options[0].option_id, en.options[1].option_id);

    // Act + Assert: 중복 이름 = already_registered, 없는 id = not_found, 빈 이름·잘못된 옵션 = invalid_input
    let dup = call(&fx, |u| {
        u.create_metadata_field(" 프로젝트 ", ValueType::Keyword, &[])
    })
    .unwrap_err();
    assert_eq!(dup.code(), ErrorCode::AlreadyRegistered);
    assert_eq!(
        call(&fx, |u| u.rename_metadata_field(9_999, "x"))
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        call(&fx, |u| u.delete_metadata_option(9_999))
            .unwrap_err()
            .code(),
        ErrorCode::NotFound
    );
    assert_eq!(
        call(&fx, |u| u.create_metadata_field(
            "  ",
            ValueType::Keyword,
            &[]
        ))
        .unwrap_err()
        .code(),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        call(&fx, |u| u.create_metadata_field(
            "메모",
            ValueType::Keyword,
            &["a".to_string()]
        ))
        .unwrap_err()
        .code(),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        call(&fx, |u| u.reorder_metadata_fields(&[]))
            .unwrap_err()
            .code(),
        ErrorCode::InvalidInput
    );
    assert_eq!(
        call(&fx, move |u| u.add_metadata_option(en.field_id, "봄"))
            .unwrap_err()
            .code(),
        ErrorCode::AlreadyRegistered
    );
    // 선택지 사용 건수 -> 삭제 -> 정리 후 값이 지워지고 검색 필터에서도 사라진다
    fx.user
        .set_metadata_values(1, &[(en.field_id, FieldPatch::Set(FieldValue::Enum(o2)))])
        .unwrap();
    fx.user
        .set_metadata_values(2, &[(en.field_id, FieldPatch::Set(FieldValue::Enum(o1)))])
        .unwrap();
    assert_eq!(call(&fx, move |u| u.metadata_option_usage(o2)).unwrap(), 1);
    assert_eq!(
        call(&fx, move |u| u.metadata_field_asset_count(en.field_id)).unwrap(),
        2
    );
    assert_eq!(call(&fx, move |u| u.delete_metadata_option(o2)).unwrap(), 1);
    kick_purge(Arc::clone(&fx.user), Some(Arc::clone(&fx.search)))
        .unwrap()
        .join()
        .unwrap();
    assert!(call(&fx, |u| u.get_metadata_values(1)).unwrap().is_empty());
    assert_eq!(call(&fx, |u| u.get_metadata_values(2)).unwrap().len(), 1);
    assert_eq!(
        search_code(
            &fx,
            vec![UserFieldFilter::EnumAnyOf {
                field_id: en.field_id,
                option_ids: vec![o2]
            }]
        ),
        ErrorCode::InvalidInput
    );
}
