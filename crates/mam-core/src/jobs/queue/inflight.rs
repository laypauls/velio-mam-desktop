//! 실행 중인 작업 추적 — 작업별 취소 플래그. USB 꺼내기 준비가 "그 디스크를 쓰는 실행 중 작업" 만 골라 멈추고,
//! 파일을 놓을 때까지 기다리는 데 쓴다 (전역 종료·일시정지와 달리 특정 작업만 양보시킨다).
//!
//! 플래그는 [`JobContext::should_stop`](super::super::JobContext::should_stop) 과 `shutdown_flag()` 에 합쳐진다 —
//! 앱 종료도 모든 플래그를 켜므로 해시·스캔처럼 `shutdown_flag()` 를 보는 코드는 한 곳만 확인하면 된다.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::{Job, JobQueue, lock};

#[derive(Default)]
pub(super) struct Inflight {
    map: Mutex<HashMap<i64, (Job, Arc<AtomicBool>)>>,
    cv: Condvar,
}

impl JobQueue {
    /// claim 직후 등록한다. 종료가 이미 요청됐으면 플래그가 켜진 채로 시작한다.
    pub(super) fn inflight_register(&self, job: &Job) {
        let mut g = lock(&self.inflight.map);
        // 종료 확인을 잠금 안에서 한다 - `request_shutdown` 이 (저장 -> 잠금 -> 전체 켜기) 순서라 놓치는 작업이 없다
        let flag = Arc::new(AtomicBool::new(self.is_shutdown()));
        g.insert(job.job_id, (job.clone(), flag));
    }

    /// 결과 반영 뒤 등록을 지우고 기다리는 쪽을 깨운다.
    pub(super) fn inflight_remove(&self, job_id: i64) {
        lock(&self.inflight.map).remove(&job_id);
        self.inflight.cv.notify_all();
    }

    /// 종료 요청 시: 실행 중인 모든 작업의 플래그를 켠다.
    pub(super) fn inflight_cancel_all(&self) {
        for (_, flag) in lock(&self.inflight.map).values() {
            flag.store(true, Ordering::SeqCst);
        }
    }

    /// 작업 실행 스레드가 쓰는 취소 플래그 (등록돼 있지 않으면 새 플래그 - 이미 끝난 작업).
    pub(crate) fn job_cancel_flag(&self, job_id: i64) -> Arc<AtomicBool> {
        lock(&self.inflight.map).get(&job_id).map_or_else(
            || Arc::new(AtomicBool::new(self.is_shutdown())),
            |(_, f)| Arc::clone(f),
        )
    }

    /// 지금 실행 중인 작업 (등록 순서 무관).
    pub fn running_jobs(&self) -> Vec<Job> {
        lock(&self.inflight.map)
            .values()
            .map(|(j, _)| j.clone())
            .collect()
    }

    /// 해당 작업들에 취소를 요청한다 (체크포인트에서 `Yielded` 로 끝나 대기열에 남는다). 켠 개수를 돌려준다.
    pub fn cancel_running(&self, job_ids: &[i64]) -> usize {
        let g = lock(&self.inflight.map);
        job_ids
            .iter()
            .filter_map(|id| g.get(id))
            .map(|(_, f)| f.store(true, Ordering::SeqCst))
            .count()
    }

    /// 해당 작업들이 모두 끝나(결과가 DB 에 반영돼) 실행 중이 아닐 때까지 기다린다. 끝났으면 true, 시간 초과면 false.
    pub fn wait_jobs_finished(&self, job_ids: &[i64], timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut g = lock(&self.inflight.map);
        loop {
            if !job_ids.iter().any(|id| g.contains_key(id)) {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            g = self
                .inflight
                .cv
                .wait_timeout(g, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}
