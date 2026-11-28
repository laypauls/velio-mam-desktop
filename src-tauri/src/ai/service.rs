//! `AiService` — AI command 의 동기 본체. command 는 `spawn_blocking` 으로 이 메서드를 부르기만 한다.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::persist_throttle::persist_due;

use mam_ai::external::cli::{
    CliClient, CliEnv, CliHost, CliLogin, CliRunner, CliStatus, UrlOpener,
};
use mam_ai::external::{
    AiClient, AuthMode, ConsentGate, ExternalClient, HttpTransport, Provider, ProviderCheck,
    RetryPolicy, UsageRecord, UsageSink, check_provider,
};
use mam_ai::keys::{KeyError, KeyStorage, KeyVault};
use mam_ai::local::embed::{CLIP_IMAGE_MODEL_ID, E5_MODEL_ID};
use mam_core::SharedClock;
use mam_core::ai_jobs::{
    AiDeps, AiExecutor, AiJobsConfig, AiRuntimeSettings, AiScheduler, Origin, QueueResult,
    StepKind, StepStatus, list_status,
};
use mam_core::ai_store::{
    AiStoreError, AiUsageRow, AnalysisEdit, AnalysisRun, AnalysisView, TranscriptView, UsageEntry,
    list_runs, list_usage, load_analysis, load_transcript, record_usage,
};
use mam_core::jobs::JobQueue;
use mam_db::{DbPool, Priority};
use mam_vector::{EmbedStore, Generations, ModelKey, OpenMode, VectorKind, sync_incremental};
use serde::{Deserialize, Serialize};

use super::embed::{EmbeddingAdapter, LocalEmbedders};
use super::events::{AnalysisNotifier, EventSink, NotifyingExecutor};
use super::external::ExternalAdapter;
use super::models::{LocalModel, ModelService};
use super::settings::{AiSettings, AiSettingsPatch, LastCheck, apply_patch, origins_turned_off};
use super::store::AiStore;
use super::stt::{LocalStt, ToolResolver};
use super::{AiFailure, sysinfo};

mod cli;

/// 연결 확인·분석 실패 사용량 보관 기간 (PA4)
pub const RETENTION_DAYS: i64 = 365;
const DAY_MS: i64 = 86_400_000;
const NEW_PAGE: i64 = 200;
const MAX_ANALYZE_IDS: usize = 10_000;
const HISTORY_LIMIT: usize = 200;

pub struct AiServiceDeps {
    pub pool: Arc<DbPool>,
    pub clock: SharedClock,
    pub queue: Arc<JobQueue>,
    pub transport: Arc<dyn HttpTransport>,
    pub models: Arc<ModelService>,
    pub embedders: Arc<LocalEmbedders>,
    pub embed_store: Arc<EmbedStore>,
    pub sink: Arc<dyn EventSink>,
    pub stt_runner: Arc<dyn mam_ai::local::stt::CommandRunner>,
    pub stt_tools: Arc<ToolResolver>,
    pub stt_threads: usize,
    pub data_root: std::path::PathBuf,
    pub jobs_cfg: AiJobsConfig,
    /// 로컬 - UTC (분). 시험에서 고정한다.
    pub utc_offset: Arc<dyn Fn() -> i32 + Send + Sync>,
    /// 구독 모드(Claude Code CLI 대리 실행)용 — 프로세스 실행기·환경·브라우저 열기. 시험은 가짜를 넣는다.
    pub cli_runner: Arc<dyn CliRunner>,
    pub cli_env: Arc<dyn CliEnv>,
    pub url_opener: Arc<dyn UrlOpener>,
}

pub struct AiService {
    pub store: Arc<AiStore>,
    pub vault: Arc<KeyVault>,
    pub models: Arc<ModelService>,
    pub notifier: Arc<AnalysisNotifier>,
    pub embedders: Arc<LocalEmbedders>,
    pub embed_store: Arc<EmbedStore>,
    pub sink: Arc<dyn EventSink>,
    scheduler: AiScheduler,
    pool: Arc<DbPool>,
    clock: SharedClock,
    transport: Arc<dyn HttpTransport>,
    external: Arc<ExternalAdapter>,
    /// 구독 모드 CLI (요청 직렬화·탐지·로그아웃)와 브라우저 로그인 세션
    pub cli: Arc<CliHost>,
    cli_login: CliLogin,
    /// 마지막 CLI 탐지 결과 (`cli_status` 가 60초 재사용)
    cli_cache: std::sync::Mutex<Option<(Instant, CliStatus)>>,
    stt: Arc<LocalStt>,
    embedding: Arc<EmbeddingAdapter>,
    /// 진행 중인 외부 AI 요청 - 종료 경로가 응답·사용량 저장까지 기다린다 (`shutdown::wait_ai_requests`)
    in_flight: mam_core::ai_jobs::InFlight,
    jobs_cfg: AiJobsConfig,
    utc_offset: Arc<dyn Fn() -> i32 + Send + Sync>,
    /// 키별로 마지막으로 인덱스에 반영한 `embedding_id` 최대값
    synced: std::sync::Mutex<std::collections::HashMap<String, i64>>,
    /// 키별 마지막 인덱스 저장 `(끝난 시각, 소요 시간)` — 저장 빈도 제한용 (앱 재시작 시 비어 첫 틱은 즉시 저장)
    persisted: std::sync::Mutex<std::collections::HashMap<String, (Instant, Duration)>>,
}

/// UI `ProviderStatus` — 키 값은 없다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub provider: Provider,
    pub key_storage: KeyStorage,
    /// 저장값이 없으면 `apiKey`
    pub auth_mode: AuthMode,
    /// 구독 인증을 지원하는 공급자 (claude 만)
    pub subscription_supported: bool,
    /// 구독 모드일 때만 (그 외 null)
    pub cli: Option<CliStatus>,
    pub model: Option<String>,
    pub last_check: Option<LastCheck>,
}

/// UI `AssetAiDetail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetAi {
    pub analysis: Option<AnalysisView>,
    pub transcript: Option<TranscriptView>,
    pub status: Vec<StepStatus>,
}

/// UI `AnalysisEdit`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditInput {
    pub description: Option<String>,
    pub keywords: Option<Vec<String>>,
}

fn store_failure(e: AiStoreError) -> AiFailure {
    match e {
        AiStoreError::NotFound(_) => AiFailure::NotFound,
        AiStoreError::Invalid(what) => AiFailure::Invalid(what),
        AiStoreError::Db(e) => AiFailure::internal(&e),
    }
}

fn job_failure(e: mam_core::ai_jobs::AiJobError) -> AiFailure {
    AiFailure::internal(&e)
}

pub fn parse_provider(s: &str) -> Result<Provider, AiFailure> {
    Provider::ALL
        .into_iter()
        .find(|p| p.as_str() == s)
        .ok_or(AiFailure::Invalid("provider"))
}

/// 연결 확인 요청의 사용량을 `ai_usage` 에 남긴다 (kind = `check`).
struct CheckUsage(Arc<DbPool>);

impl UsageSink for CheckUsage {
    fn record(&self, u: &UsageRecord) {
        let entry = UsageEntry {
            provider: u.provider.as_str().to_string(),
            model: u.model.clone(),
            kind: u.kind.clone(),
            asset_id: u.asset_id,
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            request_bytes: u.request_bytes,
            images: 0,
            ok: u.ok,
            error_code: u.error_code.clone(),
            at: u.at,
            auth_mode: Some(u.auth_mode.as_str().to_string()),
        };
        let r = self
            .0
            .write(Priority::Low, move |c| Ok(record_usage(c, &entry).is_ok()));
        if !matches!(r, Ok(true)) {
            tracing::warn!("cannot record AI usage for connection check");
        }
    }
}

impl AiService {
    pub fn new(d: AiServiceDeps) -> Result<Arc<Self>, AiFailure> {
        let store = AiStore::load(Arc::clone(&d.pool), Arc::clone(&d.clock))?;
        let vault = Arc::new(KeyVault::new());
        // 요청별 빈 작업 폴더는 MAMData 가 아니라 OS 임시 폴더 아래에 만든다 — cwd 가 저장소 안이면 CLI 가 프로젝트 컨텍스트를
        // 요청에 싣는다(실측, `external::cli::workdir`). 시계는 작업 코어와 같은 것을 써서 한도 해제 시각을 같은 기준으로 비교한다
        let cli = CliHost::new(
            d.cli_runner,
            d.cli_env,
            Arc::new(cli::CoreClock(Arc::clone(&d.clock))),
            mam_ai::external::cli::default_work_root(),
        );
        let cli_login = CliLogin::new(Arc::clone(&cli), d.url_opener);
        let external = Arc::new(ExternalAdapter::new(
            Arc::clone(&store),
            Arc::clone(&vault),
            Arc::clone(&d.transport),
            Arc::clone(&d.pool),
            d.data_root.join("tmp"),
            Arc::clone(&cli),
        ));
        let stt = Arc::new(LocalStt::new(
            Arc::clone(&store),
            Arc::clone(d.models.store()),
            d.stt_runner,
            d.stt_tools,
            d.stt_threads,
            d.data_root.join("cache").join("tmp"),
        ));
        let embedding = Arc::new(EmbeddingAdapter::new(
            Arc::clone(&d.pool),
            Arc::clone(&d.clock),
            Arc::clone(&d.embedders),
            Arc::clone(&d.embed_store),
            d.data_root,
        ));
        let scheduler = AiScheduler::new(
            Arc::clone(&d.pool),
            d.queue,
            Arc::clone(&d.clock),
            d.jobs_cfg.sweep_batch,
        );
        Ok(Arc::new(Self {
            store,
            vault,
            models: d.models,
            notifier: Arc::new(AnalysisNotifier::default()),
            embedders: d.embedders,
            embed_store: d.embed_store,
            sink: d.sink,
            scheduler,
            pool: d.pool,
            clock: d.clock,
            transport: d.transport,
            external,
            cli,
            cli_login,
            cli_cache: Default::default(),
            stt,
            embedding,
            jobs_cfg: d.jobs_cfg,
            utc_offset: d.utc_offset,
            in_flight: Default::default(),
            synced: Default::default(),
            persisted: Default::default(),
        }))
    }

    pub fn pool(&self) -> &Arc<DbPool> {
        &self.pool
    }

    /// 진행 중 외부 AI 요청 계수기 (종료 대기용).
    pub fn in_flight(&self) -> mam_core::ai_jobs::InFlight {
        self.in_flight.clone()
    }

    /// `JobKind::Ai` 에 등록할 실행기 (분석 이벤트 알림 포함). `Ingest::start` 전에 등록한다.
    pub fn make_executor(&self) -> NotifyingExecutor {
        let inner = AiExecutor::new(
            AiDeps {
                pool: Arc::clone(&self.pool),
                clock: Arc::clone(&self.clock),
                settings: Arc::clone(&self.store) as _,
                external: Arc::clone(&self.external) as _,
                stt: Arc::clone(&self.stt) as _,
                embedding: Arc::clone(&self.embedding) as _,
                utc_offset: Arc::clone(&self.utc_offset),
            },
            self.jobs_cfg.clone(),
        )
        .with_in_flight(self.in_flight.clone());
        NotifyingExecutor::new(inner, Arc::clone(&self.notifier))
    }

    // ── 시작 ────────────────────────────────────────────────────────────

    /// 저장된 키 암호문을 복호해 메모리에 올린다. 한 건의 실패(다른 사용자·손상)는 그 공급자만 `none` 으로 둔다.
    pub fn load_persisted_keys(&self) {
        match self.store.key_blobs() {
            Ok(blobs) => {
                let list: Vec<(Provider, &str)> =
                    blobs.iter().map(|(p, b)| (*p, b.as_str())).collect();
                for (p, r) in self.vault.load_persisted(list) {
                    if r.is_err() {
                        tracing::warn!(provider = p.as_str(), "stored API key cannot be restored");
                    }
                }
            }
            Err(_) => tracing::warn!("cannot read stored API keys"),
        }
    }

    /// PA4 — 365일 지난 사용량·분석 이력을 지운다. 지운 행 수를 돌려준다.
    pub fn purge_old(&self) -> Result<usize, AiFailure> {
        let cutoff = self.clock.now_ms() - RETENTION_DAYS * DAY_MS;
        Ok(self.pool.write(Priority::Low, move |c| {
            let a = c.execute("DELETE FROM ai_usage WHERE at < ?1", [cutoff])?;
            let b = c.execute("DELETE FROM analysis_runs WHERE at < ?1", [cutoff])?;
            Ok(a + b)
        })?)
    }

    // ── 설정·동의 ───────────────────────────────────────────────────────

    pub fn get_settings(&self) -> AiSettings {
        self.store.settings()
    }

    pub fn update_settings(&self, patch: serde_json::Value) -> Result<AiSettings, AiFailure> {
        let patch: AiSettingsPatch =
            serde_json::from_value(patch).map_err(|_| AiFailure::Invalid("patch"))?;
        let ready = self.models.any_embedding_ready();
        let (before, doc) = self.store.update(|d| {
            let before = d.settings.clone();
            d.settings = apply_patch(&d.settings, patch, ready)?;
            Ok(before)
        })?;
        // 자동 옵션이 꺼졌다 - 이미 등록된 그 출처의 대기 작업도 정리한다 (사용자가 직접 요청한 분석은 보존)
        let off = origins_turned_off(&before, &doc.settings);
        if !off.is_empty() {
            self.scheduler
                .cancel_auto_pending(&off)
                .map_err(job_failure)?;
        }
        Ok(doc.settings)
    }

    pub fn give_consent(&self) -> Result<AiSettings, AiFailure> {
        let now = self.store.now();
        let (_, doc) = self.store.update(|d| {
            if !d.settings.consent.given {
                d.settings.consent.given = true;
                d.settings.consent.at = Some(now);
            }
            Ok(())
        })?;
        Ok(doc.settings)
    }

    /// 동의 철회 (PA3): 자동·백그라운드 분석을 끄고 대기 중 외부 단계를 건너뜀 처리한다. 기존 결과는 보존한다.
    pub fn revoke_consent(&self) -> Result<AiSettings, AiFailure> {
        let (_, doc) = self.store.update(|d| {
            d.settings.consent = Default::default();
            d.settings.auto_analyze_new = false;
            d.settings.background_analyze = false;
            Ok(())
        })?;
        self.scheduler
            .cancel_auto_pending(&[Origin::AutoNew, Origin::Background])
            .map_err(job_failure)?;
        self.scheduler
            .cancel_external_pending()
            .map_err(job_failure)?;
        Ok(doc.settings)
    }

    // ── 공급자·키 ───────────────────────────────────────────────────────

    pub(super) fn status_of(&self, p: Provider) -> ProviderStatus {
        let st = self.store.stored().provider(p);
        let auth_mode = self.auth_mode_of(p);
        ProviderStatus {
            provider: p,
            key_storage: self.vault.storage(p),
            auth_mode,
            subscription_supported: cli::subscription_supported(p),
            cli: (auth_mode == AuthMode::Subscription).then(|| self.cli_status()),
            model: st.model,
            last_check: st.last_check,
        }
    }

    pub fn list_providers(&self) -> Vec<ProviderStatus> {
        Provider::ALL
            .into_iter()
            .map(|p| self.status_of(p))
            .collect()
    }

    /// 키는 쓰기 전용: 보관 방식만 돌려준다. `persist` 면 DPAPI 암호문을 설정에 저장한다.
    pub fn set_api_key(
        &self,
        provider: &str,
        key: &str,
        persist: bool,
    ) -> Result<ProviderStatus, AiFailure> {
        let p = parse_provider(provider)?;
        self.vault.set(p, key, persist).map_err(|e| match e {
            KeyError::Invalid(_) => AiFailure::Invalid("key"),
            KeyError::Unsupported => AiFailure::Invalid("persist is not supported"),
            other => AiFailure::internal(&other),
        })?;
        let blob = if persist {
            self.vault.export_persisted(p)
        } else {
            None
        };
        let saved = self.store.set_key_blob(p, blob).and_then(|()| {
            self.store
                .update(|d| {
                    d.provider_mut(p).last_check = None;
                    if d.settings.default_provider.is_none() {
                        d.settings.default_provider = Some(p);
                    }
                    Ok(())
                })
                .map(|_| ())
        });
        if let Err(e) = saved {
            self.vault.clear(p); // 저장 실패 시 반쯤 설정된 상태를 남기지 않는다
            return Err(e);
        }
        Ok(self.status_of(p))
    }

    pub fn clear_api_key(&self, provider: &str) -> Result<ProviderStatus, AiFailure> {
        let p = parse_provider(provider)?;
        self.vault.clear(p);
        self.store.set_key_blob(p, None)?;
        self.store.update(|d| {
            d.provider_mut(p).last_check = None;
            Ok(())
        })?;
        Ok(self.status_of(p))
    }

    pub fn set_provider_model(
        &self,
        provider: &str,
        model: Option<String>,
    ) -> Result<ProviderStatus, AiFailure> {
        let p = parse_provider(provider)?;
        let model = model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        if let Some(m) = &model {
            // URL 경로(Gemini)에 들어가므로 안전한 문자만 허용한다
            let ok = m.len() <= 100
                && m.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'));
            if !ok {
                return Err(AiFailure::Invalid("model"));
            }
        }
        self.store.update(|d| {
            d.provider_mut(p).model = model;
            Ok(())
        })?;
        Ok(self.status_of(p))
    }

    /// 최소 요청 1회로 연결을 확인한다 (blocking). 동의 전에는 네트워크를 쓰지 않는다.
    pub fn check_provider(&self, provider: &str) -> Result<ProviderCheck, AiFailure> {
        let p = parse_provider(provider)?;
        let settings = self.store.settings();
        let gate =
            ConsentGate::check(settings.consent.given).map_err(|_| AiFailure::ConsentRequired)?;
        let sink = Arc::new(CheckUsage(Arc::clone(&self.pool)));
        let model = self.store.stored().provider(p).model;
        let result = if self.auth_mode_of(p) == AuthMode::Subscription {
            // 종료·로그아웃 경로가 이 요청을 기다리거나 거부하도록 진행 중 계수에 포함한다
            let _in_flight = self.in_flight.begin();
            let client = CliClient::new(Arc::clone(&self.cli), Some(sink));
            check_provider(&gate, &client, model.as_deref())
        } else {
            let mut policy = RetryPolicy::system();
            policy.max_retries = 1;
            let client = ExternalClient::new(
                p,
                Arc::clone(&self.transport),
                Arc::clone(&self.vault),
                policy,
                Some(sink),
            );
            let client: &dyn AiClient = &client;
            check_provider(&gate, client, model.as_deref())
        };
        let at = self.store.now();
        self.store.update(|d| {
            d.provider_mut(p).last_check = Some(LastCheck {
                ok: result.ok,
                at,
                error_code: result.error_code.clone(),
            });
            Ok(())
        })?;
        Ok(result)
    }

    // ── 모델 ────────────────────────────────────────────────────────────

    pub fn list_models(&self) -> Vec<LocalModel> {
        self.models.list()
    }

    pub fn download_model(&self, model_id: &str) -> Result<(), AiFailure> {
        self.models.start_download(model_id)
    }

    // ── 분석 ────────────────────────────────────────────────────────────

    fn runtime(&self) -> AiRuntimeSettings {
        self.store.settings().runtime()
    }

    /// 자동·예약 등록용 설정 — 공급자/키가 없으면 외부 단계를 뺀다 (모든 자산이 건너뜀 표식만 받는 일을 막는다).
    pub fn effective_runtime(&self) -> AiRuntimeSettings {
        let s = self.store.settings();
        let mut rt = s.runtime();
        let usable = s.default_provider.is_some_and(|p| self.provider_usable(p));
        if !usable {
            rt.external_kinds.clear();
        }
        rt
    }

    fn mark_all(&self, ids: &[i64]) {
        for id in ids {
            self.notifier.mark(*id);
        }
    }

    pub fn analyze_assets(
        &self,
        ids: &[i64],
        kinds: Option<&[String]>,
        force: bool,
    ) -> Result<QueueResult, AiFailure> {
        if ids.is_empty() || ids.len() > MAX_ANALYZE_IDS || ids.iter().any(|i| *i <= 0) {
            return Err(AiFailure::Invalid("assetIds"));
        }
        let parsed: Option<Vec<StepKind>> = match kinds {
            None => None,
            Some([]) => return Err(AiFailure::Invalid("kinds")),
            Some(k) => Some(
                k.iter()
                    .map(|s| StepKind::parse(s).ok_or(AiFailure::Invalid("kinds")))
                    .collect::<Result<_, _>>()?,
            ),
        };
        let rt = self.runtime();
        if parsed
            .as_ref()
            .is_some_and(|k| k.iter().any(|k| k.is_external()))
            && !rt.consent_given
        {
            return Err(AiFailure::ConsentRequired);
        }
        let res = self
            .scheduler
            .analyze_assets(ids, parsed.as_deref(), force, &rt)
            .map_err(job_failure)?;
        self.mark_all(ids);
        Ok(res)
    }

    pub fn analyze_all(&self, only_unanalyzed: bool) -> Result<QueueResult, AiFailure> {
        let rt = self.runtime();
        if !rt.consent_given {
            return Err(AiFailure::ConsentRequired);
        }
        self.scheduler
            .analyze_all(only_unanalyzed, &rt)
            .map_err(job_failure)
    }

    pub fn retry_failed(&self, ids: Option<&[i64]>) -> Result<QueueResult, AiFailure> {
        if ids.is_some_and(|v| v.len() > MAX_ANALYZE_IDS || v.iter().any(|i| *i <= 0)) {
            return Err(AiFailure::Invalid("assetIds"));
        }
        let res = self.scheduler.retry_failed(ids).map_err(job_failure)?;
        if let Some(v) = ids {
            self.mark_all(v);
        }
        Ok(res)
    }

    pub fn get_asset_ai(&self, asset_id: i64) -> Result<AssetAi, AiFailure> {
        let out = self.pool.read(|c| {
            if !mam_core::assets::exists(c, asset_id).unwrap_or(false) {
                return Ok(None);
            }
            Ok(Some((
                load_analysis(c, asset_id),
                load_transcript(c, asset_id),
                list_status(c, asset_id),
            )))
        })?;
        let Some((a, t, s)) = out else {
            return Err(AiFailure::NotFound);
        };
        Ok(AssetAi {
            analysis: a.map_err(store_failure)?,
            transcript: t.map_err(store_failure)?,
            status: s.map_err(|e| AiFailure::internal(&e))?,
        })
    }

    /// 사용자 편집 저장 — 모든 호출 경로가 이 한 함수를 지난다 (스케줄러 편집 API 로 바꿀 자리).
    pub fn edit_analysis(
        &self,
        asset_id: i64,
        edit: &EditInput,
    ) -> Result<AnalysisView, AiFailure> {
        let edit = AnalysisEdit {
            description: edit.description.clone(),
            keywords: edit.keywords.clone(),
        };
        let view = self
            .scheduler
            .edit_analysis(asset_id, &edit)
            .map_err(|e| match e {
                mam_core::ai_jobs::AiJobError::Store(s) => store_failure(s),
                other => job_failure(other),
            })?;
        self.notifier.mark(asset_id);
        Ok(view)
    }

    pub fn list_history(&self, asset_id: i64) -> Result<Vec<AnalysisRun>, AiFailure> {
        let r = self
            .pool
            .read(|c| Ok(list_runs(c, asset_id, HISTORY_LIMIT)))?;
        r.map_err(store_failure)
    }

    pub fn list_usage(&self, from: &str, to: &str) -> Result<Vec<AiUsageRow>, AiFailure> {
        let offset = (self.utc_offset)();
        let (from, to) = (from.to_string(), to.to_string());
        let r = self
            .pool
            .read(move |c| Ok(list_usage(c, &from, &to, offset)))?;
        r.map_err(store_failure)
    }

    // ── 벡터 인덱스 세대 (검색은 활성 세대를 읽기 전용으로만 연다) ─────────

    /// 준비된 임베딩 모델의 활성 인덱스 세대를 SQLite+raw 와 맞춘다 (없으면 구축, 있으면 증분 추가 후 저장).
    /// 마지막 동기화 뒤 새 벡터가 없으면 아무것도 읽지 않는다. 반영한 키 수를 돌려준다.
    pub fn sync_vector_indexes(&self) -> Result<usize, AiFailure> {
        let mut n = 0;
        let keys = [
            (
                CLIP_IMAGE_MODEL_ID,
                VectorKind::Visual,
                self.embedders.image_info(),
            ),
            (E5_MODEL_ID, VectorKind::Semantic, self.embedders.e5_info()),
        ];
        for (id, kind, info) in keys {
            if !self.models.is_ready(id) {
                continue;
            }
            let key = ModelKey {
                kind,
                model_id: info.model_id,
                model_version: info.model_version,
                dimension: info.dimension,
            };
            if self.sync_key(&key)? {
                n += 1;
            }
        }
        Ok(n)
    }

    /// 한 모델 키의 인덱스 세대를 맞춘다. 새 벡터가 있어 작업했으면 `true`.
    ///
    /// 저장은 인덱스 전체를 다시 쓰므로 `persist_throttle` 로 빈도를 제한한다. 아직 저장할 때가 아니면
    /// 인덱스를 읽지도, 반영 표시(`synced`)를 남기지도 않아 다음 시점에 새 벡터를 이어서 반영한다.
    fn sync_key(&self, key: &ModelKey) -> Result<bool, AiFailure> {
        self.sync_key_at(key, Instant::now())
    }

    /// [`Self::sync_key`] 의 시각 주입판 (테스트가 시계를 고정한다).
    fn sync_key_at(&self, key: &ModelKey, now: Instant) -> Result<bool, AiFailure> {
        let tag = format!(
            "{}/{}/{}",
            key.kind.as_str(),
            key.model_id,
            key.model_version
        );
        let (kind, mid, ver) = (
            key.kind.as_str(),
            key.model_id.clone(),
            key.model_version.clone(),
        );
        let max: i64 = self.pool.read(move |c| {
            Ok(c.query_row(
                "SELECT COALESCE(MAX(embedding_id), 0) FROM embeddings
                 WHERE vector_kind = ?1 AND model_id = ?2 AND model_version = ?3 AND superseded_at IS NULL",
                rusqlite::params![kind, mid, ver],
                |r| r.get(0),
            )?)
        })?;
        let seen = self
            .synced
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&tag)
            .copied();
        if seen == Some(max) {
            return Ok(false);
        }
        let last = self
            .persisted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&tag)
            .copied();
        if !persist_due(last, now) {
            return Ok(false);
        }
        let (store, key2) = (Arc::clone(&self.embed_store), key.clone());
        let r = self.pool.read(move |c| {
            Ok((|| -> mam_vector::Result<Duration> {
                let started = Instant::now();
                let gens = Generations::new(&store, &key2)?;
                let (idx, _rebuilt) = gens.open_or_rebuild(c, &store, &key2, OpenMode::Load)?;
                let report = sync_incremental(c, &store, &key2, &idx)?;
                // 재구축은 이미 저장까지 마쳤다 — 더 읽은 게 없으면 다시 쓰지 않는다
                if report.read > 0 {
                    gens.persist(&key2, &idx)?;
                }
                Ok(started.elapsed())
            })())
        })?;
        match r {
            Ok(took) => {
                self.persisted
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(tag.clone(), (now + took, took));
                self.synced
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(tag, max);
                Ok(true)
            }
            Err(e) => {
                // 저장이 실패해도(예: 검색이 세대를 매핑 중) 다음 틱에 다시 시도한다
                tracing::warn!(%e, "vector index sync failed; will retry");
                Ok(false)
            }
        }
    }

    // ── 예약 루프 틱 (runner.rs 가 부른다) ──────────────────────────────

    /// 신규 자산 자동 분석 (`autoAnalyzeNew` + 동의). 꺼져 있는 동안에는 워터마크만 따라가서, 켠 뒤에 생긴 자산만 대상이 된다.
    pub fn tick_new_assets(&self) -> Result<usize, AiFailure> {
        let doc = self.store.stored();
        let enabled = doc.settings.auto_analyze_new && doc.settings.consent.given;
        let max: i64 = self.pool.read(|c| {
            Ok(c.query_row(
                "SELECT COALESCE(MAX(asset_id), 0) FROM media_assets",
                [],
                |r| r.get(0),
            )?)
        })?;
        let wm = match doc.new_watermark {
            None => {
                self.set_watermark(max)?;
                return Ok(0);
            }
            Some(w) => w,
        };
        if !enabled || wm >= max {
            if !enabled && wm < max {
                self.set_watermark(max)?;
            }
            return Ok(0);
        }
        let rows: Vec<(i64, bool)> = self.pool.read(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT asset_id, status = 'active' FROM media_assets
                 WHERE asset_id > ?1 ORDER BY asset_id LIMIT ?2",
            )?;
            let rows = stmt.query_map([wm, NEW_PAGE], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })?;
        let rt = self.effective_runtime();
        let mut queued = 0;
        for (id, active) in &rows {
            if !*active {
                continue;
            }
            match self.scheduler.enqueue_new_asset(*id, &rt) {
                Ok(r) => queued += usize::try_from(r.queued).unwrap_or(0),
                Err(e) => tracing::warn!(%e, asset_id = id, "cannot queue new asset for AI"),
            }
            self.notifier.mark(*id);
        }
        if let Some((last, _)) = rows.last() {
            self.set_watermark(*last)?;
        }
        Ok(queued)
    }

    /// 의미 검색이 켜져 있으면 embedding 행이 없는 기존 자산에 임베딩만 등록한다. 로컬 추론이라 구독 모드·외부 AI
    /// 동의·예약 시간대와 무관하다(AI 대기열의 일시정지·동시성은 따른다).
    pub fn tick_embedding_backfill(&self) -> Result<QueueResult, AiFailure> {
        let rt = self.effective_runtime();
        self.scheduler.embedding_backfill(&rt).map_err(job_failure)
    }

    fn set_watermark(&self, w: i64) -> Result<(), AiFailure> {
        self.store.update(|d| {
            d.new_watermark = Some(w);
            Ok(())
        })?;
        Ok(())
    }

    /// 예약 시간대 안의 백그라운드 스윕.
    pub fn tick_background(&self) -> Result<QueueResult, AiFailure> {
        // 구독 모드의 일괄 분석은 처리량(1건 약 8초)과 개인 사용 가정 때문에 하지 않는다 - 수동 분석·신규 자산 자동 분석만
        if self.default_is_subscription() {
            return Ok(QueueResult::default());
        }
        let rt = self.effective_runtime();
        self.scheduler
            .background_sweep(&rt, (self.utc_offset)())
            .map_err(job_failure)
    }
}

pub fn default_resources() -> mam_ai::resources::ResourceConfig {
    mam_ai::resources::ResourceConfig::for_ram_gb(sysinfo::total_ram_gb())
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
