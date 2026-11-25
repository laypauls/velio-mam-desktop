//! 분류: Happy. External·Concurrency 는 순수 crate(IO·공유 상태 없음)라 해당 없음.

use mam_plugin_manifest::{
    ActivationEvent, AssetKind, CommandId, EngineError, Permission, WhenContext, WhenValue,
    check_engine, parse_manifest,
};
use semver::Version;

const FULL: &str = include_str!("fixtures/full.json");
const MINIMAL: &str = include_str!("fixtures/minimal.json");

#[test]
fn t_pmanifest_01_full_manifest_parses() {
    // Arrange / Act
    let m = parse_manifest(FULL).expect("full manifest");

    // Assert
    assert_eq!(m.api_version, 1);
    assert_eq!(m.id.as_str(), "com.example.exif-summary");
    assert_eq!(m.version, Version::new(1, 0, 0));
    assert_eq!(m.main.as_str(), "dist/main.js");
    assert_eq!(m.display_name.as_str(), "plugin.name");
    assert_eq!(
        m.description.as_ref().map(|d| d.as_str()),
        Some("plugin.description")
    );
    assert_eq!(m.permissions.len(), 5);
    assert!(m.permissions.contains(&Permission::TagWrite));
    assert_eq!(
        m.activation_events[0],
        ActivationEvent::OnCommand(CommandId::try_from("summarize").expect("id"))
    );
    assert_eq!(m.activation_events.len(), 2);
    assert_eq!(
        m.contributes.commands[0].title.as_str(),
        "command.summarize"
    );
    let when = m.contributes.menus.asset_context[0]
        .when
        .as_ref()
        .expect("when");
    let ctx = WhenContext::new()
        .with("selection.count", WhenValue::Num(2.0))
        .with("asset.kind", WhenValue::Str("image".into()))
        .with("asset.offline", WhenValue::Bool(false));
    assert!(when.eval(&ctx));
    assert_eq!(
        m.contributes.providers.metadata[0].asset_kinds,
        vec![AssetKind::Image]
    );
    assert!(m.contributes.providers.analysis.is_empty());
    assert_eq!(m.contributes.providers.export[0].file_extension, "csv");
}

#[test]
fn t_pmanifest_02_minimal_manifest_has_empty_defaults() {
    let m = parse_manifest(MINIMAL).expect("minimal");

    assert!(m.description.is_none());
    assert!(m.permissions.is_empty());
    assert!(m.activation_events.is_empty());
    assert!(m.contributes.commands.is_empty());
    assert!(m.contributes.menus.asset_context.is_empty());
    assert!(m.contributes.providers.metadata.is_empty());
    assert!(m.contributes.providers.export.is_empty());
}

#[test]
fn t_pmanifest_03_serialize_is_camel_case_with_string_enums() {
    let m = parse_manifest(FULL).expect("full");

    let v = serde_json::to_value(&m).expect("serialize");

    assert_eq!(v["apiVersion"], 1);
    assert_eq!(v["displayName"], "plugin.name");
    assert_eq!(v["main"], "dist/main.js");
    assert_eq!(v["activationEvents"][0], "onCommand:summarize");
    assert_eq!(v["activationEvents"][1], "onProvider:exif");
    assert!(
        v["permissions"]
            .as_array()
            .expect("array")
            .iter()
            .any(|p| p == "kv.read")
    );
    assert_eq!(
        v["contributes"]["providers"]["metadata"][0]["assetKinds"][0],
        "image"
    );
    assert_eq!(
        v["contributes"]["providers"]["export"][0]["fileExtension"],
        "csv"
    );
    assert_eq!(
        v["contributes"]["menus"]["assetContext"][0]["when"],
        "selection.count >= 1 && asset.kind == 'image' && !asset.offline"
    );
}

#[test]
fn t_pmanifest_04_permission_roundtrip() {
    for p in Permission::ALL {
        let parsed: Permission = p.as_str().parse().expect("known permission");
        assert_eq!(parsed, p);
    }
    assert_eq!(Permission::ALL.len(), 6);
}

#[test]
fn t_pmanifest_05_main_dot_slash_is_normalized() {
    let m = parse_manifest(FULL).expect("full");

    assert_eq!(m.main.as_str(), "dist/main.js");
    assert_eq!(
        m.main.to_relative_path(),
        std::path::PathBuf::from("dist").join("main.js")
    );
}

#[test]
fn t_pmanifest_06_check_engine_pass_and_prerelease() {
    let m = parse_manifest(MINIMAL).expect("minimal"); // >=0.1.0
    assert_eq!(check_engine(&m, &Version::new(0, 1, 5)), Ok(()));

    let mut m2 = m.clone();
    m2.engines.mam = ">=0.2.0".parse().expect("req");
    let beta = Version::parse("0.2.0-beta.1").expect("version");
    assert_eq!(check_engine(&m2, &beta), Ok(()));

    // 대조: 0.1.9 는 >=0.2.0 에 안 맞는다
    assert!(matches!(
        check_engine(&m2, &Version::new(0, 1, 9)),
        Err(EngineError::Incompatible { .. })
    ));
}
