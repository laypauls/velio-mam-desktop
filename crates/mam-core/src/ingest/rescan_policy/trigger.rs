//! 재스캔 트리거 계약 — 새 트리거는 `RescanTrigger` 를 구현하고 `PolicyRegistry::builtin()` 에 한 줄 등록한다.

use std::fmt::Debug;

use serde_json::{Map, Value};

use super::PolicyError;
use super::interval::IntervalTrigger;
use super::local_time::LocalZone;
use super::manual::ManualTrigger;
use super::schedule::ScheduleTrigger;

pub struct SlotCtx<'a> {
    pub now_ms: i64,
    pub zone: &'a dyn LocalZone,
    /// 마지막 전체 스캔 **완료** 시각 (없으면 한 번도 끝나지 않음)
    pub last_full_scan_at: Option<i64>,
}

/// 파싱·정규화된 트리거 한 개.
pub trait TriggerSpec: Debug + Send + Sync {
    fn kind(&self) -> &'static str;
    /// 정규화된 JSON (`"kind"` 포함)
    fn to_json(&self) -> Value;
    /// `now` 이하의 가장 최근 시작 시각 (없으면 None)
    fn last_slot(&self, cx: &SlotCtx) -> Option<i64>;
    /// `now` 보다 뒤의 가장 이른 시작 시각 (표시용, 없으면 None)
    fn next_slot(&self, cx: &SlotCtx) -> Option<i64>;
}

pub trait RescanTrigger: Send + Sync {
    /// 저장·IPC 에 쓰는 고정 kind. 배포 후 rename 하지 않는다
    fn id(&self) -> &'static str;
    /// `raw` = `kind` 를 포함한 트리거 객체. 오류는 필드 경로(`rescanPolicy.trigger.<field>`)를 담는다
    fn parse(&self, raw: &Map<String, Value>) -> Result<Box<dyn TriggerSpec>, PolicyError>;
}

pub struct PolicyRegistry {
    triggers: Vec<Box<dyn RescanTrigger>>,
}

impl PolicyRegistry {
    pub fn builtin() -> Self {
        Self {
            triggers: vec![
                Box::new(IntervalTrigger),
                Box::new(ScheduleTrigger),
                Box::new(ManualTrigger),
            ],
        }
    }

    pub fn get(&self, kind: &str) -> Option<&dyn RescanTrigger> {
        self.triggers.iter().find(|t| t.id() == kind).map(|t| &**t)
    }

    pub fn kinds(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.triggers.iter().map(|t| t.id())
    }
}
