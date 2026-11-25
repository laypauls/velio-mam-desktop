//! 외부 AI 전송용 사용자 필드 표시 값 (D5, 계획 11).
//! 분류: Happy(타입별 표시 형식) / Boundary(값 없음) / Business rule(삭제 중 필드·선택지 제외, 선택지 이름·순서)

mod common;

use common::umeta::{Fx, fixture};
use mam_core::userdata::{FieldValue, ValueType, load_external_user_fields};

fn load(fx: &Fx, asset: i64) -> Vec<(String, String)> {
    fx.env
        .read(|c| load_external_user_fields(c, asset).unwrap())
}

#[test]
fn t_ext_umeta_01_display_values_per_type_in_field_order() {
    // Arrange
    let fx = fixture(1);
    let kw = fx.field("감독", ValueType::Keyword, &[]);
    let n_int = fx.field("횟수", ValueType::Number, &[]);
    let n_frac = fx.field("점수", ValueType::Number, &[]);
    let d = fx.field("촬영일", ValueType::Date, &[]);
    let b = fx.field("확정", ValueType::Boolean, &[]);
    let e = fx.field("등급", ValueType::Enum, &["상", "중", "하"]);
    let m = fx.field("태그", ValueType::MultiEnum, &["가", "나", "다"]);
    fx.set(1, kw.field_id, FieldValue::Keyword("김감독".into()));
    fx.set(1, n_int.field_id, FieldValue::Number(3.0));
    fx.set(1, n_frac.field_id, FieldValue::Number(4.25));
    fx.set(1, d.field_id, FieldValue::Date(20240305));
    fx.set(1, b.field_id, FieldValue::Boolean(false));
    fx.set(1, e.field_id, FieldValue::Enum(e.options[1].option_id));
    // 선택 순서(다, 가)와 무관하게 선택지 표시 순서로 나온다
    fx.set(
        1,
        m.field_id,
        FieldValue::MultiEnum(vec![m.options[2].option_id, m.options[0].option_id]),
    );
    // Act
    let got = load(&fx, 1);
    // Assert
    let want: Vec<(String, String)> = [
        ("감독", "김감독"),
        ("횟수", "3"),
        ("점수", "4.25"),
        ("촬영일", "2024-03-05"),
        ("확정", "false"),
        ("등급", "중"),
        ("태그", "가, 다"),
    ]
    .iter()
    .map(|(a, b)| (a.to_string(), b.to_string()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn t_ext_umeta_02_fields_without_value_are_excluded() {
    let fx = fixture(2);
    let a = fx.field("A", ValueType::Keyword, &[]);
    fx.field("B", ValueType::Number, &[]);
    fx.set(1, a.field_id, FieldValue::Keyword("x".into()));
    assert_eq!(load(&fx, 1), vec![("A".to_string(), "x".to_string())]);
    assert!(load(&fx, 2).is_empty(), "값이 하나도 없으면 빈 목록");
}

#[test]
fn t_ext_umeta_03_deleted_field_and_option_are_excluded() {
    // Arrange
    let fx = fixture(1);
    let a = fx.field("삭제될필드", ValueType::Keyword, &[]);
    let e = fx.field("등급", ValueType::Enum, &["상", "하"]);
    let m = fx.field("태그", ValueType::MultiEnum, &["가", "나"]);
    fx.set(1, a.field_id, FieldValue::Keyword("비밀".into()));
    fx.set(1, e.field_id, FieldValue::Enum(e.options[0].option_id));
    fx.set(
        1,
        m.field_id,
        FieldValue::MultiEnum(vec![m.options[0].option_id, m.options[1].option_id]),
    );
    // Act
    fx.ud.delete_metadata_field(a.field_id).unwrap();
    fx.ud
        .delete_metadata_option(e.options[0].option_id)
        .unwrap();
    fx.ud
        .delete_metadata_option(m.options[0].option_id)
        .unwrap();
    // Assert: 필드·선택지가 삭제 중이면 이름도 값도 나가지 않는다
    assert_eq!(load(&fx, 1), vec![("태그".to_string(), "나".to_string())]);
}
