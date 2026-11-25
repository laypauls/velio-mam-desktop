//! 플러그인 provider 작업(`plugin_tasks`) 질의 — 변경 피드·전체 실행·완료·재시도·집계 (SQL 은 이 파일에만 둔다).
//!
//! 질의 형태가 계약이다(`tests/plugin_tasks_eqp.rs` 가 EXPLAIN QUERY PLAN 으로 인덱스 사용·전체 훑기 없음을 확인).
//! 쓰기 함수는 모두 호출자가 연 트랜잭션(`&Transaction`)에서 실행한다 — 호출자가 커밋·롤백을 정하고, 작업 등록 뒤
//! `JobQueue::notify()` 도 호출자 몫이다. 읽기 함수만 `&Connection` 을 받는다.
//!
//! 역전 방지: `target_revision` 은 `MAX` 로만 오르고, `done_revision` 은 `MAX` 로만 오른다. 상태는 항상
//! `target_revision > done_revision` 이면 `pending`, 아니면 `done` 이다(실패 행은 새 revision 이 와야 되살아난다).

use rusqlite::{Connection, Transaction, params};

use crate::plugin_registry;

/// `jobs.kind` 문자열 (`mam_core::jobs::JobKind::Plugin.as_str()` 과 같아야 한다 — 코어 쪽 시험이 고정)
pub const JOB_KIND: &str = "plugin";

/// 전체 실행 한 배치에서 훑는 자산 수
pub const RUN_ALL_BATCH: i64 = 5_000;

/// `asset_kinds` 가 비면(= 전체) 확장하는 미디어 유형
const ALL_MEDIA_TYPES: [&str; 3] = ["image", "video", "audio"];

/// provider 하나와 그것이 다루는 미디어 유형(`image`/`video`/`audio`, 비면 전체)
pub type ProviderTypes<'a> = (&'a str, &'a [&'a str]);

/// `(provider_ref, media_type)` 쌍 JSON — SQL 에 `json_each` 로 넘겨 고정 문장을 유지한다. provider 이름은 레지스트리에서 정수 키로 바꾼다
fn pairs_json(
    conn: &Connection,
    plugin_ref: i64,
    providers: &[ProviderTypes<'_>],
) -> rusqlite::Result<Option<String>> {
    let mut pairs: Vec<serde_json::Value> = Vec::new();
    for (provider, kinds) in providers {
        let provider_ref = plugin_registry::ensure_provider(conn, plugin_ref, provider)?;
        let kinds: &[&str] = if kinds.is_empty() {
            &ALL_MEDIA_TYPES
        } else {
            kinds
        };
        pairs.extend(
            kinds
                .iter()
                .map(|k| serde_json::json!({ "p": provider_ref, "t": k })),
        );
    }
    Ok((!pairs.is_empty()).then(|| serde_json::Value::Array(pairs).to_string()))
}

/// 변경 피드 한 구간: `search_outbox` 의 `(after, upto]` 이벤트를 자산·provider 별 최대 revision 으로 접어 넣는다 (SQL 계약).
/// 활성 자산만(삭제·병합·hard delete 된 자산의 이벤트는 행을 만들지 않는다 — FK 위반으로 트랜잭션이 깨지지 않게).
pub const FEED_SQL: &str = "INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, updated_at)
 SELECT ?1, p.provider_ref, o.asset_id, MAX(o.revision), ?5
 FROM search_outbox o JOIN media_assets a ON a.asset_id = o.asset_id AND a.status = 'active'
 JOIN (SELECT json_extract(value, '$.p') AS provider_ref, json_extract(value, '$.t') AS media_type FROM json_each(?4)) p
   ON p.media_type = a.media_type
 WHERE o.event_id > ?2 AND o.event_id <= ?3 AND o.event_type = 'upsert'
 GROUP BY o.asset_id, p.provider_ref
 ON CONFLICT (plugin_ref, provider_ref, asset_id) DO UPDATE SET
   target_revision = MAX(target_revision, excluded.target_revision),
   attempts = CASE WHEN status = 'failed' AND excluded.target_revision > target_revision THEN 0 ELSE attempts END,
   status = CASE WHEN MAX(target_revision, excluded.target_revision) > done_revision
                 THEN (CASE WHEN status = 'failed' AND excluded.target_revision <= target_revision THEN status ELSE 'pending' END)
                 ELSE status END,
   updated_at = excluded.updated_at
 RETURNING asset_id, status";

/// 전체 실행 한 구간: `(after, last]` 활성 자산을 현재 revision 으로 접어 넣는다. 이미 그 revision 이 끝난 행은 건드리지 않는다.
pub const RUN_ALL_SQL: &str = "INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, updated_at)
 SELECT ?1, p.provider_ref, a.asset_id, a.revision, ?5
 FROM media_assets a
 JOIN (SELECT json_extract(value, '$.p') AS provider_ref, json_extract(value, '$.t') AS media_type FROM json_each(?4)) p
   ON p.media_type = a.media_type
 WHERE a.asset_id > ?2 AND a.asset_id <= ?3 AND a.status = 'active'
 ON CONFLICT (plugin_ref, provider_ref, asset_id) DO UPDATE SET
   target_revision = MAX(target_revision, excluded.target_revision),
   attempts = CASE WHEN status = 'failed' AND MAX(target_revision, excluded.target_revision) > done_revision THEN 0 ELSE attempts END,
   status = CASE WHEN MAX(target_revision, excluded.target_revision) > done_revision THEN 'pending' ELSE status END,
   updated_at = excluded.updated_at
 RETURNING asset_id, status";

/// 구간 키셋 질의 — 다음 `last` 를 찾는다
pub const RUN_ALL_PAGE_SQL: &str = "SELECT asset_id FROM media_assets WHERE asset_id > ?1 AND status = 'active' ORDER BY asset_id LIMIT ?2";

pub const PENDING_FOR_ASSET_SQL: &str = "SELECT t.task_id, p.plugin_id, v.name, t.target_revision, t.attempts FROM plugin_tasks t JOIN plugins p ON p.plugin_ref = t.plugin_ref JOIN plugin_providers v ON v.provider_ref = t.provider_ref WHERE t.asset_id = ?1 AND t.status = 'pending' ORDER BY t.task_id";
pub const PENDING_COUNT_SQL: &str =
    "SELECT COUNT(*) FROM plugin_tasks WHERE plugin_ref = ?1 AND status = 'pending'";
pub const FAILED_COUNT_SQL: &str =
    "SELECT COUNT(*) FROM plugin_tasks WHERE plugin_ref = ?1 AND status = 'failed'";
pub const RETRY_FAILED_SQL: &str = "UPDATE plugin_tasks SET status = 'pending', attempts = 0, last_error = NULL, updated_at = ?2 WHERE plugin_ref = ?1 AND status = 'failed' RETURNING asset_id";
pub const COMPLETE_SQL: &str = "UPDATE plugin_tasks SET
   done_revision = MAX(done_revision, ?2, ?3),
   status = CASE WHEN target_revision > MAX(done_revision, ?2, ?3) THEN 'pending' ELSE 'done' END,
   attempts = 0, last_error = NULL, updated_at = ?4
 WHERE task_id = ?1";
pub const RETRY_LATER_SQL: &str = "UPDATE plugin_tasks SET attempts = attempts + 1, updated_at = ?2 WHERE task_id = ?1 RETURNING attempts";
pub const FAIL_SQL: &str = "UPDATE plugin_tasks SET status = 'failed', last_error = ?2, updated_at = ?3 WHERE task_id = ?1";
pub const MARK_INACTIVE_SQL: &str = "UPDATE plugin_tasks SET status = 'done', done_revision = target_revision, last_error = 'inactive', updated_at = ?2 WHERE asset_id = ?1 AND status = 'pending'";
pub const DELETE_TASK_SQL: &str = "DELETE FROM plugin_tasks WHERE task_id = ?1";

/// 새로 `pending` 이 된 자산 id (정렬·중복 제거) — 호출자가 `enqueue_jobs_tx` 에 넘긴다
fn pending_assets(rows: Vec<(i64, String)>) -> Vec<i64> {
    let mut ids: Vec<i64> = rows
        .into_iter()
        .filter(|(_, s)| s == "pending")
        .map(|(a, _)| a)
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn run_upsert(
    tx: &Transaction<'_>,
    sql: &str,
    plugin: &str,
    providers: &[ProviderTypes<'_>],
    after: i64,
    upto: i64,
    now: i64,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let plugin_ref = plugin_registry::ensure_rs(tx, plugin)?;
    let Some(pairs) = pairs_json(tx, plugin_ref, providers)? else {
        return Ok(Vec::new());
    };
    let mut stmt = tx.prepare_cached(sql)?;
    let rows = stmt.query_map(params![plugin_ref, after, upto, pairs, now], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    rows.collect()
}

/// 변경 피드 한 구간을 작업으로 접어 넣는다. 돌려주는 값 = 이 구간 때문에 `pending` 인 자산 id(정렬·중복 제거).
/// 멱등 — 같은 구간을 다시 넣어도 행이 늘지 않고 완료된 행은 되살아나지 않는다.
pub fn feed_batch(
    tx: &Transaction<'_>,
    plugin: &str,
    providers: &[ProviderTypes<'_>],
    after: i64,
    upto: i64,
    now: i64,
) -> rusqlite::Result<Vec<i64>> {
    if providers.is_empty() || upto <= after {
        return Ok(Vec::new());
    }
    Ok(pending_assets(run_upsert(
        tx, FEED_SQL, plugin, providers, after, upto, now,
    )?))
}

/// 전체 실행 한 배치 결과
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunAllPage {
    /// 이 배치 때문에 `pending` 인 자산 id (정렬·중복 제거)
    pub pending: Vec<i64>,
    /// 이 배치가 훑은 마지막 자산 id — 다음 배치의 `after` (훑은 자산이 없으면 `None`)
    pub last: Option<i64>,
    /// 훑은 활성 자산 수 (`limit` 보다 작으면 끝)
    pub scanned: usize,
}

/// 활성 자산을 `asset_id` 키셋으로 `limit` 개 훑어 작업으로 접어 넣는다 (기본 [`RUN_ALL_BATCH`]). 이미 현재 revision 이 끝난 행은 건너뛴다.
pub fn run_all_batch(
    tx: &Transaction<'_>,
    plugin: &str,
    providers: &[ProviderTypes<'_>],
    after: i64,
    limit: i64,
    now: i64,
) -> rusqlite::Result<RunAllPage> {
    let mut last = None;
    let mut scanned = 0usize;
    {
        let mut stmt = tx.prepare_cached(RUN_ALL_PAGE_SQL)?;
        for id in stmt.query_map(params![after, limit], |r| r.get::<_, i64>(0))? {
            last = Some(id?);
            scanned += 1;
        }
    }
    let Some(last_id) = last.filter(|_| !providers.is_empty()) else {
        return Ok(RunAllPage {
            pending: Vec::new(),
            last,
            scanned,
        });
    };
    let pending = pending_assets(run_upsert(
        tx,
        RUN_ALL_SQL,
        plugin,
        providers,
        after,
        last_id,
        now,
    )?);
    Ok(RunAllPage {
        pending,
        last,
        scanned,
    })
}

/// 자산마다 `jobs` 행(kind `plugin`)을 하나 올린다 — 이미 대기 중이면 무시(`uq_jobs_pending_asset`). 새로 들어간 수를 돌려준다.
pub fn enqueue_jobs_tx(
    tx: &Transaction<'_>,
    asset_ids: &[i64],
    priority: i64,
    now: i64,
) -> rusqlite::Result<usize> {
    let mut stmt = tx.prepare_cached("INSERT OR IGNORE INTO jobs (kind, asset_id, priority, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)")?;
    let mut inserted = 0;
    for id in asset_ids {
        inserted += stmt.execute(params![JOB_KIND, id, priority, now])?;
    }
    Ok(inserted)
}

/// 대기 중인 작업 한 행
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingTask {
    pub task_id: i64,
    pub plugin_id: String,
    pub provider: String,
    pub target_revision: i64,
    pub attempts: i64,
}

pub fn pending_count(conn: &Connection, plugin: &str) -> rusqlite::Result<i64> {
    let Some(plugin_ref) = plugin_registry::lookup(conn, plugin)? else {
        return Ok(0);
    };
    conn.prepare_cached(PENDING_COUNT_SQL)?
        .query_row([plugin_ref], |r| r.get(0))
}

pub fn failed_count(conn: &Connection, plugin: &str) -> rusqlite::Result<i64> {
    let Some(plugin_ref) = plugin_registry::lookup(conn, plugin)? else {
        return Ok(0);
    };
    conn.prepare_cached(FAILED_COUNT_SQL)?
        .query_row([plugin_ref], |r| r.get(0))
}

/// 한 자산의 대기 작업 (모든 플러그인, `task_id` 순)
pub fn pending_for_asset(conn: &Connection, asset_id: i64) -> rusqlite::Result<Vec<PendingTask>> {
    let mut stmt = conn.prepare_cached(PENDING_FOR_ASSET_SQL)?;
    let rows = stmt.query_map([asset_id], |r| {
        Ok(PendingTask {
            task_id: r.get(0)?,
            plugin_id: r.get(1)?,
            provider: r.get(2)?,
            target_revision: r.get(3)?,
            attempts: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// 작업을 완료로 기록한다 — `done_revision = MAX(현재, processed_rev, done_rev)`. `target_revision` 이 더 크면 `pending` 으로 남는다.
/// `done_rev` 는 루프 방지용: 이 호출의 같은 트랜잭션에서 플러그인이 붙인 태그의 revision(없으면 `processed_rev`).
/// 행이 없으면(삭제·제거) `false`.
pub fn complete(
    tx: &Transaction<'_>,
    task_id: i64,
    processed_rev: i64,
    done_rev: i64,
    now: i64,
) -> rusqlite::Result<bool> {
    Ok(tx
        .prepare_cached(COMPLETE_SQL)?
        .execute(params![task_id, processed_rev, done_rev, now])?
        > 0)
}

/// 일시 오류 — 시도 횟수만 올리고 `pending` 으로 둔다. 새 시도 횟수 (행이 없으면 `None`)
pub fn retry_later(tx: &Transaction<'_>, task_id: i64, now: i64) -> rusqlite::Result<Option<i64>> {
    use rusqlite::OptionalExtension;
    tx.prepare_cached(RETRY_LATER_SQL)?
        .query_row(params![task_id, now], |r| r.get(0))
        .optional()
}

/// 영구 실패 — `failed` 로 두고 고정 오류 코드를 남긴다 (새 revision 이 오거나 `retry_failed` 가 있어야 다시 대기)
pub fn fail(tx: &Transaction<'_>, task_id: i64, code: &str, now: i64) -> rusqlite::Result<bool> {
    Ok(tx
        .prepare_cached(FAIL_SQL)?
        .execute(params![task_id, code, now])?
        > 0)
}

/// 비활성(삭제·병합) 자산의 대기 작업을 모두 끝낸 것으로 닫는다 (`done_revision = target`, `last_error = 'inactive'`). 닫은 행 수.
pub fn mark_inactive_done(
    tx: &Transaction<'_>,
    asset_id: i64,
    now: i64,
) -> rusqlite::Result<usize> {
    tx.prepare_cached(MARK_INACTIVE_SQL)?
        .execute(params![asset_id, now])
}

/// 행 삭제 (provider 가 업그레이드로 사라진 경우)
pub fn delete_task(tx: &Transaction<'_>, task_id: i64) -> rusqlite::Result<bool> {
    Ok(tx.prepare_cached(DELETE_TASK_SQL)?.execute([task_id])? > 0)
}

/// `failed` 를 모두 `pending` 으로 되돌리고(시도 횟수 초기화) 해당 자산 id(정렬·중복 제거)를 돌려준다 — 호출자가 `enqueue_jobs_tx` 에 넘긴다.
pub fn retry_failed(tx: &Transaction<'_>, plugin: &str, now: i64) -> rusqlite::Result<Vec<i64>> {
    let Some(plugin_ref) = plugin_registry::lookup(tx, plugin)? else {
        return Ok(Vec::new());
    };
    let mut stmt = tx.prepare_cached(RETRY_FAILED_SQL)?;
    let mut ids = stmt
        .query_map(params![plugin_ref, now], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// 플러그인별 대기·실패 수
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginTaskCounts {
    pub plugin_id: String,
    pub pending: i64,
    pub failed: i64,
}

/// 주어진 플러그인들의 대기·실패 수 (플러그인마다 부분 인덱스를 SEARCH — 전체 훑기 없음). 입력 순서를 유지한다.
pub fn counts_by_plugin(
    conn: &Connection,
    plugins: &[&str],
) -> rusqlite::Result<Vec<PluginTaskCounts>> {
    plugins
        .iter()
        .map(|p| {
            Ok(PluginTaskCounts {
                plugin_id: (*p).to_owned(),
                pending: pending_count(conn, p)?,
                failed: failed_count(conn, p)?,
            })
        })
        .collect()
}

/// `search_outbox` 의 가장 큰 `event_id` (비면 0) — 기본키 끝에서 읽는다
pub fn max_event_id(conn: &Connection) -> rusqlite::Result<i64> {
    conn.prepare_cached("SELECT COALESCE(MAX(event_id), 0) FROM search_outbox")?
        .query_row([], |r| r.get(0))
}

/// 자산의 현재 revision (없으면 `None`) — 완료 트랜잭션이 루프 방지 `done_rev` 를 계산할 때 쓴다
pub fn asset_revision(conn: &Connection, asset_id: i64) -> rusqlite::Result<Option<i64>> {
    use rusqlite::OptionalExtension;
    conn.prepare_cached("SELECT revision FROM media_assets WHERE asset_id = ?1")?
        .query_row([asset_id], |r| r.get(0))
        .optional()
}

/// 변경 피드 커서 (`plugin_feed_cursor:<id>`) — 없거나 손상이면 `None`
pub fn feed_cursor(conn: &Connection, plugin: &str) -> rusqlite::Result<Option<i64>> {
    Ok(
        crate::plugin_settings::get(conn, crate::plugin_settings::FEED_CURSOR, plugin)?
            .and_then(|raw| raw.trim().parse::<i64>().ok())
            .filter(|c| *c >= 0),
    )
}

/// 커서를 쓴다 (`feed_batch` 와 같은 트랜잭션에서 — 비정상 종료가 구간을 건너뛰지 않게)
pub fn set_feed_cursor(
    tx: &Transaction<'_>,
    plugin: &str,
    cursor: i64,
) -> Result<(), crate::plugin_settings::PutError> {
    crate::plugin_settings::put(
        tx,
        crate::plugin_settings::FEED_CURSOR,
        plugin,
        &cursor.to_string(),
    )
}

/// 커서가 없으면(또는 손상이면) `max_event_id` 로 초기화한다 — 설치 이전의 이벤트는 작업으로 만들지 않는다(Q3). 현재 커서를 돌려준다.
pub fn init_feed_cursor(
    tx: &Transaction<'_>,
    plugin: &str,
) -> Result<i64, crate::plugin_settings::PutError> {
    if let Some(c) = feed_cursor(tx, plugin)? {
        return Ok(c);
    }
    let max = max_event_id(tx)?;
    set_feed_cursor(tx, plugin, max)?;
    Ok(max)
}

/// `with_immediate_tx` — 쓰기 클로저 안에서 즉시 쓰기 트랜잭션을 열고, `f` 가 `Ok` 면 커밋, `Err` 면 롤백한다
pub fn with_immediate_tx<T, E: From<rusqlite::Error>>(
    conn: &mut Connection,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, E>,
) -> Result<T, E> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let out = f(&tx)?;
    tx.commit()?;
    Ok(out)
}

/// 활성 자산의 미디어 유형(`image`/`video`/`audio`) — 비활성(삭제·병합)이거나 없으면 `None` (실행기가 비활성 판정과 종류 필터에 같이 쓴다)
pub const ACTIVE_MEDIA_TYPE_SQL: &str =
    "SELECT media_type FROM media_assets WHERE asset_id = ?1 AND status = 'active'";

pub fn active_media_type(conn: &Connection, asset_id: i64) -> rusqlite::Result<Option<String>> {
    use rusqlite::OptionalExtension;
    conn.prepare_cached(ACTIVE_MEDIA_TYPE_SQL)?
        .query_row([asset_id], |r| r.get(0))
        .optional()
}

/// 플러그인의 대기 작업을 `task_id` 키셋으로 읽는다 — `(task_id, asset_id)` (활성화 시 `jobs` 재등록용, 부분 인덱스 SEARCH)
pub const PENDING_PAGE_SQL: &str = "SELECT task_id, asset_id FROM plugin_tasks WHERE plugin_ref = ?1 AND status = 'pending' AND task_id > ?2 ORDER BY task_id LIMIT ?3";

pub fn pending_page(
    conn: &Connection,
    plugin: &str,
    after_task: i64,
    limit: i64,
) -> rusqlite::Result<Vec<(i64, i64)>> {
    let Some(plugin_ref) = plugin_registry::lookup(conn, plugin)? else {
        return Ok(Vec::new());
    };
    let mut stmt = conn.prepare_cached(PENDING_PAGE_SQL)?;
    let rows = stmt.query_map(params![plugin_ref, after_task, limit], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?;
    rows.collect()
}

/// 그 종류의 작업이 대기·실행 중인가 — 피드가 스캔 계열을 비켜 주는 판단용 (`idx_jobs_kind_status` SEARCH, 첫 행에서 멈춤)
pub const KIND_BUSY_SQL: &str =
    "SELECT 1 FROM jobs WHERE kind = ?1 AND status IN ('pending', 'running') LIMIT 1";

pub fn kind_busy(conn: &Connection, kind: &str) -> rusqlite::Result<bool> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .prepare_cached(KIND_BUSY_SQL)?
        .query_row([kind], |_| Ok(()))
        .optional()?
        .is_some())
}
