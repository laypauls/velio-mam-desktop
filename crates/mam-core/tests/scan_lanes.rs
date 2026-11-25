//! 스캔 차선 — "볼륨별 1개 + 전체 상한" (결정 E) 통합 테스트 (T-lane-01 ~ 07).
//!
//! 분류: Business rule (01 같은 볼륨 직렬, 02 다른 볼륨 동시, 04 빈 차선 우선) / Boundary (03 전체 상한) /
//! Validation·설정 (05 설정 변경 반영) / 기존 의미 유지 (06 일시정지·재개, 07 양보 후 재개).
//! 실제 SQLite + 임시 폴더 + 가짜 볼륨. 대기열을 실행기 없이 직접 집어(`claim_next`) 결정적으로 확인한다.
#![cfg(windows)]

mod common;

use std::fs;
use std::path::PathBuf;

use common::env::Env;
use mam_core::jobs::{Job, JobKind, JobOutcome};
use mam_core::VolumeKind;
use serde_json::json;

fn claim(env: &Env) -> Option<Job> {
    env.ingest.jobs().claim_next(&[JobKind::Scan]).unwrap()
}

fn finish(env: &Env, job: &Job) {
    env.ingest.jobs().finish(job, Ok(JobOutcome::Done)).unwrap();
}

/// 새 볼륨(임시 폴더) 하나에 폴더 `lib` 를 등록하고 폴더 id 를 돌려준다.
fn folder_on_new_volume(env: &Env, name: &str) -> i64 {
    let dir: PathBuf = env.vol.parent().unwrap().join(name);
    fs::create_dir_all(dir.join("lib")).unwrap();
    env.volumes.add(name, VolumeKind::Fixed, &dir);
    env.ingest.add_folder(dir.join("lib").to_str().unwrap()).unwrap().folder_id
}

/// 기본 볼륨(vol_a) 안의 하위 폴더 등록 — 같은 볼륨의 폴더가 여러 개일 때
fn folder_on_default_volume(env: &Env, rel: &str) -> i64 {
    env.write(&format!("{rel}/x.jpg"), 1);
    env.ingest.add_folder(env.path(rel).to_str().unwrap()).unwrap().folder_id
}

fn set_cap_setting(env: &Env, v: serde_json::Value) -> Result<(), mam_db::settings::SettingsError> {
    let patch = json!({ "maxConcurrentScans": v });
    let patch = patch.as_object().unwrap().clone();
    env.pool
        .write(mam_db::Priority::High, move |c| Ok(mam_db::settings::update(c, &patch)))
        .unwrap()
        .map(|_| ())
}

#[test]
fn t_lane_01_two_folders_on_the_same_volume_scan_one_at_a_time() {
    // Arrange: 같은 볼륨의 폴더 2개
    let env = Env::new();
    let a = folder_on_default_volume(&env, "lib1");
    let b = folder_on_default_volume(&env, "lib2");

    // Act / Assert: 첫 작업이 돌고 있는 동안 같은 볼륨의 두 번째는 집히지 않는다
    let first = claim(&env).expect("first scan");
    assert!(claim(&env).is_none(), "same volume must serialize");
    assert_eq!(env.ingest.jobs().running_count(JobKind::Scan), 1);
    finish(&env, &first);
    let second = claim(&env).expect("second scan after the lane is free");
    assert_ne!(first.folder_id, second.folder_id);
    assert_eq!(
        [first.folder_id, second.folder_id].into_iter().flatten().collect::<std::collections::BTreeSet<_>>(),
        [a, b].into_iter().collect()
    );
}

#[test]
fn t_lane_02_folders_on_different_volumes_scan_concurrently() {
    // Arrange
    let env = Env::new();
    folder_on_new_volume(&env, "vol_b");
    folder_on_new_volume(&env, "vol_c");

    // Act
    let first = claim(&env).expect("scan on one volume");
    let second = claim(&env).expect("scan on the other volume runs at the same time");

    // Assert
    assert_ne!(first.job_id, second.job_id);
    assert_eq!(env.ingest.jobs().running_count(JobKind::Scan), 2);
}

#[test]
fn t_lane_03_global_cap_is_honored_and_a_raised_cap_takes_effect() {
    // Arrange: 볼륨 4개, 기본 전체 상한 3
    let env = Env::new();
    for n in ["v1", "v2", "v3", "v4"] {
        folder_on_new_volume(&env, n);
    }
    assert_eq!(env.ingest.jobs().scan_cap(), 3);

    // Act / Assert: 3개까지만 동시에, 상한을 올리면 4번째가 집힌다
    let held: Vec<Job> = (0..3).map(|_| claim(&env).expect("under the cap")).collect();
    assert!(claim(&env).is_none(), "global cap of 3 reached");
    env.ingest.set_scan_concurrency(4);
    assert!(claim(&env).is_some(), "raised cap admits the fourth volume");
    assert!(claim(&env).is_none());
    assert_eq!(held.len(), 3);
}

#[test]
fn t_lane_04_a_busy_lane_is_skipped_for_a_job_on_a_free_lane() {
    // Arrange: vol_a 폴더 2개(먼저 등록) + 다른 볼륨 1개(나중 등록)
    let env = Env::new();
    folder_on_default_volume(&env, "lib1");
    folder_on_default_volume(&env, "lib2");
    let other = folder_on_new_volume(&env, "vol_b");

    // Act
    let first = claim(&env).expect("oldest job");
    let second = claim(&env).expect("skips the busy lane");

    // Assert: 두 번째 작업은 vol_a 의 두 번째 폴더가 아니라 다른 볼륨 작업이다
    assert_eq!(second.folder_id, Some(other));
    assert_ne!(first.folder_id, Some(other));
    assert!(claim(&env).is_none());
}

#[test]
fn t_lane_05_setting_change_is_applied_and_invalid_values_are_rejected() {
    // Arrange
    let env = Env::new();
    for n in ["v1", "v2", "v3"] {
        folder_on_new_volume(&env, n);
    }

    // Act 1: 설정을 1 로 → 점검이 대기열에 반영
    set_cap_setting(&env, json!(1)).unwrap();
    env.ingest.supervise_once().unwrap();

    // Assert 1
    assert_eq!(env.ingest.jobs().scan_cap(), 1);
    let first = claim(&env).expect("one scan");
    assert!(claim(&env).is_none(), "cap 1 admits a single scan");

    // Act 2: 2 로 올린다
    set_cap_setting(&env, json!(2)).unwrap();
    env.ingest.supervise_once().unwrap();
    assert!(claim(&env).is_some(), "cap 2 admits a second scan");
    assert!(claim(&env).is_none());

    // Assert 3: 잘못된 값은 거부되고 상한은 그대로
    for bad in [json!(0), json!(9), json!(-1), json!("3"), json!(2.5), json!(null)] {
        assert!(set_cap_setting(&env, bad.clone()).is_err(), "{bad} must be rejected");
    }
    env.ingest.supervise_once().unwrap();
    assert_eq!(env.ingest.jobs().scan_cap(), 2);
    // 직접 부르는 setter 도 범위 밖은 범위로 맞춘다
    env.ingest.set_scan_concurrency(100);
    assert_eq!(env.ingest.jobs().scan_cap(), 8);
    env.ingest.set_scan_concurrency(0);
    assert_eq!(env.ingest.jobs().scan_cap(), 1);
    finish(&env, &first);
}

#[test]
fn t_lane_06_paused_scans_are_not_claimed_and_resume_restores_them() {
    // Arrange
    let env = Env::new();
    folder_on_new_volume(&env, "vol_b");
    env.ingest.jobs().pause(Some(JobKind::Scan)).unwrap();

    // Act / Assert
    assert!(claim(&env).is_none(), "paused kind is not claimed");
    env.ingest.jobs().resume(Some(JobKind::Scan)).unwrap();
    assert!(claim(&env).is_some());
}

#[test]
fn t_lane_07_a_yielded_scan_frees_its_lane_and_resumes_without_a_failure() {
    // Arrange: 같은 볼륨의 폴더 2개, 첫 스캔이 양보(일시정지·취소)한다
    let env = Env::new();
    folder_on_default_volume(&env, "lib1");
    folder_on_default_volume(&env, "lib2");
    let first = claim(&env).expect("first");
    assert!(claim(&env).is_none());

    // Act
    env.ingest.jobs().finish(&first, Ok(JobOutcome::Yielded)).unwrap();

    // Assert: 차선이 비었고 양보한 작업은 실패·시도 횟수 없이 대기로 돌아왔다 (이어하기는 실행기의 체크포인트 몫)
    let again = claim(&env).expect("lane is free again");
    assert_eq!(again.job_id, first.job_id, "the yielded (older) job is picked first");
    assert_eq!(again.attempts, 0);
    assert_eq!(env.count("SELECT COUNT(*) FROM jobs WHERE status = 'failed'"), 0);
}
