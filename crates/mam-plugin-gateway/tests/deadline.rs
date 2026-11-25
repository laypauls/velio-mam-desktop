//! 오류 누출·마감 인지 DB 접근 (T-pgw-35 ~ 37) — 실제 SQLite, 실제 풀. 시계 의존은 짧은 상대 마감뿐(sleep 으로 맞추지 않는다).
//!
//! 분류: Business rule(정적 메시지) / Time(마감) / External(풀 고갈·종료)

mod common;

use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use common::*;
use mam_db::Priority;
use mam_plugin_gateway::ALLOWED_MESSAGES;
use mam_plugin_host::InvokeScope;
use serde_json::json;

const WAIT: Duration = Duration::from_secs(10);
const SHORT: Duration = Duration::from_millis(150);

#[test]
fn t_pgw_35_every_error_message_is_from_the_static_table_and_leaks_nothing() {
    // Arrange: 문자열 키·값·경로를 입력에 심고, 모든 오류 경로를 지난다
    let fx = Fx::with_cfg(|c| {
        c.kv_quota_bytes = 200;
        c.max_asset_tags = 1;
    });
    let p = fx.full_plugin("com.example.leak");
    let denied = fx.plugin(
        "com.example.denied",
        &mam_plugin_manifest::Permission::ALL,
        Some("[]"),
        Some("true"),
    );
    let id = fx.add_asset("audio", "SecretDir/PrivateName.wav");
    let other = fx.add_asset("image", "SecretDir/Other.jpg");
    let (k, v) = ("SENTINEL_KEY_91", "SENTINEL_VALUE_17");
    let mut errors: Vec<mam_plugin_host::HostError> = Vec::new();
    let mut push = |r: Result<serde_json::Value, mam_plugin_host::HostError>| {
        errors.push(r.expect_err("error path"))
    };
    push(fx.call(&p, &[id], "no.such.method", json!({"key": k})));
    push(fx.call(
        &p,
        &[id],
        "asset.get",
        json!({"assetId": id, "pluginId": k}),
    ));
    push(fx.call(&p, &[id], "asset.get", json!({"assetId": other})));
    push(fx.call(&p, &[id], "asset.get", json!("SENTINEL")));
    push(fx.call(
        &p,
        &[id],
        "asset.getDerived",
        json!({"assetId": id, "kind": "thumbnail"}),
    )); // 음성 -> unavailable
    push(fx.call(
        &p,
        &[id],
        "asset.getDerived",
        json!({"assetId": id, "kind": k}),
    ));
    push(fx.call(&p, &[id], "kv.get", json!({"key": "a\u{0}"})));
    push(fx.call(&p, &[id], "kv.list", json!({"prefix": k.repeat(20)})));
    push(fx.call(&p, &[id], "kv.list", json!({"cursor": ""})));
    push(fx.call(&p, &[id], "kv.list", json!({"limit": 0})));
    push(fx.call(
        &p,
        &[id],
        "kv.set",
        json!({"key": k, "value": "x".repeat(70_000)}),
    ));
    push(fx.call(
        &p,
        &[id],
        "kv.set",
        json!({"key": k, "value": v.repeat(20)}),
    )); // 총량 200 초과
    push(fx.call(
        &p,
        &[id],
        "kv.set",
        json!({"key": k, "value": v, "assetId": other}),
    ));
    push(fx.call(
        &p,
        &[id],
        "tag.add",
        json!({"assetId": id, "tags": [k, "second"]}),
    )); // 자산당 1개 상한
    push(fx.call(
        &p,
        &[id],
        "tag.add",
        json!({"assetId": id, "tags": ["a\nb"]}),
    ));
    push(fx.call(&p, &[id], "tag.add", json!({"assetId": id, "tags": []})));
    push(fx.call(&denied, &[id], "kv.get", json!({"key": k})));
    push(fx.call(
        &denied,
        &[id],
        "tag.add",
        json!({"assetId": id, "tags": [k]}),
    ));
    push(fx.call(
        &plugin_id("com.example.nobody"),
        &[id],
        "kv.get",
        json!({"key": k}),
    ));
    let expired = InvokeScope::new(vec![id])
        .unwrap()
        .with_deadline(Instant::now());
    push(fx.call_scope(&p, &expired, "asset.get", json!({"assetId": id}))); // busy
    // 내부 오류 — 표를 망가뜨려 SQL 오류를 만든다
    fx.exec("ALTER TABLE media_metadata RENAME TO media_metadata_gone");
    push(fx.call(&p, &[id], "asset.get", json!({"assetId": id})));
    // Assert
    let tmp = fx.dir.path().to_string_lossy().to_lowercase();
    let codes: std::collections::BTreeSet<_> = errors.iter().map(|e| e.code.as_str()).collect();
    for expected in [
        "unknown_method",
        "bad_request",
        "not_found",
        "unavailable",
        "too_large",
        "quota_exceeded",
        "permission_denied",
        "busy",
        "internal",
    ] {
        assert!(
            codes.contains(expected),
            "경로 누락: {expected} in {codes:?}"
        );
    }
    for e in &errors {
        assert!(
            ALLOWED_MESSAGES.contains(&e.message.as_str()),
            "정적 표 밖 메시지: {e:?}"
        );
        let m = e.message.to_lowercase();
        for banned in [
            "\\",
            "/",
            ":\\",
            "sqlite",
            "rusqlite",
            "sql",
            "select ",
            "secretdir",
            "privatename",
            "sentinel",
            &tmp,
            "no such table",
            "media_metadata",
        ] {
            assert!(!m.contains(&banned.to_lowercase()), "{banned} in {e:?}");
        }
    }
}

/// 쓰기 스레드를 긴 High 작업으로 막고 해제 송신자와 스레드를 돌려준다
fn block_writer(fx: &Fx) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let pool = Arc::clone(&fx.pool);
    let h = std::thread::spawn(move || {
        pool.write(Priority::High, move |_| {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(WAIT).unwrap();
            Ok(())
        })
        .unwrap();
    });
    started_rx.recv_timeout(WAIT).unwrap();
    (release_tx, h)
}

#[test]
fn t_pgw_36_blocked_writer_gives_busy_within_the_deadline_and_the_late_write_never_runs() {
    // Arrange
    let fx = Fx::new();
    let p = fx.full_plugin("com.example.busy");
    let id = fx.add_asset("image", "a/x.jpg");
    let (release, gate) = block_writer(&fx);
    let short = || {
        InvokeScope::new(vec![id])
            .unwrap()
            .with_deadline(Instant::now() + SHORT)
    };
    // Act: 쓰기 3종 — 각각 마감 안에 busy
    let t = Instant::now();
    let results = [
        fx.call_scope(&p, &short(), "kv.set", json!({"key": "k", "value": 1})),
        fx.call_scope(
            &p,
            &short(),
            "kv.set",
            json!({"key": "k", "value": 1, "assetId": id}),
        ),
        fx.call_scope(&p, &short(), "kv.delete", json!({"key": "k"})),
        fx.call_scope(
            &p,
            &short(),
            "tag.add",
            json!({"assetId": id, "tags": ["late"]}),
        ),
    ];
    // Assert
    for r in results {
        let e = r.unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("busy", "host is busy")
        );
    }
    assert!(t.elapsed() < Duration::from_secs(5));
    release.send(()).unwrap();
    gate.join().unwrap();
    fx.pool.write(Priority::High, |_| Ok(())).unwrap(); // 큐를 비운다
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM plugin_kv"),
        0,
        "마감 지난 쓰기는 이후에도 실행되지 않는다"
    );
    assert_eq!(fx.count("SELECT COUNT(*) FROM tags"), 0);
    assert_eq!(fx.count("SELECT COUNT(*) FROM search_outbox"), 0);
    // 막힘이 풀리면 같은 호출이 성공한다 (재시도 가능)
    assert!(
        fx.call(&p, &[id], "kv.set", json!({"key": "k", "value": 1}))
            .is_ok()
    );
}

#[test]
fn t_pgw_37_reader_exhaustion_is_busy_within_the_deadline_and_a_closed_pool_is_internal() {
    // Arrange: 읽기 연결 2개를 모두 쥔다
    let fx = Fx::new();
    let p = fx.full_plugin("com.example.pool");
    let id = fx.add_asset("image", "a/x.jpg");
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(std::sync::Mutex::new(release_rx));
    let holders: Vec<_> = (0..2)
        .map(|_| {
            let (pool, tx, rx) = (
                Arc::clone(&fx.pool),
                started_tx.clone(),
                Arc::clone(&release_rx),
            );
            std::thread::spawn(move || {
                pool.read(move |_| {
                    tx.send(()).unwrap();
                    let _ = rx.lock().unwrap().recv_timeout(WAIT);
                    Ok(())
                })
                .unwrap();
            })
        })
        .collect();
    for _ in 0..2 {
        started_rx.recv_timeout(WAIT).unwrap();
    }
    // Act
    let scope = InvokeScope::new(vec![id])
        .unwrap()
        .with_deadline(Instant::now() + SHORT);
    let t = Instant::now();
    let e = fx
        .call_scope(&p, &scope, "asset.get", json!({"assetId": id}))
        .unwrap_err();
    // Assert
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("busy", "host is busy")
    );
    assert!(t.elapsed() < Duration::from_secs(5));
    let e = fx
        .call_scope(&p, &scope, "kv.get", json!({"key": "k"}))
        .unwrap_err();
    assert_eq!(e.code, "busy");
    for _ in 0..2 {
        release_tx.send(()).unwrap();
    }
    for h in holders {
        h.join().unwrap();
    }
    assert!(
        fx.call(&p, &[id], "asset.get", json!({"assetId": id}))
            .is_ok()
    );
    // 닫힌 풀(앱 종료): 쓰기는 일반 internal 메시지, 범위 밖은 여전히 DB 없이 not_found
    fx.pool.shutdown();
    for (m, params) in [
        ("kv.set", json!({"key": "k", "value": 1})),
        ("tag.add", json!({"assetId": id, "tags": ["x"]})),
        ("kv.delete", json!({"key": "k"})),
    ] {
        let e = fx.call(&p, &[id], m, params).unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("internal", "host function failed"),
            "{m}"
        );
    }
    assert_eq!(
        Fx::err_code(fx.call(
            &p,
            &[id],
            "tag.add",
            json!({"assetId": id + 1, "tags": ["x"]})
        )),
        "not_found"
    );
}
