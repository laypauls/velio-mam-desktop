//! 권한·신원·params 검증 (T-pgw-01 ~ 06) — 실제 SQLite, 게이트 ON 상태로 검증한다.
//!
//! 분류: Validation(권한 부분집합, params) / Business rule(manifest ∩ granted, 실패 시 닫힘) / Concurrency(철회)

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::*;
use mam_plugin_manifest::Permission as P;
use serde_json::{Value, json};

fn calls(asset: i64) -> Vec<(&'static str, P, Value)> {
    vec![
        ("asset.get", P::AssetReadMetadata, json!({"assetId": asset})),
        ("asset.getDerived", P::AssetReadDerived, json!({"assetId": asset, "kind": "thumbnail"})),
        ("kv.get", P::KvRead, json!({"key": "k"})),
        ("kv.list", P::KvRead, json!({})),
        ("kv.set", P::KvWrite, json!({"key": "k", "value": 1})),
        ("kv.delete", P::KvWrite, json!({"key": "k"})),
        ("tag.add", P::TagWrite, json!({"assetId": asset, "tags": ["t"]})),
    ]
}

fn setup() -> (Fx, i64) {
    let fx = Fx::new();
    let id = fx.add_asset("image", "a/one.jpg");
    fx.thumbs(id);
    (fx, id)
}

#[test]
fn t_pgw_01_every_permission_subset_gates_every_method() {
    // Arrange: manifest 는 6종 모두 선언 — 허용된 부분집합만 통과해야 한다 (64 조합 × 7 메서드)
    let (fx, id) = setup();
    let pid = fx.full_plugin("com.example.subsets");
    for mask in 0u8..64 {
        let subset: Vec<P> = P::ALL.iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, p)| *p).collect();
        fx.set_settings("com.example.subsets", Some("true"), Some(&granted_json(&subset)));
        fx.gw.grants().refresh(&fx.pool, &pid, soon()).unwrap();
        for (method, need, params) in calls(id) {
            // Act
            let r = fx.call(&pid, &[id], method, params);
            // Assert
            if subset.contains(&need) {
                assert!(r.is_ok(), "{method} with {subset:?}: {r:?}");
            } else {
                let e = r.expect_err(method);
                assert_eq!(e.code, "permission_denied", "{method} with {subset:?}");
                assert_eq!(e.message, format!("permission denied: {}", need.as_str()));
            }
        }
    }
}

#[test]
fn t_pgw_02_effective_grant_is_manifest_intersect_granted() {
    let (fx, id) = setup();
    // 허용됐지만 manifest 에 없음 / manifest 에 있지만 미허용 / 둘 다
    let only_manifest = fx.plugin("com.example.m", &[P::KvRead], Some(&granted_json(&[])), Some("true"));
    let only_granted = fx.plugin("com.example.g", &[], Some(&granted_json(&[P::KvRead])), Some("true"));
    let both = fx.plugin("com.example.b", &[P::KvRead], Some(&granted_json(&[P::KvRead, P::KvWrite])), Some("true"));
    for p in [&only_manifest, &only_granted] {
        assert_eq!(Fx::err_code(fx.call(p, &[id], "kv.get", json!({"key": "k"}))), "permission_denied");
    }
    assert!(fx.call(&both, &[id], "kv.get", json!({"key": "k"})).is_ok());
    assert_eq!(Fx::err_code(fx.call(&both, &[id], "kv.set", json!({"key": "k", "value": 1}))), "permission_denied", "kv.write 는 manifest 에 없다");
    assert_eq!(fx.gw.grants().grant_of("com.example.b").unwrap().effective.count_ones(), 1);
}

#[test]
fn t_pgw_03_disabled_unknown_corrupt_and_unknown_permission_fail_closed() {
    let (fx, id) = setup();
    let ask = |p: &mam_plugin_manifest::PluginId| fx.call(p, &[id], "kv.get", json!({"key": "k"}));
    let granted = granted_json(&[P::KvRead]);
    // 비활성 / enabled 없음 / 손상
    for (name, enabled) in [("dis", Some("false")), ("none", None), ("bad", Some("\"yes\"")), ("garbage", Some("{{"))] {
        let p = fx.plugin(&format!("com.example.{name}"), &[P::KvRead], Some(&granted), enabled);
        assert_eq!(Fx::err_code(ask(&p)), "permission_denied", "{name}");
    }
    // 모르는 플러그인 (등록하지 않음)
    assert_eq!(Fx::err_code(ask(&plugin_id("com.example.stranger"))), "permission_denied");
    // 손상된 granted JSON / 배열이 아님 / granted 없음
    for (i, raw) in [Some("not json"), Some("{\"a\":1}"), Some("[1,2]"), None].into_iter().enumerate() {
        let p = fx.plugin(&format!("com.example.corrupt{i}"), &[P::KvRead], raw, Some("true"));
        assert_eq!(Fx::err_code(ask(&p)), "permission_denied", "{raw:?}");
    }
    // 모르는 권한 문자열은 교집합에서 탈락 — 아는 권한은 그대로
    let p = fx.plugin("com.example.mixed", &[P::KvRead], Some(r#"["kv.read","kv.admin","*"]"#), Some("true"));
    assert!(ask(&p).is_ok());
    let none = fx.plugin("com.example.onlyunknown", &[P::KvRead], Some(r#"["kv.admin","*"]"#), Some("true"));
    assert_eq!(Fx::err_code(ask(&none)), "permission_denied");
    // 설정을 읽지 못하면(풀 마감 경과) 닫힌다
    let pid = plugin_id("com.example.mixed");
    assert!(fx.gw.grants().refresh(&fx.pool, &pid, std::time::Instant::now()).is_err());
    assert_eq!(Fx::err_code(ask(&pid)), "permission_denied");
}

#[test]
fn t_pgw_04_revocation_applies_on_refresh_with_no_db_read_on_the_hot_path() {
    // Arrange
    let (fx, id) = setup();
    let pid = fx.plugin("com.example.rev", &[P::KvRead], Some(&granted_json(&[P::KvRead])), Some("true"));
    assert!(fx.call(&pid, &[id], "kv.get", json!({"key": "k"})).is_ok());
    // Act 1: DB 를 먼저 바꿔도 refresh 전에는 캐시(DB 읽기 없음)가 쓰인다
    fx.set_settings("com.example.rev", Some("true"), Some("[]"));
    assert!(fx.call(&pid, &[id], "kv.get", json!({"key": "k"})).is_ok(), "핫 경로는 DB 를 읽지 않는다");
    // Act 2: refresh 뒤 다음 호출부터 차단
    fx.gw.grants().refresh(&fx.pool, &pid, soon()).unwrap();
    assert_eq!(Fx::err_code(fx.call(&pid, &[id], "kv.get", json!({"key": "k"}))), "permission_denied");
    // unregister(제거)도 닫힘
    fx.set_settings("com.example.rev", Some("true"), Some(&granted_json(&[P::KvRead])));
    fx.gw.grants().refresh(&fx.pool, &pid, soon()).unwrap();
    assert!(fx.call(&pid, &[id], "kv.get", json!({"key": "k"})).is_ok());
    fx.gw.grants().unregister(&pid);
    assert_eq!(Fx::err_code(fx.call(&pid, &[id], "kv.get", json!({"key": "k"}))), "permission_denied");
}

#[test]
fn t_pgw_04b_concurrent_refresh_and_calls_do_not_deadlock_and_the_final_state_wins() {
    let (fx, id) = setup();
    let fx = Arc::new(fx);
    let pid = fx.plugin("com.example.race", &[P::KvRead], Some(&granted_json(&[P::KvRead])), Some("true"));
    let stop = Arc::new(AtomicBool::new(false));
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (fx, pid, stop) = (Arc::clone(&fx), pid.clone(), Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut n = 0u32;
                while !stop.load(Ordering::Acquire) {
                    let r = fx.call(&pid, &[id], "kv.get", json!({"key": "k"}));
                    assert!(r.is_ok() || r.unwrap_err().code == "permission_denied");
                    n += 1;
                }
                n
            })
        })
        .collect();
    for i in 0..40 {
        fx.set_settings("com.example.race", Some("true"), Some(if i % 2 == 0 { "[]" } else { r#"["kv.read"]"# }));
        fx.gw.grants().refresh(&fx.pool, &pid, soon()).unwrap();
    }
    fx.set_settings("com.example.race", Some("true"), Some("[]"));
    fx.gw.grants().refresh(&fx.pool, &pid, soon()).unwrap();
    stop.store(true, Ordering::Release);
    for r in readers {
        r.join().unwrap();
    }
    assert_eq!(Fx::err_code(fx.call(&pid, &[id], "kv.get", json!({"key": "k"}))), "permission_denied");
}

#[test]
fn t_pgw_05_plugin_id_comes_only_from_the_call_context_and_method_names_match_exactly() {
    let (fx, id) = setup();
    let a = fx.full_plugin("com.example.a");
    let b = fx.full_plugin("com.example.b");
    // pluginId 를 params 로 보내면 거부 — 아무것도 쓰이지 않는다
    for method_params in [json!({"key": "k", "value": 1, "pluginId": "com.example.b"}), json!({"key": "k", "value": 1, "plugin": "com.example.b"})] {
        assert_eq!(Fx::err_code(fx.call(&a, &[id], "kv.set", method_params)), "bad_request");
    }
    assert_eq!(fx.count("SELECT COUNT(*) FROM plugin_kv"), 0);
    fx.call(&a, &[id], "kv.set", json!({"key": "k", "value": "A"})).unwrap();
    assert_eq!(fx.call(&b, &[id], "kv.get", json!({"key": "k"})).unwrap(), json!({"found": false}));
    assert_eq!(fx.count("SELECT COUNT(*) FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = 'com.example.a'"), 1);
    // 이름 변형 -> unknown_method (권한과 무관)
    for m in ["asset.GET", "Asset.get", " asset.get", "asset.get ", "asset.get\n", "kv.Get", "kv.", "", "tag.remove", "log", "l10n.t", "asset.getderived", "export.write"] {
        let e = fx.call(&a, &[id], m, json!({})).unwrap_err();
        assert_eq!((e.code.as_str(), e.message.as_str()), ("unknown_method", "unknown method"), "{m:?}");
    }
    // params 가 객체가 아니면 bad_request
    for bad in [json!(null), json!([1]), json!("x"), json!(3)] {
        assert_eq!(Fx::err_code(fx.call(&a, &[id], "asset.get", bad)), "bad_request");
    }
}

#[test]
fn t_pgw_06_asset_id_variants_are_rejected_before_any_db_access() {
    let (fx, id) = setup();
    let pid = fx.full_plugin("com.example.ids");
    let bads = [json!(0), json!(-1), json!(1.5), json!("1"), json!(null), json!(1i64 << 53), json!(true), json!([id]), json!({"a": 1})];
    for bad in bads {
        for (method, params) in [
            ("asset.get", json!({"assetId": bad})),
            ("asset.getDerived", json!({"assetId": bad, "kind": "thumbnail"})),
            ("tag.add", json!({"assetId": bad, "tags": ["t"]})),
        ] {
            assert_eq!(Fx::err_code(fx.call(&pid, &[id], method, params)), "bad_request", "{method} {bad}");
        }
    }
    // kv 의 assetId 는 생략·null 이 전역, 그 밖은 같은 검증
    for bad in [json!(0), json!("1"), json!(1.5), json!(1i64 << 53)] {
        assert_eq!(Fx::err_code(fx.call(&pid, &[id], "kv.get", json!({"key": "k", "assetId": bad}))), "bad_request");
    }
    assert!(fx.call(&pid, &[id], "kv.get", json!({"key": "k", "assetId": null})).is_ok());
    assert_eq!(fx.count("SELECT COUNT(*) FROM tags"), 0);
}
