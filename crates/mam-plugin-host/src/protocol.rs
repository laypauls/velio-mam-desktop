//! 호스트 <-> 워커 프로토콜 v1 — 줄 단위 JSON(UTF-8, `\n` 종결, 한 줄 = 메시지 1개). 워커 stdin/stdout 으로만 오간다.
//!
//! 흐름: 호스트 `init` -> 워커 `ready`(또는 `init_error`) -> 호스트 `invoke` -> (워커 `host_call` <-> 호스트 `host_result`)* -> 워커 `result`.
//! `log` 는 언제든 워커가 보낼 수 있다. 호스트는 모르는 메시지·깨진 줄·잘못된 id 를 프로토콜 위반으로 보고 워커를 종료한다.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

pub const CODE_SCRIPT: &str = "script";
pub const CODE_LIMIT: &str = "limit";
pub const CODE_TOO_LARGE: &str = "too_large";
pub const CODE_BAD_REQUEST: &str = "bad_request";
pub const CODE_NOT_FOUND: &str = "not_found";

/// 호스트가 `init` 에서 워커에 넘기는 자원 한도. 0 = 제한 없음(시험용 — 제품은 항상 설정)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// QuickJS 힙 상한(바이트)
    pub memory_bytes: u64,
    /// JS 스택 상한(바이트)
    pub stack_bytes: u64,
    /// 호출 1회의 CPU 시간 상한(ms) — 호스트 호출 대기 시간은 제외
    pub cpu_ms: u64,
    /// 워커가 내보내는 줄 1개의 최대 길이(바이트) — 넘으면 워커가 `too_large` 오류로 바꿔 보낸다
    pub max_line_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvokeKind {
    Command,
    Provider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum HostMsg {
    /// 첫 메시지 — 플러그인 id·JS 소스 본문·한도. 워커는 경로로 파일을 열지 않는다
    Init { v: u32, plugin_id: String, source: String, limits: Limits },
    Invoke { id: u64, kind: InvokeKind, target: String, params: Value },
    HostResult {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ErrorBody>,
    },
    Quit,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WorkerMsg {
    Ready { v: u32, commands: Vec<String>, providers: Vec<String> },
    InitError { message: String },
    HostCall { id: u64, method: String, params: Value },
    Result {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<ErrorBody>,
    },
    Log { level: LogLevel, message: String },
}

/// 메시지를 한 줄(끝 `\n` 포함)로 직렬화한다
pub fn encode_line<T: Serialize>(msg: &T) -> Result<Vec<u8>, serde_json::Error> {
    let mut v = serde_json::to_vec(msg)?;
    v.push(b'\n');
    Ok(v)
}

/// 워커가 보낸 줄(끝 `\n`·`\r` 제외) 해석. 깨진 JSON·모르는 `t`·필드 누락은 오류
pub fn parse_worker_line(line: &[u8]) -> Result<WorkerMsg, String> {
    serde_json::from_slice(line).map_err(|e| format!("malformed worker message: {e}"))
}

pub fn parse_host_line(line: &[u8]) -> Result<HostMsg, String> {
    serde_json::from_slice(line).map_err(|e| format!("malformed host message: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 분류: Happy / Validation(깨진 줄·모르는 메시지)

    #[test]
    fn t_phost_proto_01_round_trip_and_newline_framing() {
        // Arrange
        let msg = HostMsg::Invoke { id: 7, kind: InvokeKind::Command, target: "a.b".into(), params: json!({"x": "줄\n바꿈"}) };
        // Act
        let line = encode_line(&msg).unwrap();
        // Assert: 내용 속 개행은 이스케이프되어 한 줄만 남는다
        assert_eq!(line.iter().filter(|b| **b == b'\n').count(), 1);
        assert_eq!(parse_host_line(&line[..line.len() - 1]).unwrap(), msg);
    }

    #[test]
    fn t_phost_proto_02_unknown_malformed_and_incomplete_worker_lines_are_errors() {
        for bad in [&b"not json"[..], b"{}", br#"{"t":"nonsense"}"#, br#"{"t":"result"}"#, br#"{"t":"host_call","id":"x","method":"m","params":null}"#, b""] {
            assert!(parse_worker_line(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn t_phost_proto_03_result_error_body_parses() {
        let m = parse_worker_line(br#"{"t":"result","id":3,"ok":false,"error":{"code":"limit","message":"oom"}}"#).unwrap();
        assert_eq!(m, WorkerMsg::Result { id: 3, ok: false, value: None, error: Some(ErrorBody { code: "limit".into(), message: "oom".into() }) });
    }
}
