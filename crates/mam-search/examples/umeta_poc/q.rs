//! 스키마·문서 구성·조건(Pred) -> Tantivy 질의 변환. 같은 Pred 를 `eval` 로 직접 평가해 정답 개수를 검증한다.

use std::collections::BTreeMap;
use std::ops::Bound;

use mam_search::analyzer::KOREAN;
use tantivy::query::{AllQuery, BooleanQuery, Occur, Query, RangeQuery, TermQuery, TermSetQuery};
use tantivy::schema::{
    FAST, Field, INDEXED, IndexRecordOption, JsonObjectOptions, OwnedValue, STRING, Schema,
    TextFieldIndexing, TextOptions,
};
use tantivy::{TantivyDocument, Term};

use crate::data::{self, Val, sortable_i64};

#[derive(Clone, Copy)]
pub struct Fx {
    pub asset_id: Field,
    pub media_type: Field,
    pub captured_at: Field,
    pub rating: Field,
    pub umeta: Field,
    pub umeta_has: Field,
    pub umeta_text: Field,
}

pub fn json_options_nofast() -> JsonObjectOptions {
    JsonObjectOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("raw")
            .set_index_option(IndexRecordOption::Basic)
            .set_fieldnorms(false),
    )
}

pub fn build_schema() -> (Schema, Fx) {
    let mut b = Schema::builder();
    let fx = Fx {
        asset_id: b.add_u64_field("asset_id", INDEXED | tantivy::schema::STORED | FAST),
        media_type: b.add_text_field("media_type", STRING),
        captured_at: b.add_i64_field("captured_at", INDEXED | FAST),
        rating: b.add_u64_field("rating", INDEXED | FAST),
        umeta: b.add_json_field("umeta", json_options()),
        umeta_has: b.add_text_field("umeta_has", STRING),
        umeta_text: b.add_text_field(
            "umeta_text",
            TextOptions::default().set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer(KOREAN)
                    .set_index_option(IndexRecordOption::WithFreqs)
                    .set_fieldnorms(true),
            ),
        ),
    };
    (b.build(), fx)
}

/// D2 형식: keyword=소문자 문자열, number=f64(+ 비교용 정렬 가능 i64 `n{id}`), 나머지 i64.
pub fn make_doc(fx: &Fx, asset_id: u64, vals: &[(u32, Val)], with_sortable: bool) -> TantivyDocument {
    let mut d = TantivyDocument::default();
    d.add_u64(fx.asset_id, asset_id);
    d.add_text(fx.media_type, data::media_type(asset_id));
    d.add_i64(fx.captured_at, data::captured_at(asset_id));
    d.add_u64(fx.rating, data::rating(asset_id));
    let mut m: BTreeMap<String, OwnedValue> = BTreeMap::new();
    let mut text = String::new();
    for (id, v) in vals {
        let key = format!("f{id}");
        match v {
            Val::Kw(s) => {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(s);
                m.insert(key.clone(), OwnedValue::Str(s.clone()));
            }
            Val::Num(x) => {
                m.insert(key.clone(), OwnedValue::F64(*x));
                if with_sortable {
                    m.insert(format!("n{id}"), OwnedValue::I64(sortable_i64(*x)));
                }
            }
            Val::Date(x) | Val::Bool(x) | Val::Enum(x) => {
                m.insert(key.clone(), OwnedValue::I64(*x));
            }
            Val::Multi(xs) => {
                m.insert(
                    key.clone(),
                    OwnedValue::Array(xs.iter().map(|x| OwnedValue::I64(*x)).collect()),
                );
            }
        }
        d.add_text(fx.umeta_has, &key);
    }
    if !m.is_empty() {
        d.add_object(fx.umeta, m);
    }
    if !text.is_empty() {
        d.add_text(fx.umeta_text, text);
    }
    d
}

// --- JSON 경로 term ---

fn base(f: Field, path: &str) -> Term {
    Term::from_field_json_path(f, path, false)
}
pub fn t_i64(f: Field, path: &str, v: i64) -> Term {
    let mut t = base(f, path);
    t.append_type_and_fast_value(v);
    t
}
pub fn t_f64(f: Field, path: &str, v: f64) -> Term {
    let mut t = base(f, path);
    t.append_type_and_fast_value(v);
    t
}
pub fn t_str(f: Field, path: &str, v: &str) -> Term {
    let mut t = base(f, path);
    t.append_type_and_str(v);
    t
}
fn inc(t: Term, u: Term) -> Box<dyn Query> {
    Box::new(RangeQuery::new(Bound::Included(t), Bound::Included(u)))
}
pub fn range_i64(f: Field, path: &str, lo: i64, hi: i64) -> Box<dyn Query> {
    inc(t_i64(f, path, lo), t_i64(f, path, hi))
}
pub fn range_f64_only(f: Field, path: &str, lo: f64, hi: f64) -> Box<dyn Query> {
    inc(t_f64(f, path, lo), t_f64(f, path, hi))
}
/// number 필드 범위 (D2 원안 보정): 정수값은 색인 시 i64 term 으로 정규화되므로 i64 범위 ∪ f64 범위.
pub fn range_num_union(f: Field, path: &str, lo: f64, hi: f64) -> Box<dyn Query> {
    let mut parts: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    let (ilo, ihi) = (lo.ceil(), hi.floor());
    if ilo <= ihi && ilo >= i64::MIN as f64 && ihi <= i64::MAX as f64 {
        parts.push((Occur::Should, range_i64(f, path, ilo as i64, ihi as i64)));
    }
    parts.push((Occur::Should, range_f64_only(f, path, lo, hi)));
    Box::new(BooleanQuery::new(parts))
}
/// 대안: 정렬 가능 i64 로 인코딩해 둔 `n{id}` 경로 단일 범위.
pub fn range_num_sortable(f: Field, id: u32, lo: f64, hi: f64) -> Box<dyn Query> {
    let p = format!("n{id}");
    range_i64(f, &p, sortable_i64(lo), sortable_i64(hi))
}
pub fn any_of(terms: Vec<Term>, f: Field) -> Box<dyn Query> {
    let _ = f;
    Box::new(BooleanQuery::new(
        terms
            .into_iter()
            .map(|t| (Occur::Should, Box::new(TermQuery::new(t, IndexRecordOption::Basic)) as Box<dyn Query>))
            .collect(),
    ))
}
pub fn all_of(terms: Vec<Term>) -> Box<dyn Query> {
    Box::new(BooleanQuery::new(
        terms
            .into_iter()
            .map(|t| (Occur::Must, Box::new(TermQuery::new(t, IndexRecordOption::Basic)) as Box<dyn Query>))
            .collect(),
    ))
}

// --- 조건 ---

#[derive(Clone, Debug)]
pub enum P {
    Media(&'static str),
    Captured(i64, i64),
    RatingMin(u64),
    Kw(u32, String),
    KwAny(u32, Vec<String>),
    KwAnySet(u32, Vec<String>),
    Num(u32, f64, f64),
    // PoC 의 정렬 가능 숫자 범위 비교 경로용으로 유지 — 현재 쿼리 파서는 생성하지 않는다
    #[allow(dead_code)]
    NumSortable(u32, f64, f64),
    Date(u32, i64, i64),
    Bool(u32, i64),
    EnumAny(u32, Vec<i64>),
    MultiAny(u32, Vec<i64>),
    MultiAll(u32, Vec<i64>),
    Missing(u32),
}

fn term_q(t: Term) -> Box<dyn Query> {
    Box::new(TermQuery::new(t, IndexRecordOption::Basic))
}

impl P {
    /// (Occur, 질의). Missing 은 MustNot.
    pub fn clause(&self, fx: &Fx) -> (Occur, Box<dyn Query>) {
        let f = fx.umeta;
        let p = |id: u32| format!("f{id}");
        let q: Box<dyn Query> = match self {
            P::Media(m) => term_q(Term::from_field_text(fx.media_type, m)),
            P::Captured(lo, hi) => Box::new(RangeQuery::new(
                Bound::Included(Term::from_field_i64(fx.captured_at, *lo)),
                Bound::Included(Term::from_field_i64(fx.captured_at, *hi)),
            )),
            P::RatingMin(m) => Box::new(RangeQuery::new(
                Bound::Included(Term::from_field_u64(fx.rating, *m)),
                Bound::Included(Term::from_field_u64(fx.rating, 5)),
            )),
            P::Kw(id, v) => term_q(t_str(f, &p(*id), v)),
            P::KwAny(id, vs) => any_of(vs.iter().map(|v| t_str(f, &p(*id), v)).collect(), f),
            P::KwAnySet(id, vs) => {
                Box::new(TermSetQuery::new(vs.iter().map(|v| t_str(f, &p(*id), v))))
            }
            P::Num(id, lo, hi) => range_f64_only(f, &p(*id), *lo, *hi),
            P::NumSortable(id, lo, hi) => range_num_sortable(f, *id, *lo, *hi),
            P::Date(id, lo, hi) => range_i64(f, &p(*id), *lo, *hi),
            P::Bool(id, v) => term_q(t_i64(f, &p(*id), *v)),
            P::EnumAny(id, vs) | P::MultiAny(id, vs) => {
                any_of(vs.iter().map(|v| t_i64(f, &p(*id), *v)).collect(), f)
            }
            P::MultiAll(id, vs) => all_of(vs.iter().map(|v| t_i64(f, &p(*id), *v)).collect()),
            P::Missing(id) => {
                return (
                    Occur::MustNot,
                    term_q(Term::from_field_text(fx.umeta_has, &p(*id))),
                );
            }
        };
        (Occur::Must, q)
    }

    /// 직접 평가 (정답 검증용).
    pub fn eval(&self, asset_id: u64, vals: &[(u32, Val)]) -> bool {
        let get = |id: u32| vals.iter().find(|(i, _)| *i == id).map(|(_, v)| v);
        match self {
            P::Media(m) => data::media_type(asset_id) == *m,
            P::Captured(lo, hi) => (*lo..=*hi).contains(&data::captured_at(asset_id)),
            P::RatingMin(m) => data::rating(asset_id) >= *m,
            P::Kw(id, v) => matches!(get(*id), Some(Val::Kw(s)) if s == v),
            P::KwAny(id, vs) | P::KwAnySet(id, vs) => {
                matches!(get(*id), Some(Val::Kw(s)) if vs.contains(s))
            }
            P::Num(id, lo, hi) | P::NumSortable(id, lo, hi) => {
                matches!(get(*id), Some(Val::Num(x)) if *x >= *lo && *x <= *hi)
            }
            P::Date(id, lo, hi) => {
                matches!(get(*id), Some(Val::Date(x)) if *x >= *lo && *x <= *hi)
            }
            P::Bool(id, v) => matches!(get(*id), Some(Val::Bool(x)) if x == v),
            P::EnumAny(id, vs) => matches!(get(*id), Some(Val::Enum(x)) if vs.contains(x)),
            P::MultiAny(id, vs) => {
                matches!(get(*id), Some(Val::Multi(xs)) if xs.iter().any(|x| vs.contains(x)))
            }
            P::MultiAll(id, vs) => {
                matches!(get(*id), Some(Val::Multi(xs)) if vs.iter().all(|v| xs.contains(v)))
            }
            P::Missing(id) => get(*id).is_none(),
        }
    }
}

/// 조건 목록 + (선택) 텍스트 토큰 -> BooleanQuery. Must 가 없으면 AllQuery 를 더한다 ("값 없음" 단독).
pub fn build_query(fx: &Fx, preds: &[P], text_tokens: &[String]) -> Box<dyn Query> {
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = preds.iter().map(|p| p.clause(fx)).collect();
    for t in text_tokens {
        clauses.push((
            Occur::Must,
            term_q(Term::from_field_text(fx.umeta_text, t)),
        ));
    }
    if !clauses.iter().any(|(o, _)| *o == Occur::Must) {
        clauses.push((Occur::Must, Box::new(AllQuery)));
    }
    Box::new(BooleanQuery::new(clauses))
}

/// 실제 PoC 에서 쓰는 옵션: 범위 질의가 JSON 에서는 FAST 열을 요구한다(0.27) — `INDEXED` 만으로는 오류.
pub fn json_options() -> JsonObjectOptions {
    json_options_nofast().set_fast("raw")
}
