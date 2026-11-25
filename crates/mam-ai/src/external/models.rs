//! 공급자별 기본 모델 ID — 한 곳에서만 정의한다.
//!
//! 주의: 모델 ID 는 공급자가 폐기·개명한다. **릴리스 전에 각 공급자 공식 문서로 현재 유효한 ID 인지 검토**할 것
//! (실제 API 호출은 개발 중 시험하지 못했다). 사용자는 `set_provider_model` 로 덮어쓸 수 있다.

use super::error::ExternalError;
use super::types::Provider;

pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4o-mini";
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.0-flash";
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-haiku-4-5";

pub fn default_model(provider: Provider) -> &'static str {
    match provider {
        Provider::OpenAi => DEFAULT_OPENAI_MODEL,
        Provider::Gemini => DEFAULT_GEMINI_MODEL,
        Provider::Claude => DEFAULT_CLAUDE_MODEL,
    }
}

/// 모델 ID 는 URL 경로(Gemini)에 들어가므로 안전한 문자만 허용한다.
pub(crate) fn validate_model(model: &str) -> Result<(), ExternalError> {
    // 앞의 `-` 는 CLI 인자(`--model <값>`)에서 옵션으로 해석될 수 있다
    let ok = !model.is_empty()
        && model.len() <= 100
        && !model.starts_with('-')
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'));
    if ok {
        Ok(())
    } else {
        Err(ExternalError::InvalidInput("model id".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Validation / Boundary
    #[test]
    fn t_aicli_model_01_ids_starting_with_a_dash_are_rejected() {
        for bad in ["-p", "--dangerously-skip-permissions", "-", "--model"] {
            assert!(validate_model(bad).is_err(), "{bad}");
        }
        for ok in ["haiku", "claude-haiku-4-5", "gpt-4o-mini", "models_x:1.5"] {
            assert!(validate_model(ok).is_ok(), "{ok}");
        }
        assert!(validate_model("").is_err());
        assert!(validate_model(&"a".repeat(101)).is_err());
    }
}
