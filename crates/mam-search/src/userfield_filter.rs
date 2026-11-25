//! 사용자 정의 필드 필터 조건 -> Tantivy 질의 절 (사전 필터 — 후필터 없음).
//!
//! 조건에 값 종류가 명시돼 있어 질의 구성에 SQLite 조회가 필요 없다. **현재 정의와의 대조**(삭제된 필드·선택지,
//! 필드 타입과 조건 종류 불일치)는 이 계층이 모른다 — 서비스 계층(작업 문서 step 5)이 SQLite 정의로 검증한 뒤 넘겨야 한다.
//! 여기서는 구조적으로 잘못된 입력(빈 목록·min>max·NaN·id 범위 초과)만 거부한다.
//!
//! PoC 금지 규칙: number 는 일치도 `[x, x]` 범위로만 질의한다(정수처럼 보이는 f64 는 term 에서 i64 로 정규화돼 누락).
//! date·boolean·enum·multi_enum 은 i64 term/범위, keyword 는 색인과 같은 [`keyword_key`] 정규화 후 문자열 term.

use std::ops::Bound;

use tantivy::Term;
use tantivy::query::{AllQuery, BooleanQuery, Occur, Query, RangeQuery, TermQuery};
use tantivy::schema::{Field, IndexRecordOption};

use crate::error::{Result, SearchError};
use crate::schema::Fields;
use crate::userfield::{canon_f64, keyword_key, path_of};

/// 사용자 필드 조건 하나. `Filters::user_fields` 의 조건들은 서로 AND 로 결합된다.
#[derive(Clone, Debug, PartialEq)]
pub enum UserFieldCondition {
    /// keyword — 여러 값 중 하나 (대소문자·NFC 무시). 값 하나 일치는 값 1개짜리 목록으로 표현한다.
    KeywordAnyOf { field_id: u64, values: Vec<String> },
    /// number — 닫힌 구간. 한쪽만 주면 그쪽은 열려 있다 (둘 다 `None` 이면 "아무 숫자나", 값 있음과 비슷하나 `Has` 가 정석).
    NumberRange {
        field_id: u64,
        min: Option<f64>,
        max: Option<f64>,
    },
    /// date — 닫힌 구간, `YYYYMMDD` 정수.
    DateRange {
        field_id: u64,
        min: Option<i64>,
        max: Option<i64>,
    },
    /// boolean
    Bool { field_id: u64, value: bool },
    /// enum — 여러 선택지 중 하나
    EnumAnyOf { field_id: u64, option_ids: Vec<u64> },
    /// multi_enum — `all` 이면 모두 포함, 아니면 하나라도 포함
    MultiEnum {
        field_id: u64,
        option_ids: Vec<u64>,
        all: bool,
    },
    /// 값 있음 (모든 타입 공통)
    Has { field_id: u64 },
    /// 값 없음 (모든 타입 공통)
    Missing { field_id: u64 },
}

fn invalid(msg: String) -> SearchError {
    SearchError::InvalidInput(format!("user field condition: {msg}"))
}

fn option_ids_ok(field_id: u64, ids: &[u64]) -> Result<()> {
    if ids.is_empty() {
        return Err(invalid(format!(
            "f{field_id}: option id list must not be empty"
        )));
    }
    if ids.iter().any(|id| i64::try_from(*id).is_err()) {
        return Err(invalid(format!("f{field_id}: option id out of range")));
    }
    Ok(())
}

fn finite_bound(field_id: u64, v: Option<f64>) -> Result<()> {
    match v {
        Some(x) if !x.is_finite() => Err(invalid(format!("f{field_id}: bound must be finite"))),
        _ => Ok(()),
    }
}

fn term_query(t: Term) -> Box<dyn Query> {
    Box::new(TermQuery::new(t, IndexRecordOption::Basic))
}

fn json_term(f: Field, field_id: u64) -> Term {
    Term::from_field_json_path(f, &path_of(field_id), false)
}

fn i64_term(f: Field, field_id: u64, v: i64) -> Term {
    let mut t = json_term(f, field_id);
    t.append_type_and_fast_value(v);
    t
}

fn f64_term(f: Field, field_id: u64, v: f64) -> Term {
    let mut t = json_term(f, field_id);
    t.append_type_and_fast_value(v);
    t
}

fn str_term(f: Field, field_id: u64, v: &str) -> Term {
    let mut t = json_term(f, field_id);
    t.append_type_and_str(v);
    t
}

fn combine(occur: Occur, parts: Vec<Box<dyn Query>>) -> Box<dyn Query> {
    Box::new(BooleanQuery::new(
        parts.into_iter().map(|q| (occur, q)).collect(),
    ))
}

impl UserFieldCondition {
    pub fn field_id(&self) -> u64 {
        match self {
            Self::KeywordAnyOf { field_id, .. }
            | Self::NumberRange { field_id, .. }
            | Self::DateRange { field_id, .. }
            | Self::Bool { field_id, .. }
            | Self::EnumAnyOf { field_id, .. }
            | Self::MultiEnum { field_id, .. }
            | Self::Has { field_id }
            | Self::Missing { field_id } => *field_id,
        }
    }

    /// 구조 검증 — 빈 목록, min > max, NaN·무한대, i64 를 넘는 option id, 정규화 후 빈 keyword 를 거부한다.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::KeywordAnyOf { field_id, values } => {
                if values.is_empty() {
                    return Err(invalid(format!(
                        "f{field_id}: keyword list must not be empty"
                    )));
                }
                if values.iter().any(|v| keyword_key(v).is_empty()) {
                    return Err(invalid(format!("f{field_id}: keyword must not be blank")));
                }
            }
            Self::NumberRange { field_id, min, max } => {
                finite_bound(*field_id, *min)?;
                finite_bound(*field_id, *max)?;
                if matches!((min, max), (Some(a), Some(b)) if a > b) {
                    return Err(invalid(format!("f{field_id}: min must be <= max")));
                }
            }
            Self::DateRange { field_id, min, max } => {
                if matches!((min, max), (Some(a), Some(b)) if a > b) {
                    return Err(invalid(format!("f{field_id}: min must be <= max")));
                }
            }
            Self::EnumAnyOf {
                field_id,
                option_ids,
            }
            | Self::MultiEnum {
                field_id,
                option_ids,
                ..
            } => {
                option_ids_ok(*field_id, option_ids)?;
            }
            Self::Bool { .. } | Self::Has { .. } | Self::Missing { .. } => {}
        }
        Ok(())
    }

    /// 질의 절. [`Self::validate`] 를 통과한 조건에만 쓴다.
    pub(crate) fn to_query(&self, f: &Fields) -> Box<dyn Query> {
        let u = f.umeta;
        let inc = |lo: Term, hi: Term| -> Box<dyn Query> {
            Box::new(RangeQuery::new(Bound::Included(lo), Bound::Included(hi)))
        };
        match self {
            Self::KeywordAnyOf { field_id, values } => {
                let mut keys: Vec<String> = values.iter().map(|v| keyword_key(v)).collect();
                keys.sort_unstable();
                keys.dedup();
                combine(
                    Occur::Should,
                    keys.iter()
                        .map(|k| term_query(str_term(u, *field_id, k)))
                        .collect(),
                )
            }
            // 열린 쪽은 타입의 극값으로 닫는다 — JSON 범위 질의는 양쪽 term 에서 경로·타입을 읽는다
            Self::NumberRange { field_id, min, max } => inc(
                f64_term(u, *field_id, canon_f64(min.unwrap_or(f64::MIN))),
                f64_term(u, *field_id, canon_f64(max.unwrap_or(f64::MAX))),
            ),
            Self::DateRange { field_id, min, max } => inc(
                i64_term(u, *field_id, min.unwrap_or(i64::MIN)),
                i64_term(u, *field_id, max.unwrap_or(i64::MAX)),
            ),
            Self::Bool { field_id, value } => term_query(i64_term(u, *field_id, i64::from(*value))),
            Self::EnumAnyOf {
                field_id,
                option_ids,
            } => combine(
                Occur::Should,
                option_ids
                    .iter()
                    .map(|o| term_query(i64_term(u, *field_id, *o as i64)))
                    .collect(),
            ),
            Self::MultiEnum {
                field_id,
                option_ids,
                all,
            } => combine(
                if *all { Occur::Must } else { Occur::Should },
                option_ids
                    .iter()
                    .map(|o| term_query(i64_term(u, *field_id, *o as i64)))
                    .collect(),
            ),
            Self::Has { field_id } => term_query(has_term(f, *field_id)),
            // 부정만으로는 아무것도 찾지 못하므로 전체 집합에서 뺀다
            Self::Missing { field_id } => Box::new(BooleanQuery::new(vec![
                (Occur::Must, Box::new(AllQuery) as Box<dyn Query>),
                (Occur::MustNot, term_query(has_term(f, *field_id))),
            ])),
        }
    }
}

fn has_term(f: &Fields, field_id: u64) -> Term {
    Term::from_field_text(f.umeta_has, &path_of(field_id))
}
