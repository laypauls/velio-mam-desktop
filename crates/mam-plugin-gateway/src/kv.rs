//! `kv.get`·`kv.list`(`kv.read`), `kv.set`·`kv.delete`(`kv.write`). 네임스페이스는 `cx.plugin` 이며 params 로 받지 않는다.
//! 자산 키는 호출 범위 안의 자산만(밖이면 DB 접근 없이 `not_found`). 값은 compact JSON ≤ 64 KiB(DB 접근 전 검사), 총량은 `Meter`.
//! 쓰기는 `Priority::Low` 한 번의 클로저(단일 writer 스레드)에서 이전 행 비용 -> 상한 확인 -> upsert -> 계량기 갱신 순으로 한다.

use std::sync::Arc;

use mam_core::assets;
use mam_db::Priority;
use mam_db::plugin_kv::{self, ROW_OVERHEAD};
use mam_plugin_host::CallCx;
use serde_json::{Value, json};

use crate::errors::GwError;
use crate::gateway::Gateway;
use crate::params::{self, KvKeyParams, KvListParams, KvSetParams, opt_asset_id, parse};

enum SetOutcome {
    Done,
    Quota,
    NoAsset,
}

pub(crate) fn cost(key: &str, value_len: usize) -> i64 {
    i64::try_from(key.len().saturating_add(value_len)).unwrap_or(i64::MAX).saturating_add(ROW_OVERHEAD)
}

impl Gateway {
    /// 자산 키 범위 검사 (전역 키는 `None` 이라 통과)
    fn kv_asset(cx: &CallCx<'_>, raw: &Option<Value>) -> Result<Option<i64>, GwError> {
        let asset = opt_asset_id(raw)?;
        if let Some(a) = asset {
            Self::in_scope(cx, a)?;
        }
        Ok(asset)
    }

    pub(crate) fn kv_get(&self, cx: &CallCx<'_>, params: Value) -> Result<Value, GwError> {
        let p = parse::<KvKeyParams>(params)?;
        params::key(&p.key)?;
        let asset = Self::kv_asset(cx, &p.asset_id)?;
        let plugin = cx.plugin.as_str().to_owned();
        let key = p.key;
        let raw = self.pool.read_until(self.read_deadline(cx), move |c| Ok(plugin_kv::get(c, &plugin, asset, &key)?))?;
        match raw {
            None => Ok(json!({ "found": false })),
            Some(text) => {
                let value: Value = serde_json::from_str(&text).map_err(|_| GwError::internal("stored kv value is not JSON"))?;
                Ok(json!({ "found": true, "value": value }))
            }
        }
    }

    pub(crate) fn kv_list(&self, cx: &CallCx<'_>, params: Value) -> Result<Value, GwError> {
        let p = parse::<KvListParams>(params)?;
        let limit = params::limit(&p.limit)?;
        if let Some(pre) = &p.prefix {
            params::prefix(pre)?;
        }
        if let Some(cur) = &p.cursor {
            params::cursor(cur)?;
        }
        let asset = Self::kv_asset(cx, &p.asset_id)?;
        let plugin = cx.plugin.as_str().to_owned();
        let mut keys = self.pool.read_until(self.read_deadline(cx), move |c| {
            Ok(plugin_kv::list_keys(c, &plugin, asset, p.prefix.as_deref(), p.cursor.as_deref(), limit + 1)?)
        })?;
        // 한 개 더 읽어 다음 페이지가 있는지 안다 (끝이면 nextCursor 는 null)
        let next = if keys.len() > limit {
            keys.truncate(limit);
            keys.last().cloned()
        } else {
            None
        };
        Ok(json!({ "keys": keys, "nextCursor": next }))
    }

    pub(crate) fn kv_set(&self, cx: &CallCx<'_>, params: Value) -> Result<Value, GwError> {
        let p = parse::<KvSetParams>(params)?;
        params::key(&p.key)?;
        let asset = Self::kv_asset(cx, &p.asset_id)?;
        let text = serde_json::to_string(&p.value).map_err(|_| GwError::BadRequest("invalid params"))?;
        if text.len() > self.cfg.kv_value_max_bytes {
            return Err(GwError::TooLarge("value too large"));
        }
        let plugin = cx.plugin.as_str().to_owned();
        let meter = self.meters.of(&plugin);
        let (key, limit, now) = (p.key, self.cfg.kv_quota_bytes, self.clock.now_ms());
        let new_cost = cost(&key, text.len());
        let outcome = self.pool.write_until(Priority::Low, self.write_deadline(cx), move |c| {
            if let Some(a) = asset
                && !assets::exists(c, a)?
            {
                return Ok(SetOutcome::NoAsset);
            }
            // 첫 쓰기 초기화도 writer 안에서 한다 — `record_provider_output` 의 계량기 갱신과 같은 스레드에서 직렬화된다 (손실 갱신 방지)
            if !meter.ready() {
                meter.reset(plugin_kv::sum_cost(c, &plugin)?);
            }
            let old = plugin_kv::row_cost(c, &plugin, asset, &key)?.unwrap_or(0);
            let delta = new_cost - old;
            if !meter.fits(delta, limit) {
                // CASCADE 삭제 등으로 카운터가 실제보다 높을 수 있다 — 거부 전에 합계를 한 번 다시 센다
                meter.reset(plugin_kv::sum_cost(c, &plugin)?);
                if !meter.fits(delta, limit) {
                    return Ok(SetOutcome::Quota);
                }
            }
            plugin_kv::upsert(c, &plugin, asset, &key, &text, now)?;
            meter.apply(delta);
            Ok(SetOutcome::Done)
        })?;
        match outcome {
            SetOutcome::Done => Ok(json!({})),
            SetOutcome::Quota => Err(GwError::Quota("kv quota exceeded")),
            SetOutcome::NoAsset => Err(GwError::NotFound),
        }
    }

    pub(crate) fn kv_delete(&self, cx: &CallCx<'_>, params: Value) -> Result<Value, GwError> {
        let p = parse::<KvKeyParams>(params)?;
        params::key(&p.key)?;
        let asset = Self::kv_asset(cx, &p.asset_id)?;
        let plugin = cx.plugin.as_str().to_owned();
        let meter: Arc<_> = self.meters.of(&plugin);
        let key = p.key;
        let deleted = self.pool.write_until(Priority::Low, self.write_deadline(cx), move |c| {
            let old = plugin_kv::row_cost(c, &plugin, asset, &key)?;
            let deleted = plugin_kv::delete(c, &plugin, asset, &key)?;
            if let (true, Some(old)) = (deleted, old) {
                meter.apply(-old);
            }
            Ok(deleted)
        })?;
        Ok(json!({ "deleted": deleted }))
    }
}
