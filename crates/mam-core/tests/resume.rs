//! 이어보기 (T-resume-01 ~ 30) — 실제 SQLite(V25 적용) 위의 저장 판정·조회·초기화·통계.
//!
//! 분류: Happy (01 02 18) / Boundary (03 04 05 06 07 19) / Validation (08 09 10) /
//! Business rule (11 12 13 14 15 16 17 22) / Concurrency (20) / Performance 계약 (21 쿼리 계획) / Time (02 주입 시계)

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use mam_core::SharedClock;
use mam_core::scan::IngestError;
use mam_core::userdata::{
    FolderResumeCount, RESUME_COUNT_ASSETS_SQL, RESUME_FOLDER_SCOPE_SQL, ResumePosition, ResumeSaveOutcome,
    ResumeStats, UserData,
};
use mam_db::{Clock, DbPool, SystemClock, migrate, open, settings};
use rusqlite::{Connection, params};
use serde_json::{Map, Value, json};
use tempfile::TempDir;

use ResumeSaveOutcome::{Cleared, Saved, Skipped};

struct FixedClock(AtomicI64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Fx {
    dir: TempDir,
    ud: Arc<UserData>,
    clock: Arc<FixedClock>,
}

impl Fx {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("mam.sqlite");
        let mut conn = open(&db).unwrap();
        migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&db, 2).unwrap());
        let clock = Arc::new(FixedClock(AtomicI64::new(1_000)));
        let shared: SharedClock = clock.clone();
        Self { dir, ud: Arc::new(UserData::new(pool, shared)), clock }
    }

    fn raw(&self) -> Connection {
        open(&self.dir.path().join("mam.sqlite")).unwrap()
    }

    fn count(&self, sql: &str) -> i64 {
        self.raw().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn set(&self, patch: Value) {
        let map: Map<String, Value> = patch.as_object().unwrap().clone();
        settings::update(&mut self.raw(), &map).unwrap();
    }

    fn volume(&self, id: i64) {
        self.raw()
            .execute(
                "INSERT INTO volumes (volume_id, kind, volume_key, last_seen_at) VALUES (?1, 'fixed', ?2, 0)",
                params![id, format!("vol-{id}")],
            )
            .unwrap();
    }

    fn folder(&self, folder_id: i64, volume_id: i64, path_key: &str, enabled: bool) {
        self.raw()
            .execute(
                "INSERT INTO managed_folders (folder_id, volume_id, rel_root, path_key, display_path, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?3, ?3, ?4, 0)",
                params![folder_id, volume_id, path_key, enabled],
            )
            .unwrap();
    }

    /// 자산 + (선택) 활성 위치
    fn asset(&self, id: i64, loc: Option<(i64, &str)>) {
        let c = self.raw();
        c.execute(
            "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             VALUES (?1, 'video', 1, 0, 0, 0)",
            [id],
        )
        .unwrap();
        if let Some((vol, key)) = loc {
            c.execute(
                "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at) VALUES (?1, ?2, ?3, ?3, 0)",
                params![id, vol, key],
            )
            .unwrap();
        }
    }

    /// 판정을 거치지 않고 기록을 직접 심는다 (초기화 시험용)
    fn seed(&self, id: i64) {
        self.raw()
            .execute(
                "INSERT INTO asset_resume_positions (asset_id, position_ms, duration_ms, updated_at) VALUES (?1, 60000, 600000, 1)",
                [id],
            )
            .unwrap();
    }

    fn save(&self, id: i64, pos: i64, dur: Option<i64>) -> ResumeSaveOutcome {
        self.ud.resume_save(id, pos, dur).unwrap()
    }
}

const TEN_MIN: Option<i64> = Some(600_000);

/// 관리 폴더 둘(`a`, 접두가 같은 `ab`)과 각각 자산 둘, 소속 없는 기록 하나
fn two_folders() -> Fx {
    let fx = Fx::new();
    fx.volume(1);
    fx.folder(1, 1, "a", true);
    fx.folder(2, 1, "ab", true);
    fx.asset(1, Some((1, "a/x.mp4")));
    fx.asset(2, Some((1, "a/sub/y.mp4")));
    fx.asset(3, Some((1, "ab/z.mp4")));
    fx.asset(4, Some((1, "ab/w.mp4")));
    fx.asset(5, None); // 활성 위치 없음 (소속 없음)
    for id in 1..=5 {
        fx.seed(id);
    }
    fx
}

#[test]
fn t_resume_01_save_then_get_round_trips_and_missing_is_none() {
    let fx = Fx::new();
    fx.asset(1, None);
    assert_eq!(fx.ud.resume_get(1).unwrap(), None);
    assert_eq!(fx.ud.resume_get(999).unwrap(), None, "없는 자산도 오류가 아니다");

    assert_eq!(fx.save(1, 123_456, TEN_MIN), Saved);

    let got = fx.ud.resume_get(1).unwrap();
    assert_eq!(got, Some(ResumePosition { position_ms: 123_456, duration_ms: TEN_MIN }));
    assert_eq!(serde_json::to_value(got).unwrap(), json!({"positionMs": 123_456, "durationMs": 600_000}));
    assert_eq!(serde_json::to_value(Saved).unwrap(), json!("saved"));
    assert_eq!(serde_json::to_value(Cleared).unwrap(), json!("cleared"));
    assert_eq!(serde_json::to_value(Skipped).unwrap(), json!("skipped"));
}

#[test]
fn t_resume_02_updated_at_comes_from_the_injected_clock() {
    let fx = Fx::new();
    fx.asset(1, None);
    fx.clock.0.store(777_000, Ordering::SeqCst);
    fx.save(1, 60_000, TEN_MIN);
    assert_eq!(fx.count("SELECT updated_at FROM asset_resume_positions WHERE asset_id = 1"), 777_000);
    fx.clock.0.store(888_000, Ordering::SeqCst);
    fx.save(1, 90_000, TEN_MIN);
    assert_eq!(fx.count("SELECT updated_at FROM asset_resume_positions WHERE asset_id = 1"), 888_000);
}

#[test]
fn t_resume_03_position_under_5s_clears_existing_and_5s_is_kept() {
    let fx = Fx::new();
    fx.asset(1, None);
    fx.save(1, 60_000, TEN_MIN);

    assert_eq!(fx.save(1, 4_999, TEN_MIN), Cleared);
    assert_eq!(fx.ud.resume_get(1).unwrap(), None);
    assert_eq!(fx.save(1, 5_000, TEN_MIN), Saved);
    assert_eq!(fx.save(1, 0, TEN_MIN), Cleared, "0 도 정상 입력 - 처음부터 본 것");
}

#[test]
fn t_resume_04_near_end_uses_the_larger_of_5_percent_and_15s() {
    let fx = Fx::new();
    fx.asset(1, None);
    // 600s: 5% = 30s → 570_000 이상 다 봄
    assert_eq!(fx.save(1, 569_999, TEN_MIN), Saved);
    assert_eq!(fx.save(1, 570_000, TEN_MIN), Cleared);
    assert_eq!(fx.ud.resume_get(1).unwrap(), None);
    // 최소 길이 0 + 200s: 5% = 10s < 15s → 185_000 이상 다 봄
    fx.set(json!({"resumeMinDurationSec": 0}));
    assert_eq!(fx.save(1, 184_999, Some(200_000)), Saved);
    assert_eq!(fx.save(1, 185_000, Some(200_000)), Cleared);
}

#[test]
fn t_resume_05_min_duration_180_skips_short_and_unknown_duration_and_deletes_existing() {
    let fx = Fx::new();
    fx.asset(1, None);
    fx.save(1, 60_000, TEN_MIN);

    // 길이 미상 → skipped + 기존 기록 삭제
    assert_eq!(fx.save(1, 60_000, None), Skipped);
    assert_eq!(fx.ud.resume_get(1).unwrap(), None);
    // 180_000 미만 skipped, 정확히 180_000 은 통과
    assert_eq!(fx.save(1, 60_000, Some(179_999)), Skipped);
    assert_eq!(fx.save(1, 60_000, Some(180_000)), Saved);
    // 짧은 콘텐츠가 되면 기존 기록도 지운다
    assert_eq!(fx.save(1, 60_000, Some(100_000)), Skipped);
    assert_eq!(fx.ud.resume_get(1).unwrap(), None);
    // 길이 0 이하는 길이 미상으로 본다
    assert_eq!(fx.save(1, 60_000, Some(0)), Skipped);
}

#[test]
fn t_resume_06_min_duration_0_skips_the_length_check_but_unknown_duration_still_saves() {
    let fx = Fx::new();
    fx.asset(1, None);
    fx.set(json!({"resumeMinDurationSec": 0}));

    assert_eq!(fx.save(1, 60_000, Some(10_000)), Cleared, "짧은 영상도 위치 규칙은 적용(다 봄)");
    assert_eq!(fx.save(1, 60_000, Some(300_000)), Saved);
    assert_eq!(fx.save(1, 60_000, None), Saved, "길이 미상은 위치 규칙(5초)만 적용");
    assert_eq!(fx.ud.resume_get(1).unwrap(), Some(ResumePosition { position_ms: 60_000, duration_ms: None }));
    assert_eq!(fx.save(1, 4_000, None), Cleared);
}

#[test]
fn t_resume_07_each_allowed_min_duration_applies_its_own_threshold() {
    let fx = Fx::new();
    fx.asset(1, None);
    for sec in [60u32, 180, 300, 600] {
        fx.set(json!({"resumeMinDurationSec": sec}));
        let min = i64::from(sec) * 1000;
        assert_eq!(fx.save(1, 20_000, Some(min - 1)), Skipped, "{sec}");
        assert_eq!(fx.save(1, 20_000, Some(min)), Saved, "{sec}");
    }
}

#[test]
fn t_resume_08_negative_position_is_invalid_input_and_changes_nothing() {
    let fx = Fx::new();
    fx.asset(1, None);
    fx.save(1, 60_000, TEN_MIN);

    for pos in [-1, i64::MIN] {
        let err = fx.ud.resume_save(1, pos, TEN_MIN).unwrap_err();
        assert!(matches!(err, IngestError::InvalidInput("positionMs")), "{pos}");
    }
    assert_eq!(fx.ud.resume_get(1).unwrap().unwrap().position_ms, 60_000);
}

#[test]
fn t_resume_09_unknown_asset_is_not_found_and_writes_nothing() {
    let fx = Fx::new();
    let err = fx.ud.resume_save(404, 60_000, TEN_MIN).unwrap_err();
    assert!(matches!(err, IngestError::NotFound));
    assert_eq!(fx.count("SELECT count(*) FROM asset_resume_positions"), 0);
}

#[test]
fn t_resume_10_unknown_folder_is_not_found_and_removed_in_progress_folder_too() {
    let fx = two_folders();
    fx.folder(9, 1, "gone", false); // 등록 해제 중
    for id in [404, 9] {
        let err = fx.ud.resume_clear_folder(id).unwrap_err();
        assert!(matches!(err, IngestError::NotFound), "{id}");
    }
    assert_eq!(fx.count("SELECT count(*) FROM asset_resume_positions"), 5);
}

#[test]
fn t_resume_11_disabled_returns_skipped_and_keeps_the_existing_record_then_enabling_saves_again() {
    let fx = Fx::new();
    fx.asset(1, None);
    fx.save(1, 60_000, TEN_MIN);

    fx.set(json!({"resumeEnabled": false}));
    // 게이트 OFF: 어떤 입력이어도 skipped 이고 기록은 그대로 (짧은 길이·5초 미만·다 봄 포함)
    for (pos, dur) in [(90_000, TEN_MIN), (1_000, TEN_MIN), (599_000, TEN_MIN), (60_000, None)] {
        assert_eq!(fx.save(1, pos, dur), Skipped, "{pos}");
    }
    assert_eq!(fx.ud.resume_get(1).unwrap(), Some(ResumePosition { position_ms: 60_000, duration_ms: TEN_MIN }));
    // 게이트 ON
    fx.set(json!({"resumeEnabled": true}));
    assert_eq!(fx.save(1, 90_000, TEN_MIN), Saved);
    assert_eq!(fx.ud.resume_get(1).unwrap().unwrap().position_ms, 90_000);
}

#[test]
fn t_resume_12_clear_folder_deletes_only_its_range_including_the_prefix_sibling_boundary() {
    let fx = two_folders();

    let n = fx.ud.resume_clear_folder(1).unwrap(); // `a` — `ab` 는 접두가 같지만 다른 폴더

    assert_eq!(n, 2);
    let left: Vec<i64> = {
        let c = fx.raw();
        let mut s = c.prepare("SELECT asset_id FROM asset_resume_positions ORDER BY asset_id").unwrap();
        s.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(left, [3, 4, 5], "ab 폴더와 소속 없는 기록은 남는다");
    assert_eq!(fx.ud.resume_clear_folder(1).unwrap(), 0, "멱등");
}

#[test]
fn t_resume_13_clear_folder_follows_the_active_location_not_detached_history() {
    let fx = two_folders();
    // 자산 3 은 옛 위치(a/old.mp4)가 detach 됐고 활성 위치는 ab — 폴더 a 의 초기화에 포함되면 안 된다
    fx.raw()
        .execute(
            "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, detached_at, last_seen_at)
             VALUES (3, 1, 'a/old.mp4', 'a/old.mp4', 5, 0)",
            [],
        )
        .unwrap();
    assert_eq!(fx.ud.resume_clear_folder(1).unwrap(), 2);
    assert_eq!(fx.ud.resume_get(3).unwrap().map(|p| p.position_ms), Some(60_000));
}

#[test]
fn t_resume_14_clear_folder_ignores_the_same_path_key_on_another_volume() {
    let fx = two_folders();
    fx.volume(2);
    fx.asset(6, Some((2, "a/other.mp4")));
    fx.seed(6);

    assert_eq!(fx.ud.resume_clear_folder(1).unwrap(), 2);
    assert!(fx.ud.resume_get(6).unwrap().is_some());
}

#[test]
fn t_resume_15_clear_all_removes_everything_including_orphans() {
    let fx = two_folders();
    assert_eq!(fx.ud.resume_clear_all().unwrap(), 5);
    assert_eq!(fx.count("SELECT count(*) FROM asset_resume_positions"), 0);
    assert_eq!(fx.ud.resume_clear_all().unwrap(), 0);
}

#[test]
fn t_resume_16_stats_lists_every_folder_with_zero_counts_and_total_includes_orphans() {
    let fx = two_folders();
    fx.folder(3, 1, "empty", true);
    fx.folder(4, 1, "removing", false); // 등록 해제 중 — 폴더 목록에 없다
    fx.ud.resume_clear_assets(&[3]).unwrap();

    let stats = fx.ud.resume_stats().unwrap();

    assert_eq!(
        stats,
        ResumeStats {
            folders: vec![
                FolderResumeCount { folder_id: 1, count: 2, enabled: true },
                FolderResumeCount { folder_id: 2, count: 1, enabled: true },
                FolderResumeCount { folder_id: 3, count: 0, enabled: true },
            ],
            total: 4, // 자산 1 2 4 5 — 5 는 소속 없는 기록
        }
    );
    assert_eq!(serde_json::to_value(&stats).unwrap()["folders"][2], json!({"folderId": 3, "count": 0, "enabled": true}));
}

#[test]
fn t_resume_17_stats_with_no_folders_or_records_is_empty() {
    let fx = Fx::new();
    assert_eq!(fx.ud.resume_stats().unwrap(), ResumeStats { folders: vec![], total: 0 });
}

#[test]
fn t_resume_18_count_and_clear_assets_only_touch_the_selection_and_ignore_unknown_ids() {
    let fx = two_folders();

    assert_eq!(fx.ud.resume_count_assets(&[1, 2, 404]).unwrap(), 2);
    assert_eq!(fx.ud.resume_count_assets(&[]).unwrap(), 0);
    assert_eq!(fx.ud.resume_count_assets(&[1, 1, 1]).unwrap(), 1, "중복 id 는 한 번만");
    assert_eq!(fx.ud.resume_clear_assets(&[1, 404]).unwrap(), 1);
    assert_eq!(fx.ud.resume_clear_assets(&[1]).unwrap(), 0, "멱등");
    assert_eq!(fx.count("SELECT count(*) FROM asset_resume_positions"), 4);
}

#[test]
fn t_resume_19_large_id_lists_are_batched_and_counted_without_a_cap() {
    let fx = Fx::new();
    fx.asset(1, None);
    fx.seed(1);
    // 한 청크(5000)를 넘는 선택 — 상한으로 거부하지 않고 모두 처리한다
    let ids: Vec<i64> = (1..=12_000).collect();
    assert_eq!(fx.ud.resume_count_assets(&ids).unwrap(), 1);
    assert_eq!(fx.ud.resume_clear_assets(&ids).unwrap(), 1);
}

#[test]
fn t_resume_20_repeated_upsert_is_idempotent_and_clear_then_resave_is_allowed() {
    let fx = Fx::new();
    fx.asset(1, None);
    for _ in 0..5 {
        assert_eq!(fx.save(1, 60_000, TEN_MIN), Saved);
    }
    assert_eq!(fx.count("SELECT count(*) FROM asset_resume_positions"), 1);

    fx.ud.resume_clear_assets(&[1]).unwrap();
    assert_eq!(fx.save(1, 70_000, TEN_MIN), Saved);
    assert_eq!(fx.ud.resume_get(1).unwrap().unwrap().position_ms, 70_000);
}

#[test]
fn t_resume_21_queries_use_indexes() {
    let fx = two_folders();
    let c = fx.raw();
    let plan = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> String {
        let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let rows: Vec<String> = s.query_map(p, |r| r.get::<_, String>(3)).unwrap().map(Result::unwrap).collect();
        rows.join(" | ")
    };
    let folder = plan(RESUME_FOLDER_SCOPE_SQL, &[&1i64, &"a/", &"a0"]);
    let count = plan(RESUME_COUNT_ASSETS_SQL, &[&"[1,2]"]);
    eprintln!("folder plan: {folder}
count plan: {count}");
    // 기록 테이블이 구동하고 file_locations 는 활성 자산 부분 인덱스로 찾는다 (전체 스캔 없음)
    assert!(folder.contains("uq_file_locations_active_asset"), "{folder}");
    assert!(!folder.contains("SCAN l") && !folder.contains("SCAN file_locations"), "{folder}");
    assert!(count.contains("json_each") && !count.contains("SCAN asset_resume_positions"), "{count}");
}

#[test]
fn t_resume_22_purge_table_list_includes_the_resume_table() {
    assert!(mam_core::purge::LINKED_TABLES.contains(&"asset_resume_positions"));
}

// ---- 관리 폴더별 사용/사용 안 함 (V25) — 분류: Business rule (23 24 25 26 29 30) / Validation (27) / Happy (28)

/// 폴더 `a`·`ab`(자산 1·2 / 3·4)와 기록이 없는 소속 없는 자산 5, 전부 기록 없이 시작
fn folders_without_records() -> Fx {
    let fx = Fx::new();
    fx.volume(1);
    fx.folder(1, 1, "a", true);
    fx.folder(2, 1, "ab", true);
    fx.asset(1, Some((1, "a/x.mp4")));
    fx.asset(3, Some((1, "ab/z.mp4")));
    fx.asset(5, None);
    fx
}

#[test]
fn t_resume_23_folder_off_skips_save_keeps_the_record_and_get_returns_none_until_turned_on_again() {
    // Arrange
    let fx = folders_without_records();
    assert_eq!(fx.save(1, 60_000, TEN_MIN), Saved);

    // Act
    fx.ud.resume_set_folder_enabled(1, false).unwrap();

    // Assert: 저장은 skipped(기존 기록 유지), 조회는 None 이지만 행은 남아 있다
    assert_eq!(fx.save(1, 90_000, TEN_MIN), Skipped);
    assert_eq!(fx.save(1, 1_000, TEN_MIN), Skipped, "시작 부분이어도 지우지 않는다");
    assert_eq!(fx.ud.resume_get(1).unwrap(), None);
    assert_eq!(fx.count("SELECT position_ms FROM asset_resume_positions WHERE asset_id = 1"), 60_000);

    // 다시 켜면 기록이 그대로 복원된다
    fx.ud.resume_set_folder_enabled(1, true).unwrap();
    assert_eq!(fx.ud.resume_get(1).unwrap().unwrap().position_ms, 60_000);
    assert_eq!(fx.save(1, 90_000, TEN_MIN), Saved);
}

#[test]
fn t_resume_24_folder_off_does_not_affect_another_folder_even_with_a_shared_prefix() {
    let fx = folders_without_records();
    fx.ud.resume_set_folder_enabled(1, false).unwrap(); // "a" — 접두가 같은 "ab" 는 별개

    assert_eq!(fx.save(3, 60_000, TEN_MIN), Saved);
    assert_eq!(fx.ud.resume_get(3).unwrap().unwrap().position_ms, 60_000);
}

#[test]
fn t_resume_25_asset_in_no_folder_follows_the_existing_rules_even_when_folders_are_off() {
    let fx = folders_without_records();
    fx.ud.resume_set_folder_enabled(1, false).unwrap();
    fx.ud.resume_set_folder_enabled(2, false).unwrap();

    assert_eq!(fx.save(5, 60_000, TEN_MIN), Saved, "활성 위치 없음 = 전역 규칙만");
    assert_eq!(fx.ud.resume_get(5).unwrap().unwrap().position_ms, 60_000);
}

#[test]
fn t_resume_26_global_off_wins_over_folder_on_and_get_keeps_global_semantics() {
    let fx = folders_without_records();
    assert_eq!(fx.save(1, 60_000, TEN_MIN), Saved);

    fx.set(json!({"resumeEnabled": false}));

    assert_eq!(fx.save(1, 90_000, TEN_MIN), Skipped, "폴더가 켜져 있어도 전역 끔이 우선");
    // resume_get 은 전역을 보지 않는다 (프론트가 판단) — 폴더가 켜져 있으니 기록 반환
    assert_eq!(fx.ud.resume_get(1).unwrap().unwrap().position_ms, 60_000);
}

#[test]
fn t_resume_27_set_folder_enabled_on_unknown_or_removing_folder_is_not_found() {
    let fx = folders_without_records();
    fx.folder(9, 1, "removing", false); // 등록 해제 중

    assert!(matches!(fx.ud.resume_set_folder_enabled(404, false), Err(IngestError::NotFound)));
    assert!(matches!(fx.ud.resume_set_folder_enabled(9, false), Err(IngestError::NotFound)));
    assert_eq!(fx.count("SELECT resume_enabled FROM managed_folders WHERE folder_id = 9"), 1, "변경 없음");
}

#[test]
fn t_resume_28_stats_reports_the_enabled_flag_and_set_is_idempotent() {
    let fx = folders_without_records();
    fx.ud.resume_set_folder_enabled(2, false).unwrap();
    fx.ud.resume_set_folder_enabled(2, false).unwrap();

    let stats = fx.ud.resume_stats().unwrap();

    assert_eq!(
        stats.folders,
        vec![
            FolderResumeCount { folder_id: 1, count: 0, enabled: true },
            FolderResumeCount { folder_id: 2, count: 0, enabled: false },
        ]
    );
    assert_eq!(serde_json::to_value(&stats).unwrap()["folders"][1], json!({"folderId": 2, "count": 0, "enabled": false}));
}

#[test]
fn t_resume_29_folder_off_follows_the_active_location_and_volume_not_detached_history() {
    let fx = folders_without_records();
    fx.volume(2);
    fx.asset(6, Some((2, "a/other-volume.mp4"))); // 같은 path_key, 다른 볼륨
    fx.ud.resume_set_folder_enabled(1, false).unwrap();
    // 자산 3: 이전에 폴더 a 에 있었으나 분리됨(detached) — 현재는 ab 소속(켬)
    fx.raw()
        .execute(
            "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at, detached_at) VALUES (3, 1, 'a/old.mp4', 'a/old.mp4', 0, 5)",
            [],
        )
        .unwrap();

    assert_eq!(fx.save(6, 60_000, TEN_MIN), Saved, "다른 볼륨은 영향 없음");
    assert_eq!(fx.save(3, 60_000, TEN_MIN), Saved, "분리된 이력이 아니라 활성 위치 기준");
}

#[test]
fn t_resume_30_folder_lookup_uses_indexes() {
    let fx = folders_without_records();
    let c = fx.raw();
    let plan = |sql: &str| -> String {
        let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let rows: Vec<String> = s.query_map([1i64], |r| r.get::<_, String>(3)).unwrap().map(Result::unwrap).collect();
        rows.join(" | ")
    };
    let loc = plan("SELECT volume_id, path_key FROM file_locations WHERE asset_id = ?1 AND detached_at IS NULL");
    eprintln!("loc plan: {loc}");
    assert!(loc.contains("USING INDEX") || loc.contains("USING COVERING INDEX"), "{loc}");
}
