//! 실행기 (B4) — `jobs` 행 하나(자산 하나)의 모든 플러그인 대기 작업을 순서대로 provider 에 넘기고 결과를 기록한다.
//!
//! 일시정지·종료는 작업 사이에서 듣는다(진행 중 호출은 끝까지). 호출 오류가 정지 신호와 겹치면 작업은 그대로 두고 `Yielded`(시도 없음).

use std::sync::Arc;

use mam_core::jobs::{Job, JobContext, JobError, JobExecutor, JobOutcome, MAX_ATTEMPTS};
use mam_db::PoolError;
use mam_db::plugin_tasks::{PendingTask, active_media_type, complete, delete_task, fail, mark_inactive_done, pending_for_asset, retry_later, with_immediate_tx};
use mam_db::Priority;
use mam_plugin_gateway::{Completion, RecordError};
use serde_json::{Value, json};

use super::core::Core;
use crate::api::{AssetKind, InvokeError, ProviderKind, RunError};

/// 한 작업을 처리한 뒤의 흐름
enum Step {
    Continue,
    /// 일시 오류로 `pending` 에 남았다 — 작업은 끝에서 재시도 가능 오류로 돌려준다
    Retry(&'static str),
    /// 자산이 비활성이 돼 모든 대기 작업을 닫았다
    AssetGone,
    Yield,
}

pub(super) struct PluginExecutor {
    core: Arc<Core>,
}

impl PluginExecutor {
    pub(super) fn new(core: Arc<Core>) -> Self {
        Self { core }
    }

    fn db_busy(e: &PoolError) -> JobError {
        tracing::warn!(error = %e, "plugin job database access failed");
        JobError::retryable("database unavailable")
    }

    /// 비활성(삭제·병합) 자산 — 대기 작업을 모두 `done(inactive)` 로 닫는다
    fn close_inactive(&self, asset: i64) -> Result<(), PoolError> {
        let now = self.core.now();
        self.core.pool.write_until(Priority::Low, self.core.deadline(), move |c| Ok(with_immediate_tx(c, |tx| mark_inactive_done(tx, asset, now))?)).map(|_| ())
    }

    /// 작업 행 하나를 고치는 쓰기 (설치 확인 포함). 쓰기 실패는 재시도 가능으로 본다
    fn settle(&self, r: Result<Option<bool>, PoolError>, plugin: &str) -> Step {
        match r {
            Ok(_) => Step::Continue,
            Err(e) => {
                tracing::warn!(plugin, error = %e, "cannot update plugin task");
                Step::Retry("db")
            }
        }
    }

    fn fail_task(&self, task: &PendingTask, code: &'static str) -> Step {
        let id = task.task_id;
        let r = self.core.write_installed(&task.plugin_id, move |tx, now| Ok(fail(tx, id, code, now)?));
        self.settle(r, &task.plugin_id)
    }

    fn delete(&self, task: &PendingTask) -> Step {
        let id = task.task_id;
        let r = self.core.write_installed(&task.plugin_id, move |tx, _| Ok(delete_task(tx, id)?));
        self.settle(r, &task.plugin_id)
    }

    /// provider 가 `assetKinds` 밖의 자산 — 대기로 남기지 않고 현재 revision 으로 닫는다 (업그레이드로 종류가 바뀐 경우)
    fn close_skipped(&self, task: &PendingTask) -> Step {
        let (id, rev) = (task.task_id, task.target_revision);
        let r = self.core.write_installed(&task.plugin_id, move |tx, now| Ok(complete(tx, id, rev, rev, now)?));
        self.settle(r, &task.plugin_id)
    }

    /// 일시 오류: 시도 +1, 대기 유지. 이 작업 행의 시도가 한도에 닿으면 `failed`
    fn retry(&self, job: &Job, task: &PendingTask, cause: &'static str) -> Step {
        let (id, last) = (task.task_id, job.attempts + 1 >= MAX_ATTEMPTS);
        let r = self.core.write_installed(&task.plugin_id, move |tx, now| {
            Ok(match retry_later(tx, id, now)? {
                Some(_) if last => {
                    fail(tx, id, cause, now)?;
                    false
                }
                Some(_) => true,
                None => false,
            })
        });
        match r {
            Ok(Some(true)) => Step::Retry(cause),
            Ok(_) => Step::Continue,
            Err(e) => {
                tracing::warn!(plugin = task.plugin_id.as_str(), error = %e, "cannot update plugin task");
                Step::Retry("db")
            }
        }
    }

    fn record(&self, job: &Job, ctx: &JobContext<'_>, asset: i64, task: &PendingTask, out: Value) -> Step {
        let done = Completion { task_id: task.task_id, processed_rev: task.target_revision };
        match self.core.gateway.record_provider_output(&task.plugin_id, asset, out, done, self.core.deadline()) {
            Ok(_) | Err(RecordError::NotInstalled | RecordError::TaskGone) => Step::Continue,
            Err(RecordError::OutputInvalid(_)) => self.fail_task(task, "output_invalid"),
            Err(RecordError::PermissionDenied(_)) => self.fail_task(task, "permission_denied"),
            Err(RecordError::QuotaExceeded) => self.fail_task(task, "quota_exceeded"),
            Err(RecordError::AssetInactive) => match self.close_inactive(asset) {
                Ok(()) => Step::AssetGone,
                Err(_) => Step::Retry("db"),
            },
            Err(e @ (RecordError::Busy | RecordError::Db(_))) => {
                if ctx.should_stop() {
                    return Step::Yield;
                }
                tracing::warn!(plugin = task.plugin_id.as_str(), error = %e, "cannot record plugin output");
                self.retry(job, task, if matches!(e, RecordError::Busy) { "busy" } else { "db" })
            }
        }
    }

    fn on_error(&self, job: &Job, ctx: &JobContext<'_>, task: &PendingTask, e: RunError) -> Step {
        match e {
            RunError::Disabled | RunError::NotInstalled | RunError::Invoke(InvokeError::Disabled) => Step::Continue,
            RunError::UnknownTarget => self.delete(task),
            RunError::PermissionDenied(_) => self.fail_task(task, "permission_denied"),
            RunError::Scope(_) => self.fail_task(task, "scope_too_large"),
            RunError::Invoke(_) if ctx.should_stop() => Step::Yield,
            RunError::Invoke(ie) => {
                tracing::warn!(plugin = task.plugin_id.as_str(), provider = task.provider.as_str(), key = ie.key(), "plugin provider failed");
                match ie {
                    InvokeError::Timeout => self.retry(job, task, "timeout"),
                    InvokeError::Crashed => self.retry(job, task, "crashed"),
                    InvokeError::Start(_) => self.retry(job, task, "start"),
                    InvokeError::Plugin { ref code, .. } if code == "busy" => self.retry(job, task, "busy"),
                    InvokeError::Plugin { ref code, .. } if code == "unavailable" => self.retry(job, task, "unavailable"),
                    InvokeError::Script(_) => self.fail_task(task, "script_error"),
                    InvokeError::Plugin { .. } => self.fail_task(task, "plugin_error"),
                    InvokeError::NotRegistered(_) => self.fail_task(task, "not_registered"),
                    InvokeError::Limit(_) => self.fail_task(task, "limit"),
                    InvokeError::Oversize => self.fail_task(task, "oversize"),
                    InvokeError::Flood => self.fail_task(task, "flood"),
                    InvokeError::Protocol(_) => self.fail_task(task, "protocol"),
                    InvokeError::Init(_) => self.fail_task(task, "init_failed"),
                    InvokeError::Disabled => Step::Continue,
                }
            }
        }
    }

    fn run_task(&self, job: &Job, ctx: &JobContext<'_>, asset: i64, media: Option<AssetKind>, task: &PendingTask) -> Step {
        let spec = match self.core.invoker.provider(&task.plugin_id, &task.provider) {
            Ok(s) => s,
            Err(RunError::Disabled | RunError::NotInstalled) => return Step::Continue,
            Err(RunError::UnknownTarget) => return self.delete(task),
            Err(e) => {
                tracing::warn!(plugin = task.plugin_id.as_str(), error = %e, "unexpected provider lookup error");
                return Step::Continue;
            }
        };
        let kind = match spec.kind {
            ProviderKind::Metadata => "metadata",
            ProviderKind::Analysis => "analysis",
            ProviderKind::Export => return self.delete(task), // 내보내기는 자동 실행하지 않는다
        };
        if !spec.asset_kinds.is_empty() && !media.is_some_and(|m| spec.asset_kinds.contains(&m)) {
            return self.close_skipped(task);
        }
        let params = json!({ "assetId": asset, "kind": kind, "revision": task.target_revision });
        match self.core.invoker.invoke_provider(&task.plugin_id, &task.provider, vec![asset], params) {
            Ok(out) => self.record(job, ctx, asset, task, out),
            Err(e) => self.on_error(job, ctx, task, e),
        }
    }
}

fn parse_media(s: &str) -> Option<AssetKind> {
    match s {
        "image" => Some(AssetKind::Image),
        "video" => Some(AssetKind::Video),
        "audio" => Some(AssetKind::Audio),
        _ => None,
    }
}

impl JobExecutor for PluginExecutor {
    fn execute(&self, job: &Job, ctx: &JobContext<'_>) -> Result<JobOutcome, JobError> {
        let Some(asset) = job.asset_id else { return Ok(JobOutcome::Done) };
        // 1. 비활성 자산은 호출하지 않고 닫는다
        let media = self.core.read(|c| Ok(active_media_type(c, asset)?)).map_err(|e| Self::db_busy(&e))?;
        let Some(media) = media else {
            self.close_inactive(asset).map_err(|e| Self::db_busy(&e))?;
            return Ok(JobOutcome::Done);
        };
        let media = parse_media(&media);
        let tasks = self.core.read(|c| Ok(pending_for_asset(c, asset)?)).map_err(|e| Self::db_busy(&e))?;
        let mut retry: Option<&'static str> = None;
        for task in &tasks {
            if ctx.should_stop() {
                return Ok(JobOutcome::Yielded);
            }
            match self.run_task(job, ctx, asset, media, task) {
                Step::Continue => {}
                Step::Retry(cause) => retry = Some(cause),
                Step::AssetGone => return Ok(JobOutcome::Done),
                Step::Yield => return Ok(JobOutcome::Yielded),
            }
        }
        match retry {
            Some(cause) => Err(JobError::retryable(format!("plugin task retry: {cause}"))),
            None => Ok(JobOutcome::Done),
        }
    }
}
