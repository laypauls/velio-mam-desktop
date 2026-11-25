//! 갤러리 카드 표시 옵션 설정(`galleryCardDisplay`) — 값 하나가 JSON 객체다.
//!
//! 알려진 필드는 있으면 타입·열거값이 맞아야 하고(아니면 [`GalleryCardDisplay::parse`] 가 `None`),
//! 빠진 필드는 필드 단위로 기본값, 모르는 필드는 무시한다(이후 버전이 필드를 추가해도 구·신 앱이 읽을 수 있다).

use serde::Serialize;
use serde_json::{Map, Value};

/// 카드 하단 정보 줄에 보일 항목.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardInfoFields {
    pub type_icon: bool,
    pub name: bool,
    pub ext: bool,
    pub resolution: bool,
    pub size: bool,
    pub date: bool,
}

/// 카드에 표시할 날짜의 기준.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CardDateBy {
    Captured,
    Modified,
}

/// 선택 체크박스 표시 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CardCheck {
    Hover,
    Always,
    Hidden,
}

/// 카드 크기 프리셋.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CardSize {
    Xs,
    S,
    M,
    L,
    Xl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryCardDisplay {
    pub info: CardInfoFields,
    pub date_by: CardDateBy,
    pub check: CardCheck,
    pub card_size: CardSize,
    pub favorite: bool,
    pub rating: bool,
    pub duration: bool,
    pub quick_actions: bool,
}

impl Default for CardInfoFields {
    fn default() -> Self {
        Self {
            type_icon: true,
            name: true,
            ext: true,
            resolution: true,
            size: true,
            date: true,
        }
    }
}

impl Default for GalleryCardDisplay {
    fn default() -> Self {
        Self {
            info: CardInfoFields::default(),
            date_by: CardDateBy::Captured,
            check: CardCheck::Hover,
            card_size: CardSize::M,
            favorite: true,
            rating: true,
            duration: true,
            quick_actions: true,
        }
    }
}

/// 필드가 없으면 `default`, 있는데 불리언이 아니면 `None`.
fn flag(obj: &Map<String, Value>, name: &str, default: bool) -> Option<bool> {
    obj.get(name).map_or(Some(default), Value::as_bool)
}

impl GalleryCardDisplay {
    /// 저장·패치 값을 읽는다. 객체가 아니거나 알려진 필드가 잘못되면 `None`.
    pub fn parse(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        let d = Self::default();
        let info = match obj.get("info") {
            None => d.info,
            Some(i) => {
                let i = i.as_object()?;
                let f = &d.info;
                CardInfoFields {
                    type_icon: flag(i, "typeIcon", f.type_icon)?,
                    name: flag(i, "name", f.name)?,
                    ext: flag(i, "ext", f.ext)?,
                    resolution: flag(i, "resolution", f.resolution)?,
                    size: flag(i, "size", f.size)?,
                    date: flag(i, "date", f.date)?,
                }
            }
        };
        let date_by = match obj.get("dateBy").map(Value::as_str) {
            None => d.date_by,
            Some(Some("captured")) => CardDateBy::Captured,
            Some(Some("modified")) => CardDateBy::Modified,
            Some(_) => return None,
        };
        let check = match obj.get("check").map(Value::as_str) {
            None => d.check,
            Some(Some("hover")) => CardCheck::Hover,
            Some(Some("always")) => CardCheck::Always,
            Some(Some("hidden")) => CardCheck::Hidden,
            Some(_) => return None,
        };
        let card_size = match obj.get("cardSize").map(Value::as_str) {
            None => d.card_size,
            Some(Some("xs")) => CardSize::Xs,
            Some(Some("s")) => CardSize::S,
            Some(Some("m")) => CardSize::M,
            Some(Some("l")) => CardSize::L,
            Some(Some("xl")) => CardSize::Xl,
            Some(_) => return None,
        };
        Some(Self {
            info,
            date_by,
            check,
            card_size,
            favorite: flag(obj, "favorite", d.favorite)?,
            rating: flag(obj, "rating", d.rating)?,
            duration: flag(obj, "duration", d.duration)?,
            quick_actions: flag(obj, "quickActions", d.quick_actions)?,
        })
    }
}
