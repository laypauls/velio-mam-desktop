//! `CliClient` — 사용자가 설치한 Claude Code CLI 를 비대화 모드로 대리 실행하는 `AiClient`.
//!
//! 규칙: 동의 게이트 필수, 토큰·자격 증명 파일 비접촉(인증은 전부 CLI 책임), 요청은 한 번에 하나,
//! 빈 임시 cwd, 전송은 MAM 파생 이미지(`PreparedImage`)와 정제 메타(프롬프트)뿐, 제한 시간·조기 오프라인 판정 시 PID 종료.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::args::{DEFAULT_CLI_MODEL, OUTPUT_SCHEMA, build_args, request_line};
use super::exec::RunEnd;
use super::host::CliHost;
use super::process::CliCommand;
use super::stream::{StreamState, map_error_result, stderr_says_not_logged_in};
use super::workdir::WorkDir;
use crate::external::client::AiClient;
use crate::external::consent::ConsentGate;
use crate::external::error::ExternalError;
use crate::external::models;
use crate::external::payload::{AnalyzeRequest, PreparedImage};
use crate::external::prompt::{CHECK_PROMPT, build_prompt, parse_output};
use crate::external::types::{AnalyzeOutput, AuthMode, Provider, UsageRecord, UsageSink};

/// 요청 1건 상한 (실측: 이미지 1건 6~8초, 오프라인 CLI 는 170초까지 재시도한다).
pub const REQUEST_CAP: Duration = Duration::from_secs(120);
/// 한도 초기화까지 남은 시간의 보정 범위(초). 주간 한도는 며칠 뒤에 풀리므로 상한은 8일이다 — 그동안 작업은 시도 횟수 없이 미뤄진다.
const RETRY_AFTER_RANGE: (u64, u64) = (60, 8 * 86_400);
/// 해제 시각을 모를 때 일시정지 시간(초).
const UNKNOWN_RESET_SECS: u64 = 3600;

pub struct CliClient {
    host: Arc<CliHost>,
    sink: Option<Arc<dyn UsageSink>>,
    cap: Duration,
    /// 마지막 요청이 환경변수 API 키로 과금됐는가 (실패 사용량에 실제 과금 방식을 남기려는 용도)
    last_billed_api_key: AtomicBool,
}

/// 성공한 실행의 결과.
struct Reply {
    text: String,
    model: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
}

/// 요청 1건의 기록용 맥락.
struct Call {
    model: String,
    kind: String,
    asset_id: Option<i64>,
    bytes: usize,
}

impl CliClient {
    pub fn new(host: Arc<CliHost>, sink: Option<Arc<dyn UsageSink>>) -> Self {
        Self {
            host,
            sink,
            cap: REQUEST_CAP,
            last_billed_api_key: AtomicBool::new(false),
        }
    }

    /// 마지막 `analyze`/`check` 가 API 키로 과금됐는가 (init 줄을 못 봤으면 false).
    pub fn last_billed_to_api_key(&self) -> bool {
        self.last_billed_api_key.load(Ordering::SeqCst)
    }

    /// 한도 일시정지 중이면 남은 시간(초, 올림)을 돌려준다 — 이때는 프로세스를 띄우지 않는다.
    fn paused_remaining_secs(&self) -> Option<u64> {
        let until = self.host.paused_until_ms()?;
        let left_ms = until.saturating_sub(self.host.clock.now_ms()).max(1);
        Some(u64::try_from(left_ms).unwrap_or(1).div_ceil(1000))
    }

    /// 시험용: 제한 시간을 줄인다.
    pub fn with_cap(mut self, cap: Duration) -> Self {
        self.cap = cap;
        self
    }

    /// 실행 1회. `Err` 에도 `billed_to_api_key` 를 알 수 있도록 상태를 함께 돌려준다.
    fn execute(
        &self,
        model: &str,
        schema: Option<&str>,
        input: Vec<u8>,
    ) -> (Result<Reply, ExternalError>, bool) {
        let _serial = self.host.serial();
        let exe = match self.host.ensure_ready() {
            Ok(e) => e,
            Err(e) => return (Err(e), false),
        };
        let Ok(dir) = WorkDir::create(&self.host.work_root) else {
            return (
                Err(ExternalError::BadResponse("cannot create work directory")),
                false,
            );
        };
        let cmd = CliCommand {
            program: exe,
            args: build_args(model, schema),
            cwd: Some(dir.path().to_path_buf()),
            stdin: Some(input),
        };
        let mut st = StreamState::default();
        let report = self
            .host
            .exec()
            .run_lines(&cmd, self.cap, |line| st.feed(line));
        drop(dir); // 정리는 결과 해석과 무관하게 즉시
        let billed = st.billed_to_api_key();
        let report = match report {
            Ok(r) => r,
            Err(_) => {
                let e = if self.host.exec().is_cancelled() {
                    ExternalError::Cancelled
                } else {
                    ExternalError::CliNotFound
                };
                return (Err(e), billed);
            }
        };
        (self.interpret(&st, report.end, &report.stderr), billed)
    }

    fn interpret(
        &self,
        st: &StreamState,
        end: RunEnd,
        stderr: &str,
    ) -> Result<Reply, ExternalError> {
        if let Some(r) = &st.result {
            if r.is_error {
                return Err(map_error_result(r, st, self.limit_retry_after(st)));
            }
            let text = match &r.structured {
                Some(v) => v.to_string(),
                None => r.text.clone(),
            };
            return Ok(Reply {
                text,
                model: st.model.clone(),
                input_tokens: r.input_tokens,
                output_tokens: r.output_tokens,
            });
        }
        // 결과 줄 없이 끝났다
        Err(match end {
            RunEnd::Timeout => ExternalError::Timeout,
            RunEnd::Cancelled => ExternalError::Cancelled,
            RunEnd::Stopped if st.offline => ExternalError::Offline,
            _ if st.limit_rejected => ExternalError::SubscriptionLimit {
                retry_after_secs: self.limit_retry_after(st),
            },
            _ if stderr_says_not_logged_in(stderr) => ExternalError::CliNotLoggedIn,
            _ => ExternalError::BadResponse("cli ended without a result"),
        })
    }

    fn limit_retry_after(&self, st: &StreamState) -> Option<u64> {
        let resets = u64::try_from(st.limit_resets_at?).ok()?;
        let now = u64::try_from(self.host.clock.now_ms() / 1000).ok()?;
        Some(
            resets
                .saturating_sub(now)
                .clamp(RETRY_AFTER_RANGE.0, RETRY_AFTER_RANGE.1),
        )
    }

    fn usage(
        &self,
        call: Call,
        reply: Option<&Reply>,
        billed_to_api_key: bool,
        err: Option<&ExternalError>,
    ) -> UsageRecord {
        let u = UsageRecord {
            provider: Provider::Claude,
            model: reply.and_then(|r| r.model.clone()).unwrap_or(call.model),
            kind: call.kind,
            asset_id: call.asset_id,
            input_tokens: reply.map_or(0, |r| r.input_tokens),
            output_tokens: reply.map_or(0, |r| r.output_tokens),
            request_bytes: call.bytes as u64,
            ok: err.is_none(),
            error_code: err.map(|e| e.error_code().to_string()),
            at: self.host.clock.now_ms(),
            // 구독 모드여도 CLI 가 환경변수 API 키를 썼다면 실제 과금은 API 키다
            auth_mode: if billed_to_api_key {
                AuthMode::ApiKey
            } else {
                AuthMode::Subscription
            },
        };
        if let Some(s) = &self.sink {
            s.record(&u);
        }
        u
    }
}

impl AiClient for CliClient {
    fn provider(&self) -> Provider {
        Provider::Claude
    }

    fn analyze(
        &self,
        _gate: &ConsentGate,
        req: &AnalyzeRequest,
    ) -> Result<AnalyzeOutput, ExternalError> {
        req.validate()?;
        self.last_billed_api_key.store(false, Ordering::SeqCst);
        // 구독 한도 중에는 CLI 를 띄우지 않는다 (사용량 행도 남기지 않는다 — 요청이 없었다)
        if let Some(secs) = self.paused_remaining_secs() {
            return Err(ExternalError::SubscriptionLimit {
                retry_after_secs: Some(secs),
            });
        }
        let model = req
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_CLI_MODEL.to_string());
        models::validate_model(&model)?;
        let images: Vec<PreparedImage> = req.prepare_images()?;
        let prompt = build_prompt(req, req.scope.include_metadata);
        let kind = req
            .kinds
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join("+");
        let input = request_line(&prompt, &images);
        let bytes = input.len();

        let call = Call {
            model: model.clone(),
            kind,
            asset_id: req.asset_id,
            bytes,
        };
        let (result, billed) = self.execute(&model, Some(OUTPUT_SCHEMA), input);
        self.last_billed_api_key.store(billed, Ordering::SeqCst);
        let outcome = result.and_then(|r| parse_output(&r.text).map(|p| (r, p)));
        match outcome {
            Ok((reply, p)) => {
                let usage = self.usage(call, Some(&reply), billed, None);
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
                    provider: Provider::Claude,
                    model: usage.model.clone(),
                    usage,
                })
            }
            Err(e) => {
                if let ExternalError::SubscriptionLimit { retry_after_secs } = &e {
                    // 같은 한도에 작업마다 CLI 를 띄우지 않도록 해제 시각까지 멈춘다
                    self.host
                        .pause_for_secs(retry_after_secs.unwrap_or(UNKNOWN_RESET_SECS));
                }
                self.usage(call, None, billed, Some(&e));
                Err(e)
            }
        }
    }

    fn check(&self, _gate: &ConsentGate, model: Option<&str>) -> Result<(), ExternalError> {
        let model = model.unwrap_or(DEFAULT_CLI_MODEL);
        models::validate_model(model)?;
        let input = request_line(CHECK_PROMPT, &[]);
        let bytes = input.len();
        let call = Call {
            model: model.to_string(),
            kind: "check".into(),
            asset_id: None,
            bytes,
        };
        let (result, billed) = self.execute(model, None, input);
        self.last_billed_api_key.store(billed, Ordering::SeqCst);
        match result {
            Ok(reply) => {
                self.host.clear_pause(); // 요청이 통과했다 - 한도가 이미 풀렸다
                self.usage(call, Some(&reply), billed, None);
                Ok(())
            }
            Err(e) => {
                self.usage(call, None, billed, Some(&e));
                Err(e)
            }
        }
    }
}
