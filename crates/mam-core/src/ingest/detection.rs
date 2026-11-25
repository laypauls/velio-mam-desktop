//! 폴더별 변경 감지 방식·재스캔 정책 API — 검증은 여기(레지스트리·정책 파서)에서 하고 command 는 위임만 한다.

use mam_db::{Priority, settings};
use serde_json::Value;

use super::Ingest;
use super::rescan_policy::{PolicySource, RescanPolicy, SlotCtx, effective_policy, parse_policy};
use crate::scan::{
    Availability, FolderRow, IngestError, ManagedFolderView, load_all, load_folder,
    set_rescan_policy, set_watch_mode,
};
use crate::watch::{ChangeDetectorInfo, ProbeResult, SourceHealth, WatchTarget};

impl Ingest {
    /// 이 폴더 기준으로 각 감지기의 사용 가능 여부 (레지스트리 순서). 없는 폴더는 `NotFound`.
    pub fn list_change_detectors(
        &self,
        folder_id: i64,
    ) -> Result<Vec<ChangeDetectorInfo>, IngestError> {
        let f = self.env.pool.read(|c| Ok(load_folder(c, folder_id)))??;
        if !f.enabled {
            return Err(IngestError::NotFound); // 등록 해제 중 — set_* 와 같다
        }
        let target = WatchTarget::of(&f);
        Ok(self
            .watchers
            .registry()
            .all()
            .map(|d| {
                let (available, reason_key) = match d.probe(&target) {
                    ProbeResult::Available => (true, None),
                    ProbeResult::Unavailable { reason_key } => (false, Some(reason_key)),
                };
                ChangeDetectorInfo {
                    id: d.id(),
                    available,
                    reason_key,
                    option_fields: d.option_fields().to_vec(),
                    experimental: d.experimental(),
                }
            })
            .collect())
    }

    /// 변경 감지 방식·옵션을 저장하고 감시를 새 방식으로 다시 건다 (오프라인·꺼내기 준비 중이면 저장만).
    /// `mode` 가 None 이면 자동 — 공통 옵션(`exclusiveOpenCheck`)만 받고 감지기 전용 옵션은 거부한다.
    /// 등록되지 않은 방식·잘못된 옵션은 `InvalidInput`(DB 미변경).
    pub fn set_folder_watch_mode(
        &self,
        folder_id: i64,
        mode: Option<String>,
        options: Option<Value>,
    ) -> Result<ManagedFolderView, IngestError> {
        let options = options.filter(|v| !v.is_null());
        let stored_options = match &mode {
            // 자동: 감지기가 정해지지 않았으므로 공통 옵션만 통과한다 (감지기 필드 목록은 비어 있다)
            None => crate::watch::parse_integer_options(&[], options.as_ref())
                .map_err(|_| IngestError::InvalidInput("watchOptions"))?
                .to_stored(),
            Some(id) => {
                let d = self
                    .watchers
                    .registry()
                    .get(id)
                    .ok_or(IngestError::InvalidInput("watchMode"))?;
                let to_err = |e: crate::watch::DetectError| match e {
                    crate::watch::DetectError::InvalidOptions { field }
                    | crate::watch::DetectError::UnknownReference { field } => {
                        IngestError::InvalidInput(field)
                    }
                    crate::watch::DetectError::Start(_) => {
                        IngestError::InvalidInput("watchOptions")
                    }
                };
                let parsed = d
                    .parse_options_with(options.as_ref(), self.watchers.registry().refs())
                    .map_err(to_err)?;
                // 옵션이 이 폴더에 맞는지 (예: 원격 접속의 경로 변환이 폴더 루트에 닿는가) — 저장 전에 거부한다
                let folder = self.env.pool.read(|c| Ok(load_folder(c, folder_id)))??;
                d.validate_target(&WatchTarget::of(&folder), &parsed)
                    .map_err(to_err)?;
                parsed.to_stored()
            }
        };
        let saved_mode = mode.clone();
        self.env.write(Priority::High, move |c| {
            set_watch_mode(
                c,
                folder_id,
                saved_mode.as_deref(),
                stored_options.as_deref(),
            )
        })?;
        {
            // 감시 교체는 시작·중지와 같은 잠금 안에서 — 점검 스레드가 옛 방식을 되살리지 못하게
            let _ctl = super::lock(&self.watch_ctl);
            self.watchers.stop(folder_id);
            super::lock(&self.watch_failed).remove(&folder_id);
            self.start_watch_locked(folder_id);
        }
        self.view_of(folder_id)
    }

    /// 재스캔 정책을 검증·정규화해 저장한다 (None = 전역 기본으로 되돌림). 저장만 하고 재스캔을 예약하지 않는다.
    pub fn set_folder_rescan_policy(
        &self,
        folder_id: i64,
        policy: Option<Value>,
    ) -> Result<ManagedFolderView, IngestError> {
        let stored = match policy.filter(|v| !v.is_null()) {
            None => None,
            Some(v) => Some(
                parse_policy(&v)
                    .map_err(|e| IngestError::InvalidInput(e.field))?
                    .to_json()
                    .to_string(),
            ),
        };
        self.env.write(Priority::High, move |c| {
            set_rescan_policy(c, folder_id, stored.as_deref())
        })?;
        super::lock(&self.policy_warned).remove(&folder_id);
        self.view_of(folder_id)
    }

    /// 폴더의 유효 재스캔 정책. 저장된 값을 해석할 수 없으면 전역 기본으로 동작하고 폴더·프로세스당 한 번만 경고한다.
    pub(super) fn effective_policy_of(&self, f: &FolderRow, default_hours: u32) -> RescanPolicy {
        let (policy, source) = effective_policy(f.rescan_policy.as_deref(), default_hours);
        if source == PolicySource::InvalidFallback
            && super::lock(&self.policy_warned).insert(f.folder_id)
        {
            tracing::warn!(
                folder_id = f.folder_id,
                "invalid rescan policy; using the global default"
            );
        }
        policy
    }

    /// `list_folders` 뷰에 변경 감지·재스캔 정책 필드를 채운다.
    pub(super) fn fill_detection(
        &self,
        views: &mut [ManagedFolderView],
    ) -> Result<(), IngestError> {
        let rows = self.env.pool.read(|c| Ok(load_all(c)))??;
        let (s, _) = self.env.pool.read(settings::load)?;
        let now = self.env.clock.now_ms();
        let zone = &*self.config.local_zone;
        let mut stalls = self.scan_stalls();
        for v in views {
            v.scan_stall = stalls.remove(&v.folder_id);
            let Some(f) = rows.iter().find(|f| f.folder_id == v.folder_id) else {
                continue;
            };
            v.watch_mode = f.watch_mode.clone();
            v.watch_options = f
                .watch_options
                .as_deref()
                .and_then(|t| serde_json::from_str::<Value>(t).ok());
            // 보고할 것이 없는 정상(`Ok` + 신호 없음)은 싣지 않는다 — 선택 필드
            v.watch_health = self.watchers.health(f.folder_id).filter(|h| {
                !matches!(
                    h,
                    SourceHealth::Ok {
                        last_signal_at: None
                    }
                )
            });
            if let Some(st) = self.watchers.status(f.folder_id) {
                v.watch_active = Some(st.active.to_string());
                v.watch_fallback_key = st.fallback.map(|r| r.key().to_string());
            }
            v.rescan_policy = f.rescan_policy.as_deref().and_then(|t| {
                let raw = serde_json::from_str::<Value>(t).ok()?;
                Some(parse_policy(&raw).map_or(raw, |p| p.to_json()))
            });
            if v.availability == Availability::Online {
                let policy = self.effective_policy_of(f, s.rescan_interval_hours);
                let cx = SlotCtx {
                    now_ms: now,
                    zone,
                    last_full_scan_at: f.last_full_scan_at,
                };
                v.next_rescan_at = policy.next_rescan_at(&cx);
            }
        }
        Ok(())
    }
}
