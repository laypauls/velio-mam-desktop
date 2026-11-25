//! 로깅 (P2-1, D7) — `tracing` + 일 단위 회전 파일(`MAMData/logs/mam.<날짜>.log`).
//!
//! 비밀 보호는 **필드 이름** 기준이다: 이름에 `password`·`api_key`·`token`·`secret` 이 든 필드는 값이
//! `[redacted]` 로 바뀐다. 메시지 문자열(`"{}"` 포매팅) 안에 비밀을 직접 넣는 것은 막지 못한다 —
//! 비밀은 항상 구조화 필드로 기록한다.

use std::path::Path;

use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::field::MakeExt;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::debug_fn;

/// 보관할 일 단위 로그 파일 수.
const LOG_FILES_KEPT: usize = 14;
const SECRET_MARKERS: [&str; 5] = ["password", "api_key", "apikey", "token", "secret"];

pub fn is_secret_field(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_MARKERS.iter().any(|m| lower.contains(m))
}

/// 파일 로깅을 전역으로 설치한다. 실패하면 로깅 없이 계속한다(로그가 기동을 막지 않는다).
/// 쓰기는 동기식이다 — 로그량이 적고, 종료 직전 줄이 유실되지 않는다.
pub fn init(logs_dir: &Path) {
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("mam")
        .filename_suffix("log")
        .max_log_files(LOG_FILES_KEPT)
        .build(logs_dir);
    let appender = match appender {
        Ok(a) => a,
        Err(e) => {
            eprintln!("cannot open log file: {e}");
            return;
        }
    };
    let level = if cfg!(debug_assertions) {
        Level::DEBUG
    } else {
        Level::INFO
    };
    if tracing::subscriber::set_global_default(subscriber(appender, level)).is_ok() {
        std::panic::set_hook(Box::new(|info| {
            tracing::error!(panic = %info, "panic");
        }));
    }
}

pub fn subscriber<W>(writer: W, level: Level) -> impl tracing::Subscriber + Send + Sync
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    let fields = debug_fn(|w, field, value| {
        let name = field.name();
        if is_secret_field(name) {
            write!(w, "{name}=[redacted]")
        } else if name == "message" {
            write!(w, "{value:?}")
        } else {
            write!(w, "{name}={value:?}")
        }
    })
    .delimited(" ");
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_max_level(level)
        .fmt_fields(fields)
        .finish()
}

#[cfg(test)]
mod tests {
    // Categories: Business rule (비밀 필드 마스킹) / Boundary (이름 대소문자·부분 일치)
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Capture {
            self.clone()
        }
    }

    #[test]
    fn t_log_01_secret_named_fields_are_redacted_others_are_kept() {
        // Arrange
        let out = Capture::default();
        let sub = subscriber(out.clone(), Level::INFO);

        // Act
        tracing::subscriber::with_default(sub, || {
            tracing::info!(
                password = "hunter2",
                api_key = "sk-live-123",
                Auth_Token = "tok-456",
                client_secret = "shh",
                asset_id = 42,
                "login attempt"
            );
        });

        // Assert
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        for leaked in ["hunter2", "sk-live-123", "tok-456", "shh"] {
            assert!(!text.contains(leaked), "{leaked} leaked: {text}");
        }
        assert!(text.contains("password=[redacted]"), "{text}");
        assert!(text.contains("asset_id=42"), "{text}");
        assert!(text.contains("login attempt"), "{text}");
    }

    #[test]
    fn t_log_02_secret_field_detection_is_case_insensitive_substring() {
        assert!(is_secret_field("PASSWORD"));
        assert!(is_secret_field("openai_api_key"));
        assert!(is_secret_field("refreshToken"));
        assert!(!is_secret_field("asset_id"));
        assert!(!is_secret_field("path"));
    }
}
