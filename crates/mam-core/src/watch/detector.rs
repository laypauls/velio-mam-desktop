//! 변경 감지기 계약 — 어댑터(컴파일 타임)가 구현하고 `DetectorRegistry` 에 등록한다.
//!
//! 불변 조건 (계약 테스트 `detector_contract_tests!` 가 강제):
//! 1. 놓쳤을 가능성이 있으면 `ChangeSink::rescan` 을 보낸다  2. 원본을 읽기 전용으로만 연다
//! 3. 핸들을 버리면 디렉터리 핸들이 유한 시간 안에 닫히고 `ReleaseToken` 으로 관찰된다  4. 변경이 아닌 이벤트는 버린다

use std::path::PathBuf;

use serde::Serialize;
use serde_json::{Map, Value};

use super::options_ref::{NoRefs, RefResolver, check_ref};
use super::sink::ChangeSink;
use crate::scan::FolderRow;
use crate::volume::VolumeKind;

pub struct WatchTarget {
    pub folder_id: i64,
    pub root: PathBuf,
    pub volume_kind: VolumeKind,
}

impl WatchTarget {
    pub fn of(f: &FolderRow) -> Self {
        Self {
            folder_id: f.folder_id,
            root: f.root_path(),
            volume_kind: f.volume_kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeResult {
    Available,
    Unavailable { reason_key: &'static str },
}

/// 정규화한 감지기 옵션 (기본값과 같은 값은 저장하지 않는다).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DetectorOptions(Map<String, Value>);

impl DetectorOptions {
    /// DB 저장용 JSON 텍스트 (비었으면 None = NULL)
    pub fn to_stored(&self) -> Option<String> {
        self.to_value().map(|v| v.to_string())
    }

    pub fn to_value(&self) -> Option<Value> {
        (!self.0.is_empty()).then(|| Value::Object(self.0.clone()))
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.0.get(key).and_then(Value::as_i64)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.0.get(key).and_then(Value::as_bool)
    }
}

/// 저장된 옵션 JSON 텍스트에서 배타적 열기 검사 옵션을 읽는다 (없음·해석 불가 = 기본값 켜짐).
pub fn exclusive_open_check_of(stored: Option<&str>) -> bool {
    stored
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .and_then(|v| v.get(EXCLUSIVE_OPEN_CHECK).and_then(Value::as_bool))
        .unwrap_or(EXCLUSIVE_OPEN_CHECK_DEFAULT)
}

/// 배타적 열기 검사 기본값 — 사용자 결정(2026-11-21)으로 켜짐. 끄면 `{"exclusiveOpenCheck": false}` 로 저장된다.
pub const EXCLUSIVE_OPEN_CHECK_DEFAULT: bool = true;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OptionField {
    pub key: &'static str,
    pub kind: OptionKind,
    pub min: i64,
    pub max: i64,
    pub default: i64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OptionKind {
    Integer,
    /// JSON `true`/`false` — `min`·`max`·`default` 는 0/1 (기본값과 같은 값은 저장하지 않는다)
    Bool,
    /// 다른 레코드(원격 접속 등)의 ID 를 가리키는 양의 정수 — `{"source": 5}`. 존재 여부는 [`RefResolver`] 가 판정한다.
    /// `min`·`max`·`default` 는 쓰지 않는다 (기본값이 없어 항상 저장한다).
    Ref,
}

/// 모든 감지기가 받는 공통 옵션 — 안정화 게이트(공통 계층)가 읽으므로 감지기 종류와 무관하다.
/// 감지기별 `option_fields` 에는 넣지 않는다(UI 가 감지기 옵션 칸으로 그리지 않게 — 전용 UI 는 후속).
pub const EXCLUSIVE_OPEN_CHECK: &str = "exclusiveOpenCheck";
pub const COMMON_OPTION_FIELDS: &[OptionField] = &[OptionField {
    key: EXCLUSIVE_OPEN_CHECK,
    kind: OptionKind::Bool,
    min: 0,
    max: 1,
    default: EXCLUSIVE_OPEN_CHECK_DEFAULT as i64,
}];

#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error("invalid detector options: {field}")]
    InvalidOptions { field: &'static str },
    #[error("unknown reference: {field}")]
    UnknownReference { field: &'static str },
    #[error("cannot start watching: {0}")]
    Start(String),
}

impl From<notify::Error> for DetectError {
    fn from(e: notify::Error) -> Self {
        Self::Start(e.to_string())
    }
}

pub trait ChangeDetector: Send + Sync {
    /// 저장·IPC 에 쓰는 고정 ID. 배포 후 rename 하지 않는다. `^[a-z][a-z0-9_]*$`
    fn id(&self) -> &'static str;
    /// 가볍게 — 막히거나 네트워크 I/O 를 하지 않는다. 오프라인 폴더도 같은 답을 낸다.
    fn probe(&self, target: &WatchTarget) -> ProbeResult;
    /// 옵션까지 안 뒤의 사용 가능 여부 — 옵션이 가리키는 대상(원격 접속 등)이 있어야 쓸 수 있는 감지기가 재정의한다.
    /// 기본은 [`Self::probe`] 와 같다. `plan` 이 옵션을 해석한 뒤에 부른다 (역시 가볍게 — 네트워크 I/O 금지).
    fn probe_with(&self, target: &WatchTarget, _opts: &DetectorOptions) -> ProbeResult {
        self.probe(target)
    }
    /// 옵션이 이 폴더에 맞는지 저장 전에 확인한다 (예: 원격 접속의 경로 변환이 폴더 루트에 닿는가). 기본은 항상 통과.
    fn validate_target(&self, _target: &WatchTarget, _opts: &DetectorOptions) -> Result<(), DetectError> {
        Ok(())
    }
    /// 실험 기능 표시 (UI 가 "실험" 으로 보여 준다) — 실측·실기 검증 전 감지기
    fn experimental(&self) -> bool {
        false
    }
    fn option_fields(&self) -> &'static [OptionField] {
        &[]
    }
    /// `None` 은 항상 성공(기본값). 알 수 없는 키는 거부한다.
    fn parse_options(&self, raw: Option<&Value>) -> Result<DetectorOptions, DetectError> {
        self.parse_options_with(raw, &NoRefs)
    }
    /// [`Self::parse_options`] + 참조 옵션(`OptionKind::Ref`) 존재 확인용 해석기. 참조가 없는 감지기는 신경 쓰지 않아도 된다.
    fn parse_options_with(
        &self,
        raw: Option<&Value>,
        refs: &dyn RefResolver,
    ) -> Result<DetectorOptions, DetectError> {
        parse_options_checked(self.option_fields(), raw, refs)
    }
    fn start(
        &self,
        target: &WatchTarget,
        opts: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError>;
}

/// 버리면 감시가 멈춘다. 해제 완료는 이 drop 이 아니라 `ReleaseToken` 이 알린다.
pub trait WatchHandle: Send {}

/// 정수 옵션 검증: raw 는 null/없음 또는 객체, 각 키는 `fields` 에 있고 `[min, max]` 정수여야 한다.
/// 오류 필드 경로는 `watchOptions` (정적 문자열이라 키는 포함하지 않는다).
pub fn parse_integer_options(
    fields: &[OptionField],
    raw: Option<&Value>,
) -> Result<DetectorOptions, DetectError> {
    parse_options_checked(fields, raw, &NoRefs)
}

/// [`parse_integer_options`] + `OptionKind::Ref` 검증: 값은 양의 정수여야 하고(아니면 `InvalidOptions`) `refs` 가 아는 ID 여야 한다
/// (모르면 `UnknownReference`). 참조 옵션은 기본값이 없어 항상 저장한다.
pub fn parse_options_checked(
    fields: &[OptionField],
    raw: Option<&Value>,
    refs: &dyn RefResolver,
) -> Result<DetectorOptions, DetectError> {
    const FIELD: &str = "watchOptions";
    let obj = match raw {
        None | Some(Value::Null) => return Ok(DetectorOptions::default()),
        Some(Value::Object(o)) => o,
        Some(_) => return Err(DetectError::InvalidOptions { field: FIELD }),
    };
    let mut out = Map::new();
    for (key, value) in obj {
        let f = fields
            .iter()
            .chain(COMMON_OPTION_FIELDS)
            .find(|f| f.key == key)
            .ok_or(DetectError::InvalidOptions { field: FIELD })?;
        match f.kind {
            OptionKind::Integer => {
                let n = value
                    .as_i64()
                    .filter(|n| (f.min..=f.max).contains(n))
                    .ok_or(DetectError::InvalidOptions { field: FIELD })?;
                if n != f.default {
                    out.insert(key.clone(), Value::from(n));
                }
            }
            OptionKind::Bool => {
                let b = value
                    .as_bool()
                    .ok_or(DetectError::InvalidOptions { field: FIELD })?;
                if i64::from(b) != f.default {
                    out.insert(key.clone(), Value::from(b));
                }
            }
            OptionKind::Ref => {
                let id = check_ref(key, value, refs)?;
                out.insert(key.clone(), Value::from(id));
            }
        }
    }
    Ok(DetectorOptions(out))
}

/// IPC `list_change_detectors` 항목.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeDetectorInfo {
    pub id: &'static str,
    pub available: bool,
    pub reason_key: Option<&'static str>,
    pub option_fields: Vec<OptionField>,
    pub experimental: bool,
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Validation / Boundary
    use serde_json::json;

    use super::*;

    const FIELDS: &[OptionField] = &[OptionField {
        key: "intervalSec",
        kind: OptionKind::Integer,
        min: 10,
        max: 3600,
        default: 300,
    }];

    #[test]
    fn t_chgdet_opt_01_none_null_and_empty_object_give_defaults() {
        assert_eq!(parse_integer_options(FIELDS, None).unwrap().to_stored(), None);
        let null = parse_integer_options(FIELDS, Some(&Value::Null)).unwrap();
        assert_eq!(null.to_stored(), None);
        let empty = parse_integer_options(FIELDS, Some(&json!({}))).unwrap();
        assert_eq!(empty.to_stored(), None);
    }

    #[test]
    fn t_chgdet_opt_02_valid_value_is_stored_but_default_value_is_not() {
        let o = parse_integer_options(FIELDS, Some(&json!({"intervalSec": 60}))).unwrap();
        assert_eq!(o.to_stored().as_deref(), Some(r#"{"intervalSec":60}"#));
        assert_eq!(o.get_i64("intervalSec"), Some(60));
        let d = parse_integer_options(FIELDS, Some(&json!({"intervalSec": 300}))).unwrap();
        assert_eq!(d.to_stored(), None);
    }

    #[test]
    fn t_chgdet_opt_03_unknown_key_non_object_and_out_of_range_are_rejected() {
        for bad in [
            json!({"nope": 1}),
            json!([]),
            json!("a"),
            json!({"intervalSec": 9}),
            json!({"intervalSec": 3601}),
            json!({"intervalSec": 1.5}),
            json!({"intervalSec": "60"}),
        ] {
            let err = parse_integer_options(FIELDS, Some(&bad));
            assert!(
                matches!(err, Err(DetectError::InvalidOptions { field: "watchOptions" })),
                "{bad}"
            );
        }
    }

    #[test]
    fn t_chgdet_opt_05_common_exclusive_open_check_is_accepted_by_any_detector_and_must_be_bool() {
        // Happy: 감지기 고유 필드가 없어도 공통 옵션은 받고, false 만 저장한다 (true = 기본값)
        let off = parse_integer_options(&[], Some(&json!({"exclusiveOpenCheck": false}))).unwrap();
        assert_eq!(off.to_stored().as_deref(), Some(r#"{"exclusiveOpenCheck":false}"#));
        assert_eq!(off.get_bool(EXCLUSIVE_OPEN_CHECK), Some(false));
        let on = parse_integer_options(FIELDS, Some(&json!({"exclusiveOpenCheck": true}))).unwrap();
        assert_eq!(on.to_stored(), None);
        // Validation: 불리언이 아닌 값은 거부
        for bad in [json!({"exclusiveOpenCheck": 1}), json!({"exclusiveOpenCheck": "true"}), json!({"exclusiveOpenCheck": null})] {
            assert!(parse_integer_options(FIELDS, Some(&bad)).is_err(), "{bad}");
        }
        // 저장 텍스트 읽기: 명시적 false 만 꺼짐, 없음·깨진 값·다른 형 = 기본값(켜짐)
        assert!(!exclusive_open_check_of(Some(r#"{"exclusiveOpenCheck":false}"#)));
        for t in [None, Some("{}"), Some("garbage"), Some(r#"{"exclusiveOpenCheck":1}"#), Some(r#"{"exclusiveOpenCheck":true}"#)] {
            assert!(exclusive_open_check_of(t), "{t:?}");
        }
    }

    #[test]
    fn t_chgdet_opt_04_boundaries_are_inclusive() {
        for n in [10, 3600] {
            assert!(parse_integer_options(FIELDS, Some(&json!({"intervalSec": n}))).is_ok());
        }
    }
}
