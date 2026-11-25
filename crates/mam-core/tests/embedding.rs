//! T-emb-09~11 - 자산 임베딩 작업(시각·의미·질의). 분류: Happy / Business rule / External / Boundary.
//! 가짜 인코더(결정적 벡터) + 실제 SQLite + 임시 디렉터리. 실제 모델 추론은 P1-6 측정으로 갈음한다.

mod common;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use common::{TestDb, migrated_db};
use mam_core::embedding::{
    EmbeddingError, ImageSource, ModelInfo, PASSAGE_PREFIX, QUERY_PREFIX, SEMANTIC_TEXT_MAX_CHARS,
    TextEncoder, VisualEncoder, embed_query, embed_semantic, embed_visual,
};
use mam_vector::{
    EmbedStore, Generations, ModelKey, OpenMode, VectorKind, active_candidates, plan_candidates,
};
use tempfile::TempDir;

const DIM: usize = 16;
const NOW: i64 = 1_700_000_000_000;

fn seeded(bytes: &[u8]) -> Vec<f32> {
    let h = bytes.iter().fold(1469598103934665603u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(1099511628211)
    });
    (0..DIM)
        .map(|i| ((h >> (i % 32)) as u32 % 97) as f32 + 1.0 + i as f32 * 0.37)
        .collect()
}

fn model(id: &str) -> ModelInfo {
    ModelInfo {
        model_id: id.into(),
        model_version: "1".into(),
        dimension: DIM,
    }
}

/// 파일 내용으로 벡터를 만든다 (읽은 경로를 기록).
struct FakeVisual {
    read: RefCell<Vec<PathBuf>>,
    fail: bool,
    wrong_dim: bool,
}

impl FakeVisual {
    fn ok() -> Self {
        Self {
            read: RefCell::new(vec![]),
            fail: false,
            wrong_dim: false,
        }
    }
}

impl VisualEncoder for FakeVisual {
    fn model(&self) -> ModelInfo {
        model("clip-fake")
    }
    fn encode_images(&self, paths: &[PathBuf]) -> Result<Vec<Vec<f32>>, String> {
        if self.fail {
            return Err("onnx session failed".into());
        }
        paths
            .iter()
            .map(|p| {
                self.read.borrow_mut().push(p.clone());
                let mut v = seeded(&std::fs::read(p).map_err(|e| e.to_string())?);
                if self.wrong_dim {
                    v.pop();
                }
                Ok(v)
            })
            .collect()
    }
}

/// 받은 텍스트를 기록하고 텍스트로 벡터를 만든다.
struct FakeText {
    seen: RefCell<Vec<String>>,
    fail: bool,
}

impl FakeText {
    fn ok() -> Self {
        Self {
            seen: RefCell::new(vec![]),
            fail: false,
        }
    }
}

impl TextEncoder for FakeText {
    fn model(&self) -> ModelInfo {
        model("e5-fake")
    }
    fn encode_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if self.fail {
            return Err("tokenizer failed".into());
        }
        self.seen.borrow_mut().extend(texts.iter().cloned());
        Ok(texts.iter().map(|t| seeded(t.as_bytes())).collect())
    }
}

struct Env {
    db: TestDb,
    store: EmbedStore,
    /// MAMData 흉내 - 썸네일·프레임이 여기에 있다
    data: TempDir,
    /// 사용자 원본 폴더 흉내
    originals: TempDir,
}

fn env() -> Env {
    let db = migrated_db();
    for _ in 0..3 {
        db.conn
            .execute(
                "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
                 VALUES ('video', 1, 1, 1, 1)",
                [],
            )
            .unwrap();
    }
    let data = TempDir::new().unwrap();
    let store = EmbedStore::new(data.path().join("indexes").join("vectors")).unwrap();
    Env {
        db,
        store,
        data,
        originals: TempDir::new().unwrap(),
    }
}

fn frame(e: &Env, name: &str, content: &[u8]) -> PathBuf {
    let p = e.data.path().join("cache").join("previews").join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, content).unwrap();
    p
}

fn key(kind: VectorKind, id: &str) -> ModelKey {
    ModelKey {
        kind,
        model_id: id.into(),
        model_version: "1".into(),
        dimension: DIM,
    }
}

fn count_active(e: &Env, k: &ModelKey) -> usize {
    e.store.active_ids(&e.db.conn, k).unwrap().len()
}

// ------------------------------------------------------------ T-emb-09 (Happy)
#[test]
fn t_emb_09_video_with_five_frames_gives_five_vectors_and_one_asset_hit() {
    // Arrange
    let mut e = env();
    let enc = FakeVisual::ok();
    let frames: Vec<ImageSource> = (0..5)
        .map(|i| ImageSource {
            path: frame(&e, &format!("1_{i}.jpg"), format!("frame-{i}").as_bytes()),
            frame_index: Some(i),
        })
        .collect();
    let other = [ImageSource {
        path: frame(&e, "2_0.jpg", b"another-asset"),
        frame_index: None,
    }];
    // Act
    let ids = embed_visual(
        &mut e.db.conn,
        &e.store,
        &enc,
        e.data.path(),
        1,
        &frames,
        NOW,
    )
    .unwrap();
    embed_visual(
        &mut e.db.conn,
        &e.store,
        &enc,
        e.data.path(),
        2,
        &other,
        NOW,
    )
    .unwrap();
    // Assert
    assert_eq!(ids.len(), 5);
    let k = key(VectorKind::Visual, "clip-fake");
    assert_eq!(count_active(&e, &k), 6);
    let (src, fi): (String, i64) =
        e.db.conn
            .query_row(
                "SELECT source, frame_index FROM embeddings WHERE embedding_id = ?1",
                [ids[3] as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
    assert_eq!((src.as_str(), fi), ("frame", 3));
    // 자산 검색: 프레임 3의 벡터로 질의 -> 자산 1 이 한 번만, 1순위
    let gens = Generations::new(&e.store, &k).unwrap();
    let (idx, _) = gens
        .open_or_rebuild(&e.db.conn, &e.store, &k, OpenMode::Load)
        .unwrap();
    let q = seeded(b"frame-3");
    let hits = active_candidates(
        &e.db.conn,
        &e.store,
        &k,
        &idx,
        &q,
        &plan_candidates(10),
        None,
    )
    .unwrap();
    assert_eq!(hits.len(), 2, "프레임 5개 + 단일 이미지 = 자산 2건");
    assert_eq!((hits[0].asset_id, hits[0].embedding_id), (1, ids[3]));
    // 재임베딩(프레임 2장) -> 옛 5개 폐기, 인덱스 검색에서 옛 프레임이 나오지 않는다
    embed_visual(
        &mut e.db.conn,
        &e.store,
        &enc,
        e.data.path(),
        1,
        &frames[..2],
        NOW + 1,
    )
    .unwrap();
    mam_vector::sync_incremental(&e.db.conn, &e.store, &k, &idx).unwrap();
    let hits = active_candidates(
        &e.db.conn,
        &e.store,
        &k,
        &idx,
        &q,
        &plan_candidates(10),
        None,
    )
    .unwrap();
    assert!(
        hits.iter().all(|h| !ids.contains(&h.embedding_id)),
        "폐기된 프레임은 나오지 않는다"
    );
    assert_eq!(count_active(&e, &k), 3);
}

// ------------------------------------------------------------ T-emb-10 (Happy / Boundary)
#[test]
fn t_emb_10_semantic_uses_passage_prefix_and_query_uses_query_prefix() {
    let mut e = env();
    let enc = FakeText::ok();

    let id = embed_semantic(
        &mut e.db.conn,
        &e.store,
        &enc,
        1,
        &[
            "바닷가 일몰 영상",
            "",
            "  ",
            "키워드: 바다, 일몰",
            "전사: 안녕하세요",
        ],
        NOW,
    )
    .unwrap();

    assert!(id.is_some());
    assert_eq!(
        enc.seen.borrow().as_slice(),
        [format!(
            "{PASSAGE_PREFIX}바닷가 일몰 영상\n키워드: 바다, 일몰\n전사: 안녕하세요"
        )]
    );
    let sem = key(VectorKind::Semantic, "e5-fake");
    assert_eq!(count_active(&e, &sem), 1);

    // 질의: 의미 = query: 접두어, 시각(CLIP 텍스트) = 접두어 없음, 결과는 정규화
    let q = embed_query(&enc, VectorKind::Semantic, "  일몰  ").unwrap();
    let qv = embed_query(&enc, VectorKind::Visual, "sunset").unwrap();
    assert_eq!(enc.seen.borrow()[1], format!("{QUERY_PREFIX}일몰"));
    assert_eq!(enc.seen.borrow()[2], "sunset");
    for v in [&q, &qv] {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5);
    }

    // 상한 절단 + 접두어 유지
    let long = "가".repeat(SEMANTIC_TEXT_MAX_CHARS + 500);
    embed_semantic(&mut e.db.conn, &e.store, &enc, 2, &[&long], NOW).unwrap();
    let sent = enc.seen.borrow().last().unwrap().clone();
    assert_eq!(
        sent.chars().count(),
        PASSAGE_PREFIX.len() + SEMANTIC_TEXT_MAX_CHARS
    );

    // 텍스트가 모두 비면 기존 의미 벡터만 폐기
    let none = embed_semantic(&mut e.db.conn, &e.store, &enc, 1, &["", " "], NOW + 1).unwrap();
    assert!(none.is_none());
    assert_eq!(count_active(&e, &sem), 1, "자산 2 만 남는다");
}

#[test]
fn t_emb_10b_encoder_failures_keep_existing_vectors_and_are_reported() {
    let mut e = env();
    let ok = FakeText::ok();
    embed_semantic(&mut e.db.conn, &e.store, &ok, 1, &["첫 설명"], NOW).unwrap();
    let sem = key(VectorKind::Semantic, "e5-fake");
    let before = e.store.active_ids(&e.db.conn, &sem).unwrap();

    let bad = FakeText {
        seen: RefCell::new(vec![]),
        fail: true,
    };
    let r = embed_semantic(&mut e.db.conn, &e.store, &bad, 1, &["새 설명"], NOW + 1);
    assert!(matches!(r, Err(EmbeddingError::Encoder(m)) if m.contains("tokenizer")));
    assert_eq!(e.store.active_ids(&e.db.conn, &sem).unwrap(), before);

    assert!(matches!(
        embed_query(&ok, VectorKind::Semantic, "   "),
        Err(EmbeddingError::InvalidInput(_))
    ));
    assert!(matches!(
        embed_query(&bad, VectorKind::Semantic, "x"),
        Err(EmbeddingError::Encoder(_))
    ));
}

#[test]
fn t_emb_10c_visual_encoder_failures_and_bad_output_store_nothing() {
    let mut e = env();
    let p = frame(&e, "a.jpg", b"a");
    let imgs = [ImageSource {
        path: p,
        frame_index: None,
    }];
    let k = key(VectorKind::Visual, "clip-fake");

    let failing = FakeVisual {
        fail: true,
        ..FakeVisual::ok()
    };
    let r = embed_visual(
        &mut e.db.conn,
        &e.store,
        &failing,
        e.data.path(),
        1,
        &imgs,
        NOW,
    );
    assert!(matches!(r, Err(EmbeddingError::Encoder(_))));

    let wrong = FakeVisual {
        wrong_dim: true,
        ..FakeVisual::ok()
    };
    let r = embed_visual(
        &mut e.db.conn,
        &e.store,
        &wrong,
        e.data.path(),
        1,
        &imgs,
        NOW,
    );
    assert!(matches!(
        r,
        Err(EmbeddingError::DimensionMismatch { expected: DIM, .. })
    ));

    let r = embed_visual(
        &mut e.db.conn,
        &e.store,
        &FakeVisual::ok(),
        e.data.path(),
        1,
        &[],
        NOW,
    );
    assert!(matches!(r, Err(EmbeddingError::InvalidInput(_))));
    assert_eq!(count_active(&e, &k), 0);
}

// ------------------------------------------------------------ T-emb-11 (Business rule: 원본 보호)
fn snapshot(p: &Path) -> (Vec<u8>, std::time::SystemTime) {
    (
        std::fs::read(p).unwrap(),
        std::fs::metadata(p).unwrap().modified().unwrap(),
    )
}

#[test]
fn t_emb_11_only_data_dir_images_are_read_and_originals_untouched() {
    let mut e = env();
    let enc = FakeVisual::ok();
    let original = e.originals.path().join("movie.mp4");
    std::fs::write(&original, b"original media bytes").unwrap();
    let before = snapshot(&original);
    let thumb = frame(&e, "thumb.jpg", b"thumb");

    // 원본 경로 / `..` 로 MAMData 를 벗어나는 경로는 인코더를 부르기 전에 거부
    let escape = e
        .data
        .path()
        .join("cache")
        .join("..")
        .join("..")
        .join("movie.mp4");
    for bad in [original.clone(), escape] {
        let r = embed_visual(
            &mut e.db.conn,
            &e.store,
            &enc,
            e.data.path(),
            1,
            &[ImageSource {
                path: bad,
                frame_index: None,
            }],
            NOW,
        );
        assert!(matches!(r, Err(EmbeddingError::OutsideDataDir(_))), "{r:?}");
    }
    assert!(enc.read.borrow().is_empty(), "인코더는 호출되지 않았다");

    // 썸네일은 허용되고, 실행 후에도 원본의 내용·수정 시각이 같다
    embed_visual(
        &mut e.db.conn,
        &e.store,
        &enc,
        e.data.path(),
        1,
        &[ImageSource {
            path: thumb.clone(),
            frame_index: None,
        }],
        NOW,
    )
    .unwrap();
    assert_eq!(enc.read.borrow().as_slice(), [thumb]);
    assert_eq!(snapshot(&original), before);
}
