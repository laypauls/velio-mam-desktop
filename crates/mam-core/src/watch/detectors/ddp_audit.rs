//! `ddp_audit` (실험) — DDP(AVFS) 스토리지의 **감사 로그를 SSH 로 이어 읽어** 바뀐 경로를 알린다. OS 변경 이벤트가 오지 않는 스토리지용.
//!
//! 옵션 `{"source": <remote_sources.source_id>}` (필수, `OptionKind::Ref`). 접속 하나를 여러 폴더가 공유한다(`RemoteHub`).
//! 자동 선택에는 넣지 않는다 — 접속 정보가 있어야 하고 사용자가 폴더에서 직접 고른다. **실제 DDP 로 검증되지 않았다**(가짜 접속으로만 시험) -> 실험.
//! 감사 로그는 주기 재스캔을 대신하지 않는다: 보고는 "이 경로를 확인하라" 는 힌트이고, 놓쳤을 수 있으면 재스캔을 요청한다.

use std::sync::Arc;

use serde_json::Value;

use crate::volume::strip_verbatim;
use crate::watch::detector::{
    ChangeDetector, DetectError, DetectorOptions, OptionField, OptionKind, ProbeResult,
    WatchHandle, WatchTarget, parse_options_checked,
};
use crate::watch::options_ref::RefResolver;
use crate::watch::remote::hub::{AttachGuard, RemoteHub};
use crate::watch::remote::{SourceKind, map_reaches};
use crate::watch::sink::ChangeSink;

pub const OPT_SOURCE: &str = "source";

const FIELDS: &[OptionField] = &[OptionField {
    key: OPT_SOURCE,
    kind: OptionKind::Ref,
    min: 1,
    max: i64::MAX,
    default: 0,
}];

/// 사유 키 (`watch.reason.<key>`)
pub const REASON_NOT_READY: &str = "remote_not_ready";
pub const REASON_SOURCE_MISSING: &str = "remote_source_missing";
pub const REASON_SOURCE_KIND: &str = "remote_source_kind";
pub const REASON_HOST_KEY: &str = "remote_host_key_unconfirmed";
pub const REASON_PATH_MAP: &str = "remote_path_unmapped";

pub struct DdpAudit {
    hub: Arc<RemoteHub>,
}

impl DdpAudit {
    pub fn new(hub: Arc<RemoteHub>) -> Self {
        Self { hub }
    }

    /// 옵션이 가리키는 접속을 확인한다 — 없음·종류 다름·호스트 키 미확인·경로 변환이 폴더에 안 닿음.
    fn check(&self, target: &WatchTarget, opts: &DetectorOptions) -> Result<(), &'static str> {
        let id = opts.get_i64(OPT_SOURCE).ok_or(REASON_SOURCE_MISSING)?;
        let v = self.hub.view(id).ok_or(REASON_SOURCE_MISSING)?;
        if v.kind != SourceKind::DdpAudit.as_str() {
            return Err(REASON_SOURCE_KIND);
        }
        if v.host_key.is_none() {
            return Err(REASON_HOST_KEY);
        }
        if !map_reaches(&v.path_map, &strip_verbatim(&target.root.to_string_lossy())) {
            return Err(REASON_PATH_MAP);
        }
        Ok(())
    }
}

struct DdpHandle(#[allow(dead_code)] AttachGuard);

impl WatchHandle for DdpHandle {}

impl ChangeDetector for DdpAudit {
    fn id(&self) -> &'static str {
        "ddp_audit"
    }

    fn experimental(&self) -> bool {
        true
    }

    /// 옵션 없이 묻는 경우(감지기 목록): 허브가 준비돼 있으면 선택할 수 있다. 접속 확인은 `probe_with`.
    fn probe(&self, _target: &WatchTarget) -> ProbeResult {
        if self.hub.is_bound() {
            ProbeResult::Available
        } else {
            ProbeResult::Unavailable {
                reason_key: REASON_NOT_READY,
            }
        }
    }

    fn probe_with(&self, target: &WatchTarget, opts: &DetectorOptions) -> ProbeResult {
        if !self.hub.is_bound() {
            return ProbeResult::Unavailable {
                reason_key: REASON_NOT_READY,
            };
        }
        match self.check(target, opts) {
            Ok(()) => ProbeResult::Available,
            Err(reason_key) => ProbeResult::Unavailable { reason_key },
        }
    }

    fn option_fields(&self) -> &'static [OptionField] {
        FIELDS
    }

    /// `source` 는 필수이고 `ddp_audit` 종류의 접속이어야 한다 (참조 존재는 주입된 해석기, 종류는 여기서).
    fn parse_options_with(
        &self,
        raw: Option<&Value>,
        refs: &dyn RefResolver,
    ) -> Result<DetectorOptions, DetectError> {
        const FIELD: &str = "watchOptions";
        let opts = parse_options_checked(FIELDS, raw, refs)?;
        let id = opts
            .get_i64(OPT_SOURCE)
            .ok_or(DetectError::InvalidOptions { field: FIELD })?;
        // 허브가 아직 없으면(시험·초기화 순서) 종류를 확인할 수 없다 — 존재 확인은 위 해석기가 했다
        if self.hub.is_bound() && self.hub.kind_of(id) != Some(SourceKind::DdpAudit) {
            return Err(DetectError::UnknownReference { field: FIELD });
        }
        Ok(opts)
    }

    /// 경로 변환이 폴더 루트에 닿지 않으면 저장 전에 거부한다 — 붙어서 줄을 읽어도 그 폴더는 영영 깨어나지 않는 조용한 실패를 막는다.
    fn validate_target(
        &self,
        target: &WatchTarget,
        opts: &DetectorOptions,
    ) -> Result<(), DetectError> {
        match self.check(target, opts) {
            Err(REASON_PATH_MAP) => Err(DetectError::InvalidOptions { field: "pathMap" }),
            _ => Ok(()),
        }
    }

    fn start(
        &self,
        target: &WatchTarget,
        opts: &DetectorOptions,
        sink: ChangeSink,
    ) -> Result<Box<dyn WatchHandle>, DetectError> {
        let id = opts
            .get_i64(OPT_SOURCE)
            .ok_or(DetectError::InvalidOptions {
                field: "watchOptions",
            })?;
        let guard = self.hub.attach(
            id,
            SourceKind::DdpAudit,
            target.folder_id,
            &target.root,
            sink,
        )?;
        Ok(Box::new(DdpHandle(guard)))
    }
}
