//! 구독 인증(Claude Code CLI 대리 실행) 관련 `AiService` 메서드 — 인증 방식 전환, CLI 탐지, 브라우저 로그인, 로그아웃.
//! 계약 정본: `docs/requests/2026-11/20261107-135215-ai-subscription-auth.md` "구현 계약".

use std::time::{Duration, Instant};

use mam_ai::external::cli::{CliStatus, LoginState, LogoutError};
use mam_ai::external::{AuthMode, Provider};

use super::{AiFailure, AiService, ProviderStatus, parse_provider};

/// 작업 코어 시계를 CLI 호스트(`mam_ai`) 시계로 쓴다 — 한도 해제 시각(`resetsAt`)·일시정지 만료를 같은 기준으로 비교한다.
pub(super) struct CoreClock(pub(super) mam_core::SharedClock);

impl mam_ai::external::Clock for CoreClock {
    fn now_ms(&self) -> i64 {
        self.0.now_ms()
    }
}

/// 탐지 결과 재사용 시간 — 설정 화면이 목록을 자주 불러도 CLI 를 매번 실행하지 않는다 (`detect_ai_cli` 는 항상 새로 확인).
const CLI_CACHE_TTL: Duration = Duration::from_secs(60);

/// 구독 인증을 지원하는 공급자는 claude 뿐이다 (약관 검토 결과 — OpenAI·Gemini 제외).
pub(super) fn subscription_supported(p: Provider) -> bool {
    p == Provider::Claude
}

fn parse_mode(s: &str) -> Result<AuthMode, AiFailure> {
    match s {
        "apiKey" => Ok(AuthMode::ApiKey),
        "subscription" => Ok(AuthMode::Subscription),
        _ => Err(AiFailure::Invalid("mode")),
    }
}

/// 로그인·로그아웃·탐지 command 의 공급자 인자 — claude 만 허용한다.
fn parse_claude(provider: &str) -> Result<Provider, AiFailure> {
    let p = parse_provider(provider)?;
    if subscription_supported(p) {
        Ok(p)
    } else {
        Err(AiFailure::Invalid("provider"))
    }
}

impl AiService {
    pub(super) fn auth_mode_of(&self, p: Provider) -> AuthMode {
        let mode = self.store.stored().provider(p).auth_mode;
        // 구독을 지원하지 않는 공급자에 저장돼 있어도(손상·수동 편집) API 키로 해석한다
        if mode == AuthMode::Subscription && !subscription_supported(p) {
            AuthMode::ApiKey
        } else {
            mode
        }
    }

    /// 기본 공급자가 구독 모드인가 (백그라운드 일괄 분석을 건너뛸 때 쓴다).
    pub(super) fn default_is_subscription(&self) -> bool {
        self.store
            .settings()
            .default_provider
            .is_some_and(|p| self.auth_mode_of(p) == AuthMode::Subscription)
    }

    /// 요청을 보낼 수 있는 상태인가: API 키 모드는 키가 있어야 하고, 구독 모드는 키가 필요 없다.
    pub(super) fn provider_usable(&self, p: Provider) -> bool {
        match self.auth_mode_of(p) {
            AuthMode::Subscription => true,
            AuthMode::ApiKey => self.vault.storage(p) != mam_ai::keys::KeyStorage::None,
        }
    }

    /// 탐지 결과 (60초 캐시). 환경변수 API 키 표시는 캐시와 무관하게 항상 현재 값이다.
    pub(super) fn cli_status(&self) -> CliStatus {
        let mut cache = self.cli_cache.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = match cache.as_ref() {
            Some((at, st)) if at.elapsed() < CLI_CACHE_TTL => st.clone(),
            _ => {
                let st = self.cli.detect();
                *cache = Some((Instant::now(), st.clone()));
                st
            }
        };
        st.api_key_env = self.cli.api_key_env();
        st
    }

    fn invalidate_cli_cache(&self) {
        *self.cli_cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    // ── command 본체 ────────────────────────────────────────────────────

    /// 인증 방식 전환. 구독은 claude 만 가능하다. 기본 공급자가 없으면 이 공급자를 기본으로 삼는다(API 키 저장과 같은 규칙).
    pub fn set_provider_auth_mode(
        &self,
        provider: &str,
        mode: &str,
    ) -> Result<ProviderStatus, AiFailure> {
        let p = parse_provider(provider)?;
        let mode = parse_mode(mode)?;
        if mode == AuthMode::Subscription && !subscription_supported(p) {
            return Err(AiFailure::Invalid("mode"));
        }
        self.store.update(|d| {
            let st = d.provider_mut(p);
            if st.auth_mode != mode {
                st.auth_mode = mode;
                st.last_check = None; // 다른 방식의 확인 결과는 이 방식에 해당하지 않는다
            }
            if mode == AuthMode::Subscription && d.settings.default_provider.is_none() {
                d.settings.default_provider = Some(p);
            }
            Ok(())
        })?;
        if mode == AuthMode::ApiKey {
            self.cli_login.cancel(); // 구독을 쓰지 않으면 진행 중인 로그인도 의미가 없다
        } else {
            self.invalidate_cli_cache(); // 방금 고른 화면은 최신 상태여야 한다
        }
        Ok(self.status_of(p))
    }

    /// CLI 설치·버전·로그인 상태를 새로 확인한다. 네트워크 요청 없음, 동의 전에도 허용.
    pub fn detect_ai_cli(&self, provider: &str) -> Result<ProviderStatus, AiFailure> {
        let p = parse_claude(provider)?;
        self.invalidate_cli_cache();
        Ok(self.status_of(p))
    }

    pub fn cli_login_start(&self, provider: &str) -> Result<LoginState, AiFailure> {
        parse_claude(provider)?;
        Ok(self.cli_login.start())
    }

    /// 끝난 결과(`done`/`failed`)는 한 번만 전달되며, 그때 CLI 상태 캐시를 비운다.
    pub fn cli_login_status(&self, provider: &str) -> Result<LoginState, AiFailure> {
        parse_claude(provider)?;
        let st = self.cli_login.status();
        if matches!(st, LoginState::Done { .. } | LoginState::Failed { .. }) {
            self.invalidate_cli_cache();
        }
        Ok(st)
    }

    pub fn cli_login_cancel(&self, provider: &str) -> Result<LoginState, AiFailure> {
        parse_claude(provider)?;
        Ok(self.cli_login.cancel())
    }

    pub fn open_cli_login_url(&self, provider: &str) -> Result<bool, AiFailure> {
        parse_claude(provider)?;
        Ok(self.cli_login.open_current_url())
    }

    /// 로그아웃. 진행 중인 외부 AI 요청이 있으면 `ai_busy` (사용 중인 자격 증명 제거 방지).
    pub fn cli_logout(&self, provider: &str) -> Result<ProviderStatus, AiFailure> {
        let p = parse_claude(provider)?;
        if self.in_flight.count() > 0 {
            return Err(AiFailure::Busy);
        }
        self.cli_login.cancel();
        let result = self.cli.logout();
        self.invalidate_cli_cache();
        match result {
            Ok(()) => Ok(self.status_of(p)),
            Err(LogoutError::Busy) => Err(AiFailure::Busy),
            Err(LogoutError::NotFound) => Err(AiFailure::Cli("cli_not_found")),
            Err(LogoutError::Failed) => Err(AiFailure::Cli("cli_logout_failed")),
        }
    }

    /// 종료 경로: 실행 중인 CLI 프로세스(요청·로그인)를 PID 로 종료하고 이후 실행을 막는다.
    pub fn terminate_cli(&self) {
        self.cli.kill_all();
    }
}
