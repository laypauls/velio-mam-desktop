//! `manual` — 자동 재스캔 없음 (수동 재스캔·볼륨 재연결·감시 넘침 재스캔만).

use serde_json::{Map, Value, json};

use super::PolicyError;
use super::trigger::{RescanTrigger, SlotCtx, TriggerSpec};

pub struct ManualTrigger;

#[derive(Debug)]
pub struct ManualSpec;

impl RescanTrigger for ManualTrigger {
    fn id(&self) -> &'static str {
        "manual"
    }

    fn parse(&self, raw: &Map<String, Value>) -> Result<Box<dyn TriggerSpec>, PolicyError> {
        if raw.keys().any(|k| k != "kind") {
            return Err(PolicyError { field: "rescanPolicy.trigger" });
        }
        Ok(Box::new(ManualSpec))
    }
}

impl TriggerSpec for ManualSpec {
    fn kind(&self) -> &'static str {
        "manual"
    }

    fn to_json(&self) -> Value {
        json!({ "kind": "manual" })
    }

    fn last_slot(&self, _cx: &SlotCtx) -> Option<i64> {
        None
    }

    fn next_slot(&self, _cx: &SlotCtx) -> Option<i64> {
        None
    }
}
