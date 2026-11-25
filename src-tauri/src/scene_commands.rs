//! 동영상 장면전환점 command (7개): 조회·사용자 컷 추가/제거·자동 컷 모두 삭제(`UserData`), 검출 요청·기존 영상
//! 백필·백필 대상 수(작업 대기열 DB 풀). 로직은 `mam_core::userdata`(`scene_cuts.rs`)와 `mam_db::scene_cuts` 에 있고
//! 여기서는 인자 변환·`spawn_blocking`·오류 변환만 한다. 장면전환점은 검색 색인 대상이 아니라 `search.wake()` 를
//! 호출하지 않는다. `scene://updated` 이벤트는 검출 실행기가 커밋 뒤 발행한다 (여기서는 발행하지 않는다).
//!
//! 오류: `not_found`(없는·비활성 자산), `invalid_input`("mediaType"·"ms"·"cuts"·"assetIds").

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use mam_core::ingest::Ingest;
use mam_core::jobs::JobQueue;
use mam_core::scan::IngestError;
use mam_core::userdata::{CutSource, SceneCuts, UserData};
use mam_db::scene_cuts::{
    self, BACKFILL_BATCH, RequestOptions, SCENE_PRIORITY_USER, SceneError, backfill_batch_tx,
};
use mam_db::{Clock, DbPool, Priority, SceneSensitivity, SystemClock};
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::error::CommandError;
use crate::ingest_commands::run_blocking;

/// 한 요청이 다루는 자산 수 상한 (사용자 데이터 command 와 같다)
pub(crate) const MAX_REQUEST_ASSETS: usize = 10_000;

/// 백필 등록 스레드 실행 중 표시 (프로세스당 하나)
static BACKFILL_RUNNING: LazyLock<Arc<AtomicBool>> =
    LazyLock::new(|| Arc::new(AtomicBool::new(false)));

/// 계약 `SceneDetectOptions` — 모든 필드 생략 가능
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct SceneDetectOptions {
    /// 이미 자동 추출된 영상도 다시 검출
    pub redetect: bool,
    /// 사용자가 지운 자동 컷의 기억(숨김 목록)을 비우고 다시 검출
    pub restore_suppressed: bool,
    /// 이번 요청에만 쓸 민감도 (생략 = 설정값)
    pub sensitivity: Option<SceneSensitivity>,
}

/// 계약 `SceneRequestResult`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneRequestResult {
    pub queued: u32,
    pub skipped: u32,
}

/// 계약 `SceneBackfillResult`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneBackfillResult {
    pub queued: u32,
}

fn now_ms() -> i64 {
    SystemClock.now_ms()
}

/// 검출 요청: 중복 제거 후 한 High 트랜잭션으로 등록한다. 0개 또는 10,000 초과는 `invalid_input("assetIds")`.
pub(crate) fn request_assets(
    pool: &DbPool,
    asset_ids: &[i64],
    opts: &SceneDetectOptions,
    now: i64,
) -> Result<SceneRequestResult, IngestError> {
    if asset_ids.is_empty() || asset_ids.len() > MAX_REQUEST_ASSETS {
        return Err(IngestError::InvalidInput("assetIds"));
    }
    let mut ids = asset_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let opts = RequestOptions {
        redetect: opts.redetect,
        restore_suppressed: opts.restore_suppressed,
        sensitivity: opts.sensitivity,
    };
    let out = pool.write(Priority::High, move |c| {
        Ok((|| -> Result<_, SceneError> {
            let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let out = scene_cuts::request_tx(&tx, &ids, &opts, SCENE_PRIORITY_USER, now)?;
            tx.commit()?;
            Ok(out)
        })())
    })??;
    Ok(SceneRequestResult {
        queued: out.queued,
        skipped: out.skipped,
    })
}

/// 백필 등록 한 묶음 실행 환경 (명령은 즉시 반환하고 등록은 백그라운드 스레드가 한다)
pub(crate) struct BackfillRun {
    pub pool: Arc<DbPool>,
    /// 등록 스레드가 도는 동안 true (중복 시작 방지)
    pub running: Arc<AtomicBool>,
    /// 앱 종료 신호 - 배치 사이에서 확인한다
    pub stop: Arc<AtomicBool>,
    /// 등록이 끝났을 때(중단 포함) 호출 - 작업 대기열 깨우기
    pub on_done: Arc<dyn Fn() + Send + Sync>,
    /// 각 배치 직전에 호출 (시험에서 느린 등록을 흉내 내는 주입점)
    pub before_batch: Arc<dyn Fn() + Send + Sync>,
    pub now: i64,
}

/// 기존 영상 백필 등록: 행 없는 활성 영상을 1000건씩 Low 트랜잭션으로 등록한다. 배치 사이에 `stop` 이면 멈춘다.
/// 등록한 총 수를 돌려준다. 멱등 (행이 있는 자산은 대상이 아니다).
pub(crate) fn backfill_all(run: &BackfillRun) -> Result<u32, IngestError> {
    let now = run.now;
    let mut after = 0_i64;
    let mut queued = 0_u32;
    loop {
        if run.stop.load(Ordering::SeqCst) {
            return Ok(queued);
        }
        (run.before_batch)();
        let batch = run.pool.write(Priority::Low, move |c| {
            Ok((|| -> Result<_, SceneError> {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let b = backfill_batch_tx(&tx, after, BACKFILL_BATCH, now)?;
                tx.commit()?;
                Ok(b)
            })())
        })??;
        queued = queued.saturating_add(batch.queued);
        match batch.last_id {
            Some(id) => after = id,
            None => return Ok(queued),
        }
    }
}

/// 백필을 시작한다: 대상 수를 세어 즉시 돌려주고, 등록은 백그라운드 스레드로 넘긴다.
/// 이미 등록 중이면 새로 시작하지 않고 현재 대상 수만 돌려준다.
pub(crate) fn start_backfill(run: BackfillRun) -> Result<SceneBackfillResult, IngestError> {
    let queued = backfill_target_count(&run.pool)?;
    if queued == 0 || run.running.swap(true, Ordering::SeqCst) {
        return Ok(SceneBackfillResult { queued });
    }
    let running = Arc::clone(&run.running);
    let spawned = std::thread::Builder::new()
        .name("scene-backfill".into())
        .spawn(move || {
            if let Err(e) = backfill_all(&run) {
                tracing::warn!(error = %e, "scene backfill registration stopped");
            }
            run.running.store(false, Ordering::SeqCst);
            (run.on_done)();
        });
    if let Err(e) = spawned {
        running.store(false, Ordering::SeqCst);
        return Err(IngestError::Io(e));
    }
    Ok(SceneBackfillResult { queued })
}

/// 백필 대상 수 (행 없는 활성 영상)
pub(crate) fn backfill_target_count(pool: &DbPool) -> Result<u32, IngestError> {
    let n = pool.read(|c| Ok(scene_cuts::backfill_count(c)))??;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

type Users<'a> = State<'a, Arc<UserData>>;

async fn blocking<T, F>(svc: &Users<'_>, f: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce(&UserData) -> Result<T, IngestError> + Send + 'static,
{
    run_blocking(Arc::clone(svc), f).await
}

/// 자산의 장면전환점 (행이 없으면 `state: "none"`).
#[tauri::command]
pub async fn scene_cuts_get(svc: Users<'_>, asset_id: i64) -> Result<SceneCuts, CommandError> {
    blocking(&svc, move |u| u.scene_cuts_get(asset_id)).await
}

/// 사용자 컷 추가 (같은 위치 ±1프레임 안에 이미 있으면 no-op). 갱신된 상태를 돌려준다.
#[tauri::command]
pub async fn scene_cut_add(
    svc: Users<'_>,
    asset_id: i64,
    ms: i64,
) -> Result<SceneCuts, CommandError> {
    blocking(&svc, move |u| u.scene_cut_add(asset_id, ms)).await
}

/// 가장 가까운 컷 1개 제거 (자동 컷은 숨김 목록에 기억). 갱신된 상태를 돌려준다.
#[tauri::command]
pub async fn scene_cut_remove(
    svc: Users<'_>,
    asset_id: i64,
    ms: i64,
    source: CutSource,
) -> Result<SceneCuts, CommandError> {
    blocking(&svc, move |u| u.scene_cut_remove(asset_id, ms, source)).await
}

/// 자동 컷을 모두 숨김으로 옮긴다 (재추출해도 되살아나지 않는다). 갱신된 상태를 돌려준다.
#[tauri::command]
pub async fn scene_cuts_clear_auto(
    svc: Users<'_>,
    asset_id: i64,
) -> Result<SceneCuts, CommandError> {
    blocking(&svc, move |u| u.scene_cuts_clear_auto(asset_id)).await
}

/// 검출 요청 (사용자 요청 우선순위 45). 활성 영상만 대상이고 나머지는 `skipped`.
/// 대기열에 올리기만 한다 — 검출(FFmpeg)은 작업 실행기가 한다.
#[tauri::command]
pub async fn scene_detect_request(
    ingest: State<'_, Arc<Ingest>>,
    asset_ids: Vec<i64>,
    options: Option<SceneDetectOptions>,
) -> Result<SceneRequestResult, CommandError> {
    let opts = options.unwrap_or_default();
    run_blocking(Arc::clone(ingest.jobs()), move |jobs: &JobQueue| {
        let out = request_assets(jobs.pool(), &asset_ids, &opts, now_ms())?;
        if out.queued > 0 {
            jobs.notify();
        }
        Ok(out)
    })
    .await
}

/// 기존 영상 모두 추출 (백그라운드 우선순위 60). 즉시 반환한다: 대상 수를 세고 등록은 백그라운드 스레드가 한다.
/// 이미 등록 중이면 새로 시작하지 않는다. 등록이 끝나면 작업 대기열을 깨운다.
#[tauri::command]
pub async fn scene_detect_backfill(
    ingest: State<'_, Arc<Ingest>>,
) -> Result<SceneBackfillResult, CommandError> {
    let jobs = Arc::clone(ingest.jobs());
    let notify = Arc::clone(&jobs);
    run_blocking(jobs, move |jobs: &JobQueue| {
        start_backfill(BackfillRun {
            pool: Arc::clone(jobs.pool()),
            running: Arc::clone(&BACKFILL_RUNNING),
            stop: jobs.shutdown_handle(),
            on_done: Arc::new(move || notify.notify()),
            before_batch: Arc::new(|| {}),
            now: now_ms(),
        })
    })
    .await
}

/// 백필 대상 영상 수 (설정 화면의 "모두 추출 (영상 N개)" 라벨).
#[tauri::command]
pub async fn scene_backfill_count(ingest: State<'_, Arc<Ingest>>) -> Result<u32, CommandError> {
    run_blocking(Arc::clone(ingest.jobs()), move |jobs: &JobQueue| {
        backfill_target_count(jobs.pool())
    })
    .await
}

#[cfg(test)]
#[path = "scene_commands_tests.rs"]
mod scene_commands_tests;
