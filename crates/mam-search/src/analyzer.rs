//! Tantivy 0.27 용 토크나이저 어댑터와 등록.
//!
//! 분석 로직은 `korean`·`preprocess` 의 순수 함수에 있고, 여기서는 그 결과를
//! `Token` 목록으로 바꿔 `Tokenizer` 트레이트에 맞춘다 (eager — 필드 값은 짧다).
//! 모든 분석기는 소문자 변환까지 포함한다.

use tantivy::Index;
use tantivy::tokenizer::{TextAnalyzer, Token, TokenStream, Tokenizer};

use crate::error::Result;
use crate::korean;
use crate::preprocess::{self, Piece};

pub const KOREAN: &str = "mam_ko";
pub const FILENAME: &str = "mam_filename";
pub const FILENAME_NGRAM: &str = "mam_filename_ngram";
pub const FILENAME_RAW: &str = "mam_filename_raw";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// 태그·설명·메모·전사: Lindera ko-dic + 조사·어미 제거 + 소문자.
    Korean,
    /// 파일명 A: 전처리 조각 중 한글 조각만 Lindera, 나머지는 조각 그대로.
    Filename,
    /// 파일명 B: 전처리 조각의 2-gram·3-gram.
    FilenameNgram,
    /// 파일명 C: 전처리 조각 그대로 (정규식 질의 대상).
    FilenameRaw,
    /// 질의 전용 `Korean` — 기능어뿐인 단어는 폴백 없이 빈 목록 (등록되지 않는 분석기).
    KoreanQuery,
    /// 질의 전용 `Filename` — 위와 같음.
    FilenameQuery,
}

#[derive(Clone)]
pub struct MamTokenizer {
    kind: Kind,
}

pub struct VecStream {
    tokens: Vec<Token>,
    next: usize,
}

impl TokenStream for VecStream {
    fn advance(&mut self) -> bool {
        self.next += 1;
        self.next <= self.tokens.len()
    }
    fn token(&self) -> &Token {
        &self.tokens[self.next - 1]
    }
    fn token_mut(&mut self) -> &mut Token {
        &mut self.tokens[self.next - 1]
    }
}

impl Tokenizer for MamTokenizer {
    type TokenStream<'a> = VecStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> VecStream {
        // `Tokenizer` 트레이트는 오류를 반환할 수 없다. 사전은 `open_or_create` 에서 미리 검증하므로
        // 실패는 드물고, 패닉·로그 없이 **전처리 조각(형태소 분석 없음)으로 대체**한다 —
        // 빈 토큰열로 두면 그 문서가 조용히 검색 불가가 되기 때문이다.
        let texts = tokenize(self.kind, text).unwrap_or_else(|_| {
            preprocess::split_pieces(text)
                .into_iter()
                .map(|p| (p.text, p.from, p.to))
                .collect()
        });
        let tokens = texts
            .into_iter()
            .enumerate()
            .map(|(position, (text, from, to))| Token {
                offset_from: from,
                offset_to: to,
                position,
                text,
                position_length: 1,
            })
            .collect();
        VecStream { tokens, next: 0 }
    }
}

/// `(토큰 문자열, 시작, 끝)` 목록. 오프셋은 정규화된 문자열 기준이다.
pub fn tokenize(kind: Kind, text: &str) -> Result<Vec<(String, usize, usize)>> {
    match kind {
        Kind::Korean => Ok(korean::analyze(text, true)?
            .into_iter()
            .map(|m| (m.text, m.from, m.to))
            .collect()),
        Kind::KoreanQuery => Ok(korean::analyze_query(text)?
            .into_iter()
            .map(|m| (m.text, m.from, m.to))
            .collect()),
        Kind::Filename => filename_tokens(text, false),
        Kind::FilenameQuery => filename_tokens(text, true),
        Kind::FilenameNgram => Ok(preprocess::split_pieces(text)
            .into_iter()
            .flat_map(|p| {
                preprocess::ngrams(&p.text)
                    .into_iter()
                    .map(move |g| (g, p.from, p.to))
            })
            .collect()),
        Kind::FilenameRaw => Ok(preprocess::split_pieces(text)
            .into_iter()
            .map(|p| (p.text, p.from, p.to))
            .collect()),
    }
}

fn filename_tokens(text: &str, query: bool) -> Result<Vec<(String, usize, usize)>> {
    let mut out = Vec::new();
    for Piece { text, from, to } in preprocess::split_pieces(text) {
        if text.chars().any(preprocess::is_hangul) {
            let morphs = if query {
                korean::analyze_query(&text)?
            } else {
                korean::analyze(&text, true)?
            };
            for m in morphs {
                out.push((m.text, from + m.from, from + m.to));
            }
        } else {
            out.push((text, from, to));
        }
    }
    Ok(out)
}

/// 인덱스의 TokenizerManager 에 분석기를 등록한다 (인덱스를 열 때마다 호출 — 영속되지 않는다).
pub fn register(index: &Index) {
    for (name, kind) in [
        (KOREAN, Kind::Korean),
        (FILENAME, Kind::Filename),
        (FILENAME_NGRAM, Kind::FilenameNgram),
        (FILENAME_RAW, Kind::FilenameRaw),
    ] {
        index
            .tokenizers()
            .register(name, TextAnalyzer::from(MamTokenizer { kind }));
    }
}
