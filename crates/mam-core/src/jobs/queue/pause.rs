//! 일시정지 상태: 메모리 값(원자 변수)이 실행 판단에 쓰이고, `settings.job_pause` 가 재시작 후 복원용 사본이다.
//! 변경은 **DB 저장 -> 성공 시 메모리 반영** 순서다 (저장 실패 시 상태 불변 — 화면과 재시작 후 상태가 어긋나지 않게).

use std::sync::atomic::Ordering;

use mam_db::Priority;
use mam_db::job_pause::{JobPauseState, load_job_pause, save_job_pause};

use super::{JobKind, JobQueue, JobsError, lock};

fn bit(k: JobKind) -> u8 {
    1 << k.idx()
}

impl JobQueue {
    /// `None` = 전역 일시정지, `Some(kind)` = 해당 종류만. 실행 중인 작업은 체크포인트에서 멈춘다.
    /// 상태를 DB 에 먼저 저장하고, 실패하면 오류를 돌려주며 상태를 바꾸지 않는다.
    pub fn pause(&self, kind: Option<JobKind>) -> Result<(), JobsError> {
        let _guard = lock(&self.pause_lock);
        let (mut all, mut mask) = (self.paused_all(), self.paused_mask.load(Ordering::SeqCst));
        match kind {
            None => all = true,
            Some(k) => mask |= bit(k),
        }
        self.apply_pause(all, mask)
    }

    /// `None` = 전역 재개(전역·종류별 모두 해제), `Some(kind)` = 해당 종류만 해제. 저장 실패 시 상태 불변.
    pub fn resume(&self, kind: Option<JobKind>) -> Result<(), JobsError> {
        let _guard = lock(&self.pause_lock);
        let (mut all, mut mask) = (self.paused_all(), self.paused_mask.load(Ordering::SeqCst));
        match kind {
            None => (all, mask) = (false, 0),
            Some(k) => mask &= !bit(k),
        }
        self.apply_pause(all, mask)
    }

    fn apply_pause(&self, all: bool, mask: u8) -> Result<(), JobsError> {
        let state = JobPauseState {
            all,
            kinds: kinds_of(mask).map(|k| k.as_str().to_owned()).collect(),
        };
        self.pool.write(Priority::High, move |c| {
            save_job_pause(c, &state)?;
            Ok(())
        })?;
        self.paused_all.store(all, Ordering::SeqCst);
        self.paused_mask.store(mask, Ordering::SeqCst);
        self.notify();
        Ok(())
    }

    /// 앱 시작 시 저장된 일시정지 상태를 메모리에 적용한다 — 실행기 시작 **전에** 호출해야 재시작 직후
    /// 일시정지된 종류의 작업이 한 건도 시작되지 않는다. 모르는 종류 이름은 무시한다.
    pub fn restore_pause(&self) -> Result<(), JobsError> {
        let _guard = lock(&self.pause_lock);
        let state = self.pool.read(load_job_pause)?;
        let mask = state
            .kinds
            .iter()
            .filter_map(|n| JobKind::parse(n))
            .fold(0u8, |m, k| m | bit(k));
        self.paused_all.store(state.all, Ordering::SeqCst);
        self.paused_mask.store(mask, Ordering::SeqCst);
        self.notify();
        Ok(())
    }

    pub fn paused_all(&self) -> bool {
        self.paused_all.load(Ordering::SeqCst)
    }

    pub fn paused_kinds(&self) -> Vec<JobKind> {
        kinds_of(self.paused_mask.load(Ordering::SeqCst)).collect()
    }

    /// 전역 또는 종류별로 멈춰 있는가
    pub fn is_paused(&self, kind: JobKind) -> bool {
        self.paused_all() || self.paused_mask.load(Ordering::SeqCst) & bit(kind) != 0
    }
}

fn kinds_of(mask: u8) -> impl Iterator<Item = JobKind> {
    JobKind::ALL
        .into_iter()
        .filter(move |k| mask & bit(*k) != 0)
}
