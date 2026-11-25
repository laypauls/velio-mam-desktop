//! 분류: Validation. External·Concurrency 는 순수 crate(IO·공유 상태 없음)라 해당 없음.

use mam_plugin_manifest::{
    MainPathError as M, ManifestError as E, Manifest, Permission, WhenErrorKind, check_engine,
    parse_manifest,
};
use semver::Version;
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

fn err(patch: Value) -> E {
    parse(&with(patch)).expect_err("should fail")
}

#[test]
fn t_pmanifest_09_unknown_permissions() {
    for p in ["network.fetch", "asset.write.original", "process.spawn", "fs.read"] {
        assert_eq!(
            err(json!({ "permissions": [p] })),
            E::UnknownPermission { value: p.into() }
        );
    }
}

#[test]
fn t_pmanifest_10_duplicate_permission() {
    assert_eq!(
        err(json!({ "permissions": ["kv.read", "kv.read"] })),
        E::DuplicatePermission { value: "kv.read".into() }
    );
}

#[test]
fn t_pmanifest_11_engine_incompatible() {
    let m = parse(&with(json!({ "engines": { "mam": "^0.2" } }))).expect("parse");
    assert!(check_engine(&m, &Version::new(0, 1, 0)).is_err());
    assert!(check_engine(&m, &Version::new(0, 2, 3)).is_ok());
}

#[test]
fn t_pmanifest_12_invalid_version_and_req() {
    for v in ["1.0", "v1.0.0"] {
        assert_eq!(err(json!({ "version": v })), E::InvalidVersion { value: v.into() });
    }
    assert_eq!(
        err(json!({ "engines": { "mam": "abc" } })),
        E::InvalidVersionReq { value: "abc".into() }
    );
}

#[test]
fn t_pmanifest_13_missing_required_fields() {
    for (field, path) in [
        ("apiVersion", "apiVersion"), ("id", "id"), ("version", "version"),
        ("engines", "engines"), ("main", "main"), ("displayName", "displayName"),
    ] {
        let mut v = base();
        v.as_object_mut().expect("obj").remove(field);
        assert_eq!(parse(&v), Err(E::MissingField { path: path.into() }), "{field}");
    }
    assert_eq!(
        err(json!({ "engines": {} })),
        E::MissingField { path: "engines.mam".into() }
    );
}

#[test]
fn t_pmanifest_14_api_version_checked_first() {
    for n in [0, 2] {
        assert!(matches!(err(json!({ "apiVersion": n })), E::UnsupportedApiVersion { .. }));
    }
    assert!(matches!(
        err(json!({ "apiVersion": 2, "futureField": true })),
        E::UnsupportedApiVersion { .. }
    ));
    assert!(matches!(err(json!({ "apiVersion": "1" })), E::UnsupportedApiVersion { .. }));
}

#[test]
fn t_pmanifest_15_excluded_features_are_rejected() {
    assert!(matches!(err(json!({ "panels": [] })), E::Json { .. }));
    assert!(matches!(
        err(json!({ "contributes": { "metadataFields": [] } })),
        E::Json { .. }
    ));
    assert!(matches!(
        err(json!({ "contributes": { "menus": { "media": { "context": [] } } } })),
        E::Json { .. }
    ));
}

#[test]
fn t_pmanifest_16_duplicate_ids() {
    let cmd = json!([{ "id": "a", "title": "t" }, { "id": "a", "title": "t" }]);
    assert!(matches!(
        err(json!({ "contributes": { "commands": cmd } })),
        E::DuplicateId { .. }
    ));
    let providers = json!({
        "metadata": [{ "id": "p", "title": "t" }],
        "export": [{ "id": "p", "title": "t", "fileExtension": "csv" }]
    });
    assert!(matches!(
        err(json!({ "permissions": ["export.write"], "contributes": { "providers": providers } })),
        E::DuplicateId { .. }
    ));
    let menus = json!({ "assetContext": [{ "command": "a" }, { "command": "a" }] });
    let cmds = json!([{ "id": "a", "title": "t" }]);
    assert!(matches!(
        err(json!({ "contributes": { "commands": cmds, "menus": menus } })),
        E::DuplicateId { .. }
    ));
}

#[test]
fn t_pmanifest_17_unknown_references() {
    let menus = json!({ "assetContext": [{ "command": "ghost" }] });
    assert!(matches!(
        err(json!({ "contributes": { "menus": menus } })),
        E::UnknownCommandRef { .. }
    ));
    assert!(matches!(
        err(json!({ "activationEvents": ["onCommand:x"] })),
        E::UnknownCommandRef { .. }
    ));
    assert!(matches!(
        err(json!({ "activationEvents": ["onProvider:y"] })),
        E::UnknownProviderRef { .. }
    ));
}

#[test]
fn t_pmanifest_18_invalid_activation_events() {
    for ev in ["onMediaSelected", "onStartup", "*", "onCommand:", "onCommand:a b"] {
        assert!(
            matches!(err(json!({ "activationEvents": [ev] })), E::InvalidActivationEvent { .. }),
            "{ev}"
        );
    }
}

#[test]
fn t_pmanifest_19_invalid_when_reports_path_and_kind() {
    let cmds = json!([{ "id": "a", "title": "t" }]);
    let menus = json!({ "assetContext": [{ "command": "a", "when": "asset.kind == 'movie'" }] });
    let e = err(json!({ "contributes": { "commands": cmds, "menus": menus } }));
    match e {
        E::InvalidWhen { path, error } => {
            assert_eq!(path, "contributes.menus.assetContext[0].when");
            assert_eq!(error.kind, WhenErrorKind::TypeMismatch);
            assert_eq!(error.offset, 11);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn t_pmanifest_20_main_path_table() {
    let table: [(&str, M); 16] = [
        ("../x.js", M::ParentEscape), ("dist/../../x.js", M::ParentEscape),
        ("/x.js", M::Absolute), ("C:/x.js", M::Colon), ("C:x.js", M::Colon),
        ("\\\\srv\\x.js", M::Backslash), ("dist\\x.js", M::Backslash),
        ("x.ts", M::NotJs), ("x.JS", M::NotJs), ("x.js:s", M::Colon),
        ("dist//x.js", M::EmptyComponent), ("dist/./x.js", M::DotComponent),
        ("con.js", M::ReservedName), ("aux/x.js", M::ReservedName),
        ("x.js.", M::TrailingDotOrSpace), ("", M::Empty),
    ];
    for (main, reason) in table {
        assert_eq!(
            err(json!({ "main": main })),
            E::InvalidMain { value: main.into(), reason },
            "{main:?}"
        );
    }
}

#[test]
fn t_pmanifest_20b_main_path_rejects_console_device_names_and_superscript_com_lpt() {
    // Validation — conin$/conout$ 와 COM¹²³/LPT¹²³ (대소문자·끝 공백 무시) 는 장치로 열린다
    for main in ["conin$.js", "CONOUT$.js", "COM\u{b9}.js", "com\u{b2}.js", "LPT\u{b3}.js", "dist/lpt\u{b9}/x.js", "con .js", "com1.js"] {
        assert_eq!(
            err(json!({ "main": main })),
            E::InvalidMain { value: main.into(), reason: M::ReservedName },
            "{main:?}"
        );
    }
    // 경계 — 비슷하지만 장치가 아닌 이름은 통과
    for main in ["COM0.js", "COM10.js", "COM\u{b9}x.js", "console.js"] {
        assert!(parse(&with(json!({ "main": main }))).is_ok(), "{main:?}");
    }
}

#[test]
fn t_pmanifest_21_plugin_id_table() {
    for id in ["Com.example", "single", "com..x", ".com.x", "com.x-", "con.example", "com.exa_mple"] {
        assert!(matches!(err(json!({ "id": id })), E::InvalidId { .. }), "{id}");
    }
}

#[test]
fn t_pmanifest_22_export_requires_permission() {
    let providers = json!({ "export": [{ "id": "p", "title": "t", "fileExtension": "csv" }] });
    assert_eq!(
        err(json!({ "contributes": { "providers": providers } })),
        E::MissingPermissionFor {
            path: "contributes.providers.export".into(),
            permission: Permission::ExportWrite
        }
    );
}

#[test]
fn t_pmanifest_23_title_key_asset_kind_extension() {
    for key in ["", "has space", "-lead"] {
        assert!(matches!(err(json!({ "displayName": key })), E::InvalidTitleKey { .. }), "{key:?}");
    }
    let bad_kind = json!({ "metadata": [{ "id": "p", "title": "t", "assetKinds": ["doc"] }] });
    assert!(matches!(
        err(json!({ "contributes": { "providers": bad_kind } })),
        E::InvalidAssetKind { .. }
    ));
    let dup_kind = json!({ "metadata": [{ "id": "p", "title": "t", "assetKinds": ["image", "image"] }] });
    assert!(matches!(
        err(json!({ "contributes": { "providers": dup_kind } })),
        E::InvalidAssetKind { .. }
    ));
    for ext in ["", "CSV", "c.s", "abcdefghijk"] {
        let p = json!({ "export": [{ "id": "p", "title": "t", "fileExtension": ext }] });
        assert!(
            matches!(
                err(json!({ "permissions": ["export.write"], "contributes": { "providers": p } })),
                E::InvalidFileExtension { .. }
            ),
            "{ext:?}"
        );
    }
}

#[test]
fn t_pmanifest_24_json_errors_and_bom() {
    assert!(matches!(
        parse_manifest("{\n  \"apiVersion\": 1,,\n}"),
        Err(E::Json { line: 2, .. })
    ));
    assert!(matches!(parse_manifest("[1]"), Err(E::Json { .. })));
    assert!(matches!(parse_manifest(""), Err(E::Json { .. })));
    let with_bom = format!("\u{feff}{}", base());
    assert!(parse_manifest(&with_bom).is_ok());
}

#[test]
fn t_pmanifest_26_display_truncates_plugin_values() {
    let e = err(json!({ "permissions": ["x".repeat(10_000)] }));
    let text = e.to_string();
    assert!(text.len() < 200, "len {}", text.len());
    assert!(text.contains(&"x".repeat(64)));
    assert!(!text.contains(&"x".repeat(65)));
}
