//! 분류: Boundary. External·Concurrency 는 순수 crate(IO·공유 상태 없음)라 해당 없음.

use mam_plugin_manifest::{Manifest, ManifestError as E, parse_manifest};
use serde_json::{Value, json};

fn base() -> Value {
    json!({
        "apiVersion": 1, "id": "com.example.min", "version": "0.1.0",
        "engines": { "mam": ">=0.1.0" }, "main": "main.js", "displayName": "plugin.name"
    })
}

fn parse(v: &Value) -> Result<Manifest, E> {
    parse_manifest(&v.to_string())
}

fn with(patch: Value) -> Value {
    let mut v = base();
    for (k, val) in patch.as_object().expect("object") {
        v[k] = val.clone();
    }
    v
}

fn is_too_many(r: Result<Manifest, E>) -> bool {
    matches!(r, Err(E::TooMany { .. }))
}

fn is_too_long(r: Result<Manifest, E>) -> bool {
    matches!(r, Err(E::TooLong { .. }))
}

fn cmds(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({ "id": format!("c{i}"), "title": "t" }))
        .collect()
}

fn provs(prefix: &str, n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({ "id": format!("{prefix}{i}"), "title": "t" }))
        .collect()
}

fn exports(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({ "id": format!("e{i}"), "title": "t", "fileExtension": "csv" }))
        .collect()
}

#[test]
fn t_pmanifest_27_empty_contributes_forms_pass() {
    assert!(parse(&with(json!({ "contributes": {} }))).is_ok());
    assert!(parse(&base()).is_ok());
    assert!(parse(&with(json!({ "permissions": [] }))).is_ok());
    assert!(
        parse(&with(
            json!({ "contributes": { "menus": {}, "providers": {} } })
        ))
        .is_ok()
    );
}

#[test]
fn t_pmanifest_28_count_limits() {
    // 명령 32 / 33
    assert!(parse(&with(json!({ "contributes": { "commands": cmds(32) } }))).is_ok());
    assert!(is_too_many(parse(&with(
        json!({ "contributes": { "commands": cmds(33) } })
    ))));

    // 메뉴 32 / 33 (명령 32개를 모두 참조)
    let menu = |n: usize| -> Value {
        let items: Vec<Value> = (0..n)
            .map(|i| json!({ "command": format!("c{}", i % 32) }))
            .collect();
        json!({ "commands": cmds(32), "menus": { "assetContext": items } })
    };
    assert!(parse(&with(json!({ "contributes": menu(32) }))).is_ok());
    // 33번째는 같은 명령 재사용이라 DuplicateId 보다 개수 검사가 먼저여야 한다
    assert!(is_too_many(parse(&with(
        json!({ "contributes": menu(33) })
    ))));

    // 종류별 16 / 17
    let kind = |k: &str, n: usize| -> Value {
        let list = if k == "export" {
            exports(n)
        } else {
            provs(k, n)
        };
        json!({ "permissions": ["export.write"], "contributes": { "providers": { k: list } } })
    };
    for k in ["metadata", "analysis", "export"] {
        assert!(parse(&with(kind(k, 16))).is_ok(), "{k} 16");
        assert!(is_too_many(parse(&with(kind(k, 17)))), "{k} 17");
    }

    // 합계 32 / 33
    let total = |export: usize| {
        json!({ "permissions": ["export.write"], "contributes": { "providers": {
            "metadata": provs("m", 16), "analysis": provs("a", 16), "export": exports(export) } } })
    };
    assert!(parse(&with(json!({
        "permissions": ["export.write"],
        "contributes": { "providers": { "metadata": provs("m", 16), "analysis": provs("a", 16) } }
    })))
    .is_ok());
    assert!(is_too_many(parse(&with(total(1)))));

    // 활성화 이벤트 64 / 65
    let events = |n: usize| -> Vec<String> { (0..n).map(|_| "onCommand:c0".to_owned()).collect() };
    assert!(is_too_many(parse(&with(json!({
        "contributes": { "commands": cmds(1) }, "activationEvents": events(65)
    })))));
}

#[test]
fn t_pmanifest_29_length_limits() {
    let id = |len: usize| {
        // "com.<a...>" 형태로 총 길이를 맞춘다
        format!("com.{}", "a".repeat(len - 4))
    };
    assert!(parse(&with(json!({ "id": id(128) }))).is_ok());
    assert!(is_too_long(parse(&with(json!({ "id": id(129) })))));

    let cmd = |len: usize| json!({ "contributes": { "commands": [{ "id": "c".repeat(len), "title": "t" }] } });
    assert!(parse(&with(cmd(64))).is_ok());
    assert!(is_too_long(parse(&with(cmd(65)))));

    let title = |len: usize| json!({ "displayName": "k".repeat(len) });
    assert!(parse(&with(title(128))).is_ok());
    assert!(is_too_long(parse(&with(title(129)))));

    let ver = |len: usize| json!({ "version": format!("1.0.0-{}", "a".repeat(len - 6)) });
    assert!(parse(&with(ver(64))).is_ok());
    assert!(is_too_long(parse(&with(ver(65)))));

    // main: 260 바이트 정확값 / +1 (MainPathError::TooLong 은 InvalidMain 으로 보고)
    let main = |len: usize| json!({ "main": format!("{}.js", "a".repeat(len - 3)) });
    assert!(parse(&with(main(260))).is_ok());
    assert!(matches!(
        parse(&with(main(261))),
        Err(E::InvalidMain { .. })
    ));

    // 구성요소 16 / 17
    let comps = |n: usize| json!({ "main": format!("{}x.js", "d/".repeat(n - 1)) });
    assert!(parse(&with(comps(16))).is_ok());
    assert!(matches!(
        parse(&with(comps(17))),
        Err(E::InvalidMain { .. })
    ));
}

#[test]
fn t_pmanifest_30_manifest_size_limit() {
    let pad = |total: usize| {
        let s = base().to_string();
        format!("{s}{}", " ".repeat(total - s.len()))
    };
    assert!(parse_manifest(&pad(65_536)).is_ok());
    // JSON 파싱 전에 거부 — 뒤에 쓰레기를 붙여도 TooLarge 가 먼저
    let big = format!("{}garbage", pad(65_536));
    assert_eq!(
        parse_manifest(&big),
        Err(E::TooLarge {
            len: 65_536 + 7,
            max: 65_536
        })
    );
    assert!(matches!(
        parse_manifest(&pad(65_537)),
        Err(E::TooLarge { .. })
    ));
}
