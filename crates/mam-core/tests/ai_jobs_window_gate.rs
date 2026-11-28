//! T-aisched-01 ~ 06 - 예약 시간대 실행 게이트: 이미 등록된 `background` 출처 작업은 시간대 밖이면 삭제하지 않고
//! 다음 시간대 시작까지 미룬다 (시도 횟수 불변). 수동·신규 자동 출처와 시간대 꺼짐은 영향 없다.
//! 분류: Time (주입 시계, 로컬 시각 22:13 → 시간대 01~05) / Business rule (출처별 정책) / 게이트 ON·OFF / Boundary (시작 시각).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::{AiSettingsSource, Schedule};
use mam_core::jobs::JobOutcome;
use mam_db::Clock;

const T0: i64 = 1_700_000_000_000; // 2023-11-14 22:13:20 UTC (오프셋 0 이므로 로컬 22시)
const WINDOW_START: i64 = 1_700_010_000_000; // 2023-11-15 01:00:00 UTC

fn night_window() -> Schedule {
    Schedule {
        enabled: true,
        start_hour: 1,
        end_hour: 5,
    }
}

/// 시간대가 꺼진 동안 백그라운드 스윕으로 등록한 이미지 1건 (출처 `background`) - 이후 시간대를 켠다.
fn registered_by_sweep(e: &AiEnv) -> i64 {
    let id = e.add_asset("image");
    e.set(|s| {
        s.stt_enabled = false;
        s.background_analyze = true;
    });
    let st = e.settings.snapshot();
    assert_eq!(e.sched.background_sweep(&st, 0).unwrap().queued, 1);
    id
}

fn not_before(e: &AiEnv) -> i64 {
    e.env.count("SELECT not_before FROM jobs WHERE kind = 'ai'")
}

fn attempts(e: &AiEnv) -> i64 {
    e.env
        .count("SELECT COALESCE(MAX(attempts), 0) FROM jobs WHERE kind = 'ai'")
}

#[test]
fn t_aisched_01_background_job_outside_the_window_is_deferred_to_the_next_start_without_attempts() {
    // Arrange: 22시에 시간대(01~05)가 켜져 있는데 이미 등록된 background 작업이 있다
    let e = AiEnv::new();
    let id = registered_by_sweep(&e);
    e.set(|s| s.schedule = night_window());
    assert_eq!(e.env.clock.now_ms(), T0);

    // Act
    let out = e.run_next(&|| false);

    // Assert: 미룸 - 실행·전송·시도·이력 없음, 단계는 그대로 queued (삭제 아님)
    assert_eq!(out, Some(Ok(JobOutcome::Yielded)));
    assert_eq!(attempts(&e), 0);
    assert_eq!(e.step(id, "describe"), Some(("queued".into(), None)));
    assert!(e.ext.calls.lock().unwrap().is_empty());
    assert_eq!(e.env.count("SELECT COUNT(*) FROM analysis_runs"), 0);
    assert_eq!(not_before(&e), WINDOW_START);
    // 시간대 시작 전에는 다시 꺼내지지 않는다 (바쁜 루프 없음)
    assert!(e.run_next(&|| false).is_none());
    e.env.clock.advance(WINDOW_START - T0 - 1);
    assert!(e.run_next(&|| false).is_none());
}

#[test]
fn t_aisched_02_deferred_background_job_runs_once_the_window_opens() {
    // Arrange
    let e = AiEnv::new();
    let id = registered_by_sweep(&e);
    e.set(|s| s.schedule = night_window());
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Yielded)));

    // Act: 시간대 시작 정각
    e.env.clock.advance(WINDOW_START - T0);
    let out = e.run_next(&|| false);

    // Assert
    assert_eq!(out, Some(Ok(JobOutcome::Done)));
    assert_eq!(e.st(id, "describe"), "done");
    assert_eq!(e.ext.calls.lock().unwrap().len(), 1);
    assert_eq!(attempts(&e), 0);
}

#[test]
fn t_aisched_03_background_job_runs_inside_the_window_and_when_the_schedule_is_off() {
    // Arrange / Act / Assert: 시간대 밖 22시에 schedule 꺼짐 -> 실행 (게이트 OFF)
    let e = AiEnv::new();
    let id = registered_by_sweep(&e);
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Done)));
    assert_eq!(e.st(id, "describe"), "done");

    // 시간대 안(23시 포함 22~06)에서는 실행
    let e = AiEnv::new();
    let id = registered_by_sweep(&e);
    e.set(|s| {
        s.schedule = Schedule {
            enabled: true,
            start_hour: 22,
            end_hour: 6,
        }
    });
    assert_eq!(e.run_next(&|| false), Some(Ok(JobOutcome::Done)));
    assert_eq!(e.st(id, "describe"), "done");
}

#[test]
fn t_aisched_04_manual_and_new_auto_origins_ignore_the_window() {
    // Arrange: 22시, 시간대 01~05
    let e = AiEnv::new();
    e.set(|s| {
        s.schedule = night_window();
        s.auto_analyze_new = true;
        s.stt_enabled = false;
    });
    let manual = e.add_asset("image");
    let fresh = e.add_asset("image");
    let st = e.settings.snapshot();
    e.sched.analyze_assets(&[manual], None, false, &st).unwrap();
    e.sched.enqueue_new_asset(fresh, &st).unwrap();

    // Act
    let a = e.run_next(&|| false);
    let b = e.run_next(&|| false);

    // Assert: 둘 다 시간대와 무관하게 실행
    assert_eq!(
        (a, b),
        (Some(Ok(JobOutcome::Done)), Some(Ok(JobOutcome::Done)))
    );
    assert_eq!(e.st(manual, "describe"), "done");
    assert_eq!(e.st(fresh, "describe"), "done");
}

#[test]
fn t_aisched_05_manual_request_over_a_queued_background_step_runs_outside_the_window() {
    // Arrange: background 로 등록된 대기 단계를 사용자가 직접 요청한다 (manual 로 승격)
    let e = AiEnv::new();
    let id = registered_by_sweep(&e);
    e.set(|s| s.schedule = night_window());
    let st = e.settings.snapshot();
    e.sched.analyze_assets(&[id], None, true, &st).unwrap();

    // Act
    let out = e.run_next(&|| false);

    // Assert
    assert_eq!(out, Some(Ok(JobOutcome::Done)));
    assert_eq!(e.st(id, "describe"), "done");
}

#[test]
fn t_aisched_06_next_start_is_the_next_local_occurrence_across_offsets_and_midnight() {
    // Arrange
    let w = night_window();
    let h = 3_600_000_i64;

    // Act / Assert: UTC 22:13 -> 다음날 01:00
    assert_eq!(w.next_start_ms(T0, 0), WINDOW_START);
    // 정각이면 오늘이 아니라 다음 날 (이미 시작됨 - 시작 시각 자체는 시간대 안이라 호출되지 않는다)
    assert_eq!(w.next_start_ms(WINDOW_START, 0), WINDOW_START + 24 * h);
    // 로컬 +9h (KST): UTC 22:13 = 로컬 07:13 -> 로컬 다음날 01:00 = UTC 16:00
    assert_eq!(w.next_start_ms(T0, 9 * 60), WINDOW_START + 15 * h);
    // 로컬 -5h: UTC 22:13 = 로컬 17:13 -> 로컬 같은 날 다음날 01:00 = UTC 06:00 (다음날)
    assert_eq!(w.next_start_ms(T0, -5 * 60), WINDOW_START + 5 * h);
    // 시간대 전 새벽 00:30 로컬 -> 같은 날 01:00
    assert_eq!(w.next_start_ms(WINDOW_START - 30 * 60_000, 0), WINDOW_START);
}
