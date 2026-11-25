//! serde 원시 구조체 — 모든 단계 `deny_unknown_fields`, 필수 필드는 `Option`(누락 경로 보고용).

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RawManifest {
    #[allow(dead_code)] // apiVersion 은 Value 단계에서 먼저 검사한다
    pub api_version: Option<u32>,
    pub id: Option<String>,
    pub version: Option<String>,
    pub engines: Option<RawEngines>,
    pub main: Option<String>,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub permissions: Option<Vec<String>>,
    pub activation_events: Option<Vec<String>>,
    pub contributes: Option<RawContributes>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawEngines {
    pub mam: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawContributes {
    pub commands: Option<Vec<RawCommand>>,
    pub menus: Option<RawMenus>,
    pub providers: Option<RawProviders>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCommand {
    pub id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RawMenus {
    pub asset_context: Option<Vec<RawMenuItem>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawMenuItem {
    pub command: Option<String>,
    pub when: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawProviders {
    pub metadata: Option<Vec<RawProvider>>,
    pub analysis: Option<Vec<RawProvider>>,
    pub export: Option<Vec<RawExportProvider>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RawProvider {
    pub id: Option<String>,
    pub title: Option<String>,
    pub asset_kinds: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct RawExportProvider {
    pub id: Option<String>,
    pub title: Option<String>,
    pub asset_kinds: Option<Vec<String>>,
    pub file_extension: Option<String>,
}
