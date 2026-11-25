//! 실행 설정 - 호출자가 정하는 동시성·STT 제한 시간·배치 상한, 그리고 실행 시점의 AI 설정 스냅샷.

use std::path::PathBuf;
use std::time::Duration;

use super::types::{StepKind, Transfer};
use crate::jobs::JobKind;

/// 예약 실행 시간대 (로컬 시각, 시작 == 끝이면 하루 종일). UI `AiSettings.schedule`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Schedule {
    pub enabled: bool,
    pub start_hour: u8,
    pub end_hour: u8,
}

impl Schedule {
    /// `local_hour`(0~23) 가 시간대 안인가. 자정을 넘기는 구간(22 -> 6)을 지원한다. 꺼져 있으면 항상 true.
    pub fn allows(&self, local_hour: u8) -> bool {
        if !self.enabled || self.start_hour == self.end_hour {
            return true;
        }
        if self.start_hour < self.end_hour {
            (self.start_hour..self.end_hour).contains(&local_hour)
        } else {
            local_hour >= self.start_hour || local_hour < self.end_hour
        }
    }
}

/// 로컬 시각의 시(0~23). `utc_offset_min` = 로컬 - UTC (분).
pub fn local_hour(now_ms: i64, utc_offset_min: i32) -> u8 {
    let secs = now_ms.div_euclid(1000) + i64::from(utc_offset_min) * 60;
    u8::try_from(secs.div_euclid(3600).rem_euclid(24)).unwrap_or(0)
}

/// 실행 시점의 AI 설정 (UI `AiSettings` 중 작업에 필요한 부분).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRuntimeSettings {
    pub consent_given: bool,
    pub language: String,
    /// 기본 외부 분석 종류 (`externalKinds`)
    pub external_kinds: Vec<StepKind>,
    pub transfer: Transfer,
    pub stt_enabled: bool,
    /// 임베딩·의미 검색
    pub semantic_enabled: bool,
    pub auto_analyze_new: bool,
    pub background_analyze: bool,
    pub schedule: Schedule,
}

impl Default for AiRuntimeSettings {
    /// UI `AI_DEFAULTS` (동의 전).
    fn default() -> Self {
        Self {
            consent_given: false,
            language: "ko".into(),
            external_kinds: vec![StepKind::Describe, StepKind::Keywords, StepKind::Ocr],
            transfer: Transfer::default(),
            stt_enabled: true,
            semantic_enabled: false,
            auto_analyze_new: false,
            background_analyze: false,
            schedule: Schedule::default(),
        }
    }
}

/// 작업이 실행될 때마다 현재 설정을 읽는다 (동의 철회·토글이 즉시 반영되도록).
pub trait AiSettingsSource: Send + Sync {
    fn snapshot(&self) -> AiRuntimeSettings;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SttTimeout {
    pub base_secs: u64,
    /// 미디어 1초당 추가 허용 시간 (초)
    pub per_media_sec: f64,
    pub cap_secs: u64,
}

impl Default for SttTimeout {
    /// 기본 120초 + 미디어 길이의 2배, 최대 6시간.
    fn default() -> Self {
        Self {
            base_secs: 120,
            per_media_sec: 2.0,
            cap_secs: 6 * 3600,
        }
    }
}

impl SttTimeout {
    /// 자산별 제한 시간 = `min(cap, base + per_media_sec × 길이)`. 길이를 모르면 `cap`.
    pub fn for_duration(&self, duration_ms: Option<u64>) -> Duration {
        let cap = Duration::from_secs(self.cap_secs);
        let Some(ms) = duration_ms else { return cap };
        let extra = self.per_media_sec.max(0.0) * (ms as f64 / 1000.0);
        if !extra.is_finite() {
            return cap;
        }
        Duration::from_secs_f64(self.base_secs as f64 + extra).min(cap)
    }
}

#[derive(Debug, Clone)]
pub struct AiJobsConfig {
    /// `MAMData/cache` (썸네일 `thumbnails/`, 프레임 스트립 `previews/`)
    pub cache_root: PathBuf,
    /// AI 작업 동시 실행 수 (P1-6 기본 1) - [`AiJobsConfig::queue_caps`] 로 큐에 반영한다
    pub concurrency: usize,
    pub stt_timeout: SttTimeout,
    /// 백그라운드 스윕이 한 번에 대기시키는 최대 자산 수 (대기 중 AI 작업 포함)
    pub sweep_batch: usize,
}

impl AiJobsConfig {
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            concurrency: 1,
            stt_timeout: SttTimeout::default(),
            sweep_batch: 200,
        }
    }

    /// `JobQueue::with_concurrency` 에 넘길 종류별 상한 (`JobKind::ALL` 순서) - AI 만 `concurrency` 로 바꾼다.
    pub fn queue_caps(&self) -> [usize; JobKind::COUNT] {
        JobKind::ALL.map(|k| {
            if k == JobKind::Ai {
                self.concurrency.max(1)
            } else {
                k.default_concurrency()
            }
        })
    }
}
