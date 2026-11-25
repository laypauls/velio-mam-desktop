//! 지연·콜드 시작 측정 (수동: `cargo test --release -p mam-plugin-host --test bench -- --ignored --nocapture`).
//! 조건(하드웨어·친화도·빌드)은 보고에 함께 기록한다. 일반 `cargo test` 에서는 실행되지 않는다.
mod common;

use std::time::{Duration, Instant};

use common::*;
use serde_json::json;

fn pct(sorted: &[Duration], p: f64) -> f64 {
    let i = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[i].as_secs_f64() * 1000.0
}

fn report(label: &str, mut v: Vec<Duration>) {
    v.sort();
    eprintln!("BENCH {label}: n={} P50={:.3}ms P95={:.3}ms P99={:.3}ms max={:.3}ms", v.len(), pct(&v, 0.50), pct(&v, 0.95), pct(&v, 0.99), v.last().map_or(0.0, |d| d.as_secs_f64() * 1000.0));
}

#[test]
#[ignore = "manual measurement"]
fn t_phost_bench_01_cold_start_and_round_trip_latency() {
    let _ = worker_exe(); // 빌드 시간을 측정에서 제외
    // 콜드 시작 = 새 호스트의 첫 호출(프로세스 생성 + init + QuickJS 부팅 + ready + 첫 echo 왕복)
    let mut cold = Vec::new();
    for _ in 0..30 {
        let (host, _, _) = new_host(PLUGIN, |_| {});
        let t = Instant::now();
        ok_echo(&host);
        cold.push(t.elapsed());
    }
    report("cold start (first call incl. spawn)", cold);
    // 왕복: 워밍업 200 + 측정 2000 을 3 회. echo(호스트 호출 없음) 와 hostcall(호스트 호출 1회, F5 와 같은 형태)
    for (label, cmd) in [("round trip echo (no host call)", "echo"), ("round trip hostcall (1 host call)", "hostcall")] {
        for rep in 1..=3 {
            let (host, _, _) = new_host(PLUGIN, |_| {});
            for _ in 0..200 {
                run(&host, cmd, json!({"n": 1})).unwrap();
            }
            let mut v = Vec::with_capacity(2000);
            for _ in 0..2000 {
                let t = Instant::now();
                run(&host, cmd, json!({"n": 1})).unwrap();
                v.push(t.elapsed());
            }
            report(&format!("{label} #{rep}"), v);
        }
    }
}
