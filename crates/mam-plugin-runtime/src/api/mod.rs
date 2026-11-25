//! P5·P6 사이의 동결된 인터페이스 — 런타임(P6)이 구현하고 작업 실행기(P5)가 쓰는 trait, IPC DTO, 오류 타입.
//! 이 폴더는 골격 단계에서 완성됐다 — P5·P6 어느 쪽도 시그니처를 바꾸지 않는다(바꾸려면 메인 세션과 합의).

use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

mod dto;
mod errors;

pub use dto::*;
pub use errors::{InstallError, RunError};
pub use mam_plugin_host::{InvokeError, ScopeTooLarge};
pub use mam_plugin_manifest::{
    AssetKind, EngineError, LocaleError, ManifestError, Permission, PluginId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    Metadata,
    Analysis,
    Export,
}

#[derive(Debug, Clone)]
pub struct ProviderSpec {
    pub plugin: PluginId,
    pub provider: String,
    pub kind: ProviderKind,
    /// 비어 있으면 모든 종류
    pub asset_kinds: Vec<AssetKind>,
    pub file_extension: Option<String>,
}

/// `PluginRuntime`(P6)이 구현하고 `PluginJobs`(P5)가 쓴다. P5 시험은 자체 구현을 쓴다.
pub trait ProviderInvoker: Send + Sync {
    /// 설치됨 + 활성 + 상태 Ok 인 플러그인의 Metadata/Analysis provider 만 (내보내기는 자동 실행하지 않는다)
    fn auto_providers(&self) -> Vec<ProviderSpec>;
    fn provider(&self, plugin: &str, provider: &str) -> Result<ProviderSpec, RunError>;
    /// `GrantRegistry` 기준 — 지금 이 권한이 유효한가
    fn allows(&self, plugin: &str, perm: Permission) -> bool;
    /// 블로킹 — 작업 스레드 또는 `spawn_blocking` 에서만. 자산을 정렬·중복 제거해 `InvokeScope` 를 만들고,
    /// 호출 중에는 런타임 락을 쥐지 않는다.
    fn invoke_provider(
        &self,
        plugin: &str,
        provider: &str,
        assets: Vec<i64>,
        params: Value,
    ) -> Result<Value, RunError>;
}

/// `PluginJobs`(P5)가 구현하고 `PluginRuntime`(P6)이 부른다. 기본 구현은 아무것도 하지 않아 P6 시험이 단독으로 돈다.
pub trait JobHooks: Send + Sync {
    /// 설정 커밋 뒤, 항목이 보이는 시점
    fn installed(&self, _plugin: &PluginId, _fresh: bool) {}
    /// `enabled=true` 커밋 + 권한 갱신 뒤
    fn enabled(&self, _plugin: &PluginId) {}
}

pub struct NoopHooks;
impl JobHooks for NoopHooks {}

pub const PLUGIN_CHANGED_EVENT: &str = "plugin://changed";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginChanged {
    pub plugin_id: String,
    pub reason: ChangeReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeReason {
    Installed,
    Upgraded,
    Uninstalled,
    Enabled,
    Disabled,
    AutoDisabled,
    PermissionsChanged,
}

pub type ChangeSink = Arc<dyn Fn(PluginChanged) + Send + Sync>;
