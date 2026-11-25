//! 로컬 모델 목록·다운로드 (PA6) — 매니페스트의 고정 URL, SHA-256 검증은 `mam_ai::models` 가 한다.
//!
//! 다운로드는 백그라운드 스레드에서 하고 `models://progress` 를 (최대 약 5Hz) 발행한다. 실패해도 해당 기능만
//! 비활성이며 이미 받은 모델·다른 기능에는 영향이 없다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_ai::AiError;
use mam_ai::models::{ModelKind, ModelSpec, ModelStatus, ModelStore, ids};
use serde::Serialize;

use super::AiFailure;
use super::events::{EventSink, MODELS_PROGRESS_EVENT};

const EMIT_EVERY: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelState {
    Missing,
    Downloading,
    Ready,
    Failed,
}

/// UI `LocalModel`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModel {
    pub model_id: String,
    pub name: String,
    pub purpose: String,
    pub size_bytes: u64,
    pub license: String,
    pub state: ModelState,
    pub progress: Option<f64>,
    pub error_code: Option<String>,
}

/// UI `ModelProgress`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelProgress {
    pub model_id: String,
    pub state: ModelState,
    pub downloaded: u64,
    pub total: u64,
}

#[derive(Debug, Clone)]
enum Live {
    Downloading { done: u64, total: u64 },
    Failed(String),
}

pub struct ModelService {
    store: Arc<ModelStore>,
    live: Mutex<HashMap<String, Live>>,
    sink: Arc<dyn EventSink>,
}

pub fn purpose_of(spec: &ModelSpec) -> String {
    match spec.kind {
        ModelKind::ImageEmbedding => "clip-image".into(),
        ModelKind::ClipTextEmbedding => "clip-text".into(),
        ModelKind::TextEmbedding => "e5".into(),
        ModelKind::Stt => spec.id.clone(),
    }
}

fn code_of(e: &AiError) -> &'static str {
    match e {
        AiError::Offline { .. } => "offline",
        AiError::InsufficientSpace { .. } => "disk_full",
        AiError::HashMismatch { .. } => "hash_mismatch",
        _ => "download_failed",
    }
}

impl ModelService {
    pub fn new(store: Arc<ModelStore>, sink: Arc<dyn EventSink>) -> Arc<Self> {
        Arc::new(Self {
            store,
            live: Mutex::new(HashMap::new()),
            sink,
        })
    }

    pub fn store(&self) -> &Arc<ModelStore> {
        &self.store
    }

    fn live(&self) -> std::sync::MutexGuard<'_, HashMap<String, Live>> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_ready(&self, model_id: &str) -> bool {
        matches!(self.store.status(model_id), Ok(ModelStatus::Ready(_)))
    }

    /// 로컬 임베딩 모델(CLIP 이미지·e5) 중 하나라도 준비됐나 — 의미 검색을 켜는 조건.
    pub fn any_embedding_ready(&self) -> bool {
        [ids::CLIP_VISION, ids::CLIP_TEXT_ML, ids::E5_SMALL]
            .iter()
            .any(|id| self.is_ready(id))
    }

    pub fn list(&self) -> Vec<LocalModel> {
        let live = self.live().clone();
        self.store
            .manifest()
            .models
            .iter()
            .map(|spec| {
                let ready = matches!(self.store.status(&spec.id), Ok(ModelStatus::Ready(_)));
                // 받는 중인 작업이 끝났다고 알리기(ready 이벤트) 전에는 파일이 이미 놓였어도 Downloading 으로 보인다 —
                // 목록이 Ready 인데 ready 이벤트가 아직 없는 순간(구독자와 목록 조회의 순서 역전)을 없앤다
                let (state, progress, error_code) = match (ready, live.get(&spec.id)) {
                    (true, None | Some(Live::Failed(_))) => (ModelState::Ready, None, None),
                    (_, Some(Live::Downloading { done, total })) => (
                        ModelState::Downloading,
                        Some(if *total > 0 {
                            *done as f64 / *total as f64
                        } else {
                            0.0
                        }),
                        None,
                    ),
                    (false, Some(Live::Failed(c))) => (ModelState::Failed, None, Some(c.clone())),
                    (false, None) => (ModelState::Missing, None, None),
                };
                LocalModel {
                    model_id: spec.id.clone(),
                    name: spec.display_name.clone(),
                    purpose: purpose_of(spec),
                    size_bytes: spec.total_size(),
                    license: spec.license.clone(),
                    state,
                    progress,
                    error_code,
                }
            })
            .collect()
    }

    /// 다운로드를 시작한다(이미 받는 중이거나 준비됐으면 아무것도 하지 않는다). 알 수 없는 ID 는 `NotFound`.
    pub fn start_download(self: &Arc<Self>, model_id: &str) -> Result<(), AiFailure> {
        let spec = self
            .store
            .manifest()
            .get(model_id)
            .map_err(|_| AiFailure::NotFound)?
            .clone();
        if self.is_ready(&spec.id) {
            return Ok(());
        }
        {
            let mut live = self.live();
            if matches!(live.get(&spec.id), Some(Live::Downloading { .. })) {
                return Ok(());
            }
            live.insert(
                spec.id.clone(),
                Live::Downloading {
                    done: 0,
                    total: spec.total_size(),
                },
            );
        }
        self.emit(&spec.id, ModelState::Downloading, 0, spec.total_size());
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name(format!("mam-model-{}", spec.id))
            .spawn(move || me.run_download(&spec))
            .map_err(|e| {
                self.live().remove(model_id);
                AiFailure::internal(&e)
            })?;
        Ok(())
    }

    fn run_download(&self, spec: &ModelSpec) {
        let total = spec.total_size();
        // 이전 파일들의 합 + 현재 파일 진행
        let sizes: Vec<(String, u64)> = spec
            .files
            .iter()
            .map(|f| (f.path.clone(), f.size))
            .collect();
        let mut last = Instant::now() - EMIT_EVERY;
        let result = self.store.ensure(&spec.id, &mut |file, received, _| {
            let before: u64 = sizes
                .iter()
                .take_while(|(p, _)| p != file)
                .map(|(_, s)| *s)
                .sum();
            let done = (before + received).min(total);
            if let Some(Live::Downloading { done: d, .. }) = self.live().get_mut(&spec.id) {
                *d = done;
            }
            if last.elapsed() >= EMIT_EVERY {
                last = Instant::now();
                self.emit(&spec.id, ModelState::Downloading, done, total);
            }
        });
        match result {
            Ok(_) => {
                // 이벤트를 먼저 낸 뒤 live 를 지운다 (목록이 Ready 로 바뀌는 시점 = 이벤트가 이미 나간 뒤)
                self.emit(&spec.id, ModelState::Ready, total, total);
                self.live().remove(&spec.id);
            }
            Err(e) => {
                let code = code_of(&e);
                tracing::warn!(model = %spec.id, code, "model download failed");
                self.live()
                    .insert(spec.id.clone(), Live::Failed(code.to_string()));
                self.emit(&spec.id, ModelState::Failed, 0, total);
            }
        }
    }

    fn emit(&self, id: &str, state: ModelState, downloaded: u64, total: u64) {
        let payload = ModelProgress {
            model_id: id.to_string(),
            state,
            downloaded,
            total,
        };
        if let Ok(v) = serde_json::to_value(payload) {
            self.sink.emit(MODELS_PROGRESS_EVENT, v);
        }
    }
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
