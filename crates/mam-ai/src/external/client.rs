//! 외부 AI 클라이언트 — 모든 진입점이 [`ConsentGate`] 를 요구한다. 키는 [`KeyVault`] 에서 빌려 헤더에만 쓰고,
//! 요청마다 [`UsageRecord`] 를 만든다 (성공·실패 모두 `UsageSink` 로 전달).

use std::sync::Arc;

use super::adapter::{Adapter, Reply};
use super::consent::ConsentGate;
use super::error::ExternalError;
use super::payload::AnalyzeRequest;
use super::prompt::{CHECK_PROMPT, MAX_OUTPUT_TOKENS, build_prompt, parse_output};
use super::retry::RetryPolicy;
use super::transport::{HttpRequest, HttpTransport};
use super::types::{AnalyzeOutput, AuthMode, Provider, ProviderCheck, UsageRecord, UsageSink};
use super::{claude::Claude, gemini::Gemini, models, openai::OpenAi};
use crate::keys::KeyVault;

pub trait AiClient: Send + Sync {
    fn provider(&self) -> Provider;
    fn analyze(
        &self,
        gate: &ConsentGate,
        req: &AnalyzeRequest,
    ) -> Result<AnalyzeOutput, ExternalError>;
    /// 연결 확인 — 이미지 없는 최소 요청 1회.
    fn check(&self, gate: &ConsentGate, model: Option<&str>) -> Result<(), ExternalError>;
}

/// UI 의 "연결 확인" — 오류를 `ProviderCheck`(정제된 코드)로 바꾼다.
pub fn check_provider(
    gate: &ConsentGate,
    client: &dyn AiClient,
    model: Option<&str>,
) -> ProviderCheck {
    match client.check(gate, model) {
        Ok(()) => ProviderCheck {
            ok: true,
            error_code: None,
        },
        Err(e) => ProviderCheck {
            ok: false,
            error_code: Some(e.check_code().into()),
        },
    }
}

pub struct ExternalClient {
    adapter: Box<dyn Adapter>,
    transport: Arc<dyn HttpTransport>,
    vault: Arc<KeyVault>,
    retry: RetryPolicy,
    sink: Option<Arc<dyn UsageSink>>,
}

impl ExternalClient {
    pub fn new(
        provider: Provider,
        transport: Arc<dyn HttpTransport>,
        vault: Arc<KeyVault>,
        retry: RetryPolicy,
        sink: Option<Arc<dyn UsageSink>>,
    ) -> Self {
        let adapter: Box<dyn Adapter> = match provider {
            Provider::OpenAi => Box::new(OpenAi),
            Provider::Gemini => Box::new(Gemini),
            Provider::Claude => Box::new(Claude),
        };
        Self {
            adapter,
            transport,
            vault,
            retry,
            sink,
        }
    }

    /// 키를 빌려 요청을 만든다. 키가 없으면 인증 실패(`API 키를 확인하세요`)로 취급한다.
    fn build(
        &self,
        model: &str,
        prompt: &str,
        images: &[super::payload::PreparedImage],
        max_tokens: u32,
    ) -> Result<HttpRequest, ExternalError> {
        self.vault
            .with_key(self.adapter.provider(), |k| {
                self.adapter.build(k, model, prompt, images, max_tokens)
            })
            .ok_or(ExternalError::AuthFailed)
    }

    fn execute(&self, http: &HttpRequest) -> Result<Reply, ExternalError> {
        let resp = self.retry.send(self.transport.as_ref(), http)?;
        self.adapter.parse(&resp.body)
    }

    fn usage(
        &self,
        model: &str,
        kind: String,
        asset_id: Option<i64>,
        bytes: usize,
        reply: Option<&Reply>,
        err: Option<&ExternalError>,
    ) -> UsageRecord {
        let u = UsageRecord {
            provider: self.adapter.provider(),
            model: model.to_string(),
            kind,
            asset_id,
            input_tokens: reply.map_or(0, |r| r.input_tokens),
            output_tokens: reply.map_or(0, |r| r.output_tokens),
            request_bytes: bytes as u64,
            ok: err.is_none(),
            error_code: err.map(|e| e.error_code().to_string()),
            at: self.retry.clock.now_ms(),
            auth_mode: AuthMode::ApiKey,
        };
        if let Some(s) = &self.sink {
            s.record(&u);
        }
        u
    }
}

impl AiClient for ExternalClient {
    fn provider(&self) -> Provider {
        self.adapter.provider()
    }

    fn analyze(
        &self,
        _gate: &ConsentGate,
        req: &AnalyzeRequest,
    ) -> Result<AnalyzeOutput, ExternalError> {
        req.validate()?;
        let provider = self.adapter.provider();
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| models::default_model(provider).to_string());
        models::validate_model(&model)?;
        let images = req.prepare_images()?;
        let prompt = build_prompt(req, req.scope.include_metadata);
        let kind = req
            .kinds
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join("+");

        let http = match self.build(&model, &prompt, &images, MAX_OUTPUT_TOKENS) {
            Ok(h) => h,
            Err(e) => {
                self.usage(&model, kind, req.asset_id, 0, None, Some(&e));
                return Err(e);
            }
        };
        let bytes = http.body.len();
        let outcome = self
            .execute(&http)
            .and_then(|r| parse_output(&r.text).map(|p| (r, p)));
        match outcome {
            Ok((reply, p)) => {
                let usage = self.usage(&model, kind, req.asset_id, bytes, Some(&reply), None);
                Ok(AnalyzeOutput {
                    description: p.description,
                    keywords: p.keywords,
                    ocr_text: p.ocr_text,
                    summary: p.summary,
                    language: if p.language.is_empty() {
                        req.language.clone()
                    } else {
                        p.language
                    },
                    provider,
                    model,
                    usage,
                })
            }
            Err(e) => {
                self.usage(&model, kind, req.asset_id, bytes, None, Some(&e));
                Err(e)
            }
        }
    }

    fn check(&self, _gate: &ConsentGate, model: Option<&str>) -> Result<(), ExternalError> {
        let model = model
            .map(str::to_string)
            .unwrap_or_else(|| models::default_model(self.adapter.provider()).to_string());
        models::validate_model(&model)?;
        let http = self.build(&model, CHECK_PROMPT, &[], 16)?;
        let bytes = http.body.len();
        let result = self.retry.send(self.transport.as_ref(), &http).map(|_| ());
        self.usage(
            &model,
            "check".into(),
            None,
            bytes,
            None,
            result.as_ref().err(),
        );
        result
    }
}
