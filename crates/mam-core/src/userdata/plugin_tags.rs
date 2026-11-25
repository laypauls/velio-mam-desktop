//! 플러그인이 호출하는 태그 추가 — 자산 1개, 한 SQLite 트랜잭션, 쓰기 우선순위 `Low`, 마감 인지.
//!
//! 태그 변경·`asset_tags`·revision 증가·`search_outbox` 기록이 같은 트랜잭션이다(db.md §3). outbox priority 는 1(사용자 편집 아님).
//! 출처(어느 플러그인이 붙였는가)는 기록하지 않는다 — 사용자 태그와 같게 취급한다(결정).
//! 상한 검사는 쓰기 트랜잭션 안에서 한다 — 검사 후 다른 쓰기가 끼어들 수 없다. 위반하면 아무것도 쓰지 않고 롤백한다.

use std::time::Instant;

use mam_db::{PoolError, Priority};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::{MAX_TAGS_PER_CALL, UserData, bump_assets, clean_name, name_key};
use crate::scan::IngestError;

/// 플러그인 쓰기의 outbox 우선순위 (0 = 사용자 편집)
const PLUGIN_OUTBOX_PRIORITY: i64 = 1;

/// 호출자가 정하는 상한
#[derive(Debug, Clone, Copy)]
pub struct PluginTagLimits {
    /// 자산당 태그 수 상한(추가 후 기준)
    pub max_per_asset: i64,
    /// 이번 호출에서 더 만들 수 있는 새 태그 이름 수
    pub max_new_names: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PluginTagOutcome {
    /// 이 자산에 새로 붙은 태그 수 (이미 붙어 있으면 0 — no-op, revision 증가 없음)
    pub added: usize,
    /// 새로 만든 태그 이름 수
    pub new_names: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum PluginTagError {
    /// 이름이 비었거나 길거나 제어 문자가 있다, 또는 이름이 없거나 너무 많다
    #[error("invalid tag name")]
    InvalidName,
    /// 자산이 없거나 활성이 아니다
    #[error("asset not found")]
    NotFound,
    #[error("asset tag limit reached")]
    AssetTagLimit,
    #[error("new tag name budget exceeded")]
    NewNameBudget,
    /// 마감에 닿았다 — 쓰기는 취소돼 이후에도 실행되지 않는다
    #[error("write deadline reached")]
    Busy,
    #[error(transparent)]
    Internal(IngestError),
}

impl From<rusqlite::Error> for PluginTagError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Internal(e.into())
    }
}

impl UserData {
    /// 자산에 태그를 붙인다(없으면 만든다). 이미 붙은 태그는 건너뛴다. 하나라도 위반이면 아무것도 쓰지 않는다.
    pub fn add_tags_low(
        &self,
        asset_id: i64,
        names: &[String],
        limits: PluginTagLimits,
        deadline: Instant,
    ) -> Result<PluginTagOutcome, PluginTagError> {
        let cleaned = clean_names(names)?;
        let now = self.clock.now_ms();
        let result = self.pool.write_until(Priority::Low, deadline, move |c| {
            let tx = match c.transaction_with_behavior(TransactionBehavior::Immediate) {
                Ok(tx) => tx,
                Err(e) => return Ok(Err(e.into())),
            };
            let out = write_tags(&tx, asset_id, &cleaned, limits, now);
            Ok(match out {
                Ok(o) => tx.commit().map(|()| o).map_err(Into::into),
                Err(e) => Err(e), // tx drop = 롤백
            })
        });
        match result {
            Ok(inner) => inner,
            Err(PoolError::DeadlineExceeded) => Err(PluginTagError::Busy),
            Err(e) => Err(PluginTagError::Internal(e.into())),
        }
    }
}

/// 이름 검증(개수 1..=100·정제·대소문자 무시 중복 제거) — 쓰기 전에 하며 DB 를 건드리지 않는다
fn clean_names(names: &[String]) -> Result<Vec<(String, String)>, PluginTagError> {
    if names.is_empty() || names.len() > MAX_TAGS_PER_CALL {
        return Err(PluginTagError::InvalidName);
    }
    let mut cleaned: Vec<(String, String)> = Vec::with_capacity(names.len());
    for raw in names {
        let name = clean_name(raw).map_err(|_| PluginTagError::InvalidName)?;
        let key = name_key(&name);
        if !cleaned.iter().any(|(_, k)| *k == key) {
            cleaned.push((name, key));
        }
    }
    Ok(cleaned)
}

/// 호출자가 연 트랜잭션 안에서 태그를 붙인다 — 검증·상한·revision 증가·`search_outbox` 기록이 호출자의 트랜잭션에 들어간다
/// (커밋·롤백은 호출자 몫). 오류를 돌려주면 이 호출이 쓴 것은 없다(검증·상한 검사는 쓰기 전에 한다).
/// 호출자가 롤백하지 않고 오류를 무시해도 일부 쓰기가 남지 않도록 오류는 항상 첫 쓰기 전에 난다(내부 SQL 오류 제외).
pub fn add_tags_tx(
    tx: &Transaction<'_>,
    asset_id: i64,
    names: &[String],
    limits: PluginTagLimits,
    now_ms: i64,
) -> Result<PluginTagOutcome, PluginTagError> {
    let cleaned = clean_names(names)?;
    write_tags(tx, asset_id, &cleaned, limits, now_ms)
}

fn write_tags(
    tx: &Transaction<'_>,
    asset_id: i64,
    names: &[(String, String)],
    limits: PluginTagLimits,
    now: i64,
) -> Result<PluginTagOutcome, PluginTagError> {
    let status: Option<String> = tx
        .query_row(
            "SELECT status FROM media_assets WHERE asset_id = ?1",
            [asset_id],
            |r| r.get(0),
        )
        .optional()?;
    if status.as_deref() != Some("active") {
        return Err(PluginTagError::NotFound);
    }
    // 기존 태그·부착 여부를 먼저 읽고(쓰기 전), 상한을 확인한 뒤에만 쓴다
    let mut plan: Vec<(&str, &str, Option<i64>, bool)> = Vec::with_capacity(names.len());
    let (mut new_names, mut to_attach) = (0u32, 0i64);
    for (name, key) in names {
        let tag_id: Option<i64> = tx
            .query_row("SELECT tag_id FROM tags WHERE name_key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        let attached = match tag_id {
            Some(t) => tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM asset_tags WHERE asset_id = ?1 AND tag_id = ?2)",
                params![asset_id, t],
                |r| r.get(0),
            )?,
            None => false,
        };
        if tag_id.is_none() {
            new_names += 1;
        }
        if !attached {
            to_attach += 1;
        }
        plan.push((name, key, tag_id, attached));
    }
    if new_names > limits.max_new_names {
        return Err(PluginTagError::NewNameBudget);
    }
    if to_attach == 0 {
        return Ok(PluginTagOutcome {
            added: 0,
            new_names: 0,
        });
    }
    if mam_db::plugin_kv::count_asset_tags(tx, asset_id)? + to_attach > limits.max_per_asset {
        return Err(PluginTagError::AssetTagLimit);
    }
    for (name, key, tag_id, attached) in plan {
        if attached {
            continue;
        }
        let tag_id = match tag_id {
            Some(t) => t,
            None => {
                tx.execute(
                    "INSERT INTO tags (name, name_key) VALUES (?1, ?2)",
                    params![name, key],
                )?;
                tx.last_insert_rowid()
            }
        };
        tx.execute(
            "INSERT OR IGNORE INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, ?2, ?3)",
            params![asset_id, tag_id, now],
        )?;
    }
    bump_assets(tx, now, &[asset_id], PLUGIN_OUTBOX_PRIORITY).map_err(PluginTagError::Internal)?;
    Ok(PluginTagOutcome {
        added: usize::try_from(to_attach).unwrap_or(usize::MAX),
        new_names,
    })
}
