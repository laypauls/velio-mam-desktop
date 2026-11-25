// Categories: Happy (변경 자산만 발행) / Boundary (100자산 상한) / Concurrency (멱등 flush)
use super::super::test_support::{CollectSink, Fx};
use super::*;

fn events(sink: &CollectSink) -> Vec<serde_json::Value> {
    sink.0
        .lock()
        .unwrap()
        .iter()
        .filter(|(e, _)| e == ANALYSIS_EVENT)
        .map(|(_, p)| p.clone())
        .collect()
}

#[test]
fn t_p3w_evt_01_flush_emits_only_changed_assets_with_contract_shape() {
    // Arrange
    let fx = Fx::new();
    fx.consent_and_key();
    fx.insert_asset(1, "image", "a.jpg");
    fx.service.analyze_assets(&[1], None, false).unwrap();
    let sink = CollectSink::default();
    // Act
    let n = fx.service.notifier.flush(&fx.pool, &sink);
    let again = fx.service.notifier.flush(&fx.pool, &sink);
    // Assert
    assert_eq!((n, again), (1, 0), "두 번째 flush 는 보낼 것이 없다");
    let ev = events(&sink);
    assert_eq!(ev.len(), 1);
    let change = &ev[0]["changes"][0];
    assert_eq!(change["assetId"], 1);
    let kinds: Vec<&str> = change["status"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"describe") && kinds.contains(&"stt"));
    assert!(change["status"][0].get("errorCode").is_some());
    assert!(change["status"][0].get("updatedAt").is_some());
    // 상태가 바뀌면 다시 발행된다
    fx.service.revoke_consent().unwrap();
    fx.service.notifier.mark(1);
    assert_eq!(fx.service.notifier.flush(&fx.pool, &sink), 1);
    assert_eq!(events(&sink).len(), 2);
}

#[test]
fn t_p3w_evt_02_flush_is_capped_per_event() {
    let fx = Fx::new();
    let sink = CollectSink::default();
    for id in 1..=150 {
        fx.service.notifier.mark(id);
    }
    assert_eq!(fx.service.notifier.flush(&fx.pool, &sink), FLUSH_MAX);
    assert_eq!(fx.service.notifier.flush(&fx.pool, &sink), 50);
    assert_eq!(fx.service.notifier.flush(&fx.pool, &sink), 0);
}

#[test]
fn t_p3w_evt_03_active_asset_is_rechecked_until_finished() {
    let fx = Fx::new();
    fx.consent_and_key();
    fx.insert_asset(1, "image", "a.jpg");
    let sink = CollectSink::default();
    let n = &fx.service.notifier;
    n.start(1);
    assert_eq!(n.flush(&fx.pool, &sink), 1);
    // 진행 중에 상태가 바뀌면 (dirty 표시 없이도) 다음 flush 가 발행한다
    fx.service.analyze_assets(&[1], None, false).unwrap();
    n.lock().dirty.clear();
    assert_eq!(n.flush(&fx.pool, &sink), 1);
    n.finish(1);
    n.lock().dirty.clear();
    assert_eq!(n.flush(&fx.pool, &sink), 0);
    assert!(n.lock().active.is_empty());
}

#[test]
fn t_p3w_evt_04_late_sink_drops_events_until_connected() {
    let late = LateSink::default();
    late.emit("x", serde_json::json!(1)); // 연결 전: 버려지고 패닉하지 않는다
    let sink = Arc::new(CollectSink::default());
    late.set(sink.clone());
    late.emit("x", serde_json::json!(2));
    assert_eq!(sink.0.lock().unwrap().len(), 1);
}
