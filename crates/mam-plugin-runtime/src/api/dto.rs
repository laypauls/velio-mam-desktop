//! IPC DTO — TS 계약(`src/lib/ipc-contract-plugin.ts`)과 1:1. 모든 필드 camelCase.
//! 모양은 `src-tauri/src/plugin_ipc_fixture.json` 이 고정한다 (Rust·TS 시험이 같은 파일을 읽는다).

use mam_plugin_manifest::{AssetKind, Permission};
use serde::Serialize;

use super::ProviderKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginStatus {
    Ok,
    /// 패키지가 없거나 손상됐다 (목록에는 남는다)
    Broken,
    /// manifest 의 `engines.mam` 이 이 앱 버전과 맞지 않는다
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallChange {
    Install,
    Upgrade,
    Reinstall,
    Downgrade,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandInfo {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginProviderInfo {
    pub id: String,
    pub title: String,
    pub kind: ProviderKind,
    pub asset_kinds: Vec<AssetKind>,
    pub file_extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: Option<String>,
    pub status: PluginStatus,
    pub enabled: bool,
    /// 항상 true — v1 은 서명 검증이 없다
    pub unsigned: bool,
    pub permissions: Vec<Permission>,
    pub granted: Vec<Permission>,
    pub commands: Vec<CommandInfo>,
    pub providers: Vec<PluginProviderInfo>,
    pub installed_at: i64,
    pub worker_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSummary {
    pub version: String,
    pub granted: Vec<Permission>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginPreview {
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub description: Option<String>,
    pub permissions: Vec<Permission>,
    pub commands: Vec<CommandInfo>,
    pub providers: Vec<PluginProviderInfo>,
    pub unsigned: bool,
    pub installed: Option<InstalledSummary>,
    pub change: InstallChange,
    pub added_permissions: Vec<Permission>,
    /// 설치돼 있지 않아도 같은 ID 의 보존 데이터(KV·작업)가 남아 있다 — 동의 대화상자가 덮어쓰기 경고를 띄운다
    pub has_kept_data: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuItemInfo {
    pub plugin_id: String,
    pub command_id: String,
    pub title: String,
    /// manifest 원문 — UI 가 평가한다 (보안 경계가 아니다)
    pub when: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProviderInfo {
    pub plugin_id: String,
    pub provider_id: String,
    pub title: String,
    pub file_extension: String,
    pub asset_kinds: Vec<AssetKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginContributions {
    pub menu_items: Vec<MenuItemInfo>,
    pub export_providers: Vec<ExportProviderInfo>,
}

/// 메뉴 명령 결과 — 문자열이면 제어 문자를 제거하고 200자로 자른 메시지, 아니면 `null`
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCommandResult {
    pub message: Option<String>,
}

// ---- P5 command 결과 (골격에서 함께 고정) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAllResult {
    pub started: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub bytes_written: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginTaskCount {
    pub plugin_id: String,
    pub pending: u64,
    pub failed: u64,
}
