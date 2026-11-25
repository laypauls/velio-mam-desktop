//! 상태 표시줄 요약에서 숨김 폴더의 스캔 정보 제외 (T-hidescan-01 ~ 07).
//!
//! 분류: Happy (03 보이는 폴더만) / Business rule (01 숨김만, 02 동시 스캔 순서 무관, 04~06 실패 건수, 07 다른 종류) /
//! Boundary (05 folder_id 없는 작업) / Time (06 숨김 해제 즉시 반영).
//! 실제 SQLite(마이그레이션 적용) + 임시 디렉터리 볼륨. 스캔 진행 칸은 주입 시계로 직접 채운다.

mod common;

use common::env::Env;
use mam_core::ingest::JobSummary;
use mam_core::jobs::JobKind;
use mam_core::scan::MediaTypeSet;

fn add(env: &Env, dir: &str) -> i64 {
    env.write(&format!("{dir}/a.jpg"), 1);
    env.ingest
        .add_folder_with_types(env.path(dir).to_str().unwrap(), MediaTypeSet::ALL)
        .unwrap()
        .folder_id
}

fn start_scan(env: &Env, id: i64, path: &str) {
    env.ingest.env().progress.start(id, path.into(), 7, env.clock_now());
}

fn insert_job(env: &Env, kind: &str, folder_id: Option<i64>, status: &str) {
    let (kind, status) = (kind.to_owned(), status.to_owned());
    env.pool
        .write(mam_db::Priority::High, move |c| {
            c.execute(
                "INSERT INTO jobs (kind, folder_id, status, created_at, updated_at) VALUES (?1, ?2, ?3, 0, 0)",
                rusqlite::params![kind, folder_id, status],
            )?;
            Ok(())
        })
        .unwrap();
}

fn summary(env: &Env) -> JobSummary {
    env.ingest.summary().unwrap()
}

fn scan_failed(env: &Env) -> u64 {
    summary(env).counts[&JobKind::Scan].failed
}

#[test]
fn t_hidescan_01_hidden_only_scan_is_not_reported() {
    // Arrange
    let env = Env::new();
    let h = add(&env, "h");
    env.ingest.set_folder_hidden(h, true).unwrap();
    start_scan(&env, h, "h");

    // Act / Assert
    assert_eq!(summary(&env).scan, None);
}

#[test]
fn t_hidescan_02_visible_scan_is_returned_regardless_of_order() {
    // Arrange
    let env = Env::new();
    let a = add(&env, "a");
    let h = add(&env, "h");
    env.ingest.set_folder_hidden(h, true).unwrap();

    // Act / Assert: 숨김이 먼저 시작
    start_scan(&env, h, "h");
    start_scan(&env, a, "a");
    assert_eq!(summary(&env).scan.unwrap().folder_id, a);

    // 보이는 것이 먼저 시작
    env.ingest.env().progress.clear(h);
    env.ingest.env().progress.clear(a);
    start_scan(&env, a, "a");
    start_scan(&env, h, "h");
    assert_eq!(summary(&env).scan.unwrap().folder_id, a);
}

#[test]
fn t_hidescan_03_visible_only_scan_is_unchanged_and_idle_is_none() {
    // Arrange
    let env = Env::new();
    let a = add(&env, "a");
    assert_eq!(summary(&env).scan, None);

    // Act
    start_scan(&env, a, "a");

    // Assert
    let s = summary(&env).scan.unwrap();
    assert_eq!((s.folder_id, s.processed, s.total), (a, 7, None));
}

#[test]
fn t_hidescan_04_failed_scan_count_excludes_hidden_but_keeps_visible_and_null_folder() {
    // Arrange
    let env = Env::new();
    let a = add(&env, "a");
    let h = add(&env, "h");
    env.ingest.set_folder_hidden(h, true).unwrap();
    insert_job(&env, "scan", Some(a), "failed");
    insert_job(&env, "scan", Some(h), "failed");
    insert_job(&env, "scan", None, "failed");

    // Act / Assert: 보이는 폴더 1 + folder_id 없음 1 (숨김 1 제외)
    assert_eq!(scan_failed(&env), 2);
}

#[test]
fn t_hidescan_05_hidden_scan_jobs_are_excluded_in_every_status() {
    // Arrange
    let env = Env::new();
    let h = add(&env, "h");
    env.ingest.set_folder_hidden(h, true).unwrap();
    insert_job(&env, "scan", Some(h), "running");
    insert_job(&env, "scan", Some(h), "failed");

    // Act
    let c = summary(&env).counts[&JobKind::Scan].clone();

    // Assert
    assert_eq!((c.pending, c.running, c.failed), (0, 0, 0));
}

#[test]
fn t_hidescan_06_unhiding_counts_the_folder_again() {
    // Arrange
    let env = Env::new();
    let h = add(&env, "h");
    insert_job(&env, "scan", Some(h), "failed");
    env.ingest.set_folder_hidden(h, true).unwrap();
    assert_eq!(scan_failed(&env), 0);
    start_scan(&env, h, "h");
    assert_eq!(summary(&env).scan, None);

    // Act
    env.ingest.set_folder_hidden(h, false).unwrap();

    // Assert
    assert_eq!(scan_failed(&env), 1);
    assert_eq!(summary(&env).scan.unwrap().folder_id, h);
}

#[test]
fn t_hidescan_07_other_kinds_are_not_filtered_by_hidden_folder() {
    // Arrange: 숨김 폴더에 걸린 scan 외 작업(folder_id 가 있어도)은 그대로 센다
    let env = Env::new();
    let h = add(&env, "h");
    env.ingest.set_folder_hidden(h, true).unwrap();
    insert_job(&env, "meta", Some(h), "failed");
    insert_job(&env, "thumb", None, "failed");

    // Act
    let s = summary(&env);

    // Assert
    assert_eq!(s.counts[&JobKind::Meta].failed, 1);
    assert_eq!(s.counts[&JobKind::Thumb].failed, 1);
}

#[test]
fn t_hidescan_08_counts_query_plan_uses_indexes_for_both_halves() {
    // Arrange: db.md §4 - 새 쿼리는 EXPLAIN QUERY PLAN 으로 인덱스 사용을 확인한다
    let env = Env::new();

    // Act
    let plan: Vec<String> = env.read(|c| {
        let mut stmt = c
            .prepare(
                "EXPLAIN QUERY PLAN SELECT kind, status, COUNT(*) FROM jobs WHERE kind <> 'scan' GROUP BY kind, status
                 UNION ALL
                 SELECT kind, status, COUNT(*) FROM jobs
                 WHERE kind = 'scan'
                   AND (folder_id IS NULL
                        OR folder_id NOT IN (SELECT folder_id FROM managed_folders WHERE hidden = 1))
                 GROUP BY kind, status",
            )
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    });

    // Assert
    println!("{plan:#?}");
    assert!(plan.iter().any(|p| p.contains("idx_jobs_kind_status")), "{plan:?}");
}
