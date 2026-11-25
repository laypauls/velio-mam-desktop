//! 앱 테마 설정(`theme`) — 값 하나가 JSON 객체 `{ "mode", "systemDark" }` 다.
//!
//! 두 필드는 모두 필수이고 열거값이 맞아야 한다. 하나라도 틀리면([`ThemeSetting::parse`] 가 `None`) 객체 전체가
//! 기본값으로 대체된다(부분 복구 없음 — 두 값은 한 번에 저장·캐시되는 한 쌍이다). 모르는 필드는 무시한다.

use serde::Serialize;
use serde_json::Value;

/// 테마 선택. `System` 은 Windows 앱 모드를 따른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Gray,
    Dim,
    Dark,
}

/// 시스템이 다크일 때 쓸 테마.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSystemDark {
    Dim,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeSetting {
    pub mode: ThemeMode,
    pub system_dark: ThemeSystemDark,
}

impl Default for ThemeSetting {
    fn default() -> Self {
        Self {
            mode: ThemeMode::System,
            system_dark: ThemeSystemDark::Dim,
        }
    }
}

impl ThemeSetting {
    /// 저장·패치 값을 읽는다. 객체가 아니거나, 필드가 없거나, 열거값이 아니면 `None`.
    pub fn parse(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        let mode = match obj.get("mode")?.as_str()? {
            "system" => ThemeMode::System,
            "light" => ThemeMode::Light,
            "gray" => ThemeMode::Gray,
            "dim" => ThemeMode::Dim,
            "dark" => ThemeMode::Dark,
            _ => return None,
        };
        let system_dark = match obj.get("systemDark")?.as_str()? {
            "dim" => ThemeSystemDark::Dim,
            "dark" => ThemeSystemDark::Dark,
            _ => return None,
        };
        Some(Self { mode, system_dark })
    }
}
