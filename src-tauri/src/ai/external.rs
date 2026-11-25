//! 외부 AI 어댑터 — 작업 코어의 `ExternalAnalyzer` 를 `mam_ai::external::ExternalClient`(API 키) 또는 `CliClient`(구독 모드, claude 전용) 위에 구현한다 (P3-2).
//!
//! - 동의 전에는 요청을 만들지도 않는다 (`ConsentGate`). 공급자·키가 없으면 네트워크 없이 `disabled`(건너뜀)로 끝낸다.
//!   구독 모드는 키가 필요 없고 CLI 를 실행한다(프로세스 0개 = 동의 전/공급자 없음/구독 미지원 공급자)
//! - 전송 범위: MAM 썸네일/대표 프레임을 축소·재인코딩한 바이트 + 정제 메타(파일명·촬영일·유형·길이, 절대 경로 제외)
//! - 재시도 루프는 두지 않는다 (`max_retries = 0`) — 백오프와 `Retry-After` 는 작업 대기열이 맡는다
//! - 사용량은 어댑터가 기록하지 않는다 — 실행기가 `ai_usage` 에 기록한다 (이중 기록 방지)

use std::sync::Arc;

use mam_ai::external::cli::{CliClient, CliHost};
use mam_ai::external::{
    AiClient, AnalysisKind, AnalyzeRequest, AuthMode, ConsentGate, ExternalClient, ExternalError,
    HttpTransport, MediaType, Provider, RetryPolicy, SanitizedMeta, TransferScope,
};
use mam_ai::keys::{KeyStorage, KeyVault};
use mam_core::ai_jobs::{
    ExternalAnalyzer, ExternalRequest, ExternalResult, ExternalUsage, StepFailure, StepKind,
};
use mam_db::DbPool;

use super::store::AiStore;
use super::strip::resolve_images;

pub struct ExternalAdapter {
    store: Arc<AiStore>,
    vault: Arc<KeyVault>,
    transport: Arc<dyn HttpTransport>,
    pool: Arc<DbPool>,
    /// 프레임 스트립 칸을 잘라 쓰는 임시 폴더 (MAMData 안, 전송 뒤 삭제)
    tmp_dir: std::path::PathBuf,
    /// 구독 모드(Claude Code CLI 대리 실행)
    cli: Arc<CliHost>,
}

impl ExternalAdapter {
    pub fn new(
        store: Arc<AiStore>,
        vault: Arc<KeyVault>,
        transport: Arc<dyn HttpTransport>,
        pool: Arc<DbPool>,
        tmp_dir: std::path::PathBuf,
        cli: Arc<CliHost>,
    ) -> Self {
        Self {
            store,
            vault,
            transport,
            pool,
            tmp_dir,
            cli,
        }
    }

    /// `include_metadata` 가 꺼져 있으면 사용자 필드는 DB 에서 읽지도 않는다 (D5).
    fn meta(
        &self,
        asset_id: i64,
        media_type: &str,
        include_metadata: bool,
    ) -> Result<SanitizedMeta, StepFailure> {
        let row = self
            .pool
            .read(|c| Ok(mam_core::assets::brief(c, asset_id)?))
            .map_err(|_| StepFailure::new("io_transient", "자산 정보를 읽지 못했습니다"))?;
        let Some(brief) = row else {
            return Err(StepFailure::new(
                "asset_inactive",
                "자산을 찾을 수 없습니다",
            ));
        };
        let (captured_ms, duration_ms) = (brief.captured_at, brief.duration_ms);
        let kind = match media_type {
            "video" => MediaType::Video,
            "audio" => MediaType::Audio,
            _ => MediaType::Image,
        };
        // SanitizedMeta::new 가 디렉터리를 제거한다 — 절대/상대 경로는 이 계층 밖으로 나가지 않는다
        let user_fields = if include_metadata {
            // 값이 있는 필드만, 삭제 중인 필드·선택지 제외. 실패하면 사용자 필드 없이 보내지 않고 단계를 재시도한다
            self.pool
                .read(|c| {
                    mam_core::userdata::load_external_user_fields(c, asset_id)
                        .map_err(|_| rusqlite::Error::InvalidQuery.into())
                })
                .map_err(|_| StepFailure::new("io_transient", "사용자 필드를 읽지 못했습니다"))?
        } else {
            Vec::new()
        };
        Ok(SanitizedMeta::new(
            brief.file_name.as_deref().unwrap_or(""),
            captured_ms.map(iso_utc),
            kind,
            duration_ms.and_then(|d| u64::try_from(d).ok()),
        )
        .with_user_fields(user_fields))
    }
}

/// 외부 오류 -> 작업 코어 실패 코드 (재시도/건너뜀/영구 분류는 코어가 코드로 한다).
pub fn failure_of(e: &ExternalError) -> StepFailure {
    let retry_after = match e {
        ExternalError::RateLimited { retry_after_secs }
        | ExternalError::SubscriptionLimit { retry_after_secs } => *retry_after_secs,
        _ => None,
    };
    StepFailure::new(e.error_code(), e.user_message()).with_retry_after(retry_after)
}

fn kind_of(k: StepKind) -> Option<AnalysisKind> {
    match k {
        StepKind::Describe => Some(AnalysisKind::Describe),
        StepKind::Keywords => Some(AnalysisKind::Keywords),
        StepKind::Ocr => Some(AnalysisKind::Ocr),
        StepKind::Summary => Some(AnalysisKind::Summary),
        StepKind::Stt | StepKind::Embedding => None,
    }
}

/// 실제로 전송되는 이미지 수 (영상은 `videoFrames` 장까지 균등 선택, 그 밖에는 1장).
fn images_sent(media_type: &str, have: usize, scope: &TransferScope) -> u32 {
    let n = if media_type == "video" {
        have.min(usize::from(scope.clamped().video_frames))
    } else {
        have.min(1)
    };
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl ExternalAnalyzer for ExternalAdapter {
    fn provider(&self) -> String {
        self.store
            .settings()
            .default_provider
            .map_or("none", Provider::as_str)
            .to_string()
    }

    fn auth_mode(&self) -> Option<String> {
        let doc = self.store.stored();
        let p = doc.settings.default_provider?;
        Some(doc.provider(p).auth_mode.as_str().to_string())
    }

    /// 구독 모드의 기본 공급자가 한도에 걸려 있으면 해제 시각 - 실행기는 그때까지 요청을 만들지 않는다.
    fn paused_until_ms(&self) -> Option<i64> {
        let doc = self.store.stored();
        let p = doc.settings.default_provider?;
        (p == Provider::Claude && doc.provider(p).auth_mode == AuthMode::Subscription)
            .then(|| self.cli.paused_until_ms())
            .flatten()
    }

    fn analyze(&self, req: &ExternalRequest) -> Result<ExternalResult, StepFailure> {
        let settings = self.store.settings();
        let gate = ConsentGate::check(settings.consent.given).map_err(|e| failure_of(&e))?;
        let provider = settings.default_provider.ok_or_else(|| {
            StepFailure::new("disabled", "기본 외부 AI 공급자가 선택되지 않았습니다")
        })?;
        let mode = self.store.stored().provider(provider).auth_mode;
        match mode {
            AuthMode::ApiKey if self.vault.storage(provider) == KeyStorage::None => {
                return Err(StepFailure::new("disabled", "API 키가 설정되지 않았습니다"));
            }
            // 구독은 Claude 만 지원한다 (다른 공급자의 저장값은 무시하고 건너뜀)
            AuthMode::Subscription if provider != Provider::Claude => {
                return Err(StepFailure::new(
                    "disabled",
                    "구독 인증을 지원하지 않는 공급자입니다",
                ));
            }
            _ => {}
        }
        let kinds: Vec<AnalysisKind> = req.kinds.iter().filter_map(|k| kind_of(*k)).collect();
        let (paths, _cells) = resolve_images(&req.images, &self.tmp_dir)?;
        let mut images = Vec::with_capacity(paths.len());
        for p in &paths {
            let bytes = std::fs::read(p)
                .map_err(|_| StepFailure::new("thumbnail_missing", "썸네일을 읽지 못했습니다"))?;
            images.push(bytes);
        }
        let scope = TransferScope {
            image_max_edge: req.transfer.image_max_edge,
            video_frames: req.transfer.video_frames,
            include_metadata: req.transfer.include_metadata,
        };
        let request = AnalyzeRequest {
            asset_id: Some(req.asset_id),
            model: self.store.stored().provider(provider).model,
            language: req.language.clone(),
            kinds,
            scope,
            meta: self.meta(req.asset_id, &req.media_type, scope.include_metadata)?,
            images,
        };
        let out = match mode {
            AuthMode::Subscription => {
                let client = CliClient::new(Arc::clone(&self.cli), None);
                client.analyze(&gate, &request).map_err(|e| {
                    // 구독 모드여도 CLI 가 환경변수 API 키를 썼다면 실제 과금은 API 키다 - 실패 사용량에도 그대로 남긴다
                    let billed = client
                        .last_billed_to_api_key()
                        .then(|| AuthMode::ApiKey.as_str().to_string());
                    failure_of(&e).with_billed_auth_mode(billed)
                })?
            }
            AuthMode::ApiKey => {
                let mut policy = RetryPolicy::system();
                policy.max_retries = 0;
                ExternalClient::new(
                    provider,
                    Arc::clone(&self.transport),
                    Arc::clone(&self.vault),
                    policy,
                    None,
                )
                .analyze(&gate, &request)
                .map_err(|e| failure_of(&e))?
            }
        };
        Ok(ExternalResult {
            description: out.description,
            keywords: out.keywords,
            ocr_text: out.ocr_text,
            summary: out.summary,
            language: out.language,
            provider: out.provider.as_str().to_string(),
            model: out.model,
            usage: ExternalUsage {
                input_tokens: out.usage.input_tokens,
                output_tokens: out.usage.output_tokens,
                request_bytes: out.usage.request_bytes,
                images: images_sent(&req.media_type, req.images.len(), &scope),
                auth_mode: Some(out.usage.auth_mode.as_str().to_string()),
            },
        })
    }
}

/// 유닉스 ms -> `YYYY-MM-DDTHH:MM:SSZ` (UTC).
pub fn iso_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
#[path = "external_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "external_umeta_tests.rs"]
mod umeta_tests;
