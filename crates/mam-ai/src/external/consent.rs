//! 동의 게이트 (PL-A2) — 클라이언트의 모든 진입점이 `&ConsentGate` 를 요구한다.
//! 값은 [`ConsentGate::check`] 로만 만들 수 있으므로, 동의 없이는 요청 생성·전송 코드에 도달할 수 없다.

use super::error::ExternalError;

#[derive(Debug)]
pub struct ConsentGate(());

impl ConsentGate {
    /// `consent_given` 은 settings 의 `consent.given`. 거짓이면 `ConsentRequired`.
    pub fn check(consent_given: bool) -> Result<Self, ExternalError> {
        if consent_given {
            Ok(Self(()))
        } else {
            Err(ExternalError::ConsentRequired)
        }
    }
}
