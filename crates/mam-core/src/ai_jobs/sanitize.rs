//! 외부 AI 응답 정제 - 저장소 검증(`ai_store`)을 통과하도록 모델 출력을 저장 전에 다듬는다.
//!
//! 모델은 상한을 지키지 않을 수 있다(긴 키워드, 수백 개의 키워드 등). 이미 비용을 낸 응답이 검증 오류로
//! 버려지고 같은 요청이 반복되지 않도록, 거부하지 않고 잘라서 저장한다.
//!
//! 상한 (문자 수, 바이트 아님 - 문자 경계에서 자른다)
//! - 키워드: 항목당 [`KEYWORD_MAX_CHARS`], 최대 [`KEYWORDS_MAX`] 개 (공백 정리 후 빈 값·중복 제거)
//! - 설명: [`DESCRIPTION_MAX_CHARS`], 요약: [`SUMMARY_MAX_CHARS`], OCR: [`OCR_MAX_CHARS`]
//! - 언어·공급자·모델 이름: [`LABEL_MAX_CHARS`]

use super::types::ExternalResult;
use crate::ai_store::{
    DESCRIPTION_MAX_CHARS, INDEX_TEXT_MAX_CHARS, KEYWORD_MAX_CHARS, KEYWORDS_MAX,
};

pub const SUMMARY_MAX_CHARS: usize = 20_000;
pub const OCR_MAX_CHARS: usize = INDEX_TEXT_MAX_CHARS;
pub const LABEL_MAX_CHARS: usize = 100;

fn cut(s: &str, max: usize) -> String {
    s.trim().chars().take(max).collect()
}

fn keywords(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in raw {
        let k = cut(k, KEYWORD_MAX_CHARS);
        if k.is_empty() || out.contains(&k) {
            continue;
        }
        out.push(k);
        if out.len() == KEYWORDS_MAX {
            break;
        }
    }
    out
}

pub(super) fn sanitize(mut r: ExternalResult) -> ExternalResult {
    r.description = cut(&r.description, DESCRIPTION_MAX_CHARS);
    r.summary = cut(&r.summary, SUMMARY_MAX_CHARS);
    r.ocr_text = cut(&r.ocr_text, OCR_MAX_CHARS);
    r.keywords = keywords(&r.keywords);
    r.language = cut(&r.language, LABEL_MAX_CHARS);
    r.provider = cut(&r.provider, LABEL_MAX_CHARS);
    r.model = cut(&r.model, LABEL_MAX_CHARS);
    r
}

#[cfg(test)]
mod tests {
    //! 분류: Boundary / Validation
    use super::*;
    use crate::ai_jobs::types::ExternalUsage;

    fn base() -> ExternalResult {
        ExternalResult {
            description: String::new(),
            keywords: vec![],
            ocr_text: String::new(),
            summary: String::new(),
            language: "ko".into(),
            provider: "openai".into(),
            model: "m".into(),
            usage: ExternalUsage::default(),
        }
    }

    #[test]
    fn t_aij_30_keywords_are_trimmed_deduped_cut_on_char_boundary_and_capped() {
        let mut r = base();
        let long = "가".repeat(KEYWORD_MAX_CHARS + 50); // 멀티바이트 - 바이트 경계에서 자르면 패닉
        r.keywords = vec!["  car ".into(), "".into(), "   ".into(), "car".into(), long];
        r.keywords
            .extend((0..KEYWORDS_MAX + 20).map(|i| format!("k{i}")));

        let out = sanitize(r);

        assert_eq!(out.keywords.len(), KEYWORDS_MAX);
        assert_eq!(out.keywords[0], "car");
        assert_eq!(out.keywords[1].chars().count(), KEYWORD_MAX_CHARS);
        assert!(out.keywords.iter().all(|k| !k.is_empty() && k == k.trim()));
    }

    #[test]
    fn t_aij_30b_text_fields_are_capped_and_empty_stays_empty() {
        let mut r = base();
        r.description = "é".repeat(DESCRIPTION_MAX_CHARS + 1);
        r.summary = "s".repeat(SUMMARY_MAX_CHARS + 1);
        r.ocr_text = "o".repeat(OCR_MAX_CHARS + 1);
        r.model = "m".repeat(LABEL_MAX_CHARS + 1);

        let out = sanitize(r);

        assert_eq!(out.description.chars().count(), DESCRIPTION_MAX_CHARS);
        assert_eq!(out.summary.chars().count(), SUMMARY_MAX_CHARS);
        assert_eq!(out.ocr_text.chars().count(), OCR_MAX_CHARS);
        assert_eq!(out.model.chars().count(), LABEL_MAX_CHARS);
        assert_eq!(sanitize(base()).keywords, Vec::<String>::new());
    }
}
