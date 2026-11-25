//! `tag.add` (T-pgw-26 ~ 30) — 실제 SQLite. revision·outbox 는 같은 트랜잭션, 사용자 태그와 같은 `tag_id`, 상한, 원자성.
//!
//! 분류: Happy / Concurrency(반복·멱등) / Validation / Boundary(상한) / External(강제 실패)

mod common;

use common::*;
use mam_plugin_host::InvokeScope;
use serde_json::json;

fn setup(
    tweak: impl FnOnce(&mut mam_plugin_gateway::GatewayConfig),
) -> (Fx, mam_plugin_manifest::PluginId, i64) {
    let fx = Fx::with_cfg(tweak);
    let p = fx.full_plugin("com.example.tags");
    let id = fx.add_asset("image", "a/x.jpg");
    (fx, p, id)
}

fn revision(fx: &Fx, id: i64) -> i64 {
    fx.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {id}"
    ))
}

fn add(
    fx: &Fx,
    p: &mam_plugin_manifest::PluginId,
    id: i64,
    tags: &[&str],
) -> Result<serde_json::Value, mam_plugin_host::HostError> {
    fx.call(p, &[id], "tag.add", json!({"assetId": id, "tags": tags}))
}

#[test]
fn t_pgw_26_tags_attach_with_one_revision_bump_and_one_priority_1_outbox_row() {
    let (fx, p, id) = setup(|_| {});
    let (rev, outbox) = (
        revision(&fx, id),
        fx.count("SELECT COUNT(*) FROM search_outbox"),
    );
    assert_eq!(
        add(&fx, &p, id, &["Alpha", "beta", "가나다"]).unwrap(),
        json!({"added": 3})
    );
    assert_eq!(revision(&fx, id), rev + 1);
    assert_eq!(fx.count("SELECT COUNT(*) FROM search_outbox") - outbox, 1);
    let (r, prio): (i64, i64) = fx.read(|c| c.query_row("SELECT revision, priority FROM search_outbox WHERE asset_id = ?1 ORDER BY event_id DESC LIMIT 1", [id], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap()))).unwrap());
    assert_eq!((r, prio), (revision(&fx, id), 1));
    assert_eq!(
        fx.count(&format!(
            "SELECT COUNT(*) FROM asset_tags WHERE asset_id = {id}"
        )),
        3
    );
    assert_eq!(
        fx.count("SELECT COUNT(*) FROM tags WHERE name IN ('Alpha','beta','가나다')"),
        3
    );
}

#[test]
fn t_pgw_27_repeat_is_a_no_op_and_variants_map_to_the_users_existing_tag() {
    let (fx, p, id) = setup(|_| {});
    // 사용자 태그 "Café" 를 같은 방식(사용자 편집 경로)으로 만든다
    let ud = mam_core::userdata::UserData::new(
        std::sync::Arc::clone(&fx.pool),
        std::sync::Arc::new(FixedClock(T0)),
    );
    ud.update_asset_tags(mam_core::userdata::UpdateTagsInput {
        asset_ids: vec![id],
        add: vec!["Café".into()],
        remove: vec![],
    })
    .unwrap();
    let user_tag: i64 = fx.count("SELECT tag_id FROM tags WHERE name = 'Café'");
    let other = fx.add_asset("image", "a/y.jpg");
    // NFD + 대문자 변형 -> 같은 tag_id, 새 이름 아님
    assert_eq!(
        fx.call(
            &p,
            &[other],
            "tag.add",
            json!({"assetId": other, "tags": ["CAFE\u{301}"]})
        )
        .unwrap(),
        json!({"added": 1})
    );
    assert_eq!(
        fx.count(&format!(
            "SELECT tag_id FROM asset_tags WHERE asset_id = {other}"
        )),
        user_tag
    );
    assert_eq!(fx.count("SELECT COUNT(*) FROM tags"), 1);
    // 이미 붙은 자산에 반복: added 0, revision·outbox 불변
    let (rev, outbox) = (
        revision(&fx, id),
        fx.count("SELECT COUNT(*) FROM search_outbox"),
    );
    assert_eq!(
        add(&fx, &p, id, &["café", "CAFÉ"]).unwrap(),
        json!({"added": 0})
    );
    assert_eq!(
        (
            revision(&fx, id),
            fx.count("SELECT COUNT(*) FROM search_outbox")
        ),
        (rev, outbox)
    );
}

#[test]
fn t_pgw_28_invalid_names_and_counts_are_bad_request() {
    let (fx, p, id) = setup(|_| {});
    let twenty_one: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
    let twenty: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();
    for tags in [
        json!([""]),
        json!(["   "]),
        json!(["a".repeat(101)]),
        json!(["a\nb"]),
        json!(["ok", "bad\u{0}"]),
        json!([]),
        json!(twenty_one),
        json!("tag"),
        json!([1]),
        json!(["ok", null]),
        json!(null),
    ] {
        let e = fx
            .call(&p, &[id], "tag.add", json!({"assetId": id, "tags": tags}))
            .unwrap_err();
        assert_eq!(e.code, "bad_request", "{tags}");
    }
    assert_eq!(fx.count("SELECT COUNT(*) FROM tags"), 0);
    // 경계: 100자·20개는 통과
    assert!(add(&fx, &p, id, &[&"a".repeat(100)]).is_ok());
    assert_eq!(
        fx.call(&p, &[id], "tag.add", json!({"assetId": id, "tags": twenty}))
            .unwrap(),
        json!({"added": 20})
    );
    assert_eq!(
        fx.call(&p, &[id], "tag.add", json!({"assetId": id}))
            .unwrap_err()
            .code,
        "bad_request"
    );
}

#[test]
fn t_pgw_29_not_found_cases_and_quotas() {
    // Arrange: 자산당 3개, 호출당 새 이름 3개
    let (fx, p, id) = setup(|c| {
        c.max_asset_tags = 3;
        c.max_new_tags_per_call = 3;
    });
    let (deleted, outside) = (
        fx.add_asset("image", "a/d.jpg"),
        fx.add_asset("image", "a/o.jpg"),
    );
    fx.exec(&format!(
        "UPDATE media_assets SET status = 'deleted' WHERE asset_id = {deleted}"
    ));
    for (assets, target) in [
        (vec![id], 999_999),
        (vec![deleted], deleted),
        (vec![id], outside),
    ] {
        let e = fx
            .call(
                &p,
                &assets,
                "tag.add",
                json!({"assetId": target, "tags": ["x"]}),
            )
            .unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("not_found", "asset not found"),
            "{target}"
        );
    }
    assert_eq!(fx.count("SELECT COUNT(*) FROM tags"), 0);
    // 자산당 상한: 3개까지 통과, 4번째 -> quota_exceeded (기존 3개 유지, 새 이름 롤백)
    assert!(add(&fx, &p, id, &["a", "b", "c"]).is_ok());
    let e = add(&fx, &p, id, &["a", "d"]).unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("quota_exceeded", "asset tag limit reached")
    );
    assert_eq!(fx.count("SELECT COUNT(*) FROM tags"), 3);
    // 호출당 새 이름 예산: 같은 InvokeScope 로 2 + 2 > 3 -> 두 번째가 거부, 기존 이름 재사용은 예산을 쓰지 않는다
    let scope = InvokeScope::new(vec![outside])
        .unwrap()
        .with_deadline(soon());
    let ask = |tags: serde_json::Value| {
        fx.call_scope(
            &p,
            &scope,
            "tag.add",
            json!({"assetId": outside, "tags": tags}),
        )
    };
    assert_eq!(ask(json!(["n1", "n2"])).unwrap(), json!({"added": 2}));
    let e = ask(json!(["n3", "n4"])).unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("quota_exceeded", "new tag budget exceeded")
    );
    assert_eq!(
        fx.count(&format!(
            "SELECT COUNT(*) FROM asset_tags WHERE asset_id = {outside}"
        )),
        2
    );
    assert!(fx.count("SELECT COUNT(*) FROM tags WHERE name IN ('n3','n4')") == 0);
    // 이미 있는 이름(a)은 새 이름 예산을 쓰지 않는다 — 예산 1 남음, 자산 상한 3 이내
    assert_eq!(ask(json!(["a"])).unwrap(), json!({"added": 1}));
}

#[test]
fn t_pgw_30_a_failure_after_the_tag_insert_leaves_no_tag_link_revision_or_outbox_row() {
    let (fx, p, id) = setup(|_| {});
    fx.exec("CREATE TRIGGER fail_outbox BEFORE INSERT ON search_outbox BEGIN SELECT RAISE(ABORT, 'forced'); END");
    let (rev, outbox) = (
        revision(&fx, id),
        fx.count("SELECT COUNT(*) FROM search_outbox"),
    );
    let e = add(&fx, &p, id, &["x", "y"]).unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("internal", "host function failed"),
        "SQL 상세는 나가지 않는다"
    );
    assert_eq!(fx.count("SELECT COUNT(*) FROM tags"), 0);
    assert_eq!(fx.count("SELECT COUNT(*) FROM asset_tags"), 0);
    assert_eq!(
        (
            revision(&fx, id),
            fx.count("SELECT COUNT(*) FROM search_outbox")
        ),
        (rev, outbox)
    );
    // 트리거를 치우면 같은 요청이 멱등하게 성공한다
    fx.exec("DROP TRIGGER fail_outbox");
    assert_eq!(add(&fx, &p, id, &["x", "y"]).unwrap(), json!({"added": 2}));
}
