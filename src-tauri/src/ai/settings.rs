//! AI 설정 (P3 PA1~PA3) — UI 계약 `AiSettings` 의 저장·검증·패치.
//!
//! 한 개의 JSON 문서(`settings.ai_settings`)로 저장한다. 키(API) 암호문은 별도 행(`ai_key_<provider>`)이며
//! 평문 키는 어디에도 저장하지 않는다. 읽을 때는 손상된 문서를 기본값으로 대체하고(앱 기동을 막지 않는다),
//! 쓸 때(프론트 입력)는 범위를 벗어난 값·모르는 필드를 거부한다.

use std::collections::BTreeMap;

use mam_ai::external::{AuthMode, Provider, TransferScope};
use mam_core::ai_jobs::{
    AiRuntimeSettings, AiSettingsSource, Origin, Schedule, StepKind, Transfer,
};
use serde::{Deserialize, Deserializer, Serialize};

use super::AiFailure;

pub const IMAGE_EDGE: (u32, u32) = (256, 2048);
pub const VIDEO_FRAMES: (u8, u8) = (1, 10);
const LANGUAGE_MAX: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Consent {
    pub given: bool,
    pub at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleSetting {
    pub enabled: bool,
    pub start_hour: u8,
    pub end_hour: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SttModel {
    Base,
    Small,
}

impl SttModel {
    pub fn model_id(self) -> &'static str {
        match self {
            Self::Base => mam_ai::models::ids::WHISPER_BASE,
            Self::Small => mam_ai::models::ids::WHISPER_SMALL,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStt {
    pub enabled: bool,
    pub model: SttModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSearch {
    pub enabled: bool,
}

/// UI 계약 `AiSettings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AiSettings {
    pub default_provider: Option<Provider>,
    pub consent: Consent,
    pub language: String,
    pub auto_analyze_new: bool,
    pub background_analyze: bool,
    pub schedule: ScheduleSetting,
    pub external_kinds: Vec<StepKind>,
    pub transfer: TransferScope,
    pub local_stt: LocalStt,
    pub semantic_search: SemanticSearch,
}

impl Default for AiSettings {
    /// UI `AI_DEFAULTS` — 동의 전에는 자동·백그라운드 분석이 꺼져 있고, 의미 검색은 모델이 준비되기 전까지 꺼져 있다.
    fn default() -> Self {
        Self {
            default_provider: None,
            consent: Consent::default(),
            language: "ko".into(),
            auto_analyze_new: false,
            background_analyze: false,
            schedule: ScheduleSetting {
                enabled: false,
                start_hour: 0,
                end_hour: 0,
            },
            external_kinds: vec![StepKind::Describe, StepKind::Keywords, StepKind::Ocr],
            transfer: TransferScope::default(),
            local_stt: LocalStt {
                enabled: true,
                model: SttModel::Base,
            },
            semantic_search: SemanticSearch { enabled: false },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCheck {
    pub ok: bool,
    pub at: i64,
    pub error_code: Option<String>,
}

/// 공급자별 상태 (키 값은 없다).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderState {
    pub model: Option<String>,
    pub last_check: Option<LastCheck>,
    /// 인증 방식. 저장값이 없으면(구 버전 설정) `apiKey`.
    pub auth_mode: AuthMode,
}

/// 저장되는 문서 전체.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StoredAi {
    pub settings: AiSettings,
    pub providers: BTreeMap<String, ProviderState>,
    /// 신규 자산 자동 분석이 마지막으로 확인한 `asset_id` (없으면 다음 틱에 현재 최대값으로 시작)
    pub new_watermark: Option<i64>,
}

impl StoredAi {
    pub fn provider(&self, p: Provider) -> ProviderState {
        self.providers.get(p.as_str()).cloned().unwrap_or_default()
    }
    pub fn provider_mut(&mut self, p: Provider) -> &mut ProviderState {
        self.providers.entry(p.as_str().to_string()).or_default()
    }
}

// ─── 패치 ───────────────────────────────────────────────────────────────────

fn nullable<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchedulePatch {
    enabled: Option<bool>,
    start_hour: Option<i64>,
    end_hour: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferPatch {
    image_max_edge: Option<i64>,
    video_frames: Option<i64>,
    include_metadata: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SttPatch {
    enabled: Option<bool>,
    model: Option<SttModel>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticPatch {
    enabled: Option<bool>,
}

/// UI 계약 `AiSettingsPatch` (`consent` 는 패치로 바꿀 수 없다 — 전용 command).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiSettingsPatch {
    #[serde(default, deserialize_with = "nullable")]
    default_provider: Option<Option<Provider>>,
    language: Option<String>,
    auto_analyze_new: Option<bool>,
    background_analyze: Option<bool>,
    schedule: Option<SchedulePatch>,
    external_kinds: Option<Vec<StepKind>>,
    transfer: Option<TransferPatch>,
    local_stt: Option<SttPatch>,
    semantic_search: Option<SemanticPatch>,
}

fn int_in(v: i64, lo: i64, hi: i64, what: &'static str) -> Result<i64, AiFailure> {
    if (lo..=hi).contains(&v) {
        Ok(v)
    } else {
        Err(AiFailure::Invalid(what))
    }
}

fn language_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= LANGUAGE_MAX
        && s.split('-')
            .all(|p| !p.is_empty() && p.len() <= 8 && p.chars().all(|c| c.is_ascii_alphanumeric()))
        && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
}

/// 패치를 적용한 새 설정을 돌려준다. 전부 검증한 뒤에만 값을 바꾼다(실패 시 원본 그대로).
/// `semantic_models_ready`: 의미 검색을 켜려면 로컬 임베딩 모델이 하나 이상 준비돼 있어야 한다.
pub fn apply_patch(
    cur: &AiSettings,
    patch: AiSettingsPatch,
    semantic_models_ready: bool,
) -> Result<AiSettings, AiFailure> {
    let mut next = cur.clone();
    if let Some(p) = patch.default_provider {
        next.default_provider = p;
    }
    if let Some(l) = patch.language {
        if !language_ok(&l) {
            return Err(AiFailure::Invalid("language"));
        }
        next.language = l;
    }
    // 동의 전에는 자동·백그라운드 분석을 켤 수 없다
    if (patch.auto_analyze_new == Some(true) || patch.background_analyze == Some(true))
        && !cur.consent.given
    {
        return Err(AiFailure::ConsentRequired);
    }
    if let Some(v) = patch.auto_analyze_new {
        next.auto_analyze_new = v;
    }
    if let Some(v) = patch.background_analyze {
        next.background_analyze = v;
    }
    if let Some(s) = patch.schedule {
        if let Some(v) = s.enabled {
            next.schedule.enabled = v;
        }
        if let Some(v) = s.start_hour {
            next.schedule.start_hour = int_in(v, 0, 23, "schedule.startHour")? as u8;
        }
        if let Some(v) = s.end_hour {
            next.schedule.end_hour = int_in(v, 0, 23, "schedule.endHour")? as u8;
        }
    }
    if let Some(kinds) = patch.external_kinds {
        if kinds.iter().any(|k| !k.is_external()) {
            return Err(AiFailure::Invalid("externalKinds"));
        }
        let mut unique: Vec<StepKind> = Vec::new();
        for k in kinds {
            if !unique.contains(&k) {
                unique.push(k);
            }
        }
        next.external_kinds = unique;
    }
    if let Some(t) = patch.transfer {
        if let Some(v) = t.image_max_edge {
            next.transfer.image_max_edge = int_in(
                v,
                i64::from(IMAGE_EDGE.0),
                i64::from(IMAGE_EDGE.1),
                "transfer.imageMaxEdge",
            )? as u32;
        }
        if let Some(v) = t.video_frames {
            next.transfer.video_frames = int_in(
                v,
                i64::from(VIDEO_FRAMES.0),
                i64::from(VIDEO_FRAMES.1),
                "transfer.videoFrames",
            )? as u8;
        }
        if let Some(v) = t.include_metadata {
            next.transfer.include_metadata = v;
        }
    }
    if let Some(s) = patch.local_stt {
        if let Some(v) = s.enabled {
            next.local_stt.enabled = v;
        }
        if let Some(v) = s.model {
            next.local_stt.model = v;
        }
    }
    if let Some(s) = patch.semantic_search
        && let Some(v) = s.enabled
    {
        if v && !cur.semantic_search.enabled && !semantic_models_ready {
            return Err(AiFailure::Invalid("semanticSearch: local models not ready"));
        }
        next.semantic_search.enabled = v;
    }
    Ok(next)
}

// ─── 작업 코어용 스냅샷 ─────────────────────────────────────────────────────

impl AiSettings {
    pub fn runtime(&self) -> AiRuntimeSettings {
        AiRuntimeSettings {
            consent_given: self.consent.given,
            language: self.language.clone(),
            external_kinds: self.external_kinds.clone(),
            transfer: Transfer {
                image_max_edge: self.transfer.image_max_edge,
                video_frames: self.transfer.video_frames,
                include_metadata: self.transfer.include_metadata,
            },
            stt_enabled: self.local_stt.enabled,
            semantic_enabled: self.semantic_search.enabled,
            auto_analyze_new: self.auto_analyze_new,
            background_analyze: self.background_analyze,
            schedule: Schedule {
                enabled: self.schedule.enabled,
                start_hour: self.schedule.start_hour,
                end_hour: self.schedule.end_hour,
            },
        }
    }
}

impl AiSettingsSource for super::store::AiStore {
    fn snapshot(&self) -> AiRuntimeSettings {
        self.settings().runtime()
    }
}

/// 설정 변경으로 새로 꺼진 자동 분석 출처 - 그 출처의 대기 단계를 정리해야 한다 (true -> false 전이만).
pub fn origins_turned_off(before: &AiSettings, after: &AiSettings) -> Vec<Origin> {
    let mut v = Vec::new();
    if before.auto_analyze_new && !after.auto_analyze_new {
        v.push(Origin::AutoNew);
    }
    if before.background_analyze && !after.background_analyze {
        v.push(Origin::Background);
    }
    if before.semantic_search.enabled && !after.semantic_search.enabled {
        v.push(Origin::Backfill);
    }
    v
}

#[cfg(test)]
mod origin_tests {
    //! 분류: Business rule - true -> false 전이만 정리 대상 (T-aioff-10)
    use super::*;

    #[test]
    fn t_aioff_10_only_true_to_false_transitions_name_an_origin() {
        // Arrange
        let mut on = AiSettings {
            auto_analyze_new: true,
            background_analyze: true,
            ..AiSettings::default()
        };
        on.semantic_search.enabled = true;
        let off = AiSettings::default();

        // Act / Assert
        assert_eq!(
            origins_turned_off(&on, &off),
            vec![Origin::AutoNew, Origin::Background, Origin::Backfill]
        );
        assert!(origins_turned_off(&off, &on).is_empty());
        assert!(origins_turned_off(&on, &on).is_empty());
    }
}
