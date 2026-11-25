//! provider 반환값 기록 — 메타데이터·분석 provider 가 돌려준 `{ kv?, tags? }` 를 호스트가 **한 트랜잭션**으로 기록하고 작업을 완료한다.
//!
//! 플러그인 호스트 함수(`kv.set`·`tag.add`)는 각자 트랜잭션이라 작업 완료와 원자적일 수 없다 — 그래서 provider 는 값을 반환하고
//! 호스트가 `write_until(Priority::Low)` 한 번으로 설치 확인 -> 권한 -> 총량 -> KV -> 태그 -> `plugin_tasks::complete` 를 기록한다.
//! 어느 단계든 실패하면 전부 롤백된다(작업도 그대로). 계량기는 커밋 뒤에만 갱신한다. 이 파일에 SQL 문자열은 없다.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use mam_core::purge::asset_is_active;
use mam_core::userdata::{PluginTagError, PluginTagLimits, add_tags_tx, clean_tag_name};
use mam_db::plugin_settings::{self, INSTALLED};
use mam_db::plugin_tasks::{self, with_immediate_tx};
use mam_db::{PoolError, Priority, plugin_kv};
use mam_plugin_manifest::Permission;
use rusqlite::Transaction;
use serde::Deserialize;
use serde_json::Value;

use crate::gateway::Gateway;
use crate::grants::GrantRegistry;
use crate::kv::cost;
use crate::params::{self, MAX_TAGS_PER_CALL};
use crate::quota::Meter;

/// 최대 KV 항목 수 (provider 반환 1회)
pub const MAX_OUTPUT_KV: usize = 32;

/// 완료할 작업 — 실행기가 provider 를 부를 때의 `task_id` 와 처리한 revision(`target_revision`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Completion {
    pub task_id: i64,
    pub processed_rev: i64,
}

/// 기록 성공 (작업은 `done`, 또는 그 사이 `target_revision` 이 올랐으면 `pending` 으로 남는다)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recorded {
    pub kv_entries: usize,
    pub tags_added: usize,
    /// 작업에 기록된 `done_revision` 후보 (루프 방지 포함)
    pub done_rev: i64,
}

/// 기록 실패 — `Db` 외에는 아무것도 쓰지 않았고 작업은 그대로다
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    /// 반환값이 계약(`null | {} | {kv?, tags?}`, 상한·이름 규칙)에 맞지 않는다 — DB 접근 전에 거른다
    #[error("provider output invalid: {0}")]
    OutputInvalid(&'static str),
    /// 반환값이 쓰려는 권한(`kv.write`/`tag.write`)이 없다 (기록 시점의 살아 있는 권한)
    #[error("permission denied: {}", .0.as_str())]
    PermissionDenied(Permission),
    /// KV 총량 또는 자산당 태그 상한
    #[error("quota exceeded")]
    QuotaExceeded,
    /// 쓰기 마감에 닿았다 — 재시도 가능, 쓰기는 취소돼 이후에도 실행되지 않는다
    #[error("write deadline reached")]
    Busy,
    /// 플러그인이 제거됐다(`plugin_installed` 없음) — 아무것도 쓰지 않았다
    #[error("plugin not installed")]
    NotInstalled,
    /// 자산이 활성이 아니다 (삭제·병합) — 실행기는 `mark_inactive_done` 으로 닫는다
    #[error("asset not active")]
    AssetInactive,
    /// 작업 행이 없다 (provider 제거로 삭제됨) — 기록을 롤백했다
    #[error("task row is gone")]
    TaskGone,
    /// SQL·writer 오류 (상세 문자열은 로그 전용)
    #[error("database error: {0}")]
    Db(String),
}

impl From<rusqlite::Error> for RecordError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.to_string())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOutput {
    kv: Option<BTreeMap<String, Value>>,
    tags: Option<Vec<String>>,
}


/// 검증을 마친 반환값 (KV 값은 compact JSON 텍스트, 태그는 `None` = 키 없음)
struct Output {
    kv: Vec<(String, String)>,
    tags: Option<Vec<String>>,
}

fn invalid(m: &'static str) -> RecordError {
    RecordError::OutputInvalid(m)
}

fn parse_output(out: Value, value_max: usize) -> Result<Output, RecordError> {
    if out.is_null() {
        return Ok(Output { kv: Vec::new(), tags: None });
    }
    if !out.is_object() {
        return Err(invalid("output must be null or an object"));
    }
    let raw: RawOutput = serde_json::from_value(out).map_err(|_| invalid("unknown or malformed field"))?;
    let mut kv = Vec::new();
    if let Some(map) = raw.kv {
        if map.len() > MAX_OUTPUT_KV {
            return Err(invalid("too many kv entries"));
        }
        for (key, value) in map {
            params::key(&key).map_err(|_| invalid("invalid kv key"))?;
            let text = serde_json::to_string(&value).map_err(|_| invalid("invalid kv value"))?;
            if text.len() > value_max {
                return Err(invalid("kv value too large"));
            }
            kv.push((key, text));
        }
    }
    if let Some(tags) = &raw.tags {
        if !(1..=MAX_TAGS_PER_CALL).contains(&tags.len()) {
            return Err(invalid("tags must have 1..=20 names"));
        }
        for t in tags {
            clean_tag_name(t).map_err(|_| invalid("invalid tag name"))?;
        }
    }
    Ok(Output { kv, tags: raw.tags })
}

/// 쓰기 스레드로 옮겨 가는 입력 묶음
struct Job {
    plugin: String,
    asset_id: i64,
    out: Output,
    completion: Completion,
    grants: Arc<GrantRegistry>,
    meter: Arc<Meter>,
    quota: u64,
    limits: PluginTagLimits,
    now: i64,
}

/// 트랜잭션 안의 본문. 성공이면 `(결과, 계량기에 더할 delta)` — 계량기는 커밋 뒤에 호출자가 갱신한다.
fn apply(tx: &Transaction<'_>, j: &Job) -> Result<(Recorded, i64), RecordError> {
    if plugin_settings::get(tx, INSTALLED, &j.plugin)?.is_none() {
        return Err(RecordError::NotInstalled);
    }
    if !asset_is_active(tx, j.asset_id)? {
        return Err(RecordError::AssetInactive);
    }
    // 권한은 살아 있는 레지스트리에서 지금 읽는다 (큐 대기 중 철회·비활성이 반영된다)
    for (needed, perm) in [(!j.out.kv.is_empty(), Permission::KvWrite), (j.out.tags.is_some(), Permission::TagWrite)] {
        if needed && !j.grants.allows(&j.plugin, perm) {
            return Err(RecordError::PermissionDenied(perm));
        }
    }
    let delta = kv_delta(tx, j)?;
    for (key, text) in &j.out.kv {
        plugin_kv::upsert(tx, &j.plugin, Some(j.asset_id), key, text, j.now)?;
    }
    // 태그를 붙이기 직전의 revision — provider 가 본 revision(`processed_rev`) 보다 앞서 있으면 그 사이 다른 편집이 있었다
    let rev_before = match &j.out.tags {
        Some(_) => plugin_tasks::asset_revision(tx, j.asset_id)?,
        None => None,
    };
    let mut tags_added = 0;
    if let Some(names) = &j.out.tags {
        match add_tags_tx(tx, j.asset_id, names, j.limits, j.now) {
            Ok(o) => tags_added = o.added,
            Err(PluginTagError::InvalidName) => return Err(invalid("invalid tag name")),
            Err(PluginTagError::NotFound) => return Err(RecordError::AssetInactive),
            Err(PluginTagError::AssetTagLimit | PluginTagError::NewNameBudget) => return Err(RecordError::QuotaExceeded),
            Err(PluginTagError::Busy) => return Err(RecordError::Busy),
            Err(PluginTagError::Internal(e)) => return Err(RecordError::Db(e.to_string())),
        }
    }
    let Completion { task_id, processed_rev } = j.completion;
    // 루프 방지: 이 트랜잭션이 태그를 붙여 올린 revision 까지 처리한 것으로 본다.
    // 단 provider 가 실행되는 사이 다른 편집(사용자·다른 플러그인)이 revision 을 올렸으면(`rev_before > processed_rev`) 그 편집은
    // provider 가 보지 못했다 — 자기 이벤트까지 건너뛰면 그 편집도 같이 삼켜지므로 `processed_rev` 만 인정하고 한 번 더 돌게 둔다.
    let own_only = rev_before.is_some_and(|b| b <= processed_rev);
    let done_rev = match (tags_added > 0 && own_only).then(|| plugin_tasks::asset_revision(tx, j.asset_id)).transpose()? {
        Some(Some(after)) => after.max(processed_rev),
        _ => processed_rev,
    };
    if !plugin_tasks::complete(tx, task_id, processed_rev, done_rev, j.now)? {
        return Err(RecordError::TaskGone);
    }
    Ok((Recorded { kv_entries: j.out.kv.len(), tags_added, done_rev }, delta))
}

/// KV 총량 검사 — 계량기를 (필요하면) 초기화하고, 늘어나면 상한을 확인한다(거부 전에 합계를 한 번 다시 센다). 반환 = 총 delta
fn kv_delta(tx: &Transaction<'_>, j: &Job) -> Result<i64, RecordError> {
    if j.out.kv.is_empty() {
        return Ok(0);
    }
    if !j.meter.ready() {
        j.meter.reset(plugin_kv::sum_cost(tx, &j.plugin)?);
    }
    let mut delta = 0i64;
    for (key, text) in &j.out.kv {
        let old = plugin_kv::row_cost(tx, &j.plugin, Some(j.asset_id), key)?.unwrap_or(0);
        delta = delta.saturating_add(cost(key, text.len()) - old);
    }
    if !j.meter.fits(delta, j.quota) {
        j.meter.reset(plugin_kv::sum_cost(tx, &j.plugin)?);
        if !j.meter.fits(delta, j.quota) {
            return Err(RecordError::QuotaExceeded);
        }
    }
    Ok(delta)
}

impl Gateway {
    /// provider 반환값을 검증해 기록하고 작업을 완료한다 (한 `Priority::Low` 쓰기 트랜잭션, 마감 인지).
    ///
    /// 순서: 반환값 검증(DB 전) -> `plugin_installed` -> 자산 활성 -> 권한(`kv.write`/`tag.write`, 살아 있는 레지스트리) -> KV 총량 ->
    /// KV upsert -> 태그(`add_tags_tx`) -> `plugin_tasks::complete`. 실패하면 전부 롤백(작업 그대로), 계량기는 커밋 뒤에만 갱신한다.
    ///
    /// 루프 방지: 이 트랜잭션이 태그를 실제로 붙였으면 `done_rev = MAX(processed_rev, 붙인 뒤의 자산 revision)` — 자기 outbox 이벤트가
    /// 이 플러그인의 작업을 다시 pending 으로 만들지 않는다(다른 플러그인은 본다). 붙인 게 없으면 `processed_rev` 그대로.
    pub fn record_provider_output(&self, plugin: &str, asset_id: i64, out: Value, completion: Completion, deadline: Instant) -> Result<Recorded, RecordError> {
        let out = parse_output(out, self.cfg.kv_value_max_bytes)?;
        let job = Job {
            plugin: plugin.to_owned(),
            asset_id,
            out,
            completion,
            grants: Arc::clone(&self.grants),
            meter: self.meters.of(plugin),
            quota: self.cfg.kv_quota_bytes,
            limits: PluginTagLimits { max_per_asset: self.cfg.max_asset_tags, max_new_names: self.cfg.max_new_tags_per_call },
            now: self.clock.now_ms(),
        };
        let wait = (Instant::now() + self.cfg.write_wait).min(deadline);
        let result = self.pool.write_until(Priority::Low, wait, move |c| {
            let done = with_immediate_tx(c, |tx| apply(tx, &job));
            // 커밋이 끝난 뒤에만 계량기를 올린다 (쓰기 스레드 안이라 다음 쓰기보다 먼저 반영된다)
            Ok(done.map(|(rec, delta)| {
                if delta != 0 {
                    job.meter.apply(delta);
                }
                rec
            }))
        });
        match result {
            Ok(inner) => inner,
            Err(PoolError::DeadlineExceeded) => Err(RecordError::Busy),
            Err(e) => Err(RecordError::Db(e.to_string())),
        }
    }
}
