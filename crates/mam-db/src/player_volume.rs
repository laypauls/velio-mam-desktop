//! 플레이어 볼륨 기억 설정(`playerVolume`) — 값 하나가 JSON 객체다.
//!
//! `layout` 과 같이 **필드 단위로 정규화**한다: 빠졌거나 타입이 잘못된 필드는 그 필드만 기본값이 되고
//! (`level` 은 0..=100 으로 clamp), 모르는 필드는 무시한다. 객체가 아니면 `None`.
//! 음소거(`muted`)는 `level` 과 별도로 저장해 해제 시 이전 level 로 복귀한다.

use serde::Serialize;
use serde_json::Value;

/// 볼륨 상한(%).
pub const MAX_VOLUME_LEVEL: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerVolumeSetting {
    pub level: u32,
    pub muted: bool,
}

impl Default for PlayerVolumeSetting {
    fn default() -> Self {
        Self {
            level: MAX_VOLUME_LEVEL,
            muted: false,
        }
    }
}

impl PlayerVolumeSetting {
    /// 저장·패치 값을 읽는다. 객체가 아니면 `None`, 그 밖의 잘못된 필드는 기본값으로 정규화한다.
    pub fn parse(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        let d = Self::default();
        let level = obj
            .get("level")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite())
            // 반올림 뒤 clamp — `as` 는 범위 밖 f64 를 포화시키므로 먼저 f64 에서 clamp 한다
            .map(|x| x.round().clamp(0.0, f64::from(MAX_VOLUME_LEVEL)) as u32)
            .unwrap_or(d.level);
        let muted = obj.get("muted").and_then(Value::as_bool).unwrap_or(d.muted);
        Some(Self { level, muted })
    }
}
