//! 호출 경로 — 메뉴 명령(`run_command`)과 provider 호출(`ProviderInvoker`). 호출 중에는 런타임 락을 쥐지 않는다.

use std::sync::Arc;

use mam_plugin_host::{InvokeKind, InvokeScope};
use serde_json::{Value, json};

use super::{Entry, PluginRuntime};
use crate::api::{Permission, PluginCommandResult, ProviderInvoker, ProviderSpec, RunError};

const MAX_MESSAGE_CHARS: usize = 200;

/// 문자열 결과 -> 제어 문자를 지우고 200자로 자른 메시지. 비었거나 문자열이 아니면 `None`
fn message_of(v: &Value) -> Option<String> {
    let text: String = v.as_str()?.chars().filter(|c| !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}') && !mam_plugin_manifest::is_format_char(*c)).take(MAX_MESSAGE_CHARS).collect();
    (!text.is_empty()).then_some(text)
}

impl PluginRuntime {
    /// 플러그인이 실행 가능한(상태 Ok + 활성) 항목을 찾는다
    fn runnable(&self, id: &str) -> Result<Arc<Entry>, RunError> {
        let entry = self.entry(id).ok_or(RunError::NotInstalled)?;
        if entry.runnable() { Ok(entry) } else { Err(RunError::Disabled) }
    }

    /// 감독자 호출 + 결함으로 비활성이 되면 자동 비활성화 처리. 레지스트리 락은 이미 놓은 상태다
    fn invoke_on(&self, entry: &Entry, kind: InvokeKind, target: &str, params: Value, scope: InvokeScope) -> Result<Value, RunError> {
        let Some(host) = entry.host.clone() else { return Err(RunError::Disabled) };
        match host.invoke(kind, target, params, scope) {
            Ok(v) => Ok(v),
            Err(e) => Err(self.after_invoke_error(entry, &host, e)),
        }
    }

    /// 메뉴 명령 실행 — 범위는 선택한 자산뿐이다
    pub fn run_command(&self, id: &str, command: &str, asset_ids: Vec<i64>) -> Result<PluginCommandResult, RunError> {
        let entry = self.runnable(id)?;
        let declared = entry.pkg.as_ref().is_some_and(|p| p.manifest.contributes.commands.iter().any(|c| c.id.as_str() == command));
        if !declared {
            return Err(RunError::UnknownTarget);
        }
        let scope = InvokeScope::new(asset_ids)?;
        let params = json!({ "assetIds": scope.assets.to_vec() });
        let value = self.invoke_on(&entry, InvokeKind::Command, command, params, scope)?;
        Ok(PluginCommandResult { message: message_of(&value) })
    }
}

impl ProviderInvoker for PluginRuntime {
    fn auto_providers(&self) -> Vec<ProviderSpec> {
        let entries: Vec<Arc<Entry>> = self.registry.read().unwrap_or_else(|p| p.into_inner()).values().cloned().collect();
        entries.iter().filter(|e| e.runnable()).flat_map(|e| e.auto_specs()).collect()
    }

    fn provider(&self, plugin: &str, provider: &str) -> Result<ProviderSpec, RunError> {
        // 꺼진 플러그인은 provider 존재 여부보다 먼저 `Disabled` — 작업이 지워지지 않고 대기한다
        let entry = self.runnable(plugin)?;
        entry.provider_spec(provider).ok_or(RunError::UnknownTarget)
    }

    fn allows(&self, plugin: &str, perm: Permission) -> bool {
        self.gateway.grants().allows(plugin, perm)
    }

    fn invoke_provider(&self, plugin: &str, provider: &str, assets: Vec<i64>, params: Value) -> Result<Value, RunError> {
        let entry = self.runnable(plugin)?;
        if entry.provider_spec(provider).is_none() {
            return Err(RunError::UnknownTarget);
        }
        let scope = InvokeScope::new(assets)?;
        self.invoke_on(&entry, InvokeKind::Provider, provider, params, scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Boundary(길이·제어 문자) / Validation(문자열이 아닌 값)

    #[test]
    fn t_pinst_inv_01_message_strips_control_chars_and_truncates() {
        let long = "가".repeat(300);
        assert_eq!(message_of(&json!("a\u{0}b\nc\u{7}")).as_deref(), Some("abc"));
        assert_eq!(message_of(&json!(long)).map(|m| m.chars().count()), Some(200));
        assert_eq!(message_of(&json!("\u{1}\u{2}")), None);
        assert_eq!(message_of(&json!({"a": 1})), None);
        assert_eq!(message_of(&Value::Null), None);
    }

    #[test]
    fn t_pinst_inv_02_message_strips_bidi_override_and_zero_width_chars() {
        assert_eq!(message_of(&json!("ok\u{202E}gpj\u{200B}")).as_deref(), Some("okgpj"));
        assert_eq!(message_of(&json!("\u{202E}\u{FEFF}")), None);
        // 줄/문단 구분자(U+2028/2029)도 지운다 — 로그·표시 줄 위조 방지 (clean_text 와 같은 집합)
        assert_eq!(message_of(&json!("a\u{2028}b\u{2029}c")).as_deref(), Some("abc"));
        assert_eq!(message_of(&json!("\u{2028}\u{2029}")), None);
    }
}
