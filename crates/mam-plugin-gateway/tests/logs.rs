//! 플러그인 로그 (T-pgw-31 ~ 34) — 마스킹·줄 위조 방지·레벨 상한·속도 제한·`plugin_id` 필드.
//!
//! 분류: Validation(마스킹, 제어 문자) / Boundary(4 KiB, 버킷) / Business rule(필드 분리)

mod common;

use std::time::{Duration, Instant};

use common::*;
use mam_plugin_gateway::{Admit, LogLimiter, MAX_LOG_BYTES, sanitize};
use mam_plugin_host::{CallCx, HostApi, InvokeScope, LogLevel};

fn log(fx: &Fx, plugin: &mam_plugin_manifest::PluginId, level: LogLevel, msg: &str) {
    let scope = InvokeScope::empty();
    fx.gw.log(
        &CallCx {
            plugin,
            scope: &scope,
        },
        level,
        msg,
    );
}

#[test]
fn t_pgw_31_secrets_are_masked_and_long_messages_are_cut_on_a_char_boundary() {
    // 4 가지 형태 (OpenAI 형, Google 형, Bearer, key=value 계열)
    let cases = [
        (
            "using sk-proj-AbCdEf1234567890xyz now",
            "using [redacted] now",
        ),
        ("k=AIzaSyA1234567890abcdefghijk end", "k=[redacted] end"),
        (
            "Authorization: Bearer eyJhbGciOi.payload-sig_1 ok",
            "Authorization: [redacted] ok",
        ),
        ("api_key = abc123, next", "api_key = [redacted], next"),
        ("password: hunter2", "password: [redacted]"),
        ("token=abc&x=1", "token=[redacted]&x=1"),
        (
            r#"{"secret":"p@ss w0rd","a":1}"#,
            r#"{"secret":"[redacted]","a":1}"#,
        ),
        ("authToken: Bearer abcdefgh12345", "authToken: [redacted]"),
        (
            "sk-ABCDEFGH12345 and bearer ABCDEFGH12345",
            "[redacted] and [redacted]",
        ),
    ];
    for (raw, want) in cases {
        assert_eq!(sanitize(raw), want, "{raw}");
    }
    // 마스킹되지 않아야 하는 평문 (경계 — 과도한 마스킹 방지)
    for plain in [
        "task-force started",
        "sk-short",
        "the bearer of news",
        "key without separator",
        "hello 가나다",
    ] {
        assert_eq!(sanitize(plain), plain);
    }
    // 4 KiB 경계 — 멀티바이트 한가운데에서 자르지 않는다
    let long = "가".repeat(3000);
    let cut = sanitize(&long);
    assert!(cut.len() <= MAX_LOG_BYTES && cut.chars().all(|c| c == '가') && cut.len() == 4095);
    assert_eq!(
        sanitize(&"a".repeat(MAX_LOG_BYTES + 10)).len(),
        MAX_LOG_BYTES
    );
    assert_eq!(sanitize(&"a".repeat(MAX_LOG_BYTES)).len(), MAX_LOG_BYTES);
}

#[test]
fn t_pgw_32_newline_injection_stays_one_line_and_levels_are_capped_at_warn() {
    let fx = Fx::new();
    let p = plugin_id("com.example.log");
    log(
        &fx,
        &p,
        LogLevel::Info,
        "line1\nFAKE [ERROR] forged\r\nline3\u{2028}x\u{2029}y\u{0}z\u{1b}[31m",
    );
    log(&fx, &p, LogLevel::Error, "boom");
    log(&fx, &p, LogLevel::Debug, "debug hidden");
    let lines = fx.sink.lines();
    assert_eq!(lines.len(), 2, "debug 는 디버그 로깅이 아니면 남지 않는다");
    let (_, level, msg) = &lines[0];
    assert_eq!(*level, LogLevel::Info);
    assert!(
        !msg.chars()
            .any(|c| c.is_control() || c == '\u{2028}' || c == '\u{2029}'),
        "{msg:?}"
    );
    assert_eq!(msg.lines().count(), 1);
    assert_eq!(lines[1].1, LogLevel::Warn, "error 는 warn 으로 낮춘다");
    // 디버그 로깅을 켜면 debug 가 남는다
    let dbg = Fx::with_cfg(|c| c.debug_logging = true);
    log(&dbg, &p, LogLevel::Debug, "visible");
    assert_eq!(dbg.sink.lines()[0].1, LogLevel::Debug);
}

#[test]
fn t_pgw_33_rate_limit_drops_the_excess_and_emits_one_summary_line_on_recovery() {
    // 결정적 시험: 합성 시각으로 버킷만 검증한다 (20/s, burst 100)
    let lim = LogLimiter::new(20, 100);
    let t0 = Instant::now();
    let passed = (0..130)
        .filter(|_| matches!(lim.admit("p", t0), Admit::Pass(_)))
        .count();
    assert_eq!(passed, 100, "burst 100 만 통과");
    assert_eq!(lim.admit("other", t0), Admit::Pass(None), "플러그인별 버킷");
    // 0.5 s 후 10 토큰 회복 — 첫 통과에 요약(버린 30 건), 이후엔 없음
    let t1 = t0 + Duration::from_millis(500);
    assert_eq!(lim.admit("p", t1), Admit::Pass(Some(30)));
    assert_eq!(lim.admit("p", t1), Admit::Pass(None));
    // 게이트웨이 수준: 같은 규칙이 `log` 에 적용된다 (burst 3, 회복 느림)
    let fx = Fx::with_cfg(|c| {
        c.log_burst = 3;
        c.log_per_sec = 1;
    });
    let p = plugin_id("com.example.rate");
    for i in 0..10 {
        log(&fx, &p, LogLevel::Info, &format!("m{i}"));
    }
    let n = fx.sink.lines().len();
    assert!((3..=4).contains(&n), "burst 만큼만 남는다: {n}");
    assert!(
        fx.sink.lines().iter().all(|l| !l.2.contains("dropped")),
        "요약은 회복 후에만"
    );
}

#[test]
fn t_pgw_34_plugin_id_is_a_separate_field_and_never_part_of_the_message() {
    let fx = Fx::new();
    let p = plugin_id("com.example.field");
    log(&fx, &p, LogLevel::Warn, "hello");
    let lines = fx.sink.lines();
    assert_eq!(
        lines,
        vec![(
            "com.example.field".to_owned(),
            LogLevel::Warn,
            "hello".to_owned()
        )]
    );
    assert!(!lines[0].2.contains("com.example"));
}
