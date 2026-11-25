//! KV 계량기 첫 초기화 경합 (L6) — `kv.set` 의 초기화가 writer 밖에서 stale 합계로 `reset` 하면
//! 같은 시점의 `record_provider_output` 이 올린 delta 를 지워 버렸다. 이제 모든 계량기 변경은 writer 스레드에서 직렬화된다.
//!
//! 분류: Concurrency(두 경로 병렬) / Business rule(계량기 = DB 합계) / Boundary(첫 쓰기 전 계량기 없음)

mod common;

use std::sync::{Arc, Barrier};
use std::thread;

use common::*;
use mam_db::Priority;
use mam_db::plugin_settings::{INSTALLED, put};
use mam_db::plugin_tasks::{self, with_immediate_tx};
use mam_plugin_gateway::Completion;
use serde_json::json;

const PLUGIN: &str = "com.example.race";
const ROUNDS: usize = 25;
const PER_THREAD: usize = 12;

fn db_sum(fx: &Fx) -> u64 {
    fx.read(|c| mam_db::plugin_kv::sum_cost(c, PLUGIN).unwrap()) as u64
}

/// 활성 자산 `n` 개 + 각 자산의 대기 작업 (공개 API 로 만든다 — 스키마에 묶이지 않는다)
fn tasks(fx: &Fx, n: usize) -> Vec<(i64, i64)> {
    for i in 0..n {
        fx.add_asset("image", &format!("a/{i}.jpg"));
    }
    fx.pool
        .write(Priority::High, |c| {
            with_immediate_tx(c, |tx| plugin_tasks::run_all_batch(tx, PLUGIN, &[("meta", &["image"])], 0, 1000, 1).map(|_| ()))?;
            Ok(())
        })
        .unwrap();
    let pairs = fx.read(|c| plugin_tasks::pending_page(c, PLUGIN, 0, 1000).unwrap());
    assert_eq!(pairs.len(), n);
    pairs
}

#[test]
fn t_pgw_race_01_first_kv_set_and_provider_record_keep_the_meter_equal_to_the_db_sum() {
    for round in 0..ROUNDS {
        // Arrange: 계량기가 아직 없는 새 게이트웨이 (첫 쓰기 경합)
        let fx = Arc::new(Fx::new());
        let plugin = fx.full_plugin(PLUGIN);
        fx.pool.write(Priority::High, |c| { put(c, INSTALLED, PLUGIN, r#"{"version":"1.0.0","installedAt":1}"#).unwrap(); Ok(()) }).unwrap();
        let pairs = tasks(&fx, PER_THREAD);
        assert_eq!(fx.gw.kv_meter_used(PLUGIN), None, "round {round}: meter starts uninitialized");
        let gate = Arc::new(Barrier::new(2));

        // Act: provider 기록(자산별 KV)과 명령 `kv.set`(전역 키) 을 동시에
        let recorder = {
            let (fx, gate) = (Arc::clone(&fx), Arc::clone(&gate));
            thread::spawn(move || {
                gate.wait();
                for (task, asset) in pairs {
                    let done = Completion { task_id: task, processed_rev: 1 };
                    fx.gw.record_provider_output(PLUGIN, asset, json!({"kv": {"from-provider": "p".repeat(40)}}), done, soon()).unwrap();
                }
            })
        };
        let setter = {
            let (fx, gate, plugin) = (Arc::clone(&fx), Arc::clone(&gate), plugin.clone());
            thread::spawn(move || {
                gate.wait();
                for i in 0..PER_THREAD {
                    fx.call(&plugin, &[], "kv.set", json!({"key": format!("cmd-{i}"), "value": "v".repeat(30)})).unwrap();
                }
            })
        };
        recorder.join().unwrap();
        setter.join().unwrap();

        // Assert: 어느 쪽 delta 도 사라지지 않았다
        let sum = db_sum(&fx);
        assert!(sum > 0);
        assert_eq!(fx.gw.kv_meter_used(PLUGIN), Some(sum), "round {round}: meter must equal SUM(cost) in the DB");
    }
}

#[test]
fn t_pgw_race_02_first_kv_set_initializes_from_existing_rows_inside_the_writer() {
    // Arrange: 계량기 없이 DB 에 이미 행이 있다 (앱 재시작 직후)
    let fx = Fx::with_cfg(|c| c.kv_quota_bytes = 400);
    let plugin = fx.full_plugin(PLUGIN);
    fx.call(&plugin, &[], "kv.set", json!({"key": "old", "value": "x".repeat(100)})).unwrap();
    fx.gw.forget_plugin(PLUGIN);
    assert_eq!(fx.gw.kv_meter_used(PLUGIN), None);

    // Act
    fx.call(&plugin, &[], "kv.set", json!({"key": "new", "value": 1})).unwrap();

    // Assert
    assert_eq!(fx.gw.kv_meter_used(PLUGIN), Some(db_sum(&fx)));
    // 실패 경로: 상한을 넘는 쓰기는 거부되고 계량기·DB 는 그대로
    let before = db_sum(&fx);
    let err = fx.call(&plugin, &[], "kv.set", json!({"key": "huge", "value": "z".repeat(500)}));
    assert!(err.is_err(), "over quota is rejected");
    assert_eq!((fx.gw.kv_meter_used(PLUGIN), db_sum(&fx)), (Some(before), before));
}

#[test]
fn t_pgw_race_03_meter_is_initialized_only_inside_the_writer() {
    // Arrange: 없는 자산을 가리키는 첫 `kv.set` — writer 안에서 `NoAsset` 으로 거부된다
    let fx = Fx::new();
    let plugin = fx.full_plugin(PLUGIN);
    let ghost = 9_999_999;

    // Act
    let err = fx.call(&plugin, &[ghost], "kv.set", json!({"key": "k", "value": 1, "assetId": ghost})).unwrap_err();

    // Assert: 거부된 쓰기는 계량기를 건드리지 않는다 (예전에는 writer 밖에서 먼저 초기화했다 — 경합 창)
    assert_eq!(err.code, "not_found");
    assert_eq!(fx.gw.kv_meter_used(PLUGIN), None);
    // 이어서 정상 쓰기는 writer 안에서 초기화한다
    fx.call(&plugin, &[], "kv.set", json!({"key": "k", "value": 1})).unwrap();
    assert_eq!(fx.gw.kv_meter_used(PLUGIN), Some(db_sum(&fx)));
}
