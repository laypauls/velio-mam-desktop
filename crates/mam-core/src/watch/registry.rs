//! 감지기 레지스트리 — 등록된 어댑터와 자동 순서, 폴더별 선택·폴백 계획(`plan`).
//!
//! `plan` 은 순수하다: `probe()` 외에는 I/O 를 하지 않고 감시를 시작하지 않는다 (시작은 `WatchManager`).

use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use super::detector::{ChangeDetector, DetectorOptions, ProbeResult, WatchTarget};
use super::detectors::ScanOnly;
use super::options_ref::{NoRefs, RefResolver};

/// 지정한 방식이 아닌 방식으로 동작하는 이유 (첫 사유가 남는다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// 저장된 방식 ID 를 이 버전이 모른다 (구버전 DB·제거된 감지기)
    UnknownMode,
    /// 저장된 옵션이 해석되지 않는다
    InvalidOptions,
    /// 이 위치에서는 쓸 수 없다 (감지기가 준 사유 키)
    Unavailable(&'static str),
    /// 시작에 실패했다 (NAS 등 — supervisor 가 다시 시도한다)
    StartFailed,
}

impl FallbackReason {
    /// 번역 키 접미사 (`watch.reason.<key>`)
    pub fn key(&self) -> &'static str {
        match self {
            Self::UnknownMode => "unknown_mode",
            Self::InvalidOptions => "invalid_options",
            Self::Unavailable(k) => k,
            Self::StartFailed => "start_failed",
        }
    }
}

pub struct Candidate<'r> {
    pub detector: &'r Arc<dyn ChangeDetector>,
    pub options: DetectorOptions,
}

pub struct Plan<'r> {
    pub candidates: Vec<Candidate<'r>>,
    pub reason: Option<FallbackReason>,
}

pub struct DetectorRegistry {
    all: Vec<Arc<dyn ChangeDetector>>,
    auto: Vec<&'static str>,
    /// 참조 옵션(`OptionKind::Ref`) 존재 확인 — 기본은 모든 참조를 거부한다
    refs: Arc<dyn RefResolver>,
}

impl fmt::Debug for DetectorRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DetectorRegistry")
            .field("all", &self.all.iter().map(|d| d.id()).collect::<Vec<_>>())
            .field("auto", &self.auto)
            .finish()
    }
}

impl DetectorRegistry {
    /// ID 중복이나 `all` 에 없는 자동 순서 ID 는 프로그래밍 오류라 패닉한다 (앱 시작 시 즉시 드러난다).
    /// `scan_only` 가 없으면 추가하고 자동 순서의 맨 끝에 둔다 — 최종 폴백은 항상 성공해야 한다.
    pub fn new(mut all: Vec<Arc<dyn ChangeDetector>>, auto_order: &[&'static str]) -> Self {
        if !all.iter().any(|d| d.id() == "scan_only") {
            all.push(Arc::new(ScanOnly));
        }
        let mut seen = HashSet::new();
        for d in &all {
            assert!(seen.insert(d.id()), "duplicate detector id: {}", d.id());
        }
        let mut auto: Vec<&'static str> = Vec::new();
        for id in auto_order.iter().filter(|id| **id != "scan_only") {
            assert!(
                seen.contains(id),
                "auto order names an unregistered detector: {id}"
            );
            assert!(!auto.contains(id), "duplicate detector in auto order: {id}");
            auto.push(id);
        }
        auto.push("scan_only");
        Self {
            all,
            auto,
            refs: Arc::new(NoRefs),
        }
    }

    /// 참조 옵션 해석기를 주입한다 (원격 접속 테이블이 생기면 DB 조회 구현을 넣는다).
    pub fn with_ref_resolver(mut self, refs: Arc<dyn RefResolver>) -> Self {
        self.refs = refs;
        self
    }

    pub fn refs(&self) -> &dyn RefResolver {
        &*self.refs
    }

    pub fn platform_default() -> Self {
        #[cfg(windows)]
        {
            // polling 은 등록만 하고 자동 순서에는 넣지 않는다 — NAS 실측 전이라 사용자가 고른 폴더에서만 쓴다 (계획 §2, 0단계 결과)
            use super::detectors::{OsEvents, Polling};
            Self::new(
                vec![
                    Arc::new(OsEvents),
                    Arc::new(Polling::new()),
                    Arc::new(ScanOnly),
                ],
                &["os_events", "scan_only"],
            )
        }
        #[cfg(not(windows))]
        {
            // 다른 OS 용 감지기(inotify/FSEvents)는 범위 밖 — `device_watch.rs` 처럼 설계 메모만 둔다 (watch/mod.rs)
            Self::new(
                vec![
                    Arc::new(ScanOnly),
                    Arc::new(super::detectors::Polling::new()),
                ],
                &["scan_only"],
            )
        }
    }

    /// [`Self::platform_default`] + 추가 감지기 — DB 의존성이 있어 `Ingest` 설정 단계에서 만들어진 감지기(`ddp_audit`)를 등록한다.
    /// 추가 감지기는 자동 순서에 들어가지 않는다 (사용자가 고른 폴더에서만 쓴다).
    pub fn platform_default_with(extra: Vec<Arc<dyn ChangeDetector>>) -> Self {
        let Self { mut all, auto, .. } = Self::platform_default();
        // 최종 폴백 `scan_only` 는 목록의 맨 끝에 둔다
        let at = all
            .iter()
            .position(|d| d.id() == "scan_only")
            .unwrap_or(all.len());
        all.splice(at..at, extra);
        Self::new(all, &auto)
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn ChangeDetector>> {
        self.all.iter().find(|d| d.id() == id)
    }

    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn ChangeDetector>> {
        self.all.iter()
    }

    /// 후보 목록(시도 순서)과 폴백 사유를 정한다.
    /// `mode` = 저장된 방식 ID (None = 자동), `raw_options` = 저장된 옵션 JSON 텍스트.
    pub fn plan(
        &self,
        target: &WatchTarget,
        mode: Option<&str>,
        raw_options: Option<&str>,
    ) -> Plan<'_> {
        let mut reason = None;
        let mut first: Option<Candidate<'_>> = None;
        if let Some(id) = mode {
            match self.get(id) {
                None => {
                    tracing::warn!(mode = id, "unknown watch mode; using automatic detection");
                    reason = Some(FallbackReason::UnknownMode);
                }
                Some(d) => {
                    let parsed = match raw_options.map(serde_json::from_str::<Value>).transpose() {
                        Ok(v) => d.parse_options_with(v.as_ref(), &*self.refs).ok(),
                        Err(_) => None,
                    };
                    match parsed {
                        None => {
                            tracing::warn!(
                                mode = id,
                                "invalid watch options; using automatic detection"
                            );
                            reason = Some(FallbackReason::InvalidOptions);
                        }
                        Some(options) => match d.probe_with(target, &options) {
                            ProbeResult::Available => {
                                first = Some(Candidate {
                                    detector: d,
                                    options,
                                })
                            }
                            ProbeResult::Unavailable { reason_key } => {
                                reason = Some(FallbackReason::Unavailable(reason_key));
                            }
                        },
                    }
                }
            }
        }
        let first_id = first.as_ref().map(|c| c.detector.id());
        let mut candidates: Vec<Candidate<'_>> = first.into_iter().collect();
        for id in &self.auto {
            if Some(*id) == first_id {
                continue;
            }
            let Some(d) = self.get(id) else { continue };
            if d.probe(target) != ProbeResult::Available {
                continue;
            }
            if let Ok(options) = d.parse_options(None) {
                candidates.push(Candidate {
                    detector: d,
                    options,
                });
            }
        }
        Plan { candidates, reason }
    }
}
