//! 질의 문자열 → Tantivy 쿼리. QueryParser 를 쓰지 않는다 (사용자 입력의 `:` `"` `-` 등이
//! 문법으로 해석되면 안 되고, 필드별 분석기가 다르다).
//!
//! 구조: 공백으로 나눈 단어마다 `Should[파일명, 확장자, 텍스트 필드들]` 를 만들고,
//! 단어들은 `Must` (AND) 로 묶는다. 단어 안의 토큰들은 같은 필드에서 AND 로 맞춘다.
//!
//! **1글자 정책**: 부분 일치(B n-gram, C 정규식)는 2글자 이상 조각에만 적용한다. 1글자 조각은
//! A 의 파일명 필드에서 **토큰 전체가 그 글자와 같은 경우**(예: 파일 `a_b.jpg` 의 `a`, 한글은
//! 형태소가 1글자인 경우)와 텍스트 필드의 같은 규칙으로만 일치한다. 1글자 질의가 부분 문자열
//! 전체 스캔(`*a*`)이 되는 것을 막기 위한 정책이다.

use tantivy::Term;
use tantivy::query::{BooleanQuery, BoostQuery, Occur, Query, RegexQuery, TermQuery};
use tantivy::schema::{Field, IndexRecordOption};

use crate::analyzer::{Kind, tokenize};
use crate::error::Result;
use crate::preprocess::{normalize, query_ngrams, split_extension, split_pieces};
use crate::schema::Fields;

/// 파일명 부분 일치 전략 (질의 쪽).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FilenameStrategy {
    /// 파일명 필드만 (Lindera 기준선).
    #[default]
    A,
    /// A + n-gram 필드. 인덱스를 `build_ngram` 으로 만들어야 한다.
    B,
    /// A + 원문 조각 정규식(`.*조각.*`). 인덱스를 `build_raw` 로 만들어야 한다.
    C,
}

/// 부분 일치(n-gram·정규식)를 적용하는 조각의 최대 글자 수. 긴 한글 조각의 `.*조각.*` 정규식은
/// 상태 수 한도(1000)를 넘겨 실패하고, 그런 질의는 어차피 A(정확 토큰)로 찾는 편이 낫다.
pub const MAX_PARTIAL_CHARS: usize = 64;

const BOOST_FILENAME: f32 = 5.0;
const BOOST_PARTIAL: f32 = 3.0;

fn term_query(field: Field, text: &str) -> Box<dyn Query> {
    Box::new(TermQuery::new(
        Term::from_field_text(field, text),
        IndexRecordOption::WithFreqs,
    ))
}

fn boosted(q: Box<dyn Query>, boost: f32) -> Box<dyn Query> {
    Box::new(BoostQuery::new(q, boost))
}

fn all_of(parts: Vec<Box<dyn Query>>) -> Option<Box<dyn Query>> {
    match parts.len() {
        0 => None,
        1 => parts.into_iter().next(),
        _ => Some(Box::new(BooleanQuery::new(
            parts.into_iter().map(|q| (Occur::Must, q)).collect(),
        ))),
    }
}

fn any_of(parts: Vec<Box<dyn Query>>) -> Option<Box<dyn Query>> {
    match parts.len() {
        0 => None,
        1 => parts.into_iter().next(),
        _ => Some(Box::new(BooleanQuery::new(
            parts.into_iter().map(|q| (Occur::Should, q)).collect(),
        ))),
    }
}

/// 이름 부분(확장자 제외) 질의 — 전략별.
fn stem_clause(
    f: &Fields,
    stem: &str,
    strategy: FilenameStrategy,
) -> Result<Option<Box<dyn Query>>> {
    let tokens = tokenize(Kind::FilenameQuery, stem)?;
    if tokens.is_empty() {
        return Ok(None); // 기호·조사뿐인 이름은 질의에서 버린다 (부분 일치 포함)
    }
    let a = all_of(
        tokens
            .iter()
            .map(|(t, _, _)| term_query(f.file_name, t))
            .collect(),
    )
    .map(|q| boosted(q, BOOST_FILENAME));
    // 부분 일치 보조: 2글자 이상 조각만 (1글자 정책)
    let pieces: Vec<_> = split_pieces(stem)
        .into_iter()
        .filter(|p| (2..=MAX_PARTIAL_CHARS).contains(&p.char_len()))
        .collect();
    let partial = if pieces.is_empty() {
        None
    } else {
        match strategy {
            FilenameStrategy::A => None,
            FilenameStrategy::B => all_of(
                pieces
                    .iter()
                    .flat_map(|p| query_ngrams(&p.text))
                    .map(|g| term_query(f.file_name_ngram, &g))
                    .collect(),
            ),
            FilenameStrategy::C => {
                let mut parts: Vec<Box<dyn Query>> = Vec::new();
                for p in &pieces {
                    // 조각은 영숫자·한글뿐이라 정규식 이스케이프가 필요 없다. 정규식 빌드가
                    // 실패하면(상태 수 초과 등) 그 조각의 부분 일치만 포기하고 질의는 유지한다.
                    if let Ok(q) =
                        RegexQuery::from_pattern(&format!(".*{}.*", p.text), f.file_name_raw)
                    {
                        parts.push(Box::new(q));
                    }
                }
                all_of(parts)
            }
        }
    }
    .map(|q| boosted(q, BOOST_PARTIAL));
    Ok(any_of(a.into_iter().chain(partial).collect()))
}

/// 단어 하나의 절. 질의 단어가 분석 후 아무 토큰도 만들지 못하면 `None` (무시).
pub(crate) fn word_clause(
    f: &Fields,
    word: &str,
    strategy: FilenameStrategy,
    search_title: bool,
) -> Result<Option<Box<dyn Query>>> {
    let mut should: Vec<Box<dyn Query>> = Vec::new();

    // 파일명(+확장자). 확장자처럼 보이는 접미사는 오탐이 있다(`Mr.Kim`, `trip.final`) —
    // `(ext AND 이름) OR (단어 전체를 이름 조각으로)` 로 둘 다 시도한다.
    let (stem, ext) = split_extension(word);
    let mut name_alts: Vec<Box<dyn Query>> = Vec::new();
    if let Some(e) = &ext {
        let mut parts: Vec<Box<dyn Query>> = vec![term_query(f.ext, e)];
        if let Some(q) = stem_clause(f, &stem, strategy)? {
            parts.push(q);
        }
        name_alts.extend(all_of(parts));
        name_alts.extend(stem_clause(f, word, strategy)?);
    } else {
        name_alts.extend(stem_clause(f, word, strategy)?);
    }
    // 확장자만 있는 단어(`.jpg`, `jpg`)는 ext 필드로도 해석
    let bare = normalize(word).trim_start_matches('.').to_lowercase();
    if (1..=5).contains(&bare.len()) && bare.bytes().all(|b| b.is_ascii_alphanumeric()) {
        should.push(term_query(f.ext, &bare));
    }
    should.extend(any_of(name_alts));

    // 텍스트 필드 (조사 변형 흡수는 한국어 분석기가 한다)
    let ko_tokens = tokenize(Kind::KoreanQuery, word)?;
    if !ko_tokens.is_empty() {
        for (field, boost) in [
            (f.title, 5.0),
            (f.user_tags, 5.0),
            (f.ai_keywords, 4.0),
            (f.ai_description, 3.0),
            (f.user_memo, 3.0),
            (f.transcript, 1.0),
            // 사용자 정의 keyword 필드 값 (질의 시점에 필드 정의를 알 필요가 없다). 제목 5.0·메모 3.0 보다 낮게
            (f.umeta_text, 2.0),
        ] {
            if field == f.title && !search_title {
                continue;
            }
            if let Some(q) = all_of(
                ko_tokens
                    .iter()
                    .map(|(t, _, _)| term_query(field, t))
                    .collect(),
            ) {
                should.push(boosted(q, boost));
            }
        }
    }
    Ok(any_of(should))
}
