//! 합성 라이브러리 — 어휘, 파일명, SQLite 일괄 적재(자산·위치·메타데이터·파생물 상태·태그), 합성 임베딩 일괄 적재.
//!
//! 메타데이터만 합성한다: 파일은 디스크에 없다. 그래서 이 데이터로 판단할 수 있는 것은 SQLite/Tantivy/벡터 경로의 지연뿐이다
//! (실제 파일시스템·코덱·UI 성능이 아니다). 색인은 `SyncWorker` 가 처음 열 때 SQLite 에서 재구축한다(제품의 시작 경로).

use std::path::Path;

use mam_vector::{EmbedStore, ModelKey, VectorKind, raw::RawFile};
use rusqlite::{Connection, params};

use super::stats::Lcg;

pub const EN_WORDS: [&str; 20] = [
    "sunset",
    "beach",
    "portrait",
    "meeting",
    "birthday",
    "mountain",
    "street",
    "family",
    "product",
    "interview",
    "drone",
    "concert",
    "wedding",
    "forest",
    "city",
    "night",
    "travel",
    "holiday",
    "office",
    "kitchen",
];
pub const KO_WORDS: [&str; 20] = [
    "제주도",
    "바다",
    "가족여행",
    "회의록",
    "생일파티",
    "설악산",
    "서울야경",
    "졸업식",
    "인터뷰",
    "결혼식",
    "강아지",
    "놀이공원",
    "벚꽃",
    "단풍",
    "해외출장",
    "촬영본",
    "현장스케치",
    "시연영상",
    "녹음",
    "강의",
];
pub const TAG_NAMES: [&str; 40] = [
    "travel",
    "work",
    "family",
    "friends",
    "event",
    "nature",
    "city",
    "food",
    "pet",
    "sports",
    "project-a",
    "project-b",
    "archive",
    "draft",
    "final",
    "selects",
    "b-roll",
    "interview",
    "music",
    "podcast",
    "여행",
    "업무",
    "가족",
    "친구",
    "행사",
    "자연",
    "도시",
    "음식",
    "반려동물",
    "운동",
    "프로젝트",
    "보관",
    "초안",
    "최종",
    "선별",
    "인터뷰",
    "음악",
    "팟캐스트",
    "회사",
    "휴가",
];
pub const SEMANTIC_DIM: usize = 384;
pub const VISUAL_DIM: usize = 512;
const YEARS: [i64; 9] = [2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025, 2026];
/// 2018-01-01T00:00:00Z (ms)
const T2018_MS: i64 = 1_514_764_800_000;
const YEAR_MS: i64 = 365 * 86_400_000;

pub fn semantic_key() -> ModelKey {
    ModelKey {
        kind: VectorKind::Semantic,
        model_id: "synthetic-e5".into(),
        model_version: "p4".into(),
        dimension: SEMANTIC_DIM,
    }
}

pub fn visual_key() -> ModelKey {
    ModelKey {
        kind: VectorKind::Visual,
        model_id: "synthetic-clip".into(),
        model_version: "p4".into(),
        dimension: VISUAL_DIM,
    }
}

/// 어휘에서 고른 두 단어 + 번호 — 영어/한국어가 섞인 파일명 (확장자 제외).
pub fn file_stem(rng: &mut Lcg, id: i64) -> String {
    let word = |rng: &mut Lcg| {
        if rng.below(2) == 0 {
            *rng.pick(&EN_WORDS)
        } else {
            *rng.pick(&KO_WORDS)
        }
    };
    let a = word(rng);
    let b = word(rng);
    format!("{a}_{b}_{id:07}")
}

/// 결정적 단위 벡터 (시드 → 차원 `dim`).
pub fn unit_vector(seed: u64, dim: usize) -> Vec<f32> {
    let mut rng = Lcg(seed ^ 0x9E37_79B9_7F4A_7C15);
    let mut v: Vec<f32> = (0..dim).map(|_| rng.unit() - 0.5).collect();
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

pub struct SeedSpec<'a> {
    pub volume_id: i64,
    /// 볼륨 기준 관리 폴더 상대 경로 (표시용 그대로, 슬래시 구분)
    pub root_rel: &'a str,
    pub first_id: i64,
    pub count: usize,
    pub seed: u64,
    pub now_ms: i64,
}

fn media_of(rng: &mut Lcg) -> (&'static str, &'static str) {
    match rng.below(100) {
        0..=79 => ("image", ["jpg", "png", "heic", "webp"][rng.below(4)]),
        80..=94 => ("video", ["mp4", "mov", "mkv"][rng.below(3)]),
        _ => ("audio", ["wav", "mp3", "m4a"][rng.below(3)]),
    }
}

/// 자산 `count` 건을 한 트랜잭션으로 적재한다 (처리 완료 상태: 해시 done, 메타·썸네일[·스트립] done).
/// 색인 Outbox 이벤트는 만들지 않는다 — 처음 열리는 `SyncWorker` 가 SQLite 에서 재구축한다.
pub fn seed_assets(conn: &mut Connection, spec: &SeedSpec<'_>) -> rusqlite::Result<()> {
    let mut rng = Lcg(spec.seed);
    let tx = conn.transaction()?;
    {
        let mut a = tx.prepare(
            "INSERT INTO media_assets (asset_id, media_type, content_hash, hash_algo, hash_state,
                file_size, modified_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'blake3', 'done', ?4, ?5, ?6, ?6)",
        )?;
        let mut l = tx.prepare(
            "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
             VALUES (?1, ?2, ?3, lower(?3), ?4)",
        )?;
        let mut m = tx.prepare(
            "INSERT INTO media_metadata (asset_id, width, height, duration_ms, codec, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        let mut d = tx.prepare(
            "INSERT INTO asset_derivatives (asset_id, kind, state, updated_at) VALUES (?1, ?2, ?3, ?4)",
        )?;
        let mut t = tx.prepare(
            "INSERT OR IGNORE INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, ?2, ?3)",
        )?;
        for i in 0..spec.count {
            let id = spec.first_id + i as i64;
            let (kind, ext) = media_of(&mut rng);
            let year = YEARS[rng.below(YEARS.len())];
            let month = 1 + rng.below(12);
            let rel = format!(
                "{}/{year}/{month:02}/{}.{ext}",
                spec.root_rel,
                file_stem(&mut rng, id)
            );
            let captured = if rng.below(10) == 0 {
                None
            } else {
                Some(
                    T2018_MS
                        + (year - 2018) * YEAR_MS
                        + (month as i64 - 1) * 30 * 86_400_000
                        + (rng.raw() % (28 * 86_400_000)) as i64,
                )
            };
            let size = match kind {
                "image" => 200_000 + (rng.raw() % 12_000_000) as i64,
                "video" => 5_000_000 + (rng.raw() % 3_000_000_000) as i64,
                _ => 500_000 + (rng.raw() % 60_000_000) as i64,
            };
            let hash: Vec<u8> = (0..4).flat_map(|_| rng.raw().to_le_bytes()).collect();
            let modified = captured.unwrap_or(spec.now_ms) + (rng.raw() % 86_400_000) as i64;
            a.execute(params![id, kind, hash, size, modified, spec.now_ms])?;
            l.execute(params![id, spec.volume_id, rel, spec.now_ms])?;
            let duration = (kind != "image").then(|| 1_000 + (rng.raw() % 600_000) as i64);
            m.execute(params![
                id,
                1920,
                1080,
                duration,
                if kind == "image" { "jpeg" } else { "h264" },
                captured
            ])?;
            d.execute(params![id, "meta", "done", spec.now_ms])?;
            d.execute(params![
                id,
                "thumb",
                if kind == "audio" {
                    "unsupported"
                } else {
                    "done"
                },
                spec.now_ms
            ])?;
            if kind == "video" {
                d.execute(params![id, "strip", "done", spec.now_ms])?;
            }
            for _ in 0..rng.below(4) {
                t.execute(params![
                    id,
                    1 + rng.below(TAG_NAMES.len()) as i64,
                    spec.now_ms
                ])?;
            }
        }
    }
    tx.commit()
}

/// 태그 목록 (`TAG_NAMES`, tag_id 1..)
pub fn seed_tags(conn: &Connection) -> rusqlite::Result<()> {
    for (i, name) in TAG_NAMES.iter().enumerate() {
        conn.execute(
            "INSERT INTO tags (tag_id, name, name_key) VALUES (?1, ?2, lower(?2))",
            params![i as i64 + 1, name],
        )?;
    }
    Ok(())
}

/// `asset_ids` 각각에 합성 의미 벡터 1개를 raw 파일 + `embeddings` 행으로 적재한다 (제품 저장 형식, `EmbedStore::put` 우회).
/// `put` 은 호출마다 모델 전체 행을 훑는 `MAX(raw_offset)` 질의를 하므로 수십만 건 시드에 쓰면 이차 비용이 된다 —
/// 라이브 경로(AI 작업)는 실제 `put` 을 쓰고 그 지연을 따로 잰다.
pub fn seed_semantic_vectors(
    conn: &mut Connection,
    store: &EmbedStore,
    asset_ids: impl Iterator<Item = i64>,
    now_ms: i64,
) -> Result<usize, String> {
    let key = semantic_key();
    std::fs::create_dir_all(store.key_dir(&key)).map_err(|e| e.to_string())?;
    let mut raw = RawFile::open(Path::new(&store.raw_path(&key)), key.dimension)
        .map_err(|e| e.to_string())?;
    let mut total = 0;
    let mut batch: Vec<i64> = Vec::with_capacity(5_000);
    let mut flush = |batch: &mut Vec<i64>, conn: &mut Connection| -> Result<(), String> {
        if batch.is_empty() {
            return Ok(());
        }
        let vecs: Vec<Vec<f32>> = batch
            .iter()
            .map(|id| mam_vector::normalized(&unit_vector(*id as u64, key.dimension)))
            .collect();
        let refs: Vec<&[f32]> = vecs.iter().map(Vec::as_slice).collect();
        let offsets = raw.append(&refs).map_err(|e| e.to_string())?;
        raw.sync().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        {
            let mut ins = tx
                .prepare(
                    "INSERT INTO embeddings (asset_id, vector_kind, source, model_id, model_version,
                        dimension, raw_offset, created_at)
                     VALUES (?1, 'semantic', 'text', ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(|e| e.to_string())?;
            for (id, off) in batch.iter().zip(&offsets) {
                ins.execute(params![
                    id,
                    key.model_id,
                    key.model_version,
                    key.dimension as i64,
                    *off as i64,
                    now_ms
                ])
                .map_err(|e| e.to_string())?;
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        batch.clear();
        Ok(())
    };
    for id in asset_ids {
        batch.push(id);
        total += 1;
        if batch.len() >= 5_000 {
            flush(&mut batch, conn)?;
        }
    }
    flush(&mut batch, conn)?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (CHECK 제약을 만족하는 적재) / Boundary (결정성)
    use super::*;
    use mam_db::{SystemClock, migrate, open};

    #[test]
    fn t_p4_synth_01_seed_is_deterministic_and_satisfies_schema() {
        // Arrange
        let dir = tempfile::TempDir::new().unwrap();
        let mut conn = open(&dir.path().join("a.sqlite")).unwrap();
        migrate(&mut conn, &dir.path().join("b"), &SystemClock).unwrap();
        conn.execute(
            "INSERT INTO volumes (volume_id, kind, volume_key, last_seen_at) VALUES (1, 'fixed', 'v', 0)",
            [],
        )
        .unwrap();
        seed_tags(&conn).unwrap();
        let spec = SeedSpec {
            volume_id: 1,
            root_rel: "bench/base",
            first_id: 1,
            count: 300,
            seed: 7,
            now_ms: 1_700_000_000_000,
        };
        // Act
        seed_assets(&mut conn, &spec).unwrap();
        let first: String = conn
            .query_row(
                "SELECT rel_path FROM file_locations WHERE asset_id = 5",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM media_assets WHERE hash_state = 'done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Assert
        assert_eq!(n, 300);
        assert!(first.starts_with("bench/base/20"), "{first}");
    }

    #[test]
    fn t_p4_synth_02_unit_vectors_are_deterministic_unit_length() {
        // Arrange / Act
        let a = unit_vector(42, 16);
        let b = unit_vector(42, 16);
        let c = unit_vector(43, 16);
        // Assert
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!((a.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-4);
    }
}
