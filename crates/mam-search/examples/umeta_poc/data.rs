//! 합성 사용자 필드 데이터 — 자산 id 로 결정적(Tantivy·SQLite 가 같은 값을 쓴다).

use crate::rng::Rng;
use crate::vocab::TEXT_KO;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    Keyword,
    Number,
    Date,
    Boolean,
    Enum,
    Multi,
}

impl Ty {
    pub fn sql(self) -> &'static str {
        match self {
            Ty::Keyword => "keyword",
            Ty::Number => "number",
            Ty::Date => "date",
            Ty::Boolean => "boolean",
            Ty::Enum => "enum",
            Ty::Multi => "multi_enum",
        }
    }
}

pub struct Spec {
    pub id: u32,
    pub ty: Ty,
    pub fill: f64,
    /// keyword: 서로 다른 값 수 / enum·multi: 선택지 수
    pub card: usize,
}

pub fn specs() -> Vec<Spec> {
    use Ty::*;
    let s = |id, ty, fill, card| Spec { id, ty, fill, card };
    vec![
        s(1, Keyword, 0.95, 300),
        s(2, Keyword, 0.90, 5000),
        s(3, Keyword, 0.80, 2000),
        s(4, Keyword, 0.70, 500),
        s(5, Keyword, 0.50, 100_000),
        s(6, Number, 0.90, 0),
        s(7, Number, 0.80, 0),
        s(8, Number, 0.70, 0),
        s(9, Number, 0.60, 0),
        s(10, Date, 0.95, 0),
        s(11, Date, 0.80, 0),
        s(12, Date, 0.60, 0),
        s(13, Date, 0.50, 0),
        s(14, Boolean, 0.90, 0),
        s(15, Boolean, 0.70, 0),
        s(16, Enum, 0.95, 4),
        s(17, Enum, 0.85, 12),
        s(18, Enum, 0.60, 50),
        s(19, Multi, 0.80, 8),
        s(20, Multi, 0.60, 30),
    ]
}

#[derive(Clone, Debug)]
pub enum Val {
    Kw(String),
    Num(f64),
    Date(i64),
    Bool(i64),
    Enum(i64),
    Multi(Vec<i64>),
}

/// enum / multi_enum 의 option_id (필드 간 유일).
pub fn option_id(field: u32, k: usize) -> i64 {
    field as i64 * 100 + k as i64 + 1
}

/// keyword 값 k 번째 — 어휘 단어를 base-n 자릿수로 이어 붙인다 (k < n 이면 한 단어).
pub fn kw_value(mut k: usize) -> String {
    let n = TEXT_KO.len();
    let mut parts = vec![TEXT_KO[k % n]];
    k /= n;
    while k > 0 {
        parts.push(TEXT_KO[k % n]);
        k /= n;
    }
    parts.join(" ")
}

pub fn gen_values(asset_id: u64, specs: &[Spec], salt: u64) -> Vec<(u32, Val)> {
    let mut r = Rng::new(asset_id.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ salt.wrapping_mul(0xD6E8_FEB8_6659_FD93));
    let mut out = Vec::with_capacity(specs.len());
    for s in specs {
        if !r.chance(s.fill) {
            continue;
        }
        let v = match s.ty {
            Ty::Keyword => Val::Kw(kw_value(r.skewed(s.card))),
            Ty::Number => Val::Num(match s.id {
                6 => (r.unit() * 100_000.0).round() / 100.0, // 0..1000, 소수 2자리
                7 => r.below(101) as f64,                    // 정수처럼 보이는 값
                8 => (r.unit() * 100_000_000.0).round() / 100.0, // 0..1e6
                _ => ((r.unit() + r.unit() + r.unit()) / 3.0 * 100_000.0).round() / 1000.0,
            }),
            Ty::Date => {
                let y = 2015 + r.below(12) as i64;
                let m = 1 + r.below(12) as i64;
                let d = 1 + r.below(28) as i64;
                Val::Date(y * 10_000 + m * 100 + d)
            }
            Ty::Boolean => Val::Bool(r.chance(0.3) as i64),
            Ty::Enum => Val::Enum(option_id(s.id, r.skewed(s.card))),
            Ty::Multi => {
                let mut v = vec![option_id(s.id, r.skewed(s.card))];
                for p in [0.5, 0.25] {
                    if r.chance(p) {
                        v.push(option_id(s.id, r.skewed(s.card)));
                    }
                }
                v.sort_unstable();
                v.dedup();
                Val::Multi(v)
            }
        };
        out.push((s.id, v));
    }
    out
}

/// 값 행 수 (multi_enum 은 선택지마다 1행).
pub fn row_count(vals: &[(u32, Val)]) -> usize {
    vals.iter()
        .map(|(_, v)| if let Val::Multi(m) = v { m.len() } else { 1 })
        .sum()
}

// --- 고정 필드 (결합 필터 현실성용) ---

fn h(asset_id: u64, salt: u64) -> u64 {
    Rng::new(asset_id ^ salt).next_u64()
}

pub fn media_type(asset_id: u64) -> &'static str {
    match h(asset_id, 0xA1) % 100 {
        0..=49 => "image",
        50..=89 => "video",
        _ => "audio",
    }
}

/// 유닉스 초 (2015-01 ~ 2026-12).
pub fn captured_at(asset_id: u64) -> i64 {
    1_420_070_400 + (h(asset_id, 0xB2) % (12 * 365 * 86_400)) as i64
}

pub fn rating(asset_id: u64) -> u64 {
    h(asset_id, 0xC3) % 6
}

/// order-preserving f64 -> i64 (NaN/Infinity 는 입력 단계에서 거부되는 값).
pub fn sortable_i64(x: f64) -> i64 {
    let x = if x == 0.0 { 0.0 } else { x }; // -0.0 정규화
    let bits = x.to_bits();
    let u = if bits >> 63 == 1 { !bits } else { bits | (1 << 63) };
    (u ^ (1 << 63)) as i64
}
