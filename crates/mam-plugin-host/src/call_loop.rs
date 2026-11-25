//! 호출 1회의 메시지 루프 — 워커가 `result` 를 보낼 때까지 `host_call`·`log` 를 처리하고, 마감·폭주·위반을 감시한다.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

use mam_plugin_manifest::PluginId;
use serde_json::Value;

use crate::error::InvokeError;
use crate::host_api::{CallCx, HostApi, HostError, InvokeScope, codes};
use crate::line_reader::LineEvent;
use crate::protocol::{CODE_LIMIT, CODE_SCRIPT, ErrorBody, HostMsg, WorkerMsg, parse_worker_line};
use crate::worker_proc::WorkerProc;

/// `mam.log` 한 건을 호스트로 넘길 때의 최대 길이(바이트)
const MAX_LOG_BYTES: usize = 4096;

pub(crate) struct CallCtx<'a> {
    pub plugin: &'a PluginId,
    pub api: &'a dyn HostApi,
    pub scope: &'a InvokeScope,
    pub max_messages: usize,
}

impl CallCtx<'_> {
    fn cx(&self) -> CallCx<'_> {
        CallCx {
            plugin: self.plugin,
            scope: self.scope,
        }
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn host_reply(c: &CallCtx<'_>, id: u64, method: &str, params: Value) -> HostMsg {
    // 호스트 함수의 패닉이 감독자 스레드를 죽이지 않게 오류로 바꾼다
    let out = catch_unwind(AssertUnwindSafe(|| c.api.call(&c.cx(), method, params)))
        .unwrap_or_else(|_| Err(HostError::new(codes::HOST_PANIC, "host function failed")));
    match out {
        Ok(v) => HostMsg::HostResult {
            id,
            ok: true,
            value: Some(v),
            error: None,
        },
        Err(e) => HostMsg::HostResult {
            id,
            ok: false,
            value: None,
            error: Some(ErrorBody {
                code: e.code,
                message: e.message,
            }),
        },
    }
}

fn map_error(e: Option<ErrorBody>) -> InvokeError {
    match e {
        Some(b) if b.code == CODE_LIMIT => InvokeError::Limit(b.message),
        Some(b) if b.code == CODE_SCRIPT => InvokeError::Script(b.message),
        Some(b) => InvokeError::Plugin {
            code: b.code,
            message: b.message,
        },
        None => InvokeError::Protocol("failed result without error body".into()),
    }
}

/// `call_id` 에 대한 `result` 가 올 때까지 처리한다. 호출자는 오류가 결함이면 워커를 종료한다
pub(crate) fn run_call(
    w: &mut WorkerProc,
    c: &CallCtx<'_>,
    call_id: u64,
) -> Result<Value, InvokeError> {
    let deadline = c.scope.deadline;
    let mut seen = 0usize;
    loop {
        let remain = deadline.saturating_duration_since(Instant::now());
        let ev = w.rx.recv_timeout(remain).map_err(|e| match e {
            std::sync::mpsc::RecvTimeoutError::Timeout => InvokeError::Timeout,
            std::sync::mpsc::RecvTimeoutError::Disconnected => InvokeError::Crashed,
        })?;
        let line = match ev {
            LineEvent::Line(l) => l,
            LineEvent::Oversize => return Err(InvokeError::Oversize),
            LineEvent::Eof => return Err(InvokeError::Crashed),
        };
        seen += 1;
        if seen > c.max_messages {
            return Err(InvokeError::Flood);
        }
        match parse_worker_line(&line).map_err(InvokeError::Protocol)? {
            WorkerMsg::HostCall { id, method, params } => {
                let reply = host_reply(c, id, &method, params);
                w.send(&reply).map_err(|_| InvokeError::Crashed)?;
            }
            WorkerMsg::Log { level, message } => {
                c.api.log(&c.cx(), level, truncate(&message, MAX_LOG_BYTES))
            }
            WorkerMsg::Result {
                id,
                ok,
                value,
                error,
            } if id == call_id => {
                return if ok {
                    Ok(value.unwrap_or(Value::Null))
                } else {
                    Err(map_error(error))
                };
            }
            WorkerMsg::Result { .. } => {
                return Err(InvokeError::Protocol("result for a different call".into()));
            }
            WorkerMsg::Ready { .. } | WorkerMsg::InitError { .. } => {
                return Err(InvokeError::Protocol("unexpected handshake message".into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Boundary(UTF-8 경계 자르기)

    #[test]
    fn t_phost_loop_01_truncate_respects_char_boundary() {
        assert_eq!(truncate("abc", 8), "abc");
        assert_eq!(truncate("가나다", 4), "가");
        assert_eq!(truncate("가나다", 0), "");
    }
}
