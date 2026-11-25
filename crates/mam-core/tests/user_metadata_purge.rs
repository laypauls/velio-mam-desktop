//! 사용자 정의 메타데이터 필드·선택지 청크 삭제 (T-umeta-40~) — 실제 SQLite + 마이그레이션.
//!
//! 분류: Happy (41 청크 진행, 43 선택지 청크) / Boundary (41 청크 경계·정확한 배수, 47 multi_enum 한 자산 여러 행) /
//! Business rule (40 요청 직후 숨김·이름 재사용·쓰기 거부·상한, 42 revision·outbox) /
//! Concurrency·멱등 (44 대기 없음 재실행, 45 중단 후 재개)

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::umeta::{Fx, fixture};
use mam_core::scan::IngestError;
use mam_core::userdata::{
    FieldPatch, FieldValue, MAX_METADATA_FIELDS, METADATA_PURGE_CHUNK, UserData, ValueType,
};

const NO_PAUSE: Duration = Duration::ZERO;

fn restarted(f: &Fx) -> UserData {
    UserData::new(Arc::clone(&f.env.pool), f.env.clock.clone())
}

#[test]
fn t_umeta_40_requested_delete_hides_the_field_frees_the_name_and_rejects_writes() {
    // Arrange: 값이 있는 enum 필드 + 다른 필드로 상한(20)을 채운다
    let f = fixture(2);
    let doomed = f.field("프로젝트", ValueType::Enum, &["봄"]);
    let opt = doomed.options[0].option_id;
    f.set(1, doomed.field_id, FieldValue::Enum(opt));
    let keep = f.field("메모", ValueType::Keyword, &[]);
    f.set(1, keep.field_id, FieldValue::Keyword("keep".into()));
    for i in 0..MAX_METADATA_FIELDS - 2 {
        f.field(&format!("f{i}"), ValueType::Boolean, &[]);
    }
    assert!(f.ud.create_metadata_field("over", ValueType::Boolean, &[]).is_err());
    let rev = f.revision(1);

    // Act: 요청만 (정리 전)
    let affected = f.ud.delete_metadata_field(doomed.field_id).unwrap();

    // Assert: 목록·값·상한·이름
    assert_eq!(affected, 1);
    let list = f.ud.list_metadata_fields().unwrap();
    assert!(list.iter().all(|x| x.field_id != doomed.field_id));
    assert_eq!(f.get(1, doomed.field_id), None);
    assert_eq!(f.get(1, keep.field_id), Some(FieldValue::Keyword("keep".into())));
    let again = f.field("프로젝트", ValueType::Number, &[]);
    assert!(again.field_id > doomed.field_id, "20개 중 하나가 삭제 중이라 새로 만들 수 있다");
    assert!(f.ud.create_metadata_field("over", ValueType::Boolean, &[]).is_err(), "다시 20개");
    // 쓰기·옵션 작업·재삭제는 없는 필드로 거부
    let set = f.ud.set_metadata_values(2, &[(doomed.field_id, FieldPatch::Set(FieldValue::Enum(opt)))]);
    let clear = f.ud.bulk_set_metadata_values(&[1, 2], &[(doomed.field_id, FieldPatch::Clear)]);
    assert!(matches!(set, Err(IngestError::NotFound)));
    assert!(matches!(clear, Err(IngestError::NotFound)));
    assert!(matches!(f.ud.rename_metadata_field(doomed.field_id, "x"), Err(IngestError::NotFound)));
    assert!(matches!(f.ud.add_metadata_option(doomed.field_id, "여름"), Err(IngestError::NotFound)));
    assert!(matches!(f.ud.delete_metadata_option(opt), Err(IngestError::NotFound)));
    assert!(matches!(f.ud.metadata_option_usage(opt), Err(IngestError::NotFound)));
    assert!(matches!(f.ud.metadata_field_asset_count(doomed.field_id), Err(IngestError::NotFound)));
    assert!(matches!(f.ud.delete_metadata_field(doomed.field_id), Err(IngestError::NotFound)));
    // 표시 순서 변경은 살아 있는 필드만 요구한다
    let live: Vec<i64> = list.iter().rev().map(|x| x.field_id).chain([again.field_id]).collect();
    f.ud.reorder_metadata_fields(&live).unwrap();
    assert_eq!(f.revision(1), rev, "정리 전에는 재색인 없음");
    assert_eq!(f.value_rows(), 2, "값은 정리 전까지 남아 있다");
}

#[test]
fn t_umeta_41_field_purge_runs_in_chunks_until_the_row_is_gone() {
    // Arrange: 자산 5개, chunk=2 -> 2, 2, 1(마지막, 필드 행도 삭제)
    let f = fixture(5);
    let fld = f.field("k", ValueType::Keyword, &[]);
    for a in 1..=5 {
        f.set(a, fld.field_id, FieldValue::Keyword(format!("v{a}")));
    }
    let revs: Vec<i64> = (1..=5).map(|a| f.revision(a)).collect();
    f.ud.delete_metadata_field(fld.field_id).unwrap();

    // Act / Assert
    assert_eq!(f.ud.purge_metadata_chunk(2).unwrap(), Some(2));
    assert_eq!(f.value_rows(), 3);
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_fields"), 1, "아직 필드 행이 남아 있다");
    assert_eq!(f.ud.purge_metadata_chunk(2).unwrap(), Some(2));
    assert_eq!(f.ud.purge_metadata_chunk(2).unwrap(), Some(1));
    assert_eq!(f.value_rows(), 0);
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_fields"), 0);
    assert_eq!(f.ud.purge_metadata_chunk(2).unwrap(), None);
    // 자산마다 정확히 한 번 revision 증가 + priority 1
    for a in 1..=5i64 {
        let i = (a - 1) as usize;
        assert_eq!(f.revision(a), revs[i] + 1, "asset {a}");
        assert_eq!(f.outbox(a).last(), Some(&(revs[i] + 1, 1)), "asset {a}");
    }
}

#[test]
fn t_umeta_41b_exact_multiple_of_chunk_needs_one_empty_chunk_to_finish_and_zero_chunk_is_clamped() {
    let f = fixture(4);
    let fld = f.field("k", ValueType::Number, &[]);
    for a in 1..=4 {
        f.set(a, fld.field_id, FieldValue::Number(1.0));
    }
    f.ud.delete_metadata_field(fld.field_id).unwrap();

    assert_eq!(f.ud.purge_metadata_chunk(2).unwrap(), Some(2));
    assert_eq!(f.ud.purge_metadata_chunk(2).unwrap(), Some(2));
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_fields"), 1);
    assert_eq!(f.ud.purge_metadata_chunk(2).unwrap(), Some(0));
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_fields"), 0);

    // chunk 0 은 1 로 보정 — 무한 루프 방지
    let g = fixture(2);
    let fld = g.field("k", ValueType::Number, &[]);
    g.set(1, fld.field_id, FieldValue::Number(1.0));
    g.set(2, fld.field_id, FieldValue::Number(1.0));
    g.ud.delete_metadata_field(fld.field_id).unwrap();
    assert_eq!(g.ud.purge_pending_metadata_deletions_with(0, NO_PAUSE).unwrap(), 2);
    assert_eq!(g.env.count("SELECT count(*) FROM metadata_fields"), 0);
}

#[test]
fn t_umeta_42_production_chunk_is_5000_and_purge_of_an_empty_field_just_removes_it() {
    assert_eq!(METADATA_PURGE_CHUNK, 5_000);
    let f = fixture(1);
    let fld = f.field("empty", ValueType::Enum, &["a"]);
    f.ud.delete_metadata_field(fld.field_id).unwrap();

    let n = f.ud.purge_pending_metadata_deletions().unwrap();

    assert_eq!(n, 0);
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_fields"), 0);
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_field_options"), 0, "선택지 CASCADE");
    assert_eq!(f.outbox_total(), 0);
}

#[test]
fn t_umeta_43_option_delete_hides_values_frees_the_label_and_purges_in_chunks() {
    // Arrange: enum X 를 5개 자산이, Y 를 자산 6 이 고르고, multi_enum 자산 1 은 P·X' 둘 다 고른다
    let f = fixture(6);
    let en = f.field("e", ValueType::Enum, &["X", "Y"]);
    let me = f.field("m", ValueType::MultiEnum, &["P", "Q"]);
    let (x, y) = (en.options[0].option_id, en.options[1].option_id);
    let (p, q) = (me.options[0].option_id, me.options[1].option_id);
    for a in 1..=5 {
        f.set(a, en.field_id, FieldValue::Enum(x));
    }
    f.set(6, en.field_id, FieldValue::Enum(y));
    f.set(1, me.field_id, FieldValue::MultiEnum(vec![p, q]));
    let revs: Vec<i64> = (1..=6).map(|a| f.revision(a)).collect();

    // Act 1: 요청 직후
    assert_eq!(f.ud.delete_metadata_option(x).unwrap(), 5);
    assert_eq!(f.ud.delete_metadata_option(q).unwrap(), 1);

    // Assert 1: 읽기에서는 이미 없다, 다른 값은 그대로
    assert_eq!(f.get(1, en.field_id), None);
    assert_eq!(f.get(5, en.field_id), None);
    assert_eq!(f.get(6, en.field_id), Some(FieldValue::Enum(y)));
    assert_eq!(f.get(1, me.field_id), Some(FieldValue::MultiEnum(vec![p])));
    let labels: Vec<_> = f.ud.list_metadata_fields().unwrap()[0].options.iter().map(|o| o.label.clone()).collect();
    assert_eq!(labels, ["Y"]);
    assert!(matches!(f.ud.metadata_option_usage(x), Err(IngestError::NotFound)));
    assert!(matches!(f.ud.rename_metadata_option(x, "z"), Err(IngestError::NotFound)));
    let set = f.ud.set_metadata_values(6, &[(en.field_id, FieldPatch::Set(FieldValue::Enum(x)))]);
    assert!(matches!(set, Err(IngestError::InvalidInput(_))), "삭제 중 선택지는 고를 수 없다");
    let readded = f.ud.add_metadata_option(en.field_id, "X").unwrap();
    assert!(readded.option_id > q, "같은 이름을 새 선택지로 즉시 추가");
    assert_eq!(f.revision(1), revs[0], "정리 전에는 재색인 없음");

    // Act 2: chunk=2 — X(자산 1..5) 2+2+1, 그 다음 Q(자산 1)
    let mut done = Vec::new();
    while let Some(n) = f.ud.purge_metadata_chunk(2).unwrap() {
        done.push(n);
    }

    // Assert 2
    assert_eq!(done, [2, 2, 1, 1]);
    assert_eq!(f.env.count(&format!("SELECT count(*) FROM metadata_field_options WHERE option_id IN ({x}, {q})")), 0);
    assert_eq!(f.env.count(&format!("SELECT count(*) FROM asset_metadata_values WHERE value_int = {x}")), 0);
    assert_eq!(f.get(6, en.field_id), Some(FieldValue::Enum(y)), "Y 는 보존");
    assert_eq!(f.get(1, me.field_id), Some(FieldValue::MultiEnum(vec![p])));
    // 자산 1 은 X·Q 두 번 삭제 -> +2, 2~5 는 X 한 번 -> +1, 6 은 그대로
    let now: Vec<i64> = (1..=6).map(|a| f.revision(a)).collect();
    assert_eq!(now, [revs[0] + 2, revs[1] + 1, revs[2] + 1, revs[3] + 1, revs[4] + 1, revs[5]]);
    assert!(f.outbox(2).last().is_some_and(|e| e.1 == 1), "priority 1");
    assert_eq!(f.outbox(6).len(), 1);
}

#[test]
fn t_umeta_44_purge_with_nothing_pending_is_a_noop() {
    let f = fixture(2);
    let fld = f.field("k", ValueType::Keyword, &[]);
    f.set(1, fld.field_id, FieldValue::Keyword("v".into()));
    let (rows, events, rev) = (f.value_rows(), f.outbox_total(), f.revision(1));

    assert_eq!(f.ud.purge_metadata_chunk(10).unwrap(), None);
    assert_eq!(f.ud.purge_pending_metadata_deletions().unwrap(), 0);
    assert_eq!(f.ud.purge_pending_metadata_deletions().unwrap(), 0);

    assert_eq!((f.value_rows(), f.outbox_total(), f.revision(1)), (rows, events, rev));
    // 삭제 후 한 번 더 호출해도 두 번 처리되지 않는다
    f.ud.delete_metadata_field(fld.field_id).unwrap();
    assert_eq!(f.ud.purge_pending_metadata_deletions().unwrap(), 1);
    assert_eq!(f.ud.purge_pending_metadata_deletions().unwrap(), 0);
    assert_eq!(f.revision(1), rev + 1);
}

#[test]
fn t_umeta_45_interrupted_purge_resumes_after_restart_without_double_bumping() {
    // Arrange: 필드 삭제 + 선택지 삭제가 둘 다 대기
    let f = fixture(5);
    let fld = f.field("k", ValueType::Keyword, &[]);
    let en = f.field("e", ValueType::Enum, &["X"]);
    let x = en.options[0].option_id;
    for a in 1..=5 {
        f.set(a, fld.field_id, FieldValue::Keyword("v".into()));
    }
    f.set(1, en.field_id, FieldValue::Enum(x));
    f.set(2, en.field_id, FieldValue::Enum(x));
    let revs: Vec<i64> = (1..=5).map(|a| f.revision(a)).collect();
    f.ud.delete_metadata_field(fld.field_id).unwrap();
    f.ud.delete_metadata_option(x).unwrap();

    // Act: 청크 하나만 처리하고 "종료" -> 새 인스턴스(재시작)가 이어서 끝낸다
    assert_eq!(f.ud.purge_metadata_chunk(2).unwrap(), Some(2));
    let ud = restarted(&f);
    let rest = ud.purge_pending_metadata_deletions_with(2, NO_PAUSE).unwrap();

    // Assert: 필드 3개 자산 + 선택지 2개 자산
    assert_eq!(rest, 3 + 2);
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_fields WHERE deleting_at IS NOT NULL"), 0);
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_field_options WHERE deleting_at IS NOT NULL"), 0);
    assert_eq!(f.value_rows(), 0);
    let now: Vec<i64> = (1..=5).map(|a| f.revision(a)).collect();
    assert_eq!(now, [revs[0] + 2, revs[1] + 2, revs[2] + 1, revs[3] + 1, revs[4] + 1]);
    assert_eq!(ud.purge_pending_metadata_deletions().unwrap(), 0);
}

#[test]
fn t_umeta_46_user_edits_stay_possible_between_chunks() {
    // 청크 사이에 다른 필드 편집이 막히지 않는다 (잠금을 청크 동안만 잡는다)
    let f = fixture(4);
    let fld = f.field("k", ValueType::Keyword, &[]);
    let other = f.field("o", ValueType::Keyword, &[]);
    for a in 1..=4 {
        f.set(a, fld.field_id, FieldValue::Keyword("v".into()));
    }
    f.ud.delete_metadata_field(fld.field_id).unwrap();
    f.ud.purge_metadata_chunk(2).unwrap();

    f.set(1, other.field_id, FieldValue::Keyword("edit".into()));
    f.ud.purge_pending_metadata_deletions_with(2, NO_PAUSE).unwrap();

    assert_eq!(f.get(1, other.field_id), Some(FieldValue::Keyword("edit".into())));
    assert_eq!(f.env.count("SELECT count(*) FROM metadata_fields"), 1);
}

#[test]
fn t_umeta_47_multi_enum_rows_of_one_asset_count_once_and_are_all_removed() {
    let f = fixture(2);
    let me = f.field("m", ValueType::MultiEnum, &["a", "b", "c"]);
    let ids: Vec<i64> = me.options.iter().map(|o| o.option_id).collect();
    f.set(1, me.field_id, FieldValue::MultiEnum(ids.clone()));
    f.set(2, me.field_id, FieldValue::MultiEnum(vec![ids[0]]));
    assert_eq!(f.value_rows(), 4);
    let r1 = f.revision(1);

    assert_eq!(f.ud.delete_metadata_field(me.field_id).unwrap(), 2, "행이 아니라 자산 수");
    let n = f.ud.purge_pending_metadata_deletions_with(1, NO_PAUSE).unwrap();

    assert_eq!(n, 2);
    assert_eq!(f.value_rows(), 0);
    assert_eq!(f.revision(1), r1 + 1, "행이 3개여도 revision 은 한 번");
}
