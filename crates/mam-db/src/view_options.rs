//! 보기·동작 설정 값 타입 — `sortFieldsEnabled`(정렬 항목별 노출)·`doubleClickAction`(더블클릭 동작).
//!
//! 쓰기(프론트 패치)와 저장값 읽기 모두 같은 검증을 쓴다: 잘못된 값은 [`SortFieldsEnabled::parse`] /
//! [`DoubleClickAction::parse`] 가 `None` 을 돌려주고, 호출자(`settings`)가 거부하거나 기본값으로 대체한다.
//! 관련도 정렬은 항상 사용 가능하므로 [`SortFieldsEnabled`] 에 없다.

use serde::Serialize;
use serde_json::Value;

/// 정렬 항목별 노출 여부. 하나 이상은 켜져 있어야 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SortFieldsEnabled {
    pub captured_at: bool,
    pub modified_at: bool,
    pub name: bool,
    pub size: bool,
    pub duration: bool,
    pub rating: bool,
}

impl Default for SortFieldsEnabled {
    fn default() -> Self {
        Self {
            captured_at: true,
            modified_at: true,
            name: true,
            size: true,
            duration: true,
            rating: true,
        }
    }
}

/// 필수 키 (`rating` 은 이후 버전이 추가해 없을 수 있다 - 없으면 켜짐).
const SORT_KEYS: [&str; 5] = ["capturedAt", "modifiedAt", "name", "size", "duration"];

impl SortFieldsEnabled {
    /// 다섯 필수 키 + 선택 `rating`(없으면 `true`), 모두 불리언, 모르는 키 없음, 하나 이상 `true` 일 때만 `Some`.
    pub fn parse(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        if obj.len() != SORT_KEYS.len() + usize::from(obj.contains_key("rating")) {
            return None;
        }
        let flag = |k: &str| obj.get(k).and_then(Value::as_bool);
        let parsed = Self {
            captured_at: flag("capturedAt")?,
            modified_at: flag("modifiedAt")?,
            name: flag("name")?,
            size: flag("size")?,
            duration: flag("duration")?,
            rating: match obj.get("rating") {
                None => true,
                Some(r) => r.as_bool()?,
            },
        };
        let any = parsed.captured_at
            || parsed.modified_at
            || parsed.name
            || parsed.size
            || parsed.duration
            || parsed.rating;
        any.then_some(parsed)
    }
}

/// 콘텐츠 더블클릭(과 Enter) 동작.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DoubleClickAction {
    #[default]
    Preview,
    OpenSystem,
}

impl DoubleClickAction {
    pub fn parse(v: &Value) -> Option<Self> {
        match v.as_str()? {
            "preview" => Some(Self::Preview),
            "openSystem" => Some(Self::OpenSystem),
            _ => None,
        }
    }
}

/// 갤러리 목록 탐색 방식: 무한 스크롤 / 페이지 나눔.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GalleryPaging {
    #[default]
    Infinite,
    Paged,
}

impl GalleryPaging {
    pub fn parse(v: &Value) -> Option<Self> {
        match v.as_str()? {
            "infinite" => Some(Self::Infinite),
            "paged" => Some(Self::Paged),
            _ => None,
        }
    }
}

/// 목차 레일 표시 방식.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexRailMode {
    #[default]
    Auto,
    Always,
    Off,
}

impl IndexRailMode {
    pub fn parse(v: &Value) -> Option<Self> {
        match v.as_str()? {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// 촬영일이 없는 자산의 날짜 대체값: 비움(`none`) / 파일 수정일(`modified`, 기본).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CapturedAtFallback {
    None,
    #[default]
    Modified,
}

impl CapturedAtFallback {
    pub fn parse(v: &Value) -> Option<Self> {
        match v.as_str()? {
            "none" => Some(Self::None),
            "modified" => Some(Self::Modified),
            _ => None,
        }
    }
}

/// 페이지 모드 한 페이지 크기로 허용되는 값 (검색 `MAX_PAGE_LIMIT` 200 이하).
pub const GALLERY_PAGE_SIZES: [u64; 3] = [50, 100, 200];

/// 재생 구간 단축키 프리셋: 괄호(`[` `]`, 기본) / Premiere(`I` `O`). 대소문자까지 정확히 일치해야 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayRangeKeys {
    #[default]
    Brackets,
    Premiere,
}

impl PlayRangeKeys {
    pub fn parse(v: &Value) -> Option<Self> {
        match v.as_str()? {
            "brackets" => Some(Self::Brackets),
            "premiere" => Some(Self::Premiere),
            _ => None,
        }
    }
}

/// 장면전환점 검출 민감도: 낮음 / 보통(기본) / 높음. 임계값 대응은 `mam-media` 가 정한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SceneSensitivity {
    Low,
    #[default]
    Medium,
    High,
}

impl SceneSensitivity {
    pub fn parse(v: &Value) -> Option<Self> {
        Self::parse_str(v.as_str()?)
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// 플레이어 패널(콘텐츠 보기 영역) 위치: 사이드바 왼쪽(기본, 창 맨 왼쪽) / 갤러리 오른쪽.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewerPlacement {
    #[default]
    Left,
    Right,
}

impl ViewerPlacement {
    pub fn parse(v: &Value) -> Option<Self> {
        match v.as_str()? {
            "right" => Some(Self::Right),
            "left" => Some(Self::Left),
            _ => None,
        }
    }
}
