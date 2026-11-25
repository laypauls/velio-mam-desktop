//! `kv.*` (T-pgw-17 ~ 25) — 실제 SQLite. 네임스페이스·범위·값 상한·목록 페이지·총량(Q8)·동시성.
//!
//! 분류: Happy / Business rule(교차 플러그인·전역/자산 독립·총량) / Boundary(키·값·limit) / Validation / Concurrency

mod common;

use std::sync::Arc;

use common::*;
use mam_plugin_gateway::{Gateway, GatewayConfig};
use mam_plugin_manifest::Permission as P;
use serde_json::{Value, json};

fn setup() -> (Fx, mam_plugin_manifest::PluginId, i64) {
    let fx = Fx::new();
    let p = fx.full_plugin("com.example.kv");
    let id = fx.add_asset("image", "a/x.jpg");
    (fx, p, id)
}

fn set(
    fx: &Fx,
    p: &mam_plugin_manifest::PluginId,
    assets: &[i64],
    key: &str,
    value: Value,
) -> Result<Value, mam_plugin_host::HostError> {
    fx.call(p, assets, "kv.set", json!({"key": key, "value": value}))
}

fn get(fx: &Fx, p: &mam_plugin_manifest::PluginId, assets: &[i64], params: Value) -> Value {
    fx.call(p, assets, "kv.get", params).unwrap()
}

fn db_cost(fx: &Fx, plugin: &str) -> i64 {
    fx.count(&format!("SELECT COALESCE(SUM(length(CAST(key AS BLOB)) + length(CAST(value AS BLOB)) + 64), 0) FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = '{plugin}'"))
}

#[test]
fn t_pgw_17_round_trip_unicode_nested_json_and_overwrite() {
    let (fx, p, _) = setup();
    let nested = json!({"a": [1, 2.5, null, true, {"b": "가나다 😀 \"quoted\" \\ \n"}], "": 0});
    assert_eq!(
        set(&fx, &p, &[], "설정/키 😀", nested.clone()).unwrap(),
        json!({})
    );
    assert_eq!(
        get(&fx, &p, &[], json!({"key": "설정/키 😀"})),
        json!({"found": true, "value": nested})
    );
    set(&fx, &p, &[], "설정/키 😀", json!("덮어씀")).unwrap();
    assert_eq!(
        get(&fx, &p, &[], json!({"key": "설정/키 😀"})),
        json!({"found": true, "value": "덮어씀"})
    );
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM plugin_kv"),
        1,
        "upsert 는 1행"
    );
    // null 값도 값이다 / 없는 키는 오류가 아니다
    set(&fx, &p, &[], "n", Value::Null).unwrap();
    assert_eq!(
        get(&fx, &p, &[], json!({"key": "n"})),
        json!({"found": true, "value": null})
    );
    assert_eq!(
        get(&fx, &p, &[], json!({"key": "missing"})),
        json!({"found": false})
    );
    // 바이트 일치 — 대소문자·정규화를 합치지 않는다
    set(&fx, &p, &[], "Key", json!(1)).unwrap();
    assert_eq!(
        get(&fx, &p, &[], json!({"key": "key"})),
        json!({"found": false})
    );
}

#[test]
fn t_pgw_18_global_and_asset_keys_are_independent_and_asset_must_be_in_scope() {
    let (fx, p, id) = setup();
    let other = fx.add_asset("image", "a/y.jpg");
    fx.call(&p, &[id], "kv.set", json!({"key": "k", "value": "global"}))
        .unwrap();
    fx.call(
        &p,
        &[id],
        "kv.set",
        json!({"key": "k", "value": "asset", "assetId": id}),
    )
    .unwrap();
    assert_eq!(get(&fx, &p, &[id], json!({"key": "k"}))["value"], "global");
    assert_eq!(
        get(&fx, &p, &[id], json!({"key": "k", "assetId": id}))["value"],
        "asset"
    );
    assert_eq!(
        get(&fx, &p, &[id], json!({"key": "k", "assetId": null}))["value"],
        "global"
    );
    // 범위 밖 / 없는 자산 -> not_found (같은 메시지), 읽기·쓰기·삭제·목록 모두
    for (m, params) in [
        ("kv.get", json!({"key": "k", "assetId": other})),
        ("kv.set", json!({"key": "k", "value": 1, "assetId": other})),
        ("kv.delete", json!({"key": "k", "assetId": other})),
        ("kv.list", json!({"assetId": other})),
        ("kv.get", json!({"key": "k", "assetId": 424_242})),
    ] {
        let e = fx.call(&p, &[id], m, params).unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("not_found", "asset not found"),
            "{m}"
        );
    }
    // 범위 안이지만 DB 에 없는(삭제된) 자산에 쓰기 -> not_found (FK 오류를 내부 오류로 새지 않는다)
    let ghost = 777_777;
    let e = fx
        .call(
            &p,
            &[ghost],
            "kv.set",
            json!({"key": "k", "value": 1, "assetId": ghost}),
        )
        .unwrap_err();
    assert_eq!(e.code, "not_found");
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM plugin_kv WHERE asset_id IS NOT NULL"),
        1
    );
    // 삭제는 같은 쪽만
    assert_eq!(
        fx.call(&p, &[id], "kv.delete", json!({"key": "k", "assetId": id}))
            .unwrap(),
        json!({"deleted": true})
    );
    assert_eq!(get(&fx, &p, &[id], json!({"key": "k"}))["value"], "global");
}

#[test]
fn t_pgw_19_plugins_cannot_see_or_delete_each_others_keys() {
    let (fx, a, id) = setup();
    let b = fx.full_plugin("com.example.other");
    fx.call(
        &a,
        &[id],
        "kv.set",
        json!({"key": "shared-name", "value": "A", "assetId": id}),
    )
    .unwrap();
    set(&fx, &a, &[], "shared-name", json!("A-global")).unwrap();
    assert_eq!(
        get(&fx, &b, &[id], json!({"key": "shared-name"})),
        json!({"found": false})
    );
    assert_eq!(
        get(&fx, &b, &[id], json!({"key": "shared-name", "assetId": id})),
        json!({"found": false})
    );
    assert_eq!(
        fx.call(&b, &[id], "kv.list", json!({})).unwrap(),
        json!({"keys": [], "nextCursor": null})
    );
    assert_eq!(
        fx.call(&b, &[id], "kv.list", json!({"assetId": id}))
            .unwrap(),
        json!({"keys": [], "nextCursor": null})
    );
    assert_eq!(
        fx.call(&b, &[id], "kv.delete", json!({"key": "shared-name"}))
            .unwrap(),
        json!({"deleted": false})
    );
    // 같은 키 이름을 B 가 써도 충돌하지 않는다
    set(&fx, &b, &[], "shared-name", json!("B")).unwrap();
    assert_eq!(
        get(&fx, &a, &[id], json!({"key": "shared-name"}))["value"],
        "A-global"
    );
    assert_eq!(
        get(&fx, &b, &[id], json!({"key": "shared-name"}))["value"],
        "B"
    );
    assert_eq!(fx.count("SELECT COUNT(*) FROM plugin_kv"), 3);
}

#[test]
fn t_pgw_20_key_and_value_boundaries() {
    let (fx, p, _) = setup();
    // 키: 1~128자 통과, 빈 값·129자·제어 문자·잘못된 타입 거부
    for ok in ["a".to_owned(), "가".repeat(128)] {
        assert!(set(&fx, &p, &[], &ok, json!(1)).is_ok());
    }
    for bad in [
        String::new(),
        "a".repeat(129),
        "a\u{0}b".into(),
        "a\tb".into(),
        "a\nb".into(),
        "\u{7f}".into(),
    ] {
        assert_eq!(
            Fx::err_code(set(&fx, &p, &[], &bad, json!(1))),
            "bad_request",
            "{bad:?}"
        );
        assert_eq!(
            Fx::err_code(fx.call(&p, &[], "kv.get", json!({"key": bad}))),
            "bad_request"
        );
        assert_eq!(
            Fx::err_code(fx.call(&p, &[], "kv.delete", json!({"key": bad}))),
            "bad_request"
        );
    }
    assert_eq!(
        Fx::err_code(fx.call(&p, &[], "kv.get", json!({"key": 5}))),
        "bad_request"
    );
    // 값: compact JSON 65,536 바이트 통과, 65,537 거부(DB 에 아무것도 남지 않는다)
    let rows = fx.count("SELECT COUNT(*) FROM plugin_kv");
    assert!(
        set(&fx, &p, &[], "edge", json!("a".repeat(65_534))).is_ok(),
        "따옴표 포함 65,536"
    );
    let e = set(&fx, &p, &[], "over", json!("a".repeat(65_535))).unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("too_large", "value too large")
    );
    assert_eq!(fx.count("SELECT COUNT(*) FROM plugin_kv"), rows + 1);
    // 덮어쓰기 실패는 기존 값을 건드리지 않는다
    assert!(set(&fx, &p, &[], "edge", json!("a".repeat(70_000))).is_err());
    assert_eq!(
        get(&fx, &p, &[], json!({"key": "edge"}))["value"]
            .as_str()
            .unwrap()
            .len(),
        65_534
    );
}

#[test]
fn t_pgw_21_delete_is_idempotent_and_kv_write_alone_can_set_but_not_get() {
    let (fx, p, _) = setup();
    set(&fx, &p, &[], "k", json!(1)).unwrap();
    assert_eq!(
        fx.call(&p, &[], "kv.delete", json!({"key": "k"})).unwrap(),
        json!({"deleted": true})
    );
    assert_eq!(
        fx.call(&p, &[], "kv.delete", json!({"key": "k"})).unwrap(),
        json!({"deleted": false})
    );
    let w = fx.plugin(
        "com.example.writer",
        &P::ALL,
        Some(&granted_json(&[P::KvWrite])),
        Some("true"),
    );
    assert!(set(&fx, &w, &[], "k", json!(2)).is_ok());
    assert_eq!(
        Fx::err_code(fx.call(&w, &[], "kv.get", json!({"key": "k"}))),
        "permission_denied"
    );
    assert_eq!(
        Fx::err_code(fx.call(&w, &[], "kv.list", json!({}))),
        "permission_denied"
    );
    assert!(fx.call(&w, &[], "kv.delete", json!({"key": "k"})).is_ok());
}

#[test]
fn t_pgw_22_list_pages_have_no_gaps_or_duplicates_and_prefix_is_literal() {
    let (fx, p, id) = setup();
    let keys: Vec<String> = (0..450).map(|i| format!("k{i:03}")).collect();
    for k in &keys {
        set(&fx, &p, &[], k, json!(1)).unwrap();
    }
    for page_size in [200, 1, 7] {
        let (mut got, mut cursor) = (Vec::<String>::new(), Value::Null);
        loop {
            let r = fx
                .call(
                    &p,
                    &[],
                    "kv.list",
                    json!({"limit": page_size, "cursor": cursor}),
                )
                .unwrap();
            let page: Vec<String> = serde_json::from_value(r["keys"].clone()).unwrap();
            assert!(page.len() <= page_size);
            got.extend(page);
            cursor = r["nextCursor"].clone();
            if cursor.is_null() {
                break;
            }
            if page_size == 1 && got.len() > 460 {
                panic!("runaway");
            }
        }
        assert_eq!(got, keys, "limit {page_size}");
    }
    assert_eq!(
        fx.call(&p, &[], "kv.list", json!({})).unwrap()["keys"]
            .as_array()
            .unwrap()
            .len(),
        50,
        "기본 limit 50"
    );
    for bad in [json!(0), json!(201), json!(-1), json!(1.5), json!("5")] {
        assert_eq!(
            Fx::err_code(fx.call(&p, &[], "kv.list", json!({"limit": bad}))),
            "bad_request"
        );
    }
    assert_eq!(
        Fx::err_code(fx.call(&p, &[], "kv.list", json!({"prefix": "a".repeat(129)}))),
        "bad_request"
    );
    assert_eq!(
        Fx::err_code(fx.call(&p, &[], "kv.list", json!({"cursor": ""}))),
        "bad_request"
    );
    // prefix 는 문자 그대로(LIKE 와일드카드 아님), 자산 키 목록은 따로
    for k in ["a%x", "a%y", "abx", "a_x", "azx"] {
        fx.call(
            &p,
            &[id],
            "kv.set",
            json!({"key": k, "value": 1, "assetId": id}),
        )
        .unwrap();
    }
    let list = |params: Value| -> Vec<String> {
        serde_json::from_value(fx.call(&p, &[id], "kv.list", params).unwrap()["keys"].clone())
            .unwrap()
    };
    assert_eq!(list(json!({"assetId": id, "prefix": "a%"})), ["a%x", "a%y"]);
    assert_eq!(list(json!({"assetId": id, "prefix": "a_"})), ["a_x"]);
    assert_eq!(list(json!({"assetId": id})).len(), 5);
    assert!(
        list(json!({"prefix": "a%"})).is_empty(),
        "전역 목록에 자산 키는 없다"
    );
    assert_eq!(list(json!({"prefix": "k00", "limit": 200})).len(), 10);
}

#[test]
fn t_pgw_23_quota_blocks_growth_but_allows_shrink_delete_and_keeps_old_value_on_failure() {
    // Arrange: 행 비용 = 키 2 + 값 102(따옴표 포함) + 64 = 168, 상한 = 정확히 5행(840)
    let fx = Fx::with_cfg(|c| c.kv_quota_bytes = 840);
    let p = fx.full_plugin("com.example.quota");
    let val = |n: usize| json!("x".repeat(n));
    for i in 1..=5 {
        set(&fx, &p, &[], &format!("k{i}"), val(100)).unwrap();
    }
    assert_eq!(db_cost(&fx, "com.example.quota"), 840);
    // Act / Assert: 6번째는 거부
    let e = set(&fx, &p, &[], "k6", val(1)).unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("quota_exceeded", "kv quota exceeded")
    );
    assert_eq!(fx.count("SELECT COUNT(*) FROM plugin_kv"), 5);
    // 늘어나는 덮어쓰기는 실패하고 기존 값이 남는다
    assert_eq!(
        Fx::err_code(set(&fx, &p, &[], "k1", val(101))),
        "quota_exceeded"
    );
    assert_eq!(get(&fx, &p, &[], json!({"key": "k1"}))["value"], val(100));
    // 같은 크기·줄어드는 덮어쓰기는 통과
    assert!(set(&fx, &p, &[], "k1", val(100)).is_ok());
    assert!(set(&fx, &p, &[], "k1", val(50)).is_ok());
    assert_eq!(db_cost(&fx, "com.example.quota"), 840 - 50);
    // 삭제로 공간을 만들면 다시 들어간다 (남은 공간 = 50 + 새 행 비용 만큼)
    assert_eq!(
        Fx::err_code(set(&fx, &p, &[], "k6", val(100))),
        "quota_exceeded"
    );
    fx.call(&p, &[], "kv.delete", json!({"key": "k2"})).unwrap();
    assert!(set(&fx, &p, &[], "k6", val(100)).is_ok());
    assert!(db_cost(&fx, "com.example.quota") <= 840);
}

#[test]
fn t_pgw_24_meter_is_rebuilt_from_db_and_recomputed_after_cascade_deletes() {
    // Arrange: 상한 = 5행. 전역 2 + 자산 3 = 가득
    let cfg = |c: &mut GatewayConfig| c.kv_quota_bytes = 840;
    let fx = Fx::with_cfg(cfg);
    let p = fx.full_plugin("com.example.rebuild");
    let id = fx.add_asset("image", "a/x.jpg");
    let val = json!("x".repeat(100));
    for k in ["g1", "g2"] {
        set(&fx, &p, &[id], k, val.clone()).unwrap();
    }
    for k in ["a1", "a2", "a3"] {
        fx.call(
            &p,
            &[id],
            "kv.set",
            json!({"key": k, "value": val, "assetId": id}),
        )
        .unwrap();
    }
    // 새 게이트웨이 인스턴스(앱 재시작) — 계량기를 DB 합계로 다시 만든다
    let gw2 = Arc::new(Gateway::new(
        Arc::clone(&fx.pool),
        Arc::new(FixedClock(T0)),
        {
            let mut c = GatewayConfig::new(fx.cache.clone());
            cfg(&mut c);
            c
        },
    ));
    let pid = plugin_id("com.example.rebuild");
    gw2.grants()
        .register(&pid, &P::ALL.iter().copied().collect());
    gw2.grants().refresh(&fx.pool, &pid, soon()).unwrap();
    let scope = mam_plugin_host::InvokeScope::new(vec![id])
        .unwrap()
        .with_deadline(soon());
    let call2 = |params: Value| {
        mam_plugin_host::HostApi::call(
            &*gw2,
            &mam_plugin_host::CallCx {
                plugin: &pid,
                scope: &scope,
            },
            "kv.set",
            params,
        )
    };
    assert_eq!(
        call2(json!({"key": "g3", "value": val})).unwrap_err().code,
        "quota_exceeded",
        "재구성된 계량기가 가득 참을 안다"
    );
    // 자산을 지우면 CASCADE 로 자산 키 3행이 사라진다 — 계량기는 모르지만, 거부 전 재계산으로 다시 허용된다
    fx.exec(&format!("DELETE FROM file_locations WHERE asset_id = {id}; DELETE FROM media_assets WHERE asset_id = {id}"));
    assert_eq!(fx.count("SELECT COUNT(*) FROM plugin_kv"), 2);
    assert!(call2(json!({"key": "g3", "value": val})).is_ok());
    assert!(call2(json!({"key": "g4", "value": val})).is_ok());
    assert!(call2(json!({"key": "g5", "value": val})).is_ok());
    assert_eq!(
        call2(json!({"key": "g6", "value": val})).unwrap_err().code,
        "quota_exceeded"
    );
    assert_eq!(db_cost(&fx, "com.example.rebuild"), 840);
}

#[test]
fn t_pgw_25_concurrent_writers_keep_per_plugin_totals_exact_and_quotas_separate() {
    // Arrange: 플러그인마다 상한 = 16,900 = 키 3글자 행(3 + 102 + 64 = 169) 100개
    let fx = Arc::new(Fx::with_cfg(|c| c.kv_quota_bytes = 100 * 169));
    let a = fx.full_plugin("com.example.a");
    let b = fx.full_plugin("com.example.b");
    let val = json!("x".repeat(100));
    // Act: 두 플러그인이 동시에 120개씩 시도한다 (101번째부터는 상한에 걸린다)
    let handles: Vec<_> = [(a, "A"), (b, "B")]
        .into_iter()
        .map(|(p, name)| {
            let (fx, val) = (Arc::clone(&fx), val.clone());
            std::thread::spawn(move || {
                (0..120)
                    .filter(|i| set(&fx, &p, &[], &format!("{name}{i:02}"), val.clone()).is_ok())
                    .count()
            })
        })
        .collect();
    let ok: Vec<usize> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    // Assert: 플러그인별 합계가 정확하고(서로의 상한을 쓰지 않음) DB 합계와 일치한다
    assert_eq!(ok, [100, 100]);
    for name in ["com.example.a", "com.example.b"] {
        assert_eq!(fx.count(&format!("SELECT COUNT(*) FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = '{name}'")), 100);
        assert_eq!(db_cost(&fx, name), 100 * 169);
    }
    let c = fx.full_plugin("com.example.c");
    assert!(set(&fx, &c, &[], "C00", val).is_ok());
}
