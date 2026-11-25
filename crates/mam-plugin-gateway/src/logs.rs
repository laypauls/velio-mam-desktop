//! 플러그인 로그 — 제어 문자 제거(줄 위조 방지), 비밀 마스킹, 레벨 상한, 속도 제한. 메시지는 `tracing` 필드로만 나가며 target·형식 문자열에 넣지 않는다.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use mam_plugin_host::LogLevel;

pub const MAX_LOG_BYTES: usize = 4096;
const REDACTED: &str = "[redacted]";

/// 로그 출력 대상 — 기본은 `tracing`(target `mam::plugin`). 시험이 기록용 구현을 끼운다
pub trait LogSink: Send + Sync {
    fn emit(&self, plugin_id: &str, level: LogLevel, message: &str);
}

pub struct TracingSink;

impl LogSink for TracingSink {
    fn emit(&self, plugin_id: &str, level: LogLevel, message: &str) {
        match level {
            LogLevel::Debug => tracing::debug!(target: "mam::plugin", plugin_id = plugin_id, message = message),
            LogLevel::Info => tracing::info!(target: "mam::plugin", plugin_id = plugin_id, message = message),
            LogLevel::Warn | LogLevel::Error => tracing::warn!(target: "mam::plugin", plugin_id = plugin_id, message = message),
        }
    }
}

/// 줄 바꿈·제어 문자(U+2028·2029 포함) -> 공백, 비밀 마스킹, 4 KiB 자르기
pub fn sanitize(raw: &str) -> String {
    let flat: String = raw.chars().map(|c| if c.is_control() || c == '\u{2028}' || c == '\u{2029}' { ' ' } else { c }).collect();
    let mut out = redact(&flat);
    if out.len() > MAX_LOG_BYTES {
        let mut end = MAX_LOG_BYTES;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

fn is_tok(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'~' | b'+' | b'/' | b'=')
}

fn word_before(b: &[u8], i: usize) -> bool {
    i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')
}

fn starts_ci(b: &[u8], i: usize, pat: &str) -> bool {
    b.len() >= i + pat.len() && b[i..i + pat.len()].eq_ignore_ascii_case(pat.as_bytes())
}

fn run(b: &[u8], from: usize, pred: impl Fn(u8) -> bool) -> usize {
    from + b[from..].iter().take_while(|c| pred(**c)).count()
}

/// `i` 위치에서 시작하는 비밀 패턴의 마스킹 구간(시작, 끝)
fn secret_at(b: &[u8], i: usize) -> Option<(usize, usize)> {
    // sk-... / AIza... (단어 경계에서 시작, 최소 길이)
    for (prefix, min) in [("sk-", 8usize), ("AIza", 16)] {
        if !word_before(b, i) && b[i..].starts_with(prefix.as_bytes()) {
            let end = run(b, i + prefix.len(), |c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
            if end - (i + prefix.len()) >= min {
                return Some((i, end));
            }
        }
    }
    // Bearer <token> — 통째로 마스킹
    if !word_before(b, i) && starts_ci(b, i, "bearer") {
        let sp = run(b, i + 6, |c| c == b' ');
        let end = run(b, sp, is_tok);
        if sp > i + 6 && end - sp >= 8 {
            return Some((i, end));
        }
    }
    // key|token|secret|password (= | :) <value> — 값만 마스킹한다 (따옴표 값은 따옴표 안쪽)
    for name in ["password", "secret", "token", "key"] {
        if !starts_ci(b, i, name) {
            continue;
        }
        let mut j = run(b, i + name.len(), |c| c.is_ascii_alphanumeric() || c == b'_');
        if matches!(b.get(j), Some(b'"' | b'\'')) {
            j += 1;
        }
        j = run(b, j, |c| c == b' ');
        if !matches!(b.get(j), Some(b'=' | b':')) {
            continue;
        }
        j = run(b, j + 1, |c| c == b' ');
        let (start, end) = match b.get(j) {
            Some(q @ (b'"' | b'\'')) => {
                let s = j + 1;
                (s, s + b[s..].iter().take_while(|c| *c != q).count())
            }
            _ if starts_ci(b, j, "bearer ") => (j, run(b, run(b, j + 6, |c| c == b' '), is_tok)), // `token: Bearer <tok>` 는 스킴 뒤 토큰까지
            _ => (j, run(b, j, |c| !matches!(c, b' ' | b',' | b';' | b'&' | b'"' | b'\'' | b'}' | b')'))),
        };
        if end > start {
            return Some((start, end));
        }
    }
    None
}

fn redact(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut copied) = (0, 0);
    while i < b.len() {
        match secret_at(b, i) {
            Some((start, end)) if end > copied => {
                out.push_str(&s[copied..start]);
                out.push_str(REDACTED);
                copied = end;
                i = end;
            }
            _ => i += 1,
        }
    }
    out.push_str(&s[copied..]);
    out
}

/// 토큰 버킷(플러그인별) — 초과분은 버리고 세며, 다시 통과할 때 요약 1줄을 위해 개수를 돌려준다
pub struct LogLimiter {
    per_sec: f64,
    burst: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    last: Instant,
    dropped: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Admit {
    Drop,
    /// 통과. `Some(n)` 이면 그 전에 n 건을 버렸다 — 요약 줄을 먼저 남긴다
    Pass(Option<u64>),
}

impl LogLimiter {
    pub fn new(per_sec: u32, burst: u32) -> Self {
        Self { per_sec: f64::from(per_sec), burst: f64::from(burst), buckets: Mutex::new(HashMap::new()) }
    }

    pub fn admit(&self, plugin: &str, now: Instant) -> Admit {
        let mut map = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        let b = map.entry(plugin.to_owned()).or_insert(Bucket { tokens: self.burst, last: now, dropped: 0 });
        let elapsed = now.saturating_duration_since(b.last);
        b.tokens = (b.tokens + elapsed.as_secs_f64() * self.per_sec).min(self.burst);
        b.last = now.max(b.last);
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            let dropped = std::mem::take(&mut b.dropped);
            Admit::Pass((dropped > 0).then_some(dropped))
        } else {
            b.dropped += 1;
            Admit::Drop
        }
    }
}
