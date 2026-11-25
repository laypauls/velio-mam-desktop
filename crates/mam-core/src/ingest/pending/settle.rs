//! 안정화 게이트 — "복사가 끝났는가"를 시계 비교가 아니라 **표본 비교**로 판정한다 (NAS 시계가 PC 와 달라도 맞다).
//!
//! 표본 = (크기, 수정 시각). 같은 표본이 `samples` 회 연속이면 반영한다. 표본 사이는 `min_gap_ms` 이상 띄운다.
//! 상태(표본·연속 횟수)는 메모리가 아니라 `watch_pending` 행에 있어 앱이 꺼졌다 켜져도 이어진다
//! (시작 복구는 연속 횟수를 0 으로 되돌려 처음부터 다시 잰다).
//!
//! 원본 보호: 이 모듈은 파일을 읽기 전용으로만 연다. 배타적 열기 검사(결정 D)도 읽기 권한 + 쓰기 공유 거부
//! (`FILE_SHARE_READ`)일 뿐 쓰기 권한으로는 열지 않는다.

use super::store::PendingRow;

/// 한 번의 관찰 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obs {
    /// 경로가 없다 (삭제·이름 변경으로 사라짐)
    Gone,
    Present {
        size: i64,
        mtime_ms: i64,
    },
}

/// 배타적 열기 검사 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusiveOutcome {
    /// 열렸다 — 다른 쓰기 핸들이 없다
    Free,
    /// 공유 위반 — 누군가 쓰기로 열고 있다 (아직 복사 중)
    Busy,
    /// 권한 등으로 검사할 수 없다 — 크기·수정 시각만으로 판정한다
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// 지금 `apply_path` 로 반영하고 행을 지운다 (복사 완료 또는 사라짐)
    Apply,
    /// 아직 — 표본을 갱신하고 `due_at` 에 다시 확인한다
    Resample {
        size: i64,
        mtime: i64,
        stable: i64,
        due_at: i64,
        /// `give_up_ms` 를 넘겼다 (호출자가 경고를 한 번만 남긴다)
        overdue: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettleGate {
    /// 같은 표본이 이만큼 연속이면 안정
    pub samples: u32,
    /// 표본 사이 최소 간격 (ms)
    pub min_gap_ms: i64,
    /// 처음 본 뒤 이 시간이 지나도 안정이 안 되면 "복사가 끝나지 않음" 경고 (ms). 포기하지는 않는다
    pub give_up_ms: i64,
}

impl Default for SettleGate {
    fn default() -> Self {
        Self {
            samples: 3,
            min_gap_ms: 2_000,
            give_up_ms: 30 * 60 * 1000,
        }
    }
}

/// 경고 뒤에도 계속 안정이 안 되는 파일(긴 녹화 등)은 이 간격으로 느리게 본다.
pub const OVERDUE_GAP_MS: i64 = 30_000;

impl SettleGate {
    /// 관찰 1건으로 다음 단계를 정한다 (순수 — 시각은 호출자가 넘긴다).
    ///
    /// - 사라짐(`Gone`) -> 즉시 `Apply`: 삭제도 한 번은 `apply_path` 에 닿아야 자산이 missing 으로 표시된다
    ///   (`apply_path` 는 사라진 경로를 실제 상태로 판단하고 멱등이다). 삭제는 안정화를 기다리지 않는다.
    /// - 표본이 행의 표본과 같고 이전 연속이 있었다 -> 연속 +1, 아니면 표본 교체 후 연속 1
    /// - 연속이 `samples` 에 닿으면 `Apply`. 배타적 열기 검사가 켜져 있으면(`exclusive` 가 `Some`) 그것도 통과해야 한다
    pub fn step(
        &self,
        row: &PendingRow,
        obs: Obs,
        now: i64,
        exclusive: Option<&mut dyn FnMut() -> ExclusiveOutcome>,
    ) -> Step {
        let Obs::Present { size, mtime_ms } = obs else {
            return Step::Apply;
        };
        let same = row.stable_count > 0
            && row.sample_size == Some(size)
            && row.sample_mtime == Some(mtime_ms);
        let stable = if same { row.stable_count + 1 } else { 1 };
        if stable >= i64::from(self.samples.max(1)) {
            let busy = exclusive.is_some_and(|f| f() == ExclusiveOutcome::Busy);
            if !busy {
                return Step::Apply;
            }
        }
        let overdue = now.saturating_sub(row.first_seen_at) >= self.give_up_ms;
        let gap = if overdue {
            self.min_gap_ms.max(OVERDUE_GAP_MS)
        } else {
            self.min_gap_ms
        };
        Step::Resample {
            size,
            mtime: mtime_ms,
            stable,
            due_at: now + gap,
            overdue,
        }
    }
}

/// 읽기 전용 + 쓰기 공유 거부로 열어 본다 (Windows). 쓰기 권한으로는 절대 열지 않는다.
/// 공유 위반(32)·잠금 위반(33) = `Busy`, 그 밖의 실패(접근 거부 등) = `Unknown` (로그는 호출자가 한 번만).
#[cfg(windows)]
pub fn probe_exclusive(path: &std::path::Path) -> ExclusiveOutcome {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
    {
        Ok(_) => ExclusiveOutcome::Free,
        Err(e) => match e.raw_os_error() {
            Some(32 | 33) => ExclusiveOutcome::Busy,
            _ => ExclusiveOutcome::Unknown,
        },
    }
}

/// Windows 가 아니면 배타적 열기 개념이 없다 — 검사하지 않는다(표본 비교만).
#[cfg(not(windows))]
pub fn probe_exclusive(_path: &std::path::Path) -> ExclusiveOutcome {
    ExclusiveOutcome::Unknown
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (연속 일치) / Boundary (samples=1, give_up 경계) / Business rule (삭제·배타적 열기) / Time (주입 시각)
    use super::super::store::PendingKind;
    use super::*;

    const GATE: SettleGate = SettleGate {
        samples: 3,
        min_gap_ms: 2_000,
        give_up_ms: 60_000,
    };

    fn row(stable: i64, sample: Option<(i64, i64)>) -> PendingRow {
        PendingRow {
            folder_id: 1,
            path_key: "k".into(),
            abs_path: "k".into(),
            kind: PendingKind::Path,
            source: "t".into(),
            first_seen_at: 0,
            last_seen_at: 0,
            due_at: 0,
            sample_size: sample.map(|s| s.0),
            sample_mtime: sample.map(|s| s.1),
            stable_count: stable,
            attempts: 0,
            last_error: None,
        }
    }

    fn present(size: i64) -> Obs {
        Obs::Present { size, mtime_ms: 5 }
    }

    #[test]
    fn t_settle_01_three_equal_samples_apply_and_a_change_restarts_the_count() {
        // Arrange / Act: 첫 표본 -> 1, 같음 -> 2, 같음 -> 3 = Apply
        let s1 = GATE.step(&row(0, None), present(10), 100, None);
        assert_eq!(
            s1,
            Step::Resample {
                size: 10,
                mtime: 5,
                stable: 1,
                due_at: 2_100,
                overdue: false
            }
        );
        let s2 = GATE.step(&row(1, Some((10, 5))), present(10), 2_100, None);
        assert!(matches!(s2, Step::Resample { stable: 2, .. }));
        assert_eq!(
            GATE.step(&row(2, Some((10, 5))), present(10), 4_100, None),
            Step::Apply
        );
        // Assert: 2 회째에 크기가 바뀌면 처음부터
        let changed = GATE.step(&row(2, Some((10, 5))), present(11), 4_100, None);
        assert!(matches!(
            changed,
            Step::Resample {
                size: 11,
                stable: 1,
                ..
            }
        ));
        let mtime_only = GATE.step(
            &row(2, Some((10, 5))),
            Obs::Present {
                size: 10,
                mtime_ms: 6,
            },
            4_100,
            None,
        );
        assert!(matches!(
            mtime_only,
            Step::Resample {
                mtime: 6,
                stable: 1,
                ..
            }
        ));
    }

    #[test]
    fn t_settle_02_report_reset_means_the_next_equal_sample_counts_as_the_first() {
        // 기록기가 stable_count=0 으로 되돌린 행 (표본은 남아 있다) — 같은 값이어도 연속은 1 부터
        let s = GATE.step(&row(0, Some((10, 5))), present(10), 100, None);
        assert!(matches!(s, Step::Resample { stable: 1, .. }));
    }

    #[test]
    fn t_settle_03_a_gone_path_is_applied_at_once_so_the_deletion_reaches_apply_path() {
        assert_eq!(GATE.step(&row(0, None), Obs::Gone, 1, None), Step::Apply);
        assert_eq!(
            GATE.step(&row(2, Some((1, 1))), Obs::Gone, 1, None),
            Step::Apply
        );
    }

    #[test]
    fn t_settle_04_samples_of_one_applies_on_the_first_look() {
        let g = SettleGate { samples: 1, ..GATE };
        assert_eq!(g.step(&row(0, None), present(1), 0, None), Step::Apply);
    }

    #[test]
    fn t_settle_05_exclusive_check_runs_only_when_stable_and_busy_blocks_apply() {
        let mut calls = 0;
        let mut busy = || {
            calls += 1;
            ExclusiveOutcome::Busy
        };
        // 아직 안정 전 — 검사하지 않는다
        let early = GATE.step(&row(1, Some((10, 5))), present(10), 0, Some(&mut busy));
        assert!(matches!(early, Step::Resample { stable: 2, .. }));
        // 안정 + 사용 중 -> 반영하지 않고 연속을 유지한 채 다시 본다
        let held = GATE.step(&row(2, Some((10, 5))), present(10), 0, Some(&mut busy));
        assert!(matches!(held, Step::Resample { stable: 3, .. }));
        assert_eq!(calls, 1);
        // 열림·검사 불가 -> 반영
        let mut free = || ExclusiveOutcome::Free;
        assert_eq!(
            GATE.step(&row(2, Some((10, 5))), present(10), 0, Some(&mut free)),
            Step::Apply
        );
        let mut unknown = || ExclusiveOutcome::Unknown;
        assert_eq!(
            GATE.step(&row(2, Some((10, 5))), present(10), 0, Some(&mut unknown)),
            Step::Apply
        );
    }

    #[test]
    fn t_settle_06_give_up_marks_overdue_and_slows_sampling_without_dropping_the_row() {
        let before = GATE.step(&row(0, None), present(1), 59_999, None);
        assert!(
            matches!(before, Step::Resample { overdue: false, due_at, .. } if due_at == 61_999)
        );
        let after = GATE.step(&row(0, None), present(1), 60_000, None);
        assert!(
            matches!(after, Step::Resample { overdue: true, due_at, .. } if due_at == 60_000 + OVERDUE_GAP_MS)
        );
    }
}
