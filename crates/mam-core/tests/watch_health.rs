//! 폴더 상태의 신호원 상태 `watchHealth` (T-health-08 ~ 09) — 선택 필드, 카멜 케이스, 시계 주입.
//!
//! 분류: Happy (08 신호·저하가 `list_folders` JSON 에 실린다) / Boundary (09 보고할 것이 없으면 키가 없다 — 기존 계약 보호).
//! 실제 SQLite + 임시 폴더 + 주입 시계. 가짜 감지기가 보관한 sink 로 시험이 직접 보고한다.
#![cfg(windows)]

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::env::{Env, T0, wait_until};
use mam_core::watch::{
    ChangeDetector, ChangeSink, DetectError, DetectorOptions, DetectorRegistry, ProbeResult, WatchHandle, WatchTarget,
};
use serde_json::json;

struct Capture(Arc<Mutex<Option<ChangeSink>>>);
struct Handle;
impl WatchHandle for Handle {}

impl ChangeDetector for Capture {
    fn id(&self) -> &'static str {
        "capture"
    }
    fn probe(&self, _t: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }
    fn start(
        &self,
        _t: &WatchTarget,
        _o: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        *self.0.lock().unwrap() = Some(sink);
        Ok(Box::new(Handle))
    }
}

fn watched() -> (Env, i64, ChangeSink) {
    let slot = Arc::new(Mutex::new(None));
    let detectors = Arc::new(DetectorRegistry::new(vec![Arc::new(Capture(Arc::clone(&slot)))], &["capture"]));
    let env = Env::with_config(move |c, _| c.detectors = detectors);
    env.write("lib/a.jpg", 1);
    env.ingest.start().unwrap();
    let id = env.ingest.add_folder(env.path("lib").to_str().unwrap()).unwrap().folder_id;
    assert!(wait_until(Duration::from_secs(30), || slot.lock().unwrap().is_some()));
    let sink = slot.lock().unwrap().clone().unwrap();
    (env, id, sink)
}

fn view_json(env: &Env, id: i64) -> serde_json::Value {
    let v = env.ingest.list_folders().unwrap().into_iter().find(|f| f.folder_id == id).unwrap();
    serde_json::to_value(v).unwrap()
}

#[test]
fn t_health_08_signals_and_degraded_states_reach_the_folder_view_with_the_injected_time() {
    // Arrange
    let (env, id, sink) = watched();

    // Act 1: 신호
    env.clock.advance(7_000);
    sink.signal();

    // Assert 1
    assert_eq!(view_json(&env, id)["watchHealth"], json!({"state": "ok", "lastSignalAt": T0 + 7_000}));

    // Act 2: 저하 -> 복구
    env.clock.advance(1_000);
    sink.degraded("watch.health.reconnecting");
    let degraded = view_json(&env, id);
    sink.recovered();

    // Assert 2
    assert_eq!(
        degraded["watchHealth"],
        json!({"state": "degraded", "since": T0 + 8_000, "reasonKey": "watch.health.reconnecting"})
    );
    assert_eq!(view_json(&env, id)["watchHealth"]["state"], "ok");
    env.ingest.shutdown();
}

#[test]
fn t_health_09_nothing_to_report_leaves_the_key_out_of_the_view() {
    // Arrange / Act: 감시는 시작됐지만 아직 신호가 없다
    let (env, id, _sink) = watched();

    // Assert: 선택 필드 — 정상·신호 없음이면 JSON 에 없다 (기존 폴더 뷰 계약이 그대로다)
    assert!(view_json(&env, id).get("watchHealth").is_none());
    env.ingest.shutdown();
}
