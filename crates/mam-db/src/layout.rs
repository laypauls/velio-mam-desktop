//! 화면 레이아웃 기억 설정(`layout`) — 값 하나가 JSON 객체다.
//!
//! `galleryCardDisplay` 와 달리 **필드 단위로 정규화**한다: 빠졌거나 타입·열거값이 잘못된 필드는 그 필드만
//! 기본값이 되고(폭은 [`MIN_VIEWER_WIDTH`] 이상으로 clamp), 모르는 필드는 무시한다. 객체가 아니면 `None`.
//! 레이아웃은 UI 편의 값이라 손상됐다고 설정 전체(또는 읽기 전체)를 거부할 이유가 없다.

use serde::Serialize;
use serde_json::Value;

/// 콘텐츠 보기 폭 최소값(px).
pub const MIN_VIEWER_WIDTH: u32 = 320;
/// 콘텐츠 보기 폭 상한(px) — 손상된 거대 값 방어.
pub const MAX_VIEWER_WIDTH: u32 = 8192;
const DEFAULT_VIEWER_WIDTH: u32 = 520;

/// 보기 영역 필름스트립 높이 최소값(px).
pub const MIN_FILMSTRIP_HEIGHT: u32 = 72;
/// 보기 영역 필름스트립 높이 최대값(px).
pub const MAX_FILMSTRIP_HEIGHT: u32 = 160;
const DEFAULT_FILMSTRIP_HEIGHT: u32 = 104;

/// 상세 패널의 탭.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DetailTab {
    Info,
    Ai,
    Similar,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutSetting {
    pub detail_open: bool,
    pub sidebar_open: bool,
    pub viewer_width: u32,
    pub detail_tab: DetailTab,
    /// 필름스트립 높이(px) — 72..=160 으로 clamp, 기본 104.
    pub filmstrip_height: u32,
}

impl Default for LayoutSetting {
    fn default() -> Self {
        Self {
            detail_open: true,
            sidebar_open: true,
            viewer_width: DEFAULT_VIEWER_WIDTH,
            detail_tab: DetailTab::Info,
            filmstrip_height: DEFAULT_FILMSTRIP_HEIGHT,
        }
    }
}

impl LayoutSetting {
    /// 저장·패치 값을 읽는다. 객체가 아니면 `None`, 그 밖의 잘못된 필드는 기본값으로 정규화한다.
    pub fn parse(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        let d = Self::default();
        let flag =
            |name: &str, default: bool| obj.get(name).and_then(Value::as_bool).unwrap_or(default);
        let viewer_width = obj
            .get("viewerWidth")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite())
            // 반올림 뒤 clamp — `as` 는 범위 밖 f64 를 포화시키므로 먼저 f64 에서 clamp 한다
            .map(|x| {
                x.round()
                    .clamp(f64::from(MIN_VIEWER_WIDTH), f64::from(MAX_VIEWER_WIDTH))
                    as u32
            })
            .unwrap_or(d.viewer_width);
        let filmstrip_height = obj
            .get("filmstripHeight")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite())
            .map(|x| {
                x.round().clamp(
                    f64::from(MIN_FILMSTRIP_HEIGHT),
                    f64::from(MAX_FILMSTRIP_HEIGHT),
                ) as u32
            })
            .unwrap_or(d.filmstrip_height);
        let detail_tab = match obj.get("detailTab").and_then(Value::as_str) {
            Some("info") => DetailTab::Info,
            Some("ai") => DetailTab::Ai,
            Some("similar") => DetailTab::Similar,
            _ => d.detail_tab,
        };
        Some(Self {
            detail_open: flag("detailOpen", d.detail_open),
            sidebar_open: flag("sidebarOpen", d.sidebar_open),
            viewer_width,
            detail_tab,
            filmstrip_height,
        })
    }
}
