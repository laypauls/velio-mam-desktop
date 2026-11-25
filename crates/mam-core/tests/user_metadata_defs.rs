//! 사용자 정의 메타데이터 필드 정의·선택지 (T-umeta-01~) — 실제 SQLite + 마이그레이션.
//!
//! 분류: Happy (01 생성·목록, 05 이름 변경, 06 순서, 08 선택지 편집) /
//! Boundary (02 NFC·대소문자 이름, 03 필드 20개 상한) / Validation (02 빈·중복 이름, 07 선택지 규칙) /
//! Business rule (03 상한 해제, 04 삭제 CASCADE·재색인 priority 1·이름 재사용, 09 선택지 삭제 정리, 10 타입 불변) /
//! Concurrency·멱등 (06 순서 실패 시 불변)

mod common;

use common::umeta::fixture;
use mam_core::scan::IngestError;
use mam_core::userdata::{FieldPatch, FieldValue, MAX_METADATA_FIELDS, ValueType};

fn invalid(e: IngestError) -> bool {
    matches!(e, IngestError::InvalidInput(_))
}

#[test]
fn t_umeta_01_create_and_list_in_creation_order_with_options() {
    // Arrange / Act
    let f = fixture(0);
    let a = f.field("프로젝트", ValueType::Enum, &["봄", "여름"]);
    let b = f.field("촬영 감독", ValueType::Keyword, &[]);
    let c = f.field("납품일", ValueType::Date, &[]);

    // Assert
    let list = f.ud.list_metadata_fields().unwrap();
    assert_eq!(list.iter().map(|x| x.field_id).collect::<Vec<_>>(), [a.field_id, b.field_id, c.field_id]);
    assert_eq!(list[0].value_type, ValueType::Enum);
    assert_eq!(list[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(), ["봄", "여름"]);
    assert_eq!(list.iter().map(|x| x.position).collect::<Vec<_>>(), [0, 1, 2]);
    assert!(list[1].options.is_empty());
}

#[test]
fn t_umeta_02_name_is_trimmed_nfc_lowercase_unique_and_non_empty() {
    let f = fixture(0);
    let a = f.field("  Café  ", ValueType::Keyword, &[]);
    assert_eq!(a.name, "Café");

    // 소문자·NFC(분해형 e + 결합 악센트) 가 달라도 같은 이름
    for dup in ["CAFÉ", "cafe\u{301}", "café"] {
        let e = f.ud.create_metadata_field(dup, ValueType::Number, &[]).unwrap_err();
        assert!(matches!(e, IngestError::AlreadyRegistered), "{dup}");
    }
    for bad in ["", "   ", "a\u{0}b"] {
        let e = f.ud.create_metadata_field(bad, ValueType::Number, &[]).unwrap_err();
        assert!(invalid(e), "{bad:?}");
    }
    let too_long = "x".repeat(101);
    assert!(invalid(f.ud.create_metadata_field(&too_long, ValueType::Number, &[]).unwrap_err()));
    assert_eq!(f.ud.list_metadata_fields().unwrap().len(), 1);
}

#[test]
fn t_umeta_03_twenty_first_field_is_rejected_until_one_is_deleted() {
    let f = fixture(0);
    let mut ids = Vec::new();
    for i in 0..MAX_METADATA_FIELDS {
        ids.push(f.field(&format!("f{i}"), ValueType::Boolean, &[]).field_id);
    }

    let e = f.ud.create_metadata_field("one more", ValueType::Boolean, &[]).unwrap_err();

    assert!(invalid(e));
    assert_eq!(f.ud.list_metadata_fields().unwrap().len(), 20);
    f.ud.delete_metadata_field(ids[3]).unwrap();
    f.field("one more", ValueType::Boolean, &[]);
    assert_eq!(f.ud.list_metadata_fields().unwrap().len(), 20);
}

#[test]
fn t_umeta_04_delete_removes_values_options_bumps_affected_low_priority_and_frees_the_name() {
    // Arrange: 자산 1·2 는 값이 있고 3 은 없다. 다른 필드 값은 보존돼야 한다.
    let f = fixture(3);
    let proj = f.field("프로젝트", ValueType::Enum, &["봄"]);
    let other = f.field("메모", ValueType::Keyword, &[]);
    let opt = proj.options[0].option_id;
    f.set(1, proj.field_id, FieldValue::Enum(opt));
    f.set(2, proj.field_id, FieldValue::Enum(opt));
    f.set(1, other.field_id, FieldValue::Keyword("keep".into()));
    let (r1, r2, r3) = (f.revision(1), f.revision(2), f.revision(3));
    assert_eq!(f.ud.metadata_field_asset_count(proj.field_id).unwrap(), 2);

    // Act: 요청 + 청크 정리까지 (요청 직후 상태는 t_umeta_40~ 에서 따로 고정한다)
    let affected = f.ud.delete_metadata_field(proj.field_id).unwrap();
    f.ud.purge_pending_metadata_deletions().unwrap();

    // Assert
    assert_eq!(affected, 2);
    assert_eq!((f.revision(1), f.revision(2), f.revision(3)), (r1 + 1, r2 + 1, r3));
    assert_eq!(f.outbox(1).last(), Some(&(r1 + 1, 1)), "삭제 재색인은 priority 1");
    assert_eq!(f.outbox(2).last(), Some(&(r2 + 1, 1)));
    assert!(f.outbox(3).is_empty());
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_field_options"), 0);
    assert_eq!(f.value_rows(), 1);
    assert_eq!(f.get(1, other.field_id), Some(FieldValue::Keyword("keep".into())));
    assert!(matches!(f.ud.metadata_field_asset_count(proj.field_id), Err(IngestError::NotFound)));
    // 이름 즉시 재사용 + field_id 는 재발급되지 않는다
    let again = f.field("프로젝트", ValueType::Number, &[]);
    assert!(again.field_id > other.field_id);
    assert!(matches!(f.ud.delete_metadata_field(proj.field_id), Err(IngestError::NotFound)));
}

#[test]
fn t_umeta_05_rename_keeps_type_values_and_revision_and_rejects_duplicates() {
    let f = fixture(1);
    let a = f.field("Alpha", ValueType::Number, &[]);
    let b = f.field("Beta", ValueType::Number, &[]);
    f.set(1, a.field_id, FieldValue::Number(1.5));
    let (rev, events) = (f.revision(1), f.outbox(1).len());

    let renamed = f.ud.rename_metadata_field(a.field_id, "ALPHA 2").unwrap();
    let same_key = f.ud.rename_metadata_field(a.field_id, "alpha 2").unwrap();
    let dup = f.ud.rename_metadata_field(a.field_id, "beta").unwrap_err();
    let empty = f.ud.rename_metadata_field(a.field_id, " ").unwrap_err();
    let missing = f.ud.rename_metadata_field(999, "x").unwrap_err();

    assert_eq!(renamed.name, "ALPHA 2");
    assert_eq!(same_key.name, "alpha 2");
    assert_eq!(same_key.value_type, ValueType::Number, "타입은 이름 변경으로 바뀌지 않는다 (타입 변경 API 없음)");
    assert!(matches!(dup, IngestError::AlreadyRegistered));
    assert!(invalid(empty));
    assert!(matches!(missing, IngestError::NotFound));
    assert_eq!((f.revision(1), f.outbox(1).len()), (rev, events), "이름 변경은 재색인 없음");
    assert_eq!(f.get(1, a.field_id), Some(FieldValue::Number(1.5)));
    assert_eq!(f.ud.list_metadata_fields().unwrap()[1].name, b.name);
}

#[test]
fn t_umeta_06_reorder_requires_exactly_the_current_ids_and_is_atomic() {
    let f = fixture(0);
    let a = f.field("a", ValueType::Keyword, &[]).field_id;
    let b = f.field("b", ValueType::Keyword, &[]).field_id;
    let c = f.field("c", ValueType::Keyword, &[]).field_id;
    let order = |f: &common::umeta::Fx| {
        f.ud.list_metadata_fields().unwrap().iter().map(|x| x.field_id).collect::<Vec<_>>()
    };

    f.ud.reorder_metadata_fields(&[c, a, b]).unwrap();
    assert_eq!(order(&f), [c, a, b]);

    for bad in [vec![a, b], vec![a, a, b], vec![a, b, c, 99], vec![a, b, 99], vec![]] {
        let e = f.ud.reorder_metadata_fields(&bad).unwrap_err();
        assert!(invalid(e), "{bad:?}");
    }
    assert_eq!(order(&f), [c, a, b], "실패한 순서 변경은 아무것도 바꾸지 않는다");
    // 새 필드는 맨 뒤
    let d = f.field("d", ValueType::Keyword, &[]).field_id;
    assert_eq!(order(&f), [c, a, b, d]);
}

#[test]
fn t_umeta_07_options_only_for_enum_types_and_labels_unique() {
    let f = fixture(0);

    let e1 = f.ud.create_metadata_field("k", ValueType::Keyword, &["x".into()]).unwrap_err();
    let e2 = f.ud.create_metadata_field("e", ValueType::Enum, &["A".into(), "a".into()]).unwrap_err();
    let e3 = f.ud.create_metadata_field("e", ValueType::Enum, &["".into()]).unwrap_err();

    assert!(invalid(e1));
    assert!(matches!(e2, IngestError::AlreadyRegistered), "대소문자만 다른 선택지");
    assert!(invalid(e3));
    assert!(f.ud.list_metadata_fields().unwrap().is_empty(), "실패한 생성은 필드를 남기지 않는다");
    let kw = f.field("k", ValueType::Number, &[]);
    assert!(invalid(f.ud.add_metadata_option(kw.field_id, "x").unwrap_err()));
    assert!(matches!(f.ud.add_metadata_option(999, "x"), Err(IngestError::NotFound)));
    // 선택지 없이 만든 enum 에 나중에 추가
    let en = f.field("enum", ValueType::MultiEnum, &[]);
    f.ud.add_metadata_option(en.field_id, "x").unwrap();
}

#[test]
fn t_umeta_08_option_add_rename_reorder_without_reindex() {
    let f = fixture(1);
    let fld = f.field("p", ValueType::Enum, &["A", "B"]);
    let (a, b) = (fld.options[0].option_id, fld.options[1].option_id);
    f.set(1, fld.field_id, FieldValue::Enum(a));
    let (rev, events) = (f.revision(1), f.outbox(1).len());

    let c = f.ud.add_metadata_option(fld.field_id, "C").unwrap();
    let dup = f.ud.add_metadata_option(fld.field_id, " c ").unwrap_err();
    let renamed = f.ud.rename_metadata_option(a, "Alpha").unwrap();
    let clash = f.ud.rename_metadata_option(a, "b").unwrap_err();
    let case_only = f.ud.rename_metadata_option(a, "ALPHA").unwrap();
    f.ud.reorder_metadata_options(fld.field_id, &[c.option_id, b, a]).unwrap();
    let bad_order = f.ud.reorder_metadata_options(fld.field_id, &[a, b]).unwrap_err();

    assert_eq!(c.position, 2);
    assert!(matches!(dup, IngestError::AlreadyRegistered));
    assert_eq!(renamed.label, "Alpha");
    assert!(matches!(clash, IngestError::AlreadyRegistered));
    assert_eq!(case_only.label, "ALPHA");
    assert!(invalid(bad_order));
    let labels: Vec<_> =
        f.ud.list_metadata_fields().unwrap()[0].options.iter().map(|o| o.label.clone()).collect();
    assert_eq!(labels, ["C", "B", "ALPHA"]);
    assert_eq!((f.revision(1), f.outbox(1).len()), (rev, events));
    assert_eq!(f.get(1, fld.field_id), Some(FieldValue::Enum(a)), "이름·순서 변경은 값에 영향 없음");
}

#[test]
fn t_umeta_09_delete_option_clears_its_values_only_and_reindexes_low_priority() {
    // Arrange: enum(자산 1·2 가 X, 3 이 Y) + multi_enum(자산 1 이 X·Y)
    let f = fixture(4);
    let en = f.field("e", ValueType::Enum, &["X", "Y"]);
    let me = f.field("m", ValueType::MultiEnum, &["P", "Q"]);
    let (x, y) = (en.options[0].option_id, en.options[1].option_id);
    let (p, q) = (me.options[0].option_id, me.options[1].option_id);
    f.set(1, en.field_id, FieldValue::Enum(x));
    f.set(2, en.field_id, FieldValue::Enum(x));
    f.set(3, en.field_id, FieldValue::Enum(y));
    f.set(1, me.field_id, FieldValue::MultiEnum(vec![p, q]));
    f.set(4, me.field_id, FieldValue::MultiEnum(vec![q]));
    let revs: Vec<i64> = (1..=4).map(|i| f.revision(i)).collect();
    assert_eq!(f.ud.metadata_option_usage(x).unwrap(), 2);

    // Act
    let n = f.ud.delete_metadata_option(x).unwrap();
    let m = f.ud.delete_metadata_option(q).unwrap();
    f.ud.purge_pending_metadata_deletions().unwrap();

    // Assert: x 를 고른 1·2, q 를 고른 1·4 만 재색인 (1 은 두 번)
    assert_eq!((n, m), (2, 2));
    assert_eq!(f.get(1, en.field_id), None);
    assert_eq!(f.get(2, en.field_id), None);
    assert_eq!(f.get(3, en.field_id), Some(FieldValue::Enum(y)));
    assert_eq!(f.get(1, me.field_id), Some(FieldValue::MultiEnum(vec![p])));
    assert_eq!(f.get(4, me.field_id), None);
    assert_eq!(
        (1..=4).map(|i| f.revision(i)).collect::<Vec<_>>(),
        [revs[0] + 2, revs[1] + 1, revs[2], revs[3] + 1]
    );
    assert!(f.outbox(1).iter().rev().take(2).all(|e| e.1 == 1), "선택지 삭제 재색인은 priority 1");
    assert_eq!(f.outbox(3).len(), 1, "자산 3 은 재색인되지 않는다");
    assert!(matches!(f.ud.metadata_option_usage(x), Err(IngestError::NotFound)));
    assert!(matches!(f.ud.delete_metadata_option(x), Err(IngestError::NotFound)));
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_field_options WHERE option_id IN (1, 4)"), 0);
    // 삭제된 선택지는 더 이상 고를 수 없다
    let e = f
        .ud
        .set_metadata_values(3, &[(en.field_id, FieldPatch::Set(FieldValue::Enum(x)))])
        .unwrap_err();
    assert!(invalid(e));
}

#[test]
fn t_umeta_10_value_type_cannot_be_changed_through_the_repository() {
    // 타입 변경 API 가 없다 — 정의를 되읽어 타입이 생성 시점 그대로임을 고정한다.
    let f = fixture(0);
    let a = f.field("a", ValueType::Date, &[]);
    f.ud.rename_metadata_field(a.field_id, "b").unwrap();
    f.ud.reorder_metadata_fields(&[a.field_id]).unwrap();

    assert_eq!(f.ud.list_metadata_fields().unwrap()[0].value_type, ValueType::Date);
}
