//! AI 분석 등록 - 신규 자동·백그라운드 스윕·선택/일괄·실패 재시도·동의 철회 (D7). 시계는 주입한다.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use mam_db::{DbError, DbPool, Priority};
use rusqlite::Connection;

use super::config::{AiRuntimeSettings, local_hour};
use super::origin::Origin;
use super::plan::{
    ACTIVE_SQL, CHUNK, EMBEDDING_BACKFILL_SQL, MISSING_SUBTITLE_SQL, QueueResult, UNANALYZED_SQL,
    ids_page, mark_skipped, plan,
};
use super::types::StepKind;

mod ops;
use super::{AiJobError, state};
use crate::SharedClock;
use crate::jobs::{JobKind, JobQueue, NewJob};

pub struct AiScheduler {
    pool: Arc<DbPool>,
    queue: Arc<JobQueue>,
    clock: SharedClock,
    sweep_batch: usize,
    /// 임베딩 백필이 마지막으로 고른 asset_id (keyset 시작점)
    backfill_cursor: AtomicI64,
}

impl AiScheduler {
    pub fn new(
        pool: Arc<DbPool>,
        queue: Arc<JobQueue>,
        clock: SharedClock,
        sweep_batch: usize,
    ) -> Self {
        Self {
            pool,
            queue,
            clock,
            sweep_batch,
            backfill_cursor: AtomicI64::new(0),
        }
    }

    pub(super) fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection, i64) -> Result<T, AiJobError> + Send + 'static,
    ) -> Result<T, AiJobError> {
        let now = self.clock.now_ms();
        self.pool
            .write(Priority::Low, move |c| Ok::<_, DbError>(f(c, now)))?
    }

    fn enqueue_jobs(&self, ids: &[i64]) -> Result<(), AiJobError> {
        for id in ids {
            self.queue.enqueue(NewJob::for_asset(JobKind::Ai, *id))?;
        }
        Ok(())
    }

    /// 자산들의 단계를 queued 로 만들고 작업을 등록한다. `mark_skipped_when_empty` 면 적용할 단계가 없는 자산에
    /// skipped 표식을 남겨 스윕이 같은 자산을 반복해서 고르지 않게 한다.
    fn queue_assets(
        &self,
        ids: Vec<i64>,
        requested: Option<Vec<StepKind>>,
        force: bool,
        st: &AiRuntimeSettings,
        mark_skipped_when_empty: bool,
        origin: Origin,
    ) -> Result<QueueResult, AiJobError> {
        let st = st.clone();
        let (res, to_enqueue) = self.write(move |c, now| {
            let tx = c.transaction()?;
            let mut res = QueueResult::default();
            let mut jobs = Vec::new();
            for id in ids {
                let media: Option<String> = tx
                    .query_row(
                        "SELECT media_type FROM media_assets WHERE asset_id = ?1 AND status = 'active'",
                        [id],
                        |r| r.get(0),
                    )
                    .ok();
                let Some(media) = media else {
                    res.skipped += 1;
                    continue;
                };
                let kinds = plan(&media, requested.as_deref(), &st);
                if kinds.is_empty() {
                    res.skipped += 1;
                    if mark_skipped_when_empty && st.consent_given {
                        mark_skipped(&tx, now, id, &media)?;
                    }
                } else if state::upsert_queued(&tx, now, id, &kinds, force, origin)? > 0 {
                    res.queued += 1;
                    jobs.push(id);
                }
            }
            tx.commit()?;
            Ok((res, jobs))
        })?;
        self.enqueue_jobs(&to_enqueue)?;
        Ok(res)
    }

    /// 신규 자산 자동 등록. 종류는 두 옵션을 합친 것이다 - 자동 분석(`autoAnalyzeNew` + 동의) = 외부 종류(+ 의미 검색이
    /// 켜져 있으면 임베딩, 자막 제외), 자동 자막(`auto_subtitle_new` 실효값 + 자막 스위치) = `[Stt]` (동의 불필요).
    /// 등록할 종류가 없으면 아무것도 하지 않는다 (skipped 1).
    pub fn enqueue_new_asset(
        &self,
        asset_id: i64,
        st: &AiRuntimeSettings,
    ) -> Result<QueueResult, AiJobError> {
        let mut kinds: Vec<StepKind> = Vec::new();
        if st.auto_analyze_new && st.consent_given {
            kinds.extend(
                st.external_kinds
                    .iter()
                    .copied()
                    .filter(|k| *k != StepKind::Stt),
            );
            if st.semantic_enabled {
                kinds.push(StepKind::Embedding);
            }
        }
        if st.auto_subtitle_new && st.stt_enabled {
            kinds.push(StepKind::Stt);
        }
        if kinds.is_empty() {
            return Ok(QueueResult {
                queued: 0,
                skipped: 1,
            });
        }
        self.queue_assets(
            vec![asset_id],
            Some(kinds),
            false,
            st,
            false,
            Origin::AutoNew,
        )
    }

    /// 자막이 없는 활성 동영상·음성 전체에 자막(`stt`)만 등록한다 (없음·skipped·failed 대상, done·queued·running 제외).
    /// 자막 스위치가 꺼져 있으면 거부한다. 500건 keyset 단위, 출처 `manual`.
    pub fn generate_subtitles_all(
        &self,
        st: &AiRuntimeSettings,
    ) -> Result<QueueResult, AiJobError> {
        if !st.stt_enabled {
            return Err(AiJobError::SubtitleDisabled);
        }
        let mut total = QueueResult::default();
        let mut after = 0;
        loop {
            let ids = self
                .pool
                .read(|c| Ok::<_, DbError>(ids_page(c, MISSING_SUBTITLE_SQL, after, CHUNK)))??;
            let Some(last) = ids.last().copied() else {
                return Ok(total);
            };
            after = last;
            total.add(self.queue_assets(
                ids,
                Some(vec![StepKind::Stt]),
                false,
                st,
                false,
                Origin::Manual,
            )?);
        }
    }

    /// 미분석 자산 백그라운드 분석: `backgroundAnalyze` + 동의 + 예약 시간대 안일 때만, 대기 중 AI 작업을 포함해
    /// `sweep_batch` 까지 등록한다. `utc_offset_min` = 로컬 - UTC (분).
    pub fn background_sweep(
        &self,
        st: &AiRuntimeSettings,
        utc_offset_min: i32,
    ) -> Result<QueueResult, AiJobError> {
        let hour = local_hour(self.clock.now_ms(), utc_offset_min);
        if !st.background_analyze || !st.consent_given || !st.schedule.allows(hour) {
            return Ok(QueueResult::default());
        }
        let budget = self.sweep_budget()?;
        if budget == 0 {
            return Ok(QueueResult::default());
        }
        let ids = self
            .pool
            .read(|c| Ok::<_, DbError>(ids_page(c, UNANALYZED_SQL, 0, budget)))?;
        self.queue_assets(ids?, None, false, st, true, Origin::Background)
    }

    /// `sweep_batch` 에서 대기 중 AI 작업 수를 뺀 남은 등록 예산.
    fn sweep_budget(&self) -> Result<usize, AiJobError> {
        let pending: i64 = self.pool.read(|c| {
            Ok::<_, DbError>(
                c.query_row(
                    "SELECT COUNT(*) FROM jobs WHERE kind = 'ai' AND status = 'pending'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0),
            )
        })?;
        Ok(self
            .sweep_batch
            .saturating_sub(usize::try_from(pending).unwrap_or(usize::MAX)))
    }

    /// 의미 검색(`semanticSearch.enabled`) 켜짐 상태의 임베딩 백필: embedding 단계 행이 없는 활성 자산에
    /// 임베딩만 등록한다. 로컬 추론이라 외부 AI 동의·예약 시간대와 무관하고(AI 대기열 동시성·일시정지는 따른다),
    /// 어떤 상태든 embedding 행이 생긴 자산(완료·실패·skipped)은 다시 고르지 않는다 - 실패는 수동 재시도.
    /// 선택 커서는 메모리에만 두어 이미 처리된 앞부분을 틱마다 다시 훑지 않는다(재시작·꺼짐 시 처음부터).
    pub fn embedding_backfill(&self, st: &AiRuntimeSettings) -> Result<QueueResult, AiJobError> {
        if !st.semantic_enabled {
            self.backfill_cursor.store(0, Ordering::Relaxed);
            return Ok(QueueResult::default());
        }
        let budget = self.sweep_budget()?;
        if budget == 0 {
            return Ok(QueueResult::default());
        }
        let after = self.backfill_cursor.load(Ordering::Relaxed);
        let ids = self
            .pool
            .read(|c| Ok::<_, DbError>(ids_page(c, EMBEDDING_BACKFILL_SQL, after, budget)))??;
        let last = ids.last().copied();
        let res = self.queue_assets(
            ids,
            Some(vec![StepKind::Embedding]),
            false,
            st,
            false,
            Origin::Backfill,
        )?;
        // 등록이 성공한 뒤에만 커서를 옮긴다 (실패하면 같은 자산을 다시 시도)
        if let Some(last) = last {
            self.backfill_cursor.store(last, Ordering::Relaxed);
        }
        Ok(res)
    }

    /// 선택 자산 분석/재분석. `kinds = None` 이면 설정 기본. `force` 면 완료된 단계도 다시 한다 (덮어쓰기).
    pub fn analyze_assets(
        &self,
        ids: &[i64],
        kinds: Option<&[StepKind]>,
        force: bool,
        st: &AiRuntimeSettings,
    ) -> Result<QueueResult, AiJobError> {
        let mut total = QueueResult::default();
        for chunk in ids.chunks(CHUNK) {
            total.add(self.queue_assets(
                chunk.to_vec(),
                kinds.map(<[_]>::to_vec),
                force,
                st,
                false,
                Origin::Manual,
            )?);
        }
        Ok(total)
    }

    /// 전체 일괄. `only_unanalyzed = false` 면 모든 활성 자산을 덮어쓰기로 재분석한다.
    pub fn analyze_all(
        &self,
        only_unanalyzed: bool,
        st: &AiRuntimeSettings,
    ) -> Result<QueueResult, AiJobError> {
        let sql = if only_unanalyzed {
            UNANALYZED_SQL
        } else {
            ACTIVE_SQL
        };
        let mut total = QueueResult::default();
        let mut after = 0;
        loop {
            let ids = self
                .pool
                .read(|c| Ok::<_, DbError>(ids_page(c, sql, after, CHUNK)))??;
            let Some(last) = ids.last().copied() else {
                return Ok(total);
            };
            after = last;
            total.add(self.queue_assets(
                ids,
                None,
                !only_unanalyzed,
                st,
                only_unanalyzed,
                Origin::Manual,
            )?);
        }
    }
}
