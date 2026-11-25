//! 파일명 전처리 — NFKC 정규화, 구분자·camelCase·숫자↔문자·문자 종류 경계로 조각 분리,
//! 확장자 분리, n-gram 생성. 색인과 질의가 같은 함수를 쓴다.

use unicode_normalization::UnicodeNormalization;

/// 전처리로 나온 조각. `text` 는 소문자, 오프셋은 정규화된 문자열의 바이트 위치.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub from: usize,
    pub to: usize,
}

impl Piece {
    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn has_hangul(&self) -> bool {
        self.text.chars().any(is_hangul)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Hangul,
    Upper,
    Lower,
    Digit,
    Other,
    Sep,
}

pub(crate) fn is_hangul(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xA960..=0xA97F | 0xAC00..=0xD7FF)
}

fn classify(c: char) -> Class {
    if is_hangul(c) {
        Class::Hangul
    } else if c.is_uppercase() {
        Class::Upper
    } else if c.is_lowercase() {
        Class::Lower
    } else if c.is_numeric() {
        Class::Digit
    } else if c.is_alphabetic() {
        Class::Other
    } else {
        Class::Sep // `_` `-` `.` 공백, 기호, 이모지 — 모두 구분자
    }
}

/// 한글 호환 자모 등을 포함해 NFKC 로 정규화한다 (NFD 한글 → 완성형, 전각 → 반각).
pub fn normalize(text: &str) -> String {
    text.nfkc().collect()
}

/// 파일명(확장자 제외)·질의 단어를 조각으로 나눈다. 입력은 내부에서 정규화한다.
/// `IMG_20240815_부산여행` → `img`, `20240815`, `부산여행`
pub fn split_pieces(text: &str) -> Vec<Piece> {
    let normalized = normalize(text);
    let chars: Vec<(usize, char, Class)> = normalized
        .char_indices()
        .map(|(i, c)| (i, c, classify(c)))
        .collect();
    let mut pieces = Vec::new();
    let mut start: Option<usize> = None; // chars 인덱스
    let flush = |pieces: &mut Vec<Piece>, s: usize, e: usize| {
        let from = chars[s].0;
        let to = chars.get(e).map_or(normalized.len(), |x| x.0);
        pieces.push(Piece {
            text: normalized[from..to].to_lowercase(),
            from,
            to,
        });
    };
    for i in 0..chars.len() {
        let cls = chars[i].2;
        if cls == Class::Sep {
            if let Some(s) = start.take() {
                flush(&mut pieces, s, i);
            }
            continue;
        }
        if let Some(s) = start {
            let prev = chars[i - 1].2;
            let boundary = match (prev, cls) {
                (Class::Upper, Class::Lower) => false,
                (Class::Lower, Class::Upper) => true, // camelCase
                // HTTPServer: 대문자 연속 뒤 소문자가 오면 마지막 대문자부터 새 조각
                (Class::Upper, Class::Upper) => {
                    chars.get(i + 1).is_some_and(|n| n.2 == Class::Lower)
                }
                (a, b) => a != b,
            };
            if boundary {
                flush(&mut pieces, s, i);
                start = Some(i);
            }
        } else {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        flush(&mut pieces, s, chars.len());
    }
    pieces
}

/// 마지막 `.` 뒤가 1~5자 ASCII 영숫자이고 영문자를 하나 이상 포함하면 확장자로 본다
/// (`v1.0` 의 `0`, `1.5` 의 `5` 는 확장자가 아니다). 반환은 (이름, 소문자 확장자).
pub fn split_extension(file_name: &str) -> (String, Option<String>) {
    let name = normalize(file_name);
    if let Some(dot) = name.rfind('.') {
        let ext = &name[dot + 1..];
        let valid = (1..=5).contains(&ext.len())
            && ext.bytes().all(|b| b.is_ascii_alphanumeric())
            && ext.bytes().any(|b| b.is_ascii_alphabetic());
        if valid {
            return (name[..dot].to_string(), Some(ext.to_ascii_lowercase()));
        }
    }
    (name, None)
}

/// 문자(바이트 아님) 기준 2-gram·3-gram. 1글자 조각은 n-gram 이 없다.
pub fn ngrams(piece: &str) -> Vec<String> {
    let chars: Vec<char> = piece.chars().collect();
    let mut out = Vec::new();
    for n in [2usize, 3] {
        if chars.len() >= n {
            for w in chars.windows(n) {
                out.push(w.iter().collect());
            }
        }
    }
    out
}

/// 질의 조각에 대해 모두 만족해야 하는 n-gram 목록. 2글자는 2-gram 하나,
/// 3글자 이상은 연속 3-gram 전부(AND). 1글자 조각은 빈 목록(부분 일치 불가).
pub fn query_ngrams(piece: &str) -> Vec<String> {
    let chars: Vec<char> = piece.chars().collect();
    let n = if chars.len() >= 3 { 3 } else { 2 };
    if chars.len() < 2 {
        return Vec::new();
    }
    let mut out: Vec<String> = chars.windows(n).map(|w| w.iter().collect()).collect();
    out.dedup();
    out
}
