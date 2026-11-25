//! 사용자 정의 메타데이터 값 저장·대량 지정 (T-umeta-2x~) — 실제 SQLite + 마이그레이션.
//!
//! 분류: Happy (20 타입별 저장·조회, 30 대량 덮어쓰기) / Boundary (21 keyword 200·201자, 25 날짜 달력 경계,
//! 33 1,000건 priority 경계, 34 10,000/10,001건) / Validation (22 NaN·Inf, 23 타입 불일치, 24 선택지 소속,
//! 26 multi_enum 중복·빈 값) / Business rule (27 revision·outbox 같은 트랜잭션·no-op, 31 미지정 필드 보존,
//! 32 지우기, 35 색인용 읽기) / Concurrency·멱등 (29 롤백, 36 재적용)

mod common;

use common::umeta::fixture;
use mam_core::scan::IngestError;
use mam_core::userdata::{FieldPatch, FieldValue, ValueType};

fn set(v: FieldValue) -> FieldPatch {
    FieldPatch::Set(v)
}

fn invalid(e: IngestError) -> bool {
    matches!(e, IngestError::InvalidInput(_))
}

#[test]
fn t_umeta_20_every_type_round_trips() {
    let f = fixture(1);
    let kw = f.field("kw", ValueType::Keyword, &[]).field_id;
    let num = f.field("num", ValueType::Number, &[]).field_id;
    let date = f.field("date", ValueType::Date, &[]).field_id;
    let flag = f.field("flag", ValueType::Boolean, &[]).field_id;
    let en = f.field("en", ValueType::Enum, &["A"]);
    let me = f.field("me", ValueType::MultiEnum, &["P", "Q"]);
    let (a, p, q) = (
        en.options[0].option_id,
        me.options[0].option_id,
        me.options[1].option_id,
    );

    f.ud.set_metadata_values(
        1,
        &[
            (kw, set(FieldValue::Keyword("김 감독".into()))),
            (num, set(FieldValue::Number(-12.5))),
            (date, set(FieldValue::Date(20261120))),
            (flag, set(FieldValue::Boolean(false))),
            (en.field_id, set(FieldValue::Enum(a))),
            (me.field_id, set(FieldValue::MultiEnum(vec![q, p]))),
        ],
    )
    .unwrap();

    assert_eq!(f.get(1, kw), Some(FieldValue::Keyword("김 감독".into())));
    assert_eq!(f.get(1, num), Some(FieldValue::Number(-12.5)));
    assert_eq!(f.get(1, date), Some(FieldValue::Date(20261120)));
    assert_eq!(
        f.get(1, flag),
        Some(FieldValue::Boolean(false)),
        "false 도 '값 있음'"
    );
    assert_eq!(f.get(1, en.field_id), Some(FieldValue::Enum(a)));
    assert_eq!(
        f.get(1, me.field_id),
        Some(FieldValue::MultiEnum(vec![p, q]))
    );
    // multi_enum 은 ord = option_id, 단일값은 ord 0
    assert_eq!(
        f.env.count(&format!(
            "SELECT count(*) FROM asset_metadata_values WHERE field_id = {} AND ord = value_int",
            me.field_id
        )),
        2
    );
    assert_eq!(f.env.count(&format!("SELECT count(*) FROM asset_metadata_values WHERE field_id = {num} AND ord = 0 AND value_real = -12.5")), 1);
    assert_eq!(f.env.count(&format!("SELECT count(*) FROM asset_metadata_values WHERE field_id = {date} AND value_int = 20261120")), 1);
}

#[test]
fn t_umeta_21_keyword_is_trimmed_nfc_limited_to_200_and_empty_means_clear() {
    let f = fixture(1);
    let kw = f.field("kw", ValueType::Keyword, &[]).field_id;

    f.set(1, kw, FieldValue::Keyword("  e\u{301}  ".into()));
    assert_eq!(
        f.get(1, kw),
        Some(FieldValue::Keyword("\u{e9}".into())),
        "trim + NFC, 대소문자는 보존"
    );
    f.set(1, kw, FieldValue::Keyword("가".repeat(200)));
    assert!(f.get(1, kw).is_some(), "200자는 허용");
    let e =
        f.ud.set_metadata_values(1, &[(kw, set(FieldValue::Keyword("가".repeat(201))))])
            .unwrap_err();
    assert!(invalid(e), "201자 거부");
    assert!(
        f.get(1, kw).is_some(),
        "거부된 입력은 기존 값을 건드리지 않는다"
    );
    let e =
        f.ud.set_metadata_values(1, &[(kw, set(FieldValue::Keyword("a\nb".into())))])
            .unwrap_err();
    assert!(invalid(e), "제어 문자 거부");
    // 앞뒤 공백 포함 201자는 trim 후 200자라 허용
    f.set(
        1,
        kw,
        FieldValue::Keyword(format!("  {}  ", "x".repeat(200))),
    );
    for empty in ["", "   ", "\t \u{a0}"] {
        f.set(1, kw, FieldValue::Keyword("v".into()));
        f.set(1, kw, FieldValue::Keyword(empty.into()));
        assert_eq!(f.get(1, kw), None, "{empty:?}");
    }
    assert_eq!(f.value_rows(), 0);
}

#[test]
fn t_umeta_22_number_rejects_nan_and_infinity() {
    let f = fixture(1);
    let n = f.field("n", ValueType::Number, &[]).field_id;
    f.set(1, n, FieldValue::Number(0.0));

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let e =
            f.ud.set_metadata_values(1, &[(n, set(FieldValue::Number(bad)))])
                .unwrap_err();
        assert!(invalid(e), "{bad}");
    }
    assert_eq!(
        f.get(1, n),
        Some(FieldValue::Number(0.0)),
        "0 은 값 있음, 거부는 기존 값 보존"
    );
    f.set(1, n, FieldValue::Number(f64::MAX));
    f.set(1, n, FieldValue::Number(f64::MIN_POSITIVE));
    assert_eq!(f.get(1, n), Some(FieldValue::Number(f64::MIN_POSITIVE)));
}

#[test]
fn t_umeta_23_value_must_match_field_type_and_targets_must_exist() {
    let f = fixture(2);
    let num = f.field("n", ValueType::Number, &[]).field_id;
    let kw = f.field("k", ValueType::Keyword, &[]).field_id;
    let mismatches = [
        (num, FieldValue::Keyword("1".into())),
        (kw, FieldValue::Number(1.0)),
        (kw, FieldValue::Boolean(true)),
        (num, FieldValue::Date(20260101)),
        (kw, FieldValue::Enum(1)),
        (num, FieldValue::MultiEnum(vec![])),
    ];

    for (field, v) in mismatches {
        let e =
            f.ud.set_metadata_values(1, &[(field, set(v.clone()))])
                .unwrap_err();
        assert!(invalid(e), "{v:?}");
    }
    assert!(matches!(
        f.ud.set_metadata_values(1, &[(999, set(FieldValue::Number(1.0)))]),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        f.ud.set_metadata_values(99, &[(num, set(FieldValue::Number(1.0)))]),
        Err(IngestError::NotFound)
    ));
    assert!(invalid(
        f.ud.set_metadata_values(
            1,
            &[
                (num, FieldPatch::Clear),
                (num, set(FieldValue::Number(1.0)))
            ]
        )
        .unwrap_err()
    ));
    assert_eq!(f.value_rows(), 0);
    assert_eq!(f.outbox_total(), 0, "실패한 지정은 outbox 를 남기지 않는다");
}

#[test]
fn t_umeta_24_enum_option_must_belong_to_the_field() {
    let f = fixture(1);
    let a = f.field("a", ValueType::Enum, &["X"]);
    let b = f.field("b", ValueType::MultiEnum, &["Y"]);
    let (x, y) = (a.options[0].option_id, b.options[0].option_id);

    for (field, v) in [
        (a.field_id, FieldValue::Enum(y)),
        (a.field_id, FieldValue::Enum(9999)),
        (b.field_id, FieldValue::MultiEnum(vec![y, x])),
        (b.field_id, FieldValue::MultiEnum(vec![-1])),
    ] {
        let e =
            f.ud.set_metadata_values(1, &[(field, set(v.clone()))])
                .unwrap_err();
        assert!(invalid(e), "{v:?}");
    }
    assert_eq!(f.value_rows(), 0);
}

#[test]
fn t_umeta_25_date_must_be_a_real_calendar_date() {
    let f = fixture(1);
    let d = f.field("d", ValueType::Date, &[]).field_id;

    for bad in [
        20260229,
        20261301,
        20260001,
        20260431,
        0,
        -1,
        202_611_200,
        20260230,
    ] {
        let e =
            f.ud.set_metadata_values(1, &[(d, set(FieldValue::Date(bad)))])
                .unwrap_err();
        assert!(invalid(e), "{bad}");
    }
    assert_eq!(f.value_rows(), 0);
    f.set(1, d, FieldValue::Date(20240229));
    assert_eq!(f.get(1, d), Some(FieldValue::Date(20240229)));
}

#[test]
fn t_umeta_26_multi_enum_dedups_and_empty_set_clears() {
    let f = fixture(1);
    let m = f.field("m", ValueType::MultiEnum, &["P", "Q"]);
    let (p, q) = (m.options[0].option_id, m.options[1].option_id);

    f.set(1, m.field_id, FieldValue::MultiEnum(vec![q, p, q, p]));
    assert_eq!(
        f.get(1, m.field_id),
        Some(FieldValue::MultiEnum(vec![p, q])),
        "중복은 제거(dedup)"
    );
    assert_eq!(f.value_rows(), 2);
    f.set(1, m.field_id, FieldValue::MultiEnum(vec![q]));
    assert_eq!(
        f.get(1, m.field_id),
        Some(FieldValue::MultiEnum(vec![q])),
        "선택 해제된 선택지 행은 삭제"
    );
    f.set(1, m.field_id, FieldValue::MultiEnum(vec![]));
    assert_eq!(f.get(1, m.field_id), None);
    assert_eq!(f.value_rows(), 0);
}

#[test]
fn t_umeta_27_change_bumps_revision_and_outbox_at_priority_0_and_same_value_is_noop() {
    let f = fixture(2);
    let n = f.field("n", ValueType::Number, &[]).field_id;
    let (r1, r2) = (f.revision(1), f.revision(2));

    assert_eq!(
        f.ud.set_metadata_values(1, &[(n, set(FieldValue::Number(3.0)))])
            .unwrap(),
        1
    );
    assert_eq!(
        f.ud.set_metadata_values(1, &[(n, set(FieldValue::Number(3.0)))])
            .unwrap(),
        0
    );
    assert_eq!(
        f.ud.set_metadata_values(1, &[(n, FieldPatch::Clear)])
            .unwrap(),
        1
    );
    assert_eq!(
        f.ud.set_metadata_values(1, &[(n, FieldPatch::Clear)])
            .unwrap(),
        0
    );

    assert_eq!(f.revision(1), r1 + 2);
    assert_eq!(
        f.outbox(1),
        [(r1 + 1, 0), (r1 + 2, 0)],
        "사용자 편집 = priority 0, revision 은 단조 증가"
    );
    assert_eq!(f.revision(2), r2);
    assert!(f.outbox(2).is_empty());
}

#[test]
fn t_umeta_28_failed_patch_rolls_back_earlier_valid_patches_in_the_same_call() {
    let f = fixture(1);
    let n = f.field("n", ValueType::Number, &[]).field_id;
    let k = f.field("k", ValueType::Keyword, &[]).field_id;
    let rev = f.revision(1);

    let e =
        f.ud.set_metadata_values(
            1,
            &[
                (n, set(FieldValue::Number(1.0))),
                (k, set(FieldValue::Keyword("x".repeat(201)))),
            ],
        )
        .unwrap_err();

    assert!(invalid(e));
    assert_eq!(
        (f.value_rows(), f.revision(1), f.outbox_total()),
        (0, rev, 0)
    );
}

#[test]
fn t_umeta_30_bulk_overwrites_and_clears_and_leaves_unpatched_fields_alone() {
    // Arrange: 자산 1~3 에 서로 다른 값, 4 는 비어 있음 / 필드 3개
    let f = fixture(4);
    let proj = f.field("p", ValueType::Enum, &["봄", "여름"]);
    let who = f.field("w", ValueType::Keyword, &[]).field_id;
    let due = f.field("d", ValueType::Date, &[]).field_id;
    let (spring, summer) = (proj.options[0].option_id, proj.options[1].option_id);
    f.set(1, proj.field_id, FieldValue::Enum(spring));
    f.set(2, proj.field_id, FieldValue::Enum(summer));
    f.set(1, who, FieldValue::Keyword("김".into()));
    f.set(3, who, FieldValue::Keyword("이".into()));
    f.set(2, due, FieldValue::Date(20261101));

    // Act: 프로젝트는 덮어쓰기, 납품일은 지우기, 촬영 감독은 패치에 없음
    let changed =
        f.ud.bulk_set_metadata_values(
            &[1, 2, 3, 4, 3, 2],
            &[
                (proj.field_id, set(FieldValue::Enum(summer))),
                (due, FieldPatch::Clear),
            ],
        )
        .unwrap();

    // Assert: 자산 2 는 프로젝트가 이미 summer 이지만 납품일이 지워져 바뀜 / 1·3·4 는 프로젝트 변경 / 중복 id 는 한 번만
    assert_eq!(changed, 4);
    for id in 1..=4 {
        assert_eq!(
            f.get(id, proj.field_id),
            Some(FieldValue::Enum(summer)),
            "{id}"
        );
        assert_eq!(f.get(id, due), None, "{id}");
    }
    assert_eq!(
        f.get(1, who),
        Some(FieldValue::Keyword("김".into())),
        "미지정 필드 보존"
    );
    assert_eq!(f.get(3, who), Some(FieldValue::Keyword("이".into())));
    assert_eq!(f.get(2, who), None);
}

#[test]
fn t_umeta_31_bulk_only_bumps_assets_whose_values_actually_change() {
    let f = fixture(3);
    let n = f.field("n", ValueType::Number, &[]).field_id;
    f.set(1, n, FieldValue::Number(7.0));
    let revs: Vec<i64> = (1..=3).map(|i| f.revision(i)).collect();

    let changed =
        f.ud.bulk_set_metadata_values(&[1, 2], &[(n, set(FieldValue::Number(7.0)))])
            .unwrap();

    assert_eq!(changed, 1);
    assert_eq!(
        (f.revision(1), f.revision(2), f.revision(3)),
        (revs[0], revs[1] + 1, revs[2])
    );
    // 같은 지정을 다시 적용해도 no-op (멱등)
    let before = f.outbox_total();
    assert_eq!(
        f.ud.bulk_set_metadata_values(&[1, 2], &[(n, set(FieldValue::Number(7.0)))])
            .unwrap(),
        0
    );
    assert_eq!(f.outbox_total(), before);
}

#[test]
fn t_umeta_32_bulk_rejects_missing_assets_unknown_fields_and_duplicate_fields_atomically() {
    let f = fixture(3);
    let n = f.field("n", ValueType::Number, &[]).field_id;
    let patch = [(n, set(FieldValue::Number(1.0)))];

    assert!(matches!(
        f.ud.bulk_set_metadata_values(&[1, 2, 99], &patch),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        f.ud.bulk_set_metadata_values(
            &[1, 2],
            &[(n, set(FieldValue::Number(1.0))), (999, FieldPatch::Clear)]
        ),
        Err(IngestError::NotFound)
    ));
    assert!(invalid(
        f.ud.bulk_set_metadata_values(&[1], &[(n, FieldPatch::Clear), (n, FieldPatch::Clear)])
            .unwrap_err()
    ));
    assert_eq!(
        f.ud.bulk_set_metadata_values(&[], &patch).unwrap(),
        0,
        "빈 목록은 no-op"
    );
    assert_eq!(
        f.ud.bulk_set_metadata_values(&[1], &[]).unwrap(),
        0,
        "빈 패치는 no-op"
    );
    assert_eq!((f.value_rows(), f.outbox_total()), (0, 0));
}

#[test]
fn t_umeta_33_outbox_priority_is_0_up_to_1000_changed_assets_then_1() {
    // 정확히 1,000건: 전부 priority 0
    let f = fixture(1000);
    let n = f.field("n", ValueType::Number, &[]).field_id;
    f.ud.bulk_set_metadata_values(&f.ids, &[(n, set(FieldValue::Number(1.0)))])
        .unwrap();
    assert_eq!(
        f.env
            .count("SELECT count(*) FROM search_outbox WHERE priority = 0"),
        1000
    );
    assert_eq!(
        f.env
            .count("SELECT count(*) FROM search_outbox WHERE priority <> 0"),
        0
    );

    // 1,001건: 앞 1,000건(asset_id 오름차순) 0, 마지막 1건 1
    let f = fixture(1001);
    let n = f.field("n", ValueType::Number, &[]).field_id;
    f.ud.bulk_set_metadata_values(&f.ids, &[(n, set(FieldValue::Number(1.0)))])
        .unwrap();
    assert_eq!(
        f.env
            .count("SELECT count(*) FROM search_outbox WHERE priority = 0"),
        1000
    );
    assert_eq!(f.outbox(1001), [(2, 1)]);
    assert_eq!(f.outbox(1000), [(2, 0)]);
}

#[test]
fn t_umeta_34_bulk_accepts_10000_and_rejects_10001_before_touching_anything() {
    let f = fixture(10_000);
    let n = f.field("n", ValueType::Number, &[]).field_id;
    let patch = [(n, set(FieldValue::Number(2.0)))];
    let mut too_many = f.ids.clone();
    too_many.push(10_001);

    let e =
        f.ud.bulk_set_metadata_values(&too_many, &patch)
            .unwrap_err();
    assert!(invalid(e));
    assert_eq!((f.value_rows(), f.outbox_total()), (0, 0));

    assert_eq!(
        f.ud.bulk_set_metadata_values(&f.ids, &patch).unwrap(),
        10_000
    );
    assert_eq!(f.value_rows(), 10_000);
    assert_eq!(
        f.env
            .count("SELECT count(*) FROM search_outbox WHERE priority = 0"),
        1000
    );
    assert_eq!(
        f.env
            .count("SELECT count(*) FROM search_outbox WHERE priority = 1"),
        9000
    );
}

#[test]
fn t_umeta_35_load_asset_values_returns_typed_values_in_field_order() {
    let f = fixture(2);
    let en = f.field("en", ValueType::Enum, &["A"]);
    let me = f.field("me", ValueType::MultiEnum, &["P", "Q"]);
    let kw = f.field("kw", ValueType::Keyword, &[]).field_id;
    f.ud.reorder_metadata_fields(&[kw, me.field_id, en.field_id])
        .unwrap();
    f.ud.set_metadata_values(
        1,
        &[
            (en.field_id, set(FieldValue::Enum(en.options[0].option_id))),
            (
                me.field_id,
                set(FieldValue::MultiEnum(vec![
                    me.options[1].option_id,
                    me.options[0].option_id,
                ])),
            ),
            (kw, set(FieldValue::Keyword("Hello".into()))),
        ],
    )
    .unwrap();

    let loaded = f
        .env
        .read(|c| mam_core::userdata::load_asset_values(c, 1).unwrap());
    let empty = f
        .env
        .read(|c| mam_core::userdata::load_asset_values(c, 2).unwrap());

    let shape: Vec<_> = loaded.iter().map(|v| (v.field_id, v.value_type)).collect();
    assert_eq!(
        shape,
        [
            (kw, ValueType::Keyword),
            (me.field_id, ValueType::MultiEnum),
            (en.field_id, ValueType::Enum)
        ]
    );
    assert_eq!(
        loaded[1].value,
        FieldValue::MultiEnum(vec![me.options[0].option_id, me.options[1].option_id])
    );
    assert!(empty.is_empty());
}
