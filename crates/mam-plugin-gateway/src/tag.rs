//! `tag.add`(`tag.write`) — 자산 1개에 태그 1~20개. 이름 규칙·상한·원자성은 `mam_core::userdata::UserData::add_tags_low` 가 한 트랜잭션으로 처리한다.
//! 출처는 기록하지 않는다(사용자 태그와 같게 취급). `tag.remove`·자산 밖 태그는 v1 에 없다.

use std::sync::atomic::Ordering;

use mam_core::userdata::{PluginTagError, PluginTagLimits, clean_tag_name};
use mam_plugin_host::CallCx;
use serde_json::{Value, json};

use crate::errors::GwError;
use crate::gateway::Gateway;
use crate::params::{MAX_TAGS_PER_CALL, TagAddParams, asset_id, parse};

impl Gateway {
    pub(crate) fn tag_add(&self, cx: &CallCx<'_>, params: Value) -> Result<Value, GwError> {
        let p = parse::<TagAddParams>(params)?;
        let id = asset_id(&p.asset_id)?;
        Self::in_scope(cx, id)?;
        let names: Vec<String> = match p.tags {
            Value::Array(items) if (1..=MAX_TAGS_PER_CALL).contains(&items.len()) => items
                .into_iter()
                .map(|v| v.as_str().map(str::to_owned))
                .collect::<Option<_>>()
                .ok_or(GwError::BadRequest("invalid tag list"))?,
            _ => return Err(GwError::BadRequest("invalid tag list")),
        };
        for n in &names {
            clean_tag_name(n).map_err(|_| GwError::BadRequest("invalid tag name"))?;
        }
        let used = cx.scope.new_tags.load(Ordering::Acquire);
        let limits = PluginTagLimits {
            max_per_asset: self.cfg.max_asset_tags,
            max_new_names: self.cfg.max_new_tags_per_call.saturating_sub(used),
        };
        match self
            .userdata
            .add_tags_low(id, &names, limits, self.write_deadline(cx))
        {
            Ok(out) => {
                cx.scope.new_tags.fetch_add(out.new_names, Ordering::AcqRel);
                Ok(json!({ "added": out.added }))
            }
            Err(PluginTagError::InvalidName) => Err(GwError::BadRequest("invalid tag name")),
            Err(PluginTagError::NotFound) => Err(GwError::NotFound),
            Err(PluginTagError::AssetTagLimit) => Err(GwError::Quota("asset tag limit reached")),
            Err(PluginTagError::NewNameBudget) => Err(GwError::Quota("new tag budget exceeded")),
            Err(PluginTagError::Busy) => Err(GwError::Busy),
            Err(PluginTagError::Internal(e)) => Err(GwError::internal(e)),
        }
    }
}
