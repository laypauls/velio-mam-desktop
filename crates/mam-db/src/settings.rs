//! 타입 있는 앱 설정 — `settings` 테이블(key/value TEXT, 값은 JSON) 위의 얇은 계층 (P2-1, D4).
//!
//! 키 목록·기본값·검증은 [`DEFS`] 한 곳에만 있다. 읽을 때는 모르는 키를 무시하고, 잘못된 값은 기본값으로
//! 대체하며 경고를 돌려준다(손상된 설정 한 줄이 앱 기동을 막지 않는다). 쓸 때(프론트 입력)는 반대로
//! 모르는 키·잘못된 값을 거부한다. API 키 같은 비밀은 이 계층에 두지 않는다. 비밀번호의 Argon2id PHC 해시만
//! 예외로 [`load_password_hash`]/[`swap_password_hash`] 전용 키에 둔다(P2-9 D1) — `Settings` 에는 나타나지 않는다.
//!
//! JSON 모양은 프론트 계약(`Settings`)과 같다(camelCase, `backup` 중첩). `dataDir`(MAMData 위치)는
//! DB 가 아니라 부트스트랩 파일에 있으므로 이 계층 밖에서 합친다.

use std::path::Path;

use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::error::DbError;
use crate::gallery_display::GalleryCardDisplay;
use crate::layout::LayoutSetting;
use crate::locale_code::{DEFAULT_LANGUAGE, is_valid_locale_code};
use crate::migrate::{Clock, SystemClock};
use crate::player_volume::PlayerVolumeSetting;
use crate::scene_cuts;
use crate::theme::ThemeSetting;
use crate::view_options::{
    CapturedAtFallback, DoubleClickAction, GALLERY_PAGE_SIZES, GalleryPaging, IndexRailMode,
    PlayRangeKeys, SceneSensitivity, SortFieldsEnabled, ViewerPlacement,
};

/// 백업 설정 (PL9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSettings {
    pub enabled: bool,
    pub interval_hours: u32,
    pub keep: u32,
    /// `None` 이면 `MAMData/backups`.
    pub dir: Option<String>,
}

/// DB 에 저장되는 앱 설정. 정책 근거: P2 정책 문서 PL1·PL9.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// 주기 재스캔 간격(시간, P2-2 D6)
    pub rescan_interval_hours: u32,
    /// 동시에 도는 스캔의 전체 상한 1..=8 (같은 볼륨의 스캔은 항상 하나씩 — 결정 E)
    pub max_concurrent_scans: u32,
    /// PL1 — 비밀번호 인증 면제 시간(분). 0 = 매번 인증.
    pub auth_exempt_minutes: u32,
    pub backup: BackupSettings,
    pub close_to_tray: bool,
    pub first_run_completed: bool,
    /// 갤러리 카드 표시 옵션 (값 하나가 JSON 객체)
    pub gallery_card_display: GalleryCardDisplay,
    /// 콘텐츠 제목 편집 허용 (꺼짐 = 파일명 사용, 저장한 제목은 보존)
    pub title_edit_enabled: bool,
    /// 원본 이동·삭제 동작을 UI 에 노출할지 (표시 전용 — 백엔드 비밀번호 인증은 그대로)
    pub original_actions_enabled: bool,
    /// 정렬 항목별 노출 (관련도는 항상 사용 가능해 여기에 없다)
    pub sort_fields_enabled: SortFieldsEnabled,
    /// 더블클릭(과 Enter) 동작
    pub double_click_action: DoubleClickAction,
    /// 유사 미디어 최대 표시 개수 (프론트가 `limit` 을 줄여 적용)
    pub similar_max_results: u32,
    /// 유사 미디어 최소 유사도 0..=0.99 (0 = 제한 없음, `find_similar` 가 적용)
    pub similar_min_similarity: f64,
    /// 갤러리 탐색 방식 (무한 스크롤 / 페이지)
    pub gallery_paging: GalleryPaging,
    /// 페이지 모드 한 페이지 크기 (50|100|200)
    pub gallery_page_size: u32,
    /// 목차 레일 표시 방식
    pub index_rail: IndexRailMode,
    /// 앱 테마 (값 하나가 JSON 객체)
    pub theme: ThemeSetting,
    /// 촬영일이 없을 때 날짜 대체값 (정렬·목차·범위 필터·표시)
    pub captured_at_fallback: CapturedAtFallback,
    /// 사이드바 별점 메뉴 표시 (꺼도 별점 입력·필터·정렬은 그대로)
    pub sidebar_rating_menu: bool,
    /// 동영상을 내장 플레이어(libmpv)로 재생 (꺼도 WebView 재생 불가 형식은 내장 플레이어로 대체)
    #[serde(rename = "videoPlayerBuiltIn")]
    pub video_player_builtin: bool,
    /// 재생 정지 + 창 숨기기 단축키 (`KeyboardEvent.key` 기반 문자열, `None` = 사용 안 함)
    pub hide_shortcut: Option<String>,
    /// 플레이어 패널만 닫는 단축키 (형식은 `hide_shortcut` 과 동일, `None` = 사용 안 함)
    pub close_viewer_shortcut: Option<String>,
    /// 컬렉션 연속 재생 중 이미지 표시 시간(초) 1..=60
    pub slideshow_image_seconds: u32,
    /// 동영상·음성을 열 때 자동 재생 (꺼짐 = 정지 상태로 열기, 컬렉션 연속 재생은 항상 자동 재생)
    pub media_open_autoplay: bool,
    /// 재생 구간 단축키 프리셋
    pub play_range_keys: PlayRangeKeys,
    /// 단독 재생에서 구간만 재생
    pub play_ranges_only: bool,
    /// 컬렉션 연속 재생에서 구간만 재생
    pub collection_play_ranges_only: bool,
    /// 동영상 외부 자막 표시 (꺼짐 = 자막 끔)
    pub subtitle_visible: bool,
    /// 플레이어에 AI 생성 자막(로컬 음성 인식 결과) 오버레이 표시
    pub generated_subtitle_visible: bool,
    /// 자막 기본 언어 `ko|en|ja|zh` (`None` = 기본 언어 없음)
    pub subtitle_default_language: Option<String>,
    /// 탐색 바에 장면전환점 표시
    pub scene_cuts_visible: bool,
    /// 새로 색인되는 동영상의 장면전환점 자동 추출
    pub scene_detect_auto: bool,
    /// 장면전환점 검출 민감도
    pub scene_detect_sensitivity: SceneSensitivity,
    /// 재생 중 `<` `>` 이동 폭(초) 1..=60 (정지 중에는 1프레임 이동)
    pub player_playing_step_sec: u32,
    /// 이어보기 사용 (끄면 저장·복원 중단, 기존 기록은 유지)
    pub resume_enabled: bool,
    /// 이어보기 최소 콘텐츠 길이(초) 0|60|180|300|600 — 이보다 짧으면 기억하지 않는다 (0 = 제한 없음)
    pub resume_min_duration_sec: u32,
    /// 화면 레이아웃 기억 (값 하나가 JSON 객체)
    pub layout: LayoutSetting,
    /// 플레이어 볼륨·음소거 기억 (값 하나가 JSON 객체)
    pub player_volume: PlayerVolumeSetting,
    /// 플레이어 패널 위치 (갤러리 오른쪽 `right` / 사이드바 왼쪽 `left`)
    pub viewer_placement: ViewerPlacement,
    /// UI 표시 언어 코드 (`en`(기본)·`ko` 내장, 그 외는 언어 폴더의 `<code>.json`). AI 출력 언어와 별개다
    pub language: String,
}

struct Def {
    /// `settings.key`
    key: &'static str,
    /// 프론트 계약상의 경로 (`backup.keep` 처럼 점으로 중첩)
    field: &'static str,
    default: fn() -> Value,
    valid: fn(&Value) -> bool,
}

fn uint_in(v: &Value, lo: u64, hi: u64) -> bool {
    v.as_u64().is_some_and(|n| (lo..=hi).contains(&n))
}

fn backup_dir_ok(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => {
            !s.is_empty() && s.len() <= 1024 && !s.contains('\0') && Path::new(s).is_absolute()
        }
        _ => false,
    }
}

/// 숨기기 단축키 형식: `^(Ctrl\+)?(Alt\+)?(Shift\+)?[^+\s]{1,20}$` (수정키는 이 순서로만).
fn hide_shortcut_ok(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => {
            let mut rest = s.as_str();
            for m in ["Ctrl+", "Alt+", "Shift+"] {
                rest = rest.strip_prefix(m).unwrap_or(rest);
            }
            let n = rest.chars().count();
            (1..=20).contains(&n) && !rest.chars().any(|c| c == '+' || c.is_whitespace())
        }
        _ => false,
    }
}

/// 자막 기본 언어: `null` 또는 정확히 `ko|en|ja|zh` (대소문자 구분)
fn subtitle_language_ok(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => ["ko", "en", "ja", "zh"].contains(&s.as_str()),
        _ => false,
    }
}

/// 모든 설정 키의 단일 정의.
const DEFS: &[Def] = &[
    Def {
        key: "rescan_interval_hours",
        field: "rescanIntervalHours",
        default: || json!(6),
        valid: |v| uint_in(v, 1, 24 * 30),
    },
    Def {
        key: "max_concurrent_scans",
        field: "maxConcurrentScans",
        default: || json!(3),
        valid: |v| uint_in(v, 1, 8),
    },
    Def {
        key: "auth_exempt_minutes",
        field: "authExemptMinutes",
        default: || json!(5),
        valid: |v| uint_in(v, 0, 60),
    },
    Def {
        key: "backup_enabled",
        field: "backup.enabled",
        default: || json!(true),
        valid: Value::is_boolean,
    },
    Def {
        key: "backup_interval_hours",
        field: "backup.intervalHours",
        default: || json!(24),
        valid: |v| uint_in(v, 1, 24 * 30),
    },
    Def {
        key: "backup_keep",
        field: "backup.keep",
        default: || json!(7),
        valid: |v| uint_in(v, 1, 100),
    },
    Def {
        key: "backup_dir",
        field: "backup.dir",
        default: || Value::Null,
        valid: backup_dir_ok,
    },
    Def {
        key: "close_to_tray",
        field: "closeToTray",
        default: || json!(true),
        valid: Value::is_boolean,
    },
    Def {
        key: "first_run_completed",
        field: "firstRunCompleted",
        default: || json!(false),
        valid: Value::is_boolean,
    },
    Def {
        key: "gallery_card_display",
        field: "galleryCardDisplay",
        default: || json!(GalleryCardDisplay::default()),
        valid: |v| GalleryCardDisplay::parse(v).is_some(),
    },
    Def {
        key: "title_edit_enabled",
        field: "titleEditEnabled",
        default: || json!(false),
        valid: Value::is_boolean,
    },
    Def {
        key: "original_actions_enabled",
        field: "originalActionsEnabled",
        default: || json!(false),
        valid: Value::is_boolean,
    },
    Def {
        key: "sort_fields_enabled",
        field: "sortFieldsEnabled",
        default: || json!(SortFieldsEnabled::default()),
        valid: |v| SortFieldsEnabled::parse(v).is_some(),
    },
    Def {
        key: "double_click_action",
        field: "doubleClickAction",
        default: || json!(DoubleClickAction::default()),
        valid: |v| DoubleClickAction::parse(v).is_some(),
    },
    Def {
        key: "similar_max_results",
        field: "similarMaxResults",
        default: || json!(24),
        valid: |v| uint_in(v, 6, 200),
    },
    Def {
        key: "similar_min_similarity",
        field: "similarMinSimilarity",
        default: || json!(0.75),
        valid: |v| {
            v.as_f64()
                .is_some_and(|x| x.is_finite() && (0.0..=0.99).contains(&x))
        },
    },
    Def {
        key: "gallery_paging",
        field: "galleryPaging",
        default: || json!(GalleryPaging::default()),
        valid: |v| GalleryPaging::parse(v).is_some(),
    },
    Def {
        key: "gallery_page_size",
        field: "galleryPageSize",
        default: || json!(100),
        valid: |v| v.as_u64().is_some_and(|n| GALLERY_PAGE_SIZES.contains(&n)),
    },
    Def {
        key: "index_rail",
        field: "indexRail",
        default: || json!(IndexRailMode::default()),
        valid: |v| IndexRailMode::parse(v).is_some(),
    },
    Def {
        key: "theme",
        field: "theme",
        default: || json!(ThemeSetting::default()),
        valid: |v| ThemeSetting::parse(v).is_some(),
    },
    Def {
        key: "captured_at_fallback",
        field: "capturedAtFallback",
        default: || json!(CapturedAtFallback::default()),
        valid: |v| CapturedAtFallback::parse(v).is_some(),
    },
    Def {
        key: "sidebar_rating_menu",
        field: "sidebarRatingMenu",
        default: || json!(true),
        valid: Value::is_boolean,
    },
    Def {
        key: "video_player_builtin",
        field: "videoPlayerBuiltIn",
        default: || json!(true),
        valid: Value::is_boolean,
    },
    Def {
        key: "hide_shortcut",
        field: "hideShortcut",
        default: || json!("Escape"),
        valid: hide_shortcut_ok,
    },
    Def {
        key: "close_viewer_shortcut",
        field: "closeViewerShortcut",
        default: || json!("Q"),
        valid: hide_shortcut_ok,
    },
    Def {
        key: "slideshow_image_seconds",
        field: "slideshowImageSeconds",
        default: || json!(5),
        valid: |v| uint_in(v, 1, 60),
    },
    Def {
        key: "media_open_autoplay",
        field: "mediaOpenAutoplay",
        default: || json!(false),
        valid: Value::is_boolean,
    },
    Def {
        key: "play_range_keys",
        field: "playRangeKeys",
        default: || json!(PlayRangeKeys::default()),
        valid: |v| PlayRangeKeys::parse(v).is_some(),
    },
    Def {
        key: "play_ranges_only",
        field: "playRangesOnly",
        default: || json!(false),
        valid: Value::is_boolean,
    },
    Def {
        key: "collection_play_ranges_only",
        field: "collectionPlayRangesOnly",
        default: || json!(false),
        valid: Value::is_boolean,
    },
    Def {
        key: "subtitle_visible",
        field: "subtitleVisible",
        default: || json!(true),
        valid: Value::is_boolean,
    },
    Def {
        key: "generated_subtitle_visible",
        field: "generatedSubtitleVisible",
        default: || json!(true),
        valid: Value::is_boolean,
    },
    Def {
        key: "subtitle_default_language",
        field: "subtitleDefaultLanguage",
        default: || json!("ko"),
        valid: subtitle_language_ok,
    },
    Def {
        key: "scene_cuts_visible",
        field: "sceneCutsVisible",
        default: || json!(true),
        valid: Value::is_boolean,
    },
    Def {
        key: "scene_detect_auto",
        field: "sceneDetectAuto",
        default: || json!(false),
        valid: Value::is_boolean,
    },
    Def {
        key: "scene_detect_sensitivity",
        field: "sceneDetectSensitivity",
        default: || json!(SceneSensitivity::default()),
        valid: |v| SceneSensitivity::parse(v).is_some(),
    },
    Def {
        key: "player_playing_step_sec",
        field: "playerPlayingStepSec",
        default: || json!(5),
        valid: |v| uint_in(v, 1, 60),
    },
    Def {
        key: "resume_enabled",
        field: "resumeEnabled",
        default: || json!(true),
        valid: Value::is_boolean,
    },
    Def {
        key: "resume_min_duration_sec",
        field: "resumeMinDurationSec",
        default: || json!(180),
        valid: |v| matches!(v.as_u64(), Some(0 | 60 | 180 | 300 | 600)),
    },
    Def {
        key: "layout",
        field: "layout",
        default: || json!(LayoutSetting::default()),
        valid: |v| LayoutSetting::parse(v).is_some(),
    },
    Def {
        key: "player_volume",
        field: "playerVolume",
        default: || json!(PlayerVolumeSetting::default()),
        valid: |v| PlayerVolumeSetting::parse(v).is_some(),
    },
    Def {
        key: "viewer_placement",
        field: "viewerPlacement",
        default: || json!(ViewerPlacement::default()),
        valid: |v| ViewerPlacement::parse(v).is_some(),
    },
    Def {
        key: "language",
        field: "language",
        default: || json!(DEFAULT_LANGUAGE),
        valid: |v| v.as_str().is_some_and(is_valid_locale_code),
    },
];

impl Settings {
    fn from_values(values: &Map<String, Value>) -> Self {
        let num = |f: &str| {
            values
                .get(f)
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0)
        };
        let flag = |f: &str| values.get(f).and_then(Value::as_bool).unwrap_or(false);
        Self {
            rescan_interval_hours: num("rescanIntervalHours"),
            max_concurrent_scans: num("maxConcurrentScans"),
            auth_exempt_minutes: num("authExemptMinutes"),
            backup: BackupSettings {
                enabled: flag("backup.enabled"),
                interval_hours: num("backup.intervalHours"),
                keep: num("backup.keep"),
                dir: values
                    .get("backup.dir")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            close_to_tray: flag("closeToTray"),
            first_run_completed: flag("firstRunCompleted"),
            gallery_card_display: values
                .get("galleryCardDisplay")
                .and_then(GalleryCardDisplay::parse)
                .unwrap_or_default(),
            title_edit_enabled: flag("titleEditEnabled"),
            original_actions_enabled: flag("originalActionsEnabled"),
            sort_fields_enabled: values
                .get("sortFieldsEnabled")
                .and_then(SortFieldsEnabled::parse)
                .unwrap_or_default(),
            double_click_action: values
                .get("doubleClickAction")
                .and_then(DoubleClickAction::parse)
                .unwrap_or_default(),
            similar_max_results: num("similarMaxResults"),
            similar_min_similarity: values
                .get("similarMinSimilarity")
                .and_then(Value::as_f64)
                .unwrap_or(0.75),
            gallery_paging: values
                .get("galleryPaging")
                .and_then(GalleryPaging::parse)
                .unwrap_or_default(),
            gallery_page_size: num("galleryPageSize"),
            index_rail: values
                .get("indexRail")
                .and_then(IndexRailMode::parse)
                .unwrap_or_default(),
            theme: values
                .get("theme")
                .and_then(ThemeSetting::parse)
                .unwrap_or_default(),
            captured_at_fallback: values
                .get("capturedAtFallback")
                .and_then(CapturedAtFallback::parse)
                .unwrap_or_default(),
            sidebar_rating_menu: values
                .get("sidebarRatingMenu")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            video_player_builtin: values
                .get("videoPlayerBuiltIn")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            hide_shortcut: values
                .get("hideShortcut")
                .and_then(Value::as_str)
                .map(str::to_owned),
            close_viewer_shortcut: values
                .get("closeViewerShortcut")
                .and_then(Value::as_str)
                .map(str::to_owned),
            slideshow_image_seconds: num("slideshowImageSeconds"),
            media_open_autoplay: flag("mediaOpenAutoplay"),
            play_range_keys: values
                .get("playRangeKeys")
                .and_then(PlayRangeKeys::parse)
                .unwrap_or_default(),
            play_ranges_only: flag("playRangesOnly"),
            collection_play_ranges_only: flag("collectionPlayRangesOnly"),
            subtitle_visible: values
                .get("subtitleVisible")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            generated_subtitle_visible: values
                .get("generatedSubtitleVisible")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            subtitle_default_language: values
                .get("subtitleDefaultLanguage")
                .and_then(Value::as_str)
                .map(str::to_owned),
            scene_cuts_visible: values
                .get("sceneCutsVisible")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            scene_detect_auto: flag("sceneDetectAuto"),
            scene_detect_sensitivity: values
                .get("sceneDetectSensitivity")
                .and_then(SceneSensitivity::parse)
                .unwrap_or_default(),
            player_playing_step_sec: num("playerPlayingStepSec"),
            resume_enabled: values
                .get("resumeEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            resume_min_duration_sec: num("resumeMinDurationSec"),
            layout: values
                .get("layout")
                .and_then(LayoutSetting::parse)
                .unwrap_or_default(),
            player_volume: values
                .get("playerVolume")
                .and_then(PlayerVolumeSetting::parse)
                .unwrap_or_default(),
            viewer_placement: values
                .get("viewerPlacement")
                .and_then(ViewerPlacement::parse)
                .unwrap_or_default(),
            language: values
                .get("language")
                .and_then(Value::as_str)
                .filter(|s| is_valid_locale_code(s))
                .unwrap_or(DEFAULT_LANGUAGE)
                .to_owned(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::from_values(&defaults())
    }
}

fn defaults() -> Map<String, Value> {
    DEFS.iter()
        .map(|d| (d.field.to_owned(), (d.default)()))
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// 패치에 모르는 설정 이름이 있다.
    #[error("unknown setting: {0}")]
    Unknown(String),
    /// 값의 타입 또는 범위가 잘못됐다.
    #[error("invalid value for setting: {0}")]
    Invalid(String),
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for SettingsError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.into())
    }
}

/// 저장된 설정을 읽는다. 잘못된 값은 기본값으로 대체하고 경고(설정 키 이름만, 값은 싣지 않는다)를 돌려준다.
pub fn load(conn: &Connection) -> Result<(Settings, Vec<String>), DbError> {
    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut values = defaults();
    let mut warnings = Vec::new();
    for (key, raw) in rows {
        let Some(def) = DEFS.iter().find(|d| d.key == key) else {
            continue; // 모르는 키(이후 버전이 쓴 설정 등)는 무시
        };
        match serde_json::from_str::<Value>(&raw) {
            Ok(v) if (def.valid)(&v) => {
                values.insert(def.field.to_owned(), v);
            }
            _ => warnings.push(format!("setting {key} has an invalid value; using default")),
        }
    }
    Ok((Settings::from_values(&values), warnings))
}

/// 비밀번호 PHC 해시의 `settings.key` (P2-9 D1). [`DEFS`] 밖의 키라서 [`load`] 에 보이지 않고,
/// 프론트 패치([`update`])로는 읽거나 쓸 수 없다 — 전용 함수로만 다룬다. 평문이 아니라 Argon2id 해시다.
const PASSWORD_HASH_KEY: &str = "password_hash";

/// 저장된 비밀번호 해시 (설정 전이면 `None`).
pub fn load_password_hash(conn: &Connection) -> Result<Option<String>, DbError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1",
            [PASSWORD_HASH_KEY],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    Ok(raw.and_then(|s| serde_json::from_str::<String>(&s).ok()))
}

/// 해시를 compare-and-set 으로 저장한다: 현재 값이 `expected` 일 때만 쓰고 `true`, 다르면(경합) `false`.
pub fn swap_password_hash(
    conn: &mut Connection,
    expected: Option<&str>,
    new: &str,
) -> Result<bool, DbError> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    if load_password_hash(&tx)?.as_deref() != expected {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![PASSWORD_HASH_KEY, Value::String(new.to_owned()).to_string()],
    )?;
    tx.commit()?;
    Ok(true)
}

/// 패치를 `경로 -> 값` 목록으로 편다. `backup` 은 객체여야 하고 그 안의 키는 `backup.<이름>` 이 된다.
fn flatten(patch: &Map<String, Value>) -> Result<Vec<(String, &Value)>, SettingsError> {
    let mut out = Vec::new();
    for (field, value) in patch {
        if field == "backup" {
            let Value::Object(inner) = value else {
                return Err(SettingsError::Invalid("backup".into()));
            };
            out.extend(inner.iter().map(|(k, v)| (format!("backup.{k}"), v)));
        } else {
            out.push((field.clone(), value));
        }
    }
    Ok(out)
}

/// 일부 설정을 갱신한다(`patch` 는 계약의 `SettingsPatch` — 부분 `Settings`, `backup` 도 부분). 전부 검증한 뒤
/// 한 트랜잭션으로 저장하므로 하나라도 잘못되면 아무것도 바뀌지 않는다. 갱신된 전체 설정을 돌려준다.
pub fn update(
    conn: &mut Connection,
    patch: &Map<String, Value>,
) -> Result<Settings, SettingsError> {
    let mut changes = Vec::new();
    for (field, value) in flatten(patch)? {
        let def = DEFS
            .iter()
            .find(|d| d.field == field)
            .ok_or_else(|| SettingsError::Unknown(field.clone()))?;
        if !(def.valid)(value) {
            return Err(SettingsError::Invalid(field));
        }
        changes.push((def.key, value));
    }
    let turns_off_scene_auto = changes.iter().any(|(key, value)| {
        *key == scene_cuts::SETTING_DETECT_AUTO && **value == Value::Bool(false)
    });
    let tx = conn.transaction()?;
    // 장면 자동 추출을 끄는 변경이면 자동 등록분의 대기 행·작업을 같은 트랜잭션에서 정리한다 (true -> false 전이만)
    let was_on = turns_off_scene_auto && scene_cuts::load_scene_detect_auto(&tx)?;
    for (key, value) in changes {
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value.to_string()],
        )?;
    }
    if was_on {
        let now = SystemClock.now_ms();
        scene_cuts::drop_auto_pending_tx(&tx, None, now).map_err(|e| match e {
            scene_cuts::SceneError::Sqlite(e) => SettingsError::from(e),
            scene_cuts::SceneError::InvalidInput(f) => SettingsError::Invalid(f.into()),
        })?;
    }
    tx.commit()?;
    Ok(load(conn)?.0)
}
