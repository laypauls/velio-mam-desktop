//! 재생 구간 (T-ranges-01 ~ 13) — 정규화 순수 함수 + 실제 SQLite(V13 적용) 위의 get/set/summary.
//!
//! 분류: Happy (01 06 11) / Boundary (02 04 05 11) / Validation (03 07) / Business rule (05 09 10) /
//! External·원자성 (08) / Concurrency (12) / Performance 계약 (13 쿼리 계획)

use std::sync::Arc;

use mam_core::SharedClock;
use mam_core::scan::IngestError;
use mam_core::userdata::{
    MAX_PLAY_RANGES, PLAY_RANGE_SUMMARY_SQL, PLAY_RANGES_GET_SQL, PlayRange, PlayRangeSummary,
    UserData, min_range_len_ms, normalize_ranges,
};
use mam_db::{DbPool, SystemClock, migrate, open};
use rusqlite::{Connection, params};
use tempfile::TempDir;

fn r(s: i64, e: i64) -> PlayRange {
    PlayRange {
        start_ms: s,
        end_ms: e,
    }
}

fn pairs(v: &[PlayRange]) -> Vec<(i64, i64)> {
    v.iter().map(|x| (x.start_ms, x.end_ms)).collect()
}

fn is_invalid_ranges(e: &IngestError) -> bool {
    matches!(e, IngestError::InvalidInput("ranges"))
}

// ---------- normalize_ranges (순수) ----------

#[test]
fn t_ranges_01_normalize_sorts_and_merges_overlapping_and_touching() {
    // Arrange: 역순 + 겹침 + 맞닿음 + 포함 + 떨어진 구간
    let input = [
        r(5000, 6000),
        r(1000, 2000),
        r(1500, 2500),
        r(2500, 3000),
        r(1200, 1300),
        r(8000, 9000),
    ];

    // Act
    let out = normalize_ranges(&input, None, None).unwrap();

    // Assert
    assert_eq!(pairs(&out), [(1000, 3000), (5000, 6000), (8000, 9000)]);
    // 멱등: 정규화 결과를 다시 정규화해도 같다
    assert_eq!(normalize_ranges(&out, None, None).unwrap(), out);
}

#[test]
fn t_ranges_02_normalize_empty_and_count_limit() {
    // Arrange
    let ok: Vec<PlayRange> = (0..MAX_PLAY_RANGES as i64)
        .map(|i| r(i * 1000, i * 1000 + 500))
        .collect();
    let over: Vec<PlayRange> = (0..=MAX_PLAY_RANGES as i64)
        .map(|i| r(i * 1000, i * 1000 + 500))
        .collect();

    // Act + Assert
    assert!(normalize_ranges(&[], None, None).unwrap().is_empty());
    assert_eq!(
        normalize_ranges(&ok, None, None).unwrap().len(),
        MAX_PLAY_RANGES
    );
    assert!(is_invalid_ranges(
        &normalize_ranges(&over, None, None).unwrap_err()
    ));
}

#[test]
fn t_ranges_03_normalize_rejects_negative_start_and_non_positive_length() {
    for bad in [r(-1, 100), r(100, 100), r(100, 50), r(i64::MIN, 0)] {
        // 유효한 구간과 섞여 있어도 전체 거부
        let err = normalize_ranges(&[r(0, 500), bad], None, None).unwrap_err();
        assert!(is_invalid_ranges(&err), "{bad:?}");
    }
}

#[test]
fn t_ranges_04_normalize_clamps_to_duration_and_drops_out_of_range() {
    // Arrange
    let input = [r(0, 500), r(900, 1500), r(1000, 2000), r(5000, 6000)];

    // Act
    let with = normalize_ranges(&input, Some(1000), None).unwrap();
    let none = normalize_ranges(&input, None, None).unwrap();
    let zero = normalize_ranges(&input, Some(0), None).unwrap(); // 길이 0 = 모름

    // Assert: start >= duration 은 버리고, 걸친 구간은 duration 으로 자른다
    assert_eq!(pairs(&with), [(0, 500), (900, 1000)]);
    assert_eq!(pairs(&none), [(0, 500), (900, 2000), (5000, 6000)]);
    assert_eq!(zero, none);
    // 전부 길이 밖이면 빈 결과(= 모두 삭제)
    assert!(
        normalize_ranges(&[r(2000, 3000)], Some(1000), None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn t_ranges_05_normalize_enforces_min_length_from_fps_or_100ms() {
    // fps 30 -> 33ms 경계
    assert_eq!(min_range_len_ms(Some(30.0)), 33);
    assert!(normalize_ranges(&[r(0, 33)], None, Some(30.0)).is_ok());
    assert!(is_invalid_ranges(
        &normalize_ranges(&[r(0, 32)], None, Some(30.0)).unwrap_err()
    ));
    // fps 없음/0/NaN/무한/음수 -> 100ms 경계
    for fps in [
        None,
        Some(0.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
        Some(-24.0),
    ] {
        assert_eq!(min_range_len_ms(fps), 100, "{fps:?}");
        assert!(normalize_ranges(&[r(0, 100)], None, fps).is_ok(), "{fps:?}");
        assert!(
            is_invalid_ranges(&normalize_ranges(&[r(0, 99)], None, fps).unwrap_err()),
            "{fps:?}"
        );
    }
    // 아주 큰 fps 도 최소 1ms
    assert_eq!(min_range_len_ms(Some(1.0e9)), 1);
    // 병합 뒤 길이로 검사한다: 짧은 두 조각이 맞닿아 충분히 길어지면 통과
    assert_eq!(
        pairs(&normalize_ranges(&[r(0, 60), r(60, 120)], None, None).unwrap()),
        [(0, 120)]
    );
}

// ---------- DB ----------

struct Fx {
    dir: TempDir,
    ud: Arc<UserData>,
}

impl Fx {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let db = dir.path().join("mam.sqlite");
        let mut conn = open(&db).unwrap();
        migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&db, 2).unwrap());
        let clock: SharedClock = Arc::new(SystemClock);
        Self {
            dir,
            ud: Arc::new(UserData::new(pool, clock)),
        }
    }

    fn raw(&self) -> Connection {
        open(&self.dir.path().join("mam.sqlite")).unwrap()
    }

    fn asset(&self, id: i64, media: &str, status: &str, duration: Option<i64>, fps: Option<f64>) {
        let c = self.raw();
        c.execute(
            "INSERT INTO media_assets (asset_id, media_type, status, file_size, modified_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, 10, 0, 0, 0)",
            params![id, media, status],
        )
        .unwrap();
        if duration.is_some() || fps.is_some() {
            c.execute(
                "INSERT INTO media_metadata (asset_id, duration_ms, fps) VALUES (?1, ?2, ?3)",
                params![id, duration, fps],
            )
            .unwrap();
        }
    }

    fn video(&self, id: i64) {
        self.asset(id, "video", "active", Some(60_000), Some(25.0));
    }

    fn count(&self, sql: &str) -> i64 {
        self.raw().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

#[test]
fn t_ranges_06_set_replaces_all_and_get_matches_normalized_result_idempotently() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    fx.video(2);
    fx.ud.play_ranges_set(2, &[r(100, 900)]).unwrap();

    // Act
    let saved = fx
        .ud
        .play_ranges_set(1, &[r(5000, 6000), r(1000, 2000), r(1500, 2500)])
        .unwrap();
    let again = fx
        .ud
        .play_ranges_set(1, &[r(5000, 6000), r(1000, 2000), r(1500, 2500)])
        .unwrap();
    let got = fx.ud.play_ranges_get(1).unwrap();

    // Assert
    assert_eq!(pairs(&saved), [(1000, 2500), (5000, 6000)]);
    assert_eq!(again, saved);
    assert_eq!(got, saved);
    // 전체 교체: 새 목록에 없는 구간은 사라진다
    fx.ud.play_ranges_set(1, &[r(7000, 8000)]).unwrap();
    assert_eq!(pairs(&fx.ud.play_ranges_get(1).unwrap()), [(7000, 8000)]);
    // 다른 자산은 영향 없음
    assert_eq!(pairs(&fx.ud.play_ranges_get(2).unwrap()), [(100, 900)]);
    // 빈 목록 = 모두 삭제
    assert!(fx.ud.play_ranges_set(1, &[]).unwrap().is_empty());
    assert!(fx.ud.play_ranges_get(1).unwrap().is_empty());
    // 길이(60s)·fps(25 -> 40ms) 가 적용된다
    assert_eq!(
        pairs(&fx.ud.play_ranges_set(1, &[r(59_000, 90_000)]).unwrap()),
        [(59_000, 60_000)]
    );
    assert!(is_invalid_ranges(
        &fx.ud.play_ranges_set(1, &[r(0, 39)]).unwrap_err()
    ));
    assert!(fx.ud.play_ranges_set(1, &[r(0, 40)]).is_ok());
}

#[test]
fn t_ranges_07_set_rejects_image_missing_and_non_active_assets() {
    // Arrange
    let fx = Fx::new();
    fx.asset(1, "image", "active", None, None);
    fx.asset(2, "video", "superseded", Some(5000), None);
    fx.asset(3, "video", "deleted", Some(5000), None);
    fx.asset(4, "audio", "active", Some(5000), None);

    // Act + Assert
    assert!(matches!(
        fx.ud.play_ranges_set(1, &[r(0, 1000)]),
        Err(IngestError::InvalidInput("mediaType"))
    ));
    assert!(matches!(
        fx.ud.play_ranges_set(99, &[r(0, 1000)]),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        fx.ud.play_ranges_set(2, &[r(0, 1000)]),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        fx.ud.play_ranges_set(3, &[r(0, 1000)]),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        fx.ud.play_ranges_get(99),
        Err(IngestError::NotFound)
    ));
    // 음성은 허용 (D3)
    assert_eq!(
        pairs(&fx.ud.play_ranges_set(4, &[r(0, 1000)]).unwrap()),
        [(0, 1000)]
    );
    // 실패한 호출은 아무것도 쓰지 않았다
    assert_eq!(
        fx.count("SELECT count(*) FROM asset_play_ranges WHERE asset_id <> 4"),
        0
    );
}

#[test]
fn t_ranges_08_failed_set_keeps_previous_ranges() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    fx.ud
        .play_ranges_set(1, &[r(1000, 2000), r(3000, 4000)])
        .unwrap();
    let before = fx.ud.play_ranges_get(1).unwrap();

    // Act: 잘못된 구간 / 1001개 / 최소 길이 미만
    let too_many: Vec<PlayRange> = (0..=MAX_PLAY_RANGES as i64)
        .map(|i| r(i * 10, i * 10 + 5))
        .collect();
    for bad in [vec![r(10, 5)], vec![r(-1, 5)], too_many, vec![r(0, 1)]] {
        let err = fx.ud.play_ranges_set(1, &bad).unwrap_err();
        assert!(is_invalid_ranges(&err));
    }

    // Assert
    assert_eq!(fx.ud.play_ranges_get(1).unwrap(), before);
}

#[test]
fn t_ranges_09_set_does_not_touch_revision_or_outbox() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    let rev = "SELECT revision FROM media_assets WHERE asset_id = 1";
    let outbox = "SELECT count(*) FROM search_outbox";
    let (rev0, out0) = (fx.count(rev), fx.count(outbox));

    // Act
    fx.ud.play_ranges_set(1, &[r(0, 1000)]).unwrap();
    fx.ud.play_ranges_set(1, &[]).unwrap();

    // Assert
    assert_eq!((fx.count(rev), fx.count(outbox)), (rev0, out0));
}

#[test]
fn t_ranges_10_asset_delete_cascades_to_ranges() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    fx.video(2);
    fx.ud.play_ranges_set(1, &[r(0, 1000)]).unwrap();
    fx.ud.play_ranges_set(2, &[r(0, 1000)]).unwrap();

    // Act
    fx.raw()
        .execute_batch("PRAGMA foreign_keys = ON; DELETE FROM media_assets WHERE asset_id = 1;")
        .unwrap();

    // Assert
    assert_eq!(
        fx.count("SELECT count(*) FROM asset_play_ranges WHERE asset_id = 1"),
        0
    );
    assert_eq!(
        fx.count("SELECT count(*) FROM asset_play_ranges WHERE asset_id = 2"),
        1
    );
}

#[test]
fn t_ranges_11_summary_is_sparse_scoped_to_collection_and_not_found_for_missing() {
    // Arrange: 1,2,3 이 컬렉션 A, 3,4 가 컬렉션 B. 구간은 1(2개), 3(1개), 4(1개)
    let fx = Fx::new();
    for id in 1..=4 {
        fx.video(id);
    }
    let a = fx.ud.create_collection("a").unwrap().collection_id;
    let b = fx.ud.create_collection("b").unwrap().collection_id;
    let empty = fx.ud.create_collection("empty").unwrap().collection_id;
    fx.ud.add_to_collection(a, &[1, 2, 3]).unwrap();
    fx.ud.add_to_collection(b, &[3, 4]).unwrap();
    fx.ud
        .play_ranges_set(1, &[r(0, 1000), r(5000, 5500)])
        .unwrap();
    fx.ud.play_ranges_set(3, &[r(100, 400)]).unwrap();
    fx.ud.play_ranges_set(4, &[r(0, 999)]).unwrap();

    // Act
    let sa = fx.ud.collection_play_range_summary(a).unwrap();
    let sb = fx.ud.collection_play_range_summary(b).unwrap();
    let se = fx.ud.collection_play_range_summary(empty).unwrap();

    // Assert: 구간이 있는 항목만(2 는 없음), 다른 컬렉션의 자산(4)은 제외
    let s = |asset_id, range_count, total_ms| PlayRangeSummary {
        asset_id,
        range_count,
        total_ms,
    };
    assert_eq!(sa, [s(1, 2, 1500), s(3, 1, 300)]);
    assert_eq!(sb, [s(3, 1, 300), s(4, 1, 999)]);
    assert!(se.is_empty());
    assert!(matches!(
        fx.ud.collection_play_range_summary(999),
        Err(IngestError::NotFound)
    ));
    // 구간 삭제 후에는 요약에서 사라진다
    fx.ud.play_ranges_set(1, &[]).unwrap();
    assert_eq!(
        fx.ud.collection_play_range_summary(a).unwrap(),
        [s(3, 1, 300)]
    );
}

#[test]
fn t_ranges_12_concurrent_sets_never_interleave() {
    // Arrange
    let fx = Fx::new();
    fx.video(1);
    let set_a: Vec<PlayRange> = (0..50).map(|i| r(i * 1000, i * 1000 + 500)).collect();
    let set_b: Vec<PlayRange> = (0..30).map(|i| r(i * 1000 + 100, i * 1000 + 900)).collect();
    let expect_a = normalize_ranges(&set_a, Some(60_000), Some(25.0)).unwrap();
    let expect_b = normalize_ranges(&set_b, Some(60_000), Some(25.0)).unwrap();

    // Act
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let ud = Arc::clone(&fx.ud);
            let set = if i % 2 == 0 {
                set_a.clone()
            } else {
                set_b.clone()
            };
            std::thread::spawn(move || ud.play_ranges_set(1, &set).unwrap())
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    // Assert: 마지막 승자의 완전한 결과 하나 (섞이지 않음)
    let got = fx.ud.play_ranges_get(1).unwrap();
    assert!(
        got == expect_a || got == expect_b,
        "mixed result: {} rows",
        got.len()
    );
}

#[test]
fn t_ranges_13_query_plans_use_primary_keys() {
    // Arrange
    let fx = Fx::new();
    let c = fx.raw();
    let plan = |sql: &str| -> String {
        let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        stmt.query_map(params![1], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ")
    };

    // Act
    let get = plan(PLAY_RANGES_GET_SQL);
    let summary = plan(PLAY_RANGE_SUMMARY_SQL);

    // Assert
    assert!(
        get.contains("SEARCH") && get.contains("PRIMARY KEY"),
        "{get}"
    );
    assert!(!get.contains("TEMP B-TREE"), "{get}");
    assert!(summary.contains("SCAN r"), "{summary}");
    assert!(
        summary.contains("SEARCH x USING")
            && summary.contains("sqlite_autoindex_collection_assets"),
        "{summary}"
    ); // PK 자동 인덱스
}

// 분류: Boundary / Validation / Regression (길이로 자르기 때문에만 짧아진 구간이 전체 저장을 거부하던 결함)
#[test]
fn t_ranges_14_range_shortened_only_by_duration_clamp_is_dropped_not_rejected() {
    // Arrange: 길이 10_000, 최소 길이 100(fps 없음)
    let d = Some(10_000);
    // Act / Assert: 끝이 길이에 걸쳐 50ms 만 남는 구간은 버리고 나머지는 저장된다
    let out = normalize_ranges(&[r(0, 500), r(9_950, 12_000)], d, None).unwrap();
    assert_eq!(pairs(&out), [(0, 500)]);
    // 길이에서 정확히 최소 길이만큼 남으면 유지 (경계)
    assert_eq!(
        pairs(&normalize_ranges(&[r(9_900, 12_000)], d, None).unwrap()),
        [(9_900, 10_000)]
    );
    // 99ms 남으면 버림 (경계 바로 아래) - 결과가 비어도 오류가 아니다
    assert!(
        normalize_ranges(&[r(9_901, 12_000)], d, None)
            .unwrap()
            .is_empty()
    );
    // 병합 뒤 판정: 자르기 전 병합 길이가 충분하면 버린다, 다른 구간과 맞닿아 충분히 길면 유지한다
    assert_eq!(
        pairs(&normalize_ranges(&[r(9_000, 9_980), r(9_950, 10_500)], d, None).unwrap()),
        [(9_000, 10_000)]
    );
}

// 분류: Validation / Business rule (입력 자체가 너무 짧은 구간은 자르기와 무관하게 여전히 invalid_input)
#[test]
fn t_ranges_15_explicit_too_short_range_is_still_rejected_even_with_a_duration() {
    let d = Some(10_000);
    // 길이 안쪽에서 입력 자체가 99ms
    assert!(is_invalid_ranges(
        &normalize_ranges(&[r(0, 500), r(2_000, 2_099)], d, None).unwrap_err()
    ));
    // 자르기 전에도 짧은 구간(길이에 걸쳐 있어도): 자르기 때문이 아니므로 거부
    assert!(is_invalid_ranges(
        &normalize_ranges(&[r(9_980, 9_999)], d, None).unwrap_err()
    ));
    assert!(is_invalid_ranges(
        &normalize_ranges(&[r(9_990, 10_000)], d, None).unwrap_err()
    ));
}
