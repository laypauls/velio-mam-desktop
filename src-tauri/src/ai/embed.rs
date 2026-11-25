//! 로컬 임베딩 어댑터 — 작업 코어의 `EmbeddingRunner` 를 `mam_ai` 임베더 + `mam_core::embedding` 위에 구현한다 (P3-5).
//!
//! - ONNX 세션은 모델이 준비된 뒤 처음 쓸 때 한 번 만든다 (기동 비용 없음)
//! - 추론은 DB 쓰기 스레드 밖에서 끝내고, 결과 벡터만 `embed_visual`/`embed_semantic` 에 넘겨 저장한다
//!   (사전 계산 인코더) — 긴 추론이 사용자 편집 쓰기를 막지 않게 한다
//! - 모델이 없으면 실패가 아니라 `model_missing`(건너뜀)이다

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mam_ai::AiError;
use mam_ai::local::embed::{
    CLIP_IMAGE_DIM, CLIP_IMAGE_MODEL_ID, CLIP_TEXT_DIM, CLIP_TEXT_MODEL_ID, ClipTextEmbedder,
    E5_DIM, E5_MODEL_ID, E5Embedder, EmbedConfig, ImageEmbedder, TextRole,
};
use mam_ai::models::{ModelPaths, ModelStatus, ModelStore};
use mam_core::SharedClock;
use mam_core::ai_jobs::{EmbedMode, EmbedRequest, EmbeddingRunner, ImageInput, StepFailure};
use mam_core::embedding::{
    EmbeddingError, ImageSource, ModelInfo, PASSAGE_PREFIX, QUERY_PREFIX, SEMANTIC_TEXT_MAX_CHARS,
    TextEncoder, VisualEncoder, embed_query, embed_semantic, embed_visual,
};
use mam_db::{DbPool, Priority};
use mam_vector::{EmbedStore, VectorKind};

use super::strip::resolve_images;

/// 모델 로드 실패 구분 — 미설치는 건너뜀, 로드 오류는 영구 실패.
#[derive(Debug)]
pub enum LoadError {
    Missing,
    Failed,
}

type Slot<T> = Mutex<Option<Arc<T>>>;

#[allow(dead_code)] // CLIP 텍스트·질의 임베딩은 하이브리드 검색 연결(QueryEmbedder)이 쓴다
pub struct LocalEmbedders {
    models: Arc<ModelStore>,
    cfg: EmbedConfig,
    image: Slot<ImageEmbedder>,
    e5: Slot<E5Embedder>,
    clip_text: Slot<ClipTextEmbedder>,
}

impl LocalEmbedders {
    pub fn new(models: Arc<ModelStore>, cfg: EmbedConfig) -> Self {
        Self {
            models,
            cfg,
            image: Mutex::new(None),
            e5: Mutex::new(None),
            clip_text: Mutex::new(None),
        }
    }

    fn get<T>(
        &self,
        slot: &Slot<T>,
        id: &str,
        build: impl FnOnce(&ModelPaths) -> Result<T, AiError>,
    ) -> Result<Arc<T>, LoadError> {
        let mut g = slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = g.as_ref() {
            return Ok(Arc::clone(t));
        }
        let paths = match self.models.status(id) {
            Ok(ModelStatus::Ready(p)) => p,
            _ => return Err(LoadError::Missing),
        };
        let t = Arc::new(build(&paths).map_err(|e| {
            tracing::warn!(model = id, %e, "cannot load embedding model");
            LoadError::Failed
        })?);
        *g = Some(Arc::clone(&t));
        Ok(t)
    }

    pub fn image(&self) -> Result<Arc<ImageEmbedder>, LoadError> {
        self.get(&self.image, CLIP_IMAGE_MODEL_ID, |p| {
            ImageEmbedder::from_paths(p, self.cfg)
        })
    }

    pub fn e5(&self) -> Result<Arc<E5Embedder>, LoadError> {
        self.get(&self.e5, E5_MODEL_ID, |p| {
            E5Embedder::from_paths(p, self.cfg)
        })
    }

    pub fn clip_text(&self) -> Result<Arc<ClipTextEmbedder>, LoadError> {
        self.get(&self.clip_text, CLIP_TEXT_MODEL_ID, |p| {
            ClipTextEmbedder::from_paths(p, self.cfg)
        })
    }

    /// 저장소 모델 키 — 버전은 매니페스트 첫 파일의 SHA-256 앞 12자(가중치가 바뀌면 키가 바뀐다).
    pub fn info(&self, model_id: &str, dimension: usize) -> ModelInfo {
        let version = self
            .models
            .manifest()
            .get(model_id)
            .ok()
            .and_then(|s| s.files.first())
            .map_or_else(
                || "unknown".to_string(),
                |f| f.sha256.chars().take(12).collect(),
            );
        ModelInfo {
            model_id: model_id.to_string(),
            model_version: version,
            dimension,
        }
    }

    pub fn image_info(&self) -> ModelInfo {
        self.info(CLIP_IMAGE_MODEL_ID, CLIP_IMAGE_DIM)
    }
    pub fn e5_info(&self) -> ModelInfo {
        self.info(E5_MODEL_ID, E5_DIM)
    }
    pub fn clip_text_info(&self) -> ModelInfo {
        self.info(CLIP_TEXT_MODEL_ID, CLIP_TEXT_DIM)
    }
}

/// 이미 계산된 벡터를 돌려주는 인코더 — 저장 함수가 모델 정보와 함께 쓰게 한다.
struct Precomputed {
    info: ModelInfo,
    out: Vec<Vec<f32>>,
}

impl VisualEncoder for Precomputed {
    fn model(&self) -> ModelInfo {
        self.info.clone()
    }
    fn encode_images(&self, _: &[PathBuf]) -> Result<Vec<Vec<f32>>, String> {
        Ok(self.out.clone())
    }
}

impl TextEncoder for Precomputed {
    fn model(&self) -> ModelInfo {
        self.info.clone()
    }
    fn encode_texts(&self, _: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(self.out.clone())
    }
}

/// `embed_semantic` 이 만드는 것과 같은 입력 텍스트 (접두어 포함). 비면 `None`.
pub fn semantic_input(parts: &[String]) -> Option<String> {
    let joined = parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if joined.is_empty() {
        return None;
    }
    let text: String = joined.chars().take(SEMANTIC_TEXT_MAX_CHARS).collect();
    Some(format!("{PASSAGE_PREFIX}{text}"))
}

/// e5 질의/문서 접두어를 역할로 바꿔 임베더를 부른다 (임베더가 접두어를 직접 붙이므로 한 번만 붙게 한다).
pub fn e5_encode(e5: &E5Embedder, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    texts
        .iter()
        .map(|t| {
            let (role, body) = if let Some(b) = t.strip_prefix(QUERY_PREFIX) {
                (TextRole::Query, b)
            } else if let Some(b) = t.strip_prefix(PASSAGE_PREFIX) {
                (TextRole::Passage, b)
            } else {
                return Err("e5 input needs a query:/passage: prefix".to_string());
            };
            e5.embed(role, &[body])
                .map(|mut v| v.remove(0))
                .map_err(|e| e.to_string())
        })
        .collect()
}

pub struct EmbeddingAdapter {
    pool: Arc<DbPool>,
    clock: SharedClock,
    embedders: Arc<LocalEmbedders>,
    store: Arc<EmbedStore>,
    data_root: PathBuf,
    tmp_dir: PathBuf,
}

impl EmbeddingAdapter {
    pub fn new(
        pool: Arc<DbPool>,
        clock: SharedClock,
        embedders: Arc<LocalEmbedders>,
        store: Arc<EmbedStore>,
        data_root: PathBuf,
    ) -> Self {
        Self {
            pool,
            clock,
            embedders,
            store,
            tmp_dir: data_root.join("tmp"),
            data_root,
        }
    }
}

fn store_failure(e: &EmbeddingError) -> StepFailure {
    match e {
        EmbeddingError::Vector(_) | EmbeddingError::Db(_) => {
            StepFailure::new("io_transient", "임베딩을 저장하지 못했습니다")
        }
        EmbeddingError::OutsideDataDir(_) | EmbeddingError::InvalidInput(_) => {
            StepFailure::new("invalid_input", "임베딩 입력이 올바르지 않습니다")
        }
        _ => StepFailure::new("embedding_failed", "임베딩을 만들지 못했습니다"),
    }
}

fn load_failure(e: LoadError) -> Option<StepFailure> {
    match e {
        LoadError::Missing => None,
        LoadError::Failed => Some(StepFailure::new(
            "model_error",
            "임베딩 모델을 불러오지 못했습니다",
        )),
    }
}

impl EmbeddingRunner for EmbeddingAdapter {
    fn embed(&self, req: &EmbedRequest) -> Result<(), StepFailure> {
        // 의미 전용 재계산(편집·전사 갱신 뒤)은 시각 벡터를 건드리지 않는다
        let images: &[ImageInput] = if req.mode == EmbedMode::SemanticOnly {
            &[]
        } else {
            &req.images
        };
        let (paths, _cells) = resolve_images(images, &self.tmp_dir)?;
        // 1) 추론 (DB 쓰기 밖)
        let mut visual: Option<Pre> = None;
        if !paths.is_empty() {
            match self.embedders.image() {
                Ok(img) => {
                    let out = paths
                        .iter()
                        .map(|p| img.embed_file(Path::new(p)))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| match e {
                            AiError::Decode(_) => {
                                StepFailure::new("decode_failed", "이미지를 해석하지 못했습니다")
                            }
                            _ => StepFailure::new("embedding_failed", "임베딩을 만들지 못했습니다"),
                        })?;
                    visual = Some(Precomputed {
                        info: self.embedders.image_info(),
                        out,
                    });
                }
                Err(e) => {
                    if let Some(f) = load_failure(e) {
                        return Err(f);
                    }
                }
            }
        }
        let mut semantic: Option<Pre> = None;
        match self.embedders.e5() {
            Ok(e5) => {
                let out = match semantic_input(&req.text_parts) {
                    Some(input) => e5_encode(&e5, &[input]).map_err(|_| {
                        StepFailure::new("embedding_failed", "임베딩을 만들지 못했습니다")
                    })?,
                    None => Vec::new(),
                };
                semantic = Some(Precomputed {
                    info: self.embedders.e5_info(),
                    out,
                });
            }
            Err(e) => {
                if let Some(f) = load_failure(e) {
                    return Err(f);
                }
            }
        }
        if visual.is_none() && semantic.is_none() {
            return Err(StepFailure::new(
                "model_missing",
                "임베딩 모델이 설치되지 않았습니다",
            ));
        }
        // 2) 저장 (한 번의 쓰기 작업, 벡터 저장소가 종류별로 트랜잭션을 연다)
        let (store, root, asset_id, now) = (
            Arc::clone(&self.store),
            self.data_root.clone(),
            req.asset_id,
            self.clock.now_ms(),
        );
        let text_parts = req.text_parts.clone();
        let sources: Vec<ImageSource> = paths
            .iter()
            .zip(images)
            .map(|(p, img)| ImageSource {
                path: p.clone(),
                frame_index: img.frame_index(),
            })
            .collect();
        let stored = self
            .pool
            .write(Priority::Low, move |c| {
                let mut run = || -> Result<(), EmbeddingError> {
                    if let Some(v) = &visual {
                        embed_visual(c, &store, v, &root, asset_id, &sources, now)?;
                    }
                    if let Some(s) = &semantic {
                        let parts: Vec<&str> = text_parts.iter().map(String::as_str).collect();
                        embed_semantic(c, &store, s, asset_id, &parts, now)?;
                    }
                    Ok(())
                };
                Ok(run())
            })
            .map_err(|_| StepFailure::new("io_transient", "임베딩을 저장하지 못했습니다"))?;
        stored.map_err(|e| store_failure(&e))
    }
}

type Pre = Precomputed;

// ── 질의 임베딩 (하이브리드 검색·유사 미디어용) ─────────────────────────

#[allow(dead_code)]
/// 질의 임베딩 실패 — 모델 미준비는 하이브리드 `degraded` 로, 그 밖은 오류로 다룬다.
#[derive(Debug, PartialEq, Eq)]
pub enum QueryError {
    ModelMissing,
    Failed,
}

#[allow(dead_code)]
struct E5Enc(Arc<E5Embedder>, ModelInfo);

impl TextEncoder for E5Enc {
    fn model(&self) -> ModelInfo {
        self.1.clone()
    }
    fn encode_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        e5_encode(&self.0, texts)
    }
}

#[allow(dead_code)]
struct ClipTextEnc(Arc<ClipTextEmbedder>, ModelInfo);

impl TextEncoder for ClipTextEnc {
    fn model(&self) -> ModelInfo {
        self.1.clone()
    }
    fn encode_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        self.0.embed_texts(&refs).map_err(|e| e.to_string())
    }
}

#[allow(dead_code)]
impl LocalEmbedders {
    /// 의미 검색 질의 벡터 (e5 `query:`). 모델 정보와 함께 돌려준다 — 저장소 모델 키를 만드는 데 쓴다.
    pub fn query_semantic(&self, text: &str) -> Result<(ModelInfo, Vec<f32>), QueryError> {
        let e5 = self.e5().map_err(|e| match e {
            LoadError::Missing => QueryError::ModelMissing,
            LoadError::Failed => QueryError::Failed,
        })?;
        let enc = E5Enc(e5, self.e5_info());
        let v = embed_query(&enc, VectorKind::Semantic, text).map_err(|_| QueryError::Failed)?;
        Ok((enc.1, v))
    }

    /// 시각 검색 질의 벡터 (CLIP 다국어 텍스트 타워, 접두어 없음).
    pub fn query_visual(&self, text: &str) -> Result<(ModelInfo, Vec<f32>), QueryError> {
        let clip = self.clip_text().map_err(|e| match e {
            LoadError::Missing => QueryError::ModelMissing,
            LoadError::Failed => QueryError::Failed,
        })?;
        let enc = ClipTextEnc(clip, self.clip_text_info());
        let v = embed_query(&enc, VectorKind::Visual, text).map_err(|_| QueryError::Failed)?;
        Ok((enc.1, v))
    }
}

/// 하이브리드 검색의 질의 임베딩 경계 구현 — e5 `query:` (의미). 의미 검색 설정이 꺼져 있거나 모델이 없으면 키가 없어
/// 검색은 키워드 + `degraded` 로 응답한다.
pub struct LocalQueryEmbedder {
    embedders: Arc<LocalEmbedders>,
    models: Arc<super::ModelService>,
    store: Arc<super::store::AiStore>,
}

impl LocalQueryEmbedder {
    pub fn new(
        embedders: Arc<LocalEmbedders>,
        models: Arc<super::ModelService>,
        store: Arc<super::store::AiStore>,
    ) -> Self {
        Self {
            embedders,
            models,
            store,
        }
    }

    /// 의미 검색이 켜져 있고 e5 가 설치돼 있으면 모델을 미리 읽어 둔다 — 첫 하이브리드 질의가 모델 로드(P4 측정: 100k 라이브러리
    /// 에서 첫 질의 약 1.9초 vs 이후 약 0.15초)를 떠안지 않게 한다. 조건이 아니면 아무것도 하지 않고, 로드 실패는
    /// 첫 질의가 다시 시도하므로 무시한다. 호출자는 첫 화면을 방해하지 않도록 백그라운드 스레드에서 부른다.
    pub fn warm_up(&self) -> bool {
        use crate::search_hybrid::QueryEmbedder;
        self.semantic_key().is_some() && self.embedders.e5().is_ok()
    }

    fn key(kind: VectorKind, m: ModelInfo) -> mam_vector::ModelKey {
        mam_vector::ModelKey {
            kind,
            model_id: m.model_id,
            model_version: m.model_version,
            dimension: m.dimension,
        }
    }
}

impl crate::search_hybrid::QueryEmbedder for LocalQueryEmbedder {
    fn semantic_key(&self) -> Option<mam_vector::ModelKey> {
        (self.store.settings().semantic_search.enabled && self.models.is_ready(E5_MODEL_ID))
            .then(|| Self::key(VectorKind::Semantic, self.embedders.e5_info()))
    }

    fn visual_key(&self) -> Option<mam_vector::ModelKey> {
        (self.store.settings().semantic_search.enabled && self.models.is_ready(CLIP_IMAGE_MODEL_ID))
            .then(|| Self::key(VectorKind::Visual, self.embedders.image_info()))
    }

    fn embed_semantic_query(&self, text: &str) -> Result<Vec<f32>, String> {
        self.embedders
            .query_semantic(text)
            .map(|(_, v)| v)
            .map_err(|e| format!("{e:?}"))
    }
}

#[cfg(test)]
#[path = "embed_tests.rs"]
mod tests;
