// Categories: Happy (사전 계산 저장·입력 정규화) / Business rule (모델 없음 = 건너뜀, 의미 검색 게이트, 데이터 폴더 밖 이미지 거부) /
// Boundary (빈 텍스트·1000자 절단)
use mam_core::ai_jobs::EmbedRequest;
use mam_core::embedding::{EmbeddingError, embed_semantic};
use mam_db::Priority;

use super::super::test_support::Fx;
use super::*;

struct Recording(std::sync::Mutex<Vec<String>>);

impl TextEncoder for Recording {
    fn model(&self) -> ModelInfo {
        ModelInfo {
            model_id: "rec".into(),
            model_version: "1".into(),
            dimension: 2,
        }
    }
    fn encode_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        self.0.lock().unwrap().extend(texts.iter().cloned());
        Ok(vec![vec![0.6, 0.8]; texts.len()])
    }
}

fn adapter(fx: &Fx) -> EmbeddingAdapter {
    let models = Arc::clone(fx.service.models.store());
    let embedders = Arc::new(LocalEmbedders::new(models, EmbedConfig::default()));
    let store = Arc::new(EmbedStore::new(fx.dir.path().join("vectors2")).unwrap());
    let clock: SharedClock = fx.clock.clone();
    EmbeddingAdapter::new(
        Arc::clone(&fx.pool),
        clock,
        embedders,
        store,
        fx.dir.path().to_path_buf(),
    )
}

#[test]
fn t_p3w_emb_01_semantic_input_matches_core_normalisation() {
    // Arrange: 코어 embed_semantic 이 인코더에 넘기는 문자열과 같아야 한다 (어긋나면 모델 키만 맞고 의미가 달라진다)
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    let store = Arc::new(EmbedStore::new(fx.dir.path().join("v")).unwrap());
    let cases: Vec<Vec<String>> = vec![
        vec!["  설명 ".into(), "".into(), "키워드, 둘".into()],
        vec!["가".repeat(1500)],
        vec!["only".into()],
    ];
    for parts in cases {
        let rec = Arc::new(Recording(Default::default()));
        let (r2, s2, p2) = (Arc::clone(&rec), Arc::clone(&store), parts.clone());
        fx.pool
            .write(Priority::Low, move |c| {
                let refs: Vec<&str> = p2.iter().map(String::as_str).collect();
                embed_semantic(c, &s2, &*r2, 1, &refs, 1).unwrap();
                Ok(())
            })
            .unwrap();
        // Act + Assert
        assert_eq!(
            rec.0.lock().unwrap().first().cloned(),
            semantic_input(&parts)
        );
    }
}

#[test]
fn t_p3w_emb_02_blank_text_has_no_input() {
    assert_eq!(semantic_input(&[]), None);
    assert_eq!(semantic_input(&["  ".into(), "\n".into()]), None);
    let s = semantic_input(&["x".repeat(2000)]).unwrap();
    assert_eq!(s.chars().count(), PASSAGE_PREFIX.len() + 1000);
}

#[test]
fn t_p3w_emb_03_no_models_installed_is_model_missing_and_writes_nothing() {
    // Arrange
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    let thumb = fx.write_thumb("t.jpg");
    let adapter = adapter(&fx);
    // Act
    let with_image = adapter
        .embed(&EmbedRequest {
            asset_id: 1,
            mode: EmbedMode::Full,
            images: vec![ImageInput {
                path: thumb,
                frame: None,
            }],
            text_parts: vec!["설명".into()],
        })
        .unwrap_err();
    let text_only = adapter
        .embed(&EmbedRequest {
            asset_id: 1,
            mode: EmbedMode::Full,
            images: vec![],
            text_parts: vec!["설명".into()],
        })
        .unwrap_err();
    // Assert
    assert_eq!(with_image.code, "model_missing");
    assert_eq!(text_only.code, "model_missing");
    let rows: i64 = fx
        .pool
        .read(|c| {
            Ok(
                c.query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
                    .unwrap(),
            )
        })
        .unwrap();
    assert_eq!(rows, 0);
}

#[test]
fn t_p3w_emb_04_precomputed_vectors_are_stored_and_outside_images_rejected() {
    // Arrange
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    let inside = fx.write_thumb("t.jpg");
    let store = Arc::new(EmbedStore::new(fx.dir.path().join("v")).unwrap());
    let info = ModelInfo {
        model_id: "clip-test".into(),
        model_version: "1".into(),
        dimension: 2,
    };
    let enc = Precomputed {
        info,
        out: vec![vec![1.0, 0.0]],
    };
    let root = fx.dir.path().to_path_buf();
    // Act
    let (ok, outside) = fx
        .pool
        .write(Priority::Low, move |c| {
            let src = |p: PathBuf| {
                vec![ImageSource {
                    path: p,
                    frame_index: None,
                }]
            };
            let ok = embed_visual(c, &store, &enc, &root, 1, &src(inside), 1);
            let outside = embed_visual(
                c,
                &store,
                &enc,
                &root,
                1,
                &src(PathBuf::from(r"C:\Users\someone\original.jpg")),
                2,
            );
            Ok((ok, outside))
        })
        .unwrap();
    // Assert
    assert_eq!(ok.unwrap().len(), 1);
    let err = outside.unwrap_err();
    assert!(matches!(err, EmbeddingError::OutsideDataDir(_)));
    assert_eq!(store_failure(&err).code, "invalid_input");
}

#[test]
fn t_p3w_emb_05_query_embedder_is_gated_by_setting_and_model() {
    // Arrange
    use crate::search_hybrid::QueryEmbedder;
    let fx = Fx::new();
    let q = LocalQueryEmbedder::new(
        Arc::clone(&fx.service.embedders),
        Arc::clone(&fx.service.models),
        Arc::clone(&fx.service.store),
    );
    // Act + Assert: 설정 꺼짐 + 모델 없음 -> 키 없음 (키워드 + degraded)
    assert!(q.semantic_key().is_none() && q.visual_key().is_none());
    assert!(q.embed_semantic_query("바다").is_err());
    assert_eq!(
        fx.service.embedders.query_semantic("바다").unwrap_err(),
        QueryError::ModelMissing
    );
    assert_eq!(
        fx.service.embedders.query_visual("sea").unwrap_err(),
        QueryError::ModelMissing
    );
}

#[test]
fn t_p3w_emb_05b_warm_up_does_nothing_without_the_setting_or_the_model() {
    // Arrange: 설정 꺼짐 + 모델 없음
    let fx = Fx::new();
    let q = LocalQueryEmbedder::new(
        Arc::clone(&fx.service.embedders),
        Arc::clone(&fx.service.models),
        Arc::clone(&fx.service.store),
    );
    // Act + Assert: 로드를 시도하지 않고(false) 패닉도 없다 — 이후 첫 질의는 평소처럼 ModelMissing 으로 degraded
    assert!(!q.warm_up());
    assert_eq!(
        fx.service.embedders.query_semantic("바다").unwrap_err(),
        QueryError::ModelMissing
    );
}

#[test]
fn t_p3w_emb_06_semantic_only_ignores_images() {
    // 의미 전용 재계산은 이미지를 읽지 않는다 (없는 경로여도 이미지 오류가 아니라 모델 없음으로 끝난다)
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    let e = adapter(&fx)
        .embed(&EmbedRequest {
            asset_id: 1,
            mode: EmbedMode::SemanticOnly,
            images: vec![ImageInput {
                path: fx.dir.path().join("cache").join("missing.webp"),
                frame: Some(mam_core::ai_jobs::FrameCell { index: 0, cells: 5 }),
            }],
            text_parts: vec!["편집된 설명".into()],
        })
        .unwrap_err();
    assert_eq!(e.code, "model_missing");
}
