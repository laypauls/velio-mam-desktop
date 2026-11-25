//! 장면 컷 코덱·순수 함수·손상 BLOB (T-scene-04, 05, 06, 07 일부).
//!
//! 분류: Boundary (0·1·20,000개, 시작·끝 0.5초, 최소 간격) / Validation (길이·버전·정렬·상한) /
//! External (손상 BLOB 이 읽기를 막지 않음) / Business rule (자동 컷 선택·허용 오차 표)

mod common;
mod scene_support;

use std::sync::{Arc, Mutex};

use mam_db::scene_cuts::{
    self, CODEC_MAX_CUTS, DecodeError, MAX_AUTO_CUTS, decode, encode, half_frame_ms, select_auto,
    tolerance_ms,
};
use scene_support::{NOW, add_video};

#[test]
fn t_scene_04_codec_round_trips_zero_one_and_max_cuts() {
    // Arrange
    let max: Vec<i64> = (0..CODEC_MAX_CUTS as i64).map(|i| i * 3).collect();

    // Act / Assert
    assert_eq!(encode(&[]).unwrap(), None);
    assert_eq!(decode(None).unwrap(), Vec::<i64>::new());
    let one = encode(&[258]).unwrap().unwrap();
    assert_eq!(one, vec![0x01, 2, 1, 0, 0], "version byte + u32 LE");
    assert_eq!(decode(Some(&one)).unwrap(), vec![258]);
    let big = encode(&max).unwrap().unwrap();
    assert_eq!(big.len(), 1 + 4 * CODEC_MAX_CUTS);
    assert_eq!(decode(Some(&big)).unwrap(), max);
    assert_eq!(
        decode(Some(&encode(&[0, i64::from(u32::MAX)]).unwrap().unwrap())).unwrap(),
        vec![0, i64::from(u32::MAX)]
    );
}

#[test]
fn t_scene_04_codec_rejects_oversize_unordered_and_out_of_range_input() {
    // Arrange
    let over: Vec<i64> = (0..=CODEC_MAX_CUTS as i64).collect();

    // Act / Assert: encode
    assert_eq!(encode(&over), Err(DecodeError::TooMany));
    assert_eq!(encode(&[5, 5]), Err(DecodeError::NotAscending));
    assert_eq!(encode(&[5, 4]), Err(DecodeError::NotAscending));
    assert_eq!(encode(&[-1]), Err(DecodeError::OutOfRange));
    assert_eq!(
        encode(&[i64::from(u32::MAX) + 1]),
        Err(DecodeError::OutOfRange)
    );
}

#[test]
fn t_scene_04_decode_rejects_bad_length_version_order_and_oversize() {
    // Arrange
    let good = encode(&[10, 20]).unwrap().unwrap();
    let mut trailing = good.clone();
    trailing.push(0);
    let mut wrong_version = good.clone();
    wrong_version[0] = 2;
    let mut duplicate = vec![1u8];
    duplicate.extend_from_slice(&7u32.to_le_bytes());
    duplicate.extend_from_slice(&7u32.to_le_bytes());
    let mut descending = vec![1u8];
    descending.extend_from_slice(&9u32.to_le_bytes());
    descending.extend_from_slice(&3u32.to_le_bytes());
    let mut oversize = vec![1u8];
    for i in 0..=CODEC_MAX_CUTS as u32 {
        oversize.extend_from_slice(&i.to_le_bytes());
    }

    // Act / Assert
    assert_eq!(decode(Some(&[])), Err(DecodeError::BadLength));
    assert_eq!(decode(Some(&trailing)), Err(DecodeError::BadLength));
    assert_eq!(
        decode(Some(&good[..good.len() - 1])),
        Err(DecodeError::BadLength)
    );
    assert_eq!(
        decode(Some(&wrong_version)),
        Err(DecodeError::UnknownVersion)
    );
    assert_eq!(decode(Some(&duplicate)), Err(DecodeError::NotAscending));
    assert_eq!(decode(Some(&descending)), Err(DecodeError::NotAscending));
    assert_eq!(decode(Some(&oversize)), Err(DecodeError::TooMany));
    assert_eq!(
        decode(Some(&[1])).unwrap(),
        Vec::<i64>::new(),
        "version only = empty list"
    );
}

#[test]
fn t_scene_06_select_auto_drops_the_first_and_last_half_second() {
    // Arrange: 길이 10,000ms
    let raw = [(499, 1.0), (500, 1.0), (9500, 1.0), (9501, 1.0), (0, 1.0)];

    // Act / Assert
    assert_eq!(select_auto(&raw, Some(10_000)), vec![500, 9500]);
    // 길이를 모르면(None·0·음수) 끝 필터 없음, 시작 필터만
    let tail = [(499, 1.0), (500, 1.0), (9999, 1.0)];
    assert_eq!(select_auto(&tail, Some(10_000)), vec![500]);
    for unknown in [None, Some(0), Some(-5)] {
        assert_eq!(select_auto(&tail, unknown), vec![500, 9999], "{unknown:?}");
    }
}

#[test]
fn t_scene_06_select_auto_keeps_the_highest_score_within_the_one_second_gap() {
    // Arrange / Act / Assert
    // 999ms 차이는 하나로, 1000ms 차이는 둘 다 남긴다
    assert_eq!(
        select_auto(&[(1000, 0.5), (1999, 0.9), (2999, 0.4)], None),
        vec![1999, 2999]
    );
    // 연쇄 0.5s 입력 (1000-1500-2000): 가운데 최고 점수만 남고 양쪽은 모두 버려진다
    assert_eq!(
        select_auto(&[(1000, 0.5), (1500, 0.9), (2000, 0.5)], None),
        vec![1500]
    );
    // 연쇄 0.6s 입력: 점수 순 채택 후 1000·2200 만 남는다
    assert_eq!(
        select_auto(&[(1000, 0.5), (1600, 0.6), (2200, 0.7), (2800, 0.4)], None),
        vec![1000, 2200]
    );
    // 동점이면 시각이 이른 쪽
    assert_eq!(select_auto(&[(1500, 0.5), (1000, 0.5)], None), vec![1000]);
    // 입력 순서와 무관하게 오름차순 출력, 같은 시각 중복은 하나
    assert_eq!(
        select_auto(&[(5000, 1.0), (3000, 1.0), (5000, 0.2)], None),
        vec![3000, 5000]
    );
    // 비유한 점수 버림
    assert_eq!(
        select_auto(
            &[(1000, f32::NAN), (3000, f32::INFINITY), (5000, 1.0)],
            None
        ),
        vec![5000]
    );
    assert!(select_auto(&[], Some(10_000)).is_empty());
}

#[test]
fn t_scene_06_select_auto_caps_at_20000_keeping_the_top_scores() {
    // Arrange: 25,000 개, 시각이 늦을수록 점수가 높다
    let raw: Vec<(i64, f32)> = (0..25_000).map(|i| (500 + i * 1000, i as f32)).collect();

    // Act
    let out = select_auto(&raw, None);

    // Assert: 점수 상위 2만 = 뒤쪽 2만 개, 시각 오름차순
    assert_eq!(out.len(), MAX_AUTO_CUTS);
    assert_eq!(out[0], 500 + 5000 * 1000);
    assert_eq!(*out.last().unwrap(), 500 + 24_999 * 1000);
    assert!(out.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn t_scene_07_tolerance_and_half_frame_tables() {
    // Arrange / Act / Assert: 유효 fps
    assert_eq!(tolerance_ms(Some(30.0)), 33);
    assert_eq!(tolerance_ms(Some(25.0)), 40);
    assert_eq!(tolerance_ms(Some(29.97)), 33);
    assert_eq!(tolerance_ms(Some(1_000_000.0)), 1, "하한 1ms");
    assert_eq!(half_frame_ms(Some(30.0)), 17);
    assert_eq!(half_frame_ms(Some(25.0)), 20);
    assert_eq!(half_frame_ms(Some(1_000_000.0)), 1);
    // 없음·0·음수·NaN·무한대는 기본값 (D6 100ms, D8 20ms)
    for bad in [
        None,
        Some(0.0),
        Some(-24.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
        Some(f64::NEG_INFINITY),
    ] {
        assert_eq!(tolerance_ms(bad), 100, "{bad:?}");
        assert_eq!(half_frame_ms(bad), 20, "{bad:?}");
    }
}

// --- T-scene-05: 손상 BLOB -------------------------------------------------

/// 경고 이벤트(필드 이름 → 렌더된 값)를 모으는 최소 구독자 (tracing-subscriber 를 쓰지 않는다)
#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<Vec<Field>>>>);

type Field = (String, String);

struct Fields(Vec<(String, String)>);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_owned(), format!("{value:?}")));
    }
}

impl tracing::Subscriber for Warnings {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() == tracing::Level::WARN {
            let mut f = Fields(Vec::new());
            event.record(&mut f);
            self.0.lock().unwrap().push(f.0);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn t_scene_05_corrupt_blob_reads_as_empty_with_a_value_free_warning_and_next_write_heals_it() {
    // Arrange: auto_cuts 손상(역순), user_cuts 정상
    let db = common::migrated_db();
    add_video(&db.conn, 1, Some(60_000), Some(25.0));
    let mut corrupt = vec![1u8];
    corrupt.extend_from_slice(&0xDEAD_u32.to_le_bytes());
    corrupt.extend_from_slice(&0xBEEF_u32.to_le_bytes());
    corrupt.extend_from_slice(&1u32.to_le_bytes());
    db.conn
        .execute(
            "INSERT INTO asset_scene_cuts (asset_id, state, auto_cuts, user_cuts, updated_at)
             VALUES (1, 'done', ?1, ?2, 0)",
            rusqlite::params![corrupt, scene_cuts::encode(&[7000]).unwrap()],
        )
        .unwrap();
    let warnings = Warnings::default();

    // Act
    let row = tracing::subscriber::with_default(warnings.clone(), || {
        scene_cuts::get(&db.conn, 1).unwrap().unwrap()
    });

    // Assert: 손상 열만 [] — 정상 열·상태는 유지되고 경고에는 자산 id·열 이름만 (BLOB 값 없음)
    assert!(row.auto.is_empty());
    assert_eq!(row.user, vec![7000]);
    let events = warnings.0.lock().unwrap().clone();
    assert_eq!(events.len(), 1, "{events:?}");
    let rendered = format!("{events:?}");
    assert!(rendered.contains("auto_cuts") && rendered.contains("asset_id"));
    for leaked in ["57005", "48879", "DEAD", "dead", "222"] {
        assert!(!rendered.contains(leaked), "{leaked} leaked: {rendered}");
    }

    // Act: 다음 쓰기
    let added =
        scene_cuts::add_user_tx(&db.conn, 1, 20_000, Some(25.0), Some(60_000), NOW).unwrap();

    // Assert: 정상 형식으로 덮어써졌고 사용자 컷은 보존
    assert!(added);
    let raw: Option<Vec<u8>> = db
        .conn
        .query_row(
            "SELECT auto_cuts FROM asset_scene_cuts WHERE asset_id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(decode(raw.as_deref()).is_ok());
    let healed = scene_cuts::get(&db.conn, 1).unwrap().unwrap();
    assert_eq!(healed.user, vec![7000, 20_000]);
}

#[test]
fn t_scene_05_unknown_version_and_truncated_blobs_in_every_column_never_error() {
    // Arrange
    let db = common::migrated_db();
    add_video(&db.conn, 1, None, None);
    db.conn
        .execute(
            "INSERT INTO asset_scene_cuts (asset_id, state, auto_cuts, user_cuts, suppressed, updated_at)
             VALUES (1, 'done', X'07', X'010102', X'', 0)",
            [],
        )
        .unwrap();

    // Act
    let row = scene_cuts::get(&db.conn, 1).unwrap().unwrap();

    // Assert
    assert!(row.auto.is_empty() && row.user.is_empty() && row.suppressed.is_empty());
}
