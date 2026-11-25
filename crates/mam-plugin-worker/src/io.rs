//! 워커 입출력 — stdin/stdout 줄 단위 JSON. stdout 은 프로토콜 전용이다 (플러그인은 stdout 에 쓸 수 없다).
//! 호스트가 사라지면(stdin EOF·쓰기 실패) 조용히 종료한다.

use std::cell::{Cell, RefCell};
use std::io::{BufRead, StdinLock, Write};

use mam_plugin_host::protocol::{CODE_TOO_LARGE, HostMsg, LogLevel, WorkerMsg, encode_line, parse_host_line};
use serde_json::Value;

use crate::meter::Meter;

pub struct Io {
    stdin: RefCell<StdinLock<'static>>,
    next_id: Cell<u64>,
    max_line: Cell<usize>,
}

fn gone() -> ! {
    std::process::exit(0)
}

impl Io {
    pub fn new() -> Self {
        Self { stdin: RefCell::new(std::io::stdin().lock()), next_id: Cell::new(1), max_line: Cell::new(0) }
    }

    /// 0 = 제한 없음
    pub fn set_max_line(&self, max: usize) {
        self.max_line.set(max);
    }

    pub fn max_line(&self) -> usize {
        self.max_line.get()
    }

    /// 줄을 쓴다. 너무 길면 쓰지 않고 `Err` (호출자가 오류 응답으로 바꾼다)
    pub fn send(&self, msg: &WorkerMsg) -> Result<(), String> {
        let line = encode_line(msg).map_err(|e| e.to_string())?;
        let max = self.max_line.get();
        if max > 0 && line.len() > max {
            return Err(format!("message of {} bytes exceeds the {max} byte limit", line.len()));
        }
        let mut out = std::io::stdout().lock();
        if out.write_all(&line).and_then(|_| out.flush()).is_err() {
            gone();
        }
        Ok(())
    }

    pub fn log(&self, level: LogLevel, message: &str) {
        let _ = self.send(&WorkerMsg::Log { level, message: message.to_owned() });
    }

    /// 다음 호스트 메시지. EOF 는 None. 깨진 줄은 경고만 남기고 건너뛴다
    pub fn read(&self) -> Option<HostMsg> {
        loop {
            let mut line = String::new();
            match self.stdin.borrow_mut().read_line(&mut line) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
            match parse_host_line(line.trim_end().as_bytes()) {
                Ok(m) => return Some(m),
                Err(e) => self.log(LogLevel::Warn, &e),
            }
        }
    }

    /// 호스트 함수 호출 — 응답이 올 때까지 막는다. 기다린 시간은 CPU 시간에서 뺀다. 반환: `{"ok":..}` 봉투 JSON 문자열
    pub fn host_call(&self, meter: &Meter, method: &str, params_json: &str) -> String {
        let envelope = |ok: bool, key: &str, v: Value| serde_json::json!({ "ok": ok, key: v }).to_string();
        // 파싱(Value 로 부풀림) 전에 크기부터 — 상한을 넘는 인자는 할당 없이 즉시 거절
        let max = self.max_line.get();
        if max > 0 && params_json.len() > max {
            let m = format!("params of {} bytes exceed the {max} byte limit", params_json.len());
            return envelope(false, "error", serde_json::json!({"code": CODE_TOO_LARGE, "message": m}));
        }
        let params: Value = match serde_json::from_str(params_json) {
            Ok(p) => p,
            Err(e) => return envelope(false, "error", serde_json::json!({"code": "bad_request", "message": e.to_string()})),
        };
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let began = std::time::Instant::now();
        if let Err(m) = self.send(&WorkerMsg::HostCall { id, method: method.to_owned(), params }) {
            return envelope(false, "error", serde_json::json!({"code": CODE_TOO_LARGE, "message": m}));
        }
        let reply = match self.read() {
            None | Some(HostMsg::Quit) => gone(),
            Some(HostMsg::HostResult { id: rid, ok, value, error }) if rid == id => (ok, value, error),
            // 호출 중에 다른 메시지(또는 다른 id)가 오면 호스트 버그 — 계속 진행할 수 없다
            Some(_) => std::process::exit(3),
        };
        meter.add_blocked(began.elapsed());
        match reply {
            (true, value, _) => envelope(true, "value", value.unwrap_or(Value::Null)),
            (false, _, error) => envelope(false, "error", error.map_or_else(|| serde_json::json!({"code": "host", "message": "host call failed"}), |e| serde_json::json!({"code": e.code, "message": e.message}))),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// T-pwrk-io-01 (Boundary/Validation): 상한을 넘는 host-call 인자는 **파싱 전에** too_large 로 거절된다
    /// (깨진 JSON 이어도 bad_request 가 아니라 too_large — 크기 검사가 먼저라는 증거). 호스트로는 아무것도 보내지 않는다
    #[test]
    fn t_pwrk_io_01_oversize_params_rejected_before_parsing() {
        // Arrange
        let io = Io::new();
        io.set_max_line(1024);
        let meter = Meter::new(0);
        let broken = "{".repeat(2048);
        // Act
        let out: Value = serde_json::from_str(&io.host_call(&meter, "echo", &broken)).expect("envelope");
        // Assert
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"]["code"], CODE_TOO_LARGE);
    }

    /// T-pwrk-io-02 (Boundary): 상한 이내의 깨진 JSON 은 기존대로 bad_request (경계 대조군)
    #[test]
    fn t_pwrk_io_02_in_limit_broken_params_stay_bad_request() {
        let io = Io::new();
        io.set_max_line(1024);
        let out: Value = serde_json::from_str(&io.host_call(&Meter::new(0), "echo", "{")).expect("envelope");
        assert_eq!(out["error"]["code"], "bad_request");
    }
}
