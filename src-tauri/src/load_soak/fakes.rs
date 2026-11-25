//! 가짜 외부 AI·STT·임베딩 인코더·질의 임베더 — 실호출 없음. 저장 경로(`embed_visual`/`embed_semantic`/`EmbedStore::put`,
//! 분석·전사 저장, 사용량·이력 기록)는 제품 코드를 그대로 탄다.
//!
//! 추론 CPU 는 합성 busy-spin 으로 흉내 낸다. 값은 P1-6 실측(이 PC): CLIP 이미지 임베딩 약 83ms(2 스레드) + 디코딩·전처리 8ms,
//! e5 문서 임베딩 약 9ms. 모델 RSS(CLIP 약 400MB, e5 약 730~850MB)는 재현하지 않는다 — 실제 RSS 는 그만큼 더 크다.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_core::ai_jobs::{
    AiRuntimeSettings, AiSettingsSource, EmbedMode, EmbedRequest, EmbeddingRunner,
    ExternalAnalyzer, ExternalRequest, ExternalResult, ExternalUsage, StepFailure, StepKind,
    SttRequest, SttRunner,
};
use mam_core::ai_store::{TranscriptRecord, TranscriptSegment};
use mam_core::embedding::{
    EmbeddingError, ImageSource, ModelInfo, PASSAGE_PREFIX, TextEncoder, VisualEncoder,
    embed_semantic, embed_visual,
};
use mam_db::{DbPool, Priority};
use mam_vector::{EmbedStore, ModelKey};

use super::synth::{
    EN_WORDS, KO_WORDS, SEMANTIC_DIM, VISUAL_DIM, semantic_key, unit_vector, visual_key,
};
use crate::search_hybrid::QueryEmbedder;

/// 시각 임베딩 한 장의 합성 CPU 시간
pub const VISUAL_SPIN: Duration = Duration::from_millis(91);
/// 의미 임베딩 한 건의 합성 CPU 시간
pub const SEMANTIC_SPIN: Duration = Duration::from_millis(9);

pub fn spin(d: Duration) {
    let t = Instant::now();
    while t.elapsed() < d {
        std::hint::spin_loop();
    }
}

pub fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// 동의 완료, 외부 분석(설명·키워드)·STT·의미 임베딩 켬 — 실행 시점 설정 대역.
pub struct FixedSettings(pub AiRuntimeSettings);

impl FixedSettings {
    pub fn on() -> Self {
        Self(AiRuntimeSettings {
            consent_given: true,
            external_kinds: vec![StepKind::Describe, StepKind::Keywords],
            stt_enabled: true,
            semantic_enabled: true,
            ..AiRuntimeSettings::default()
        })
    }
}

impl AiSettingsSource for FixedSettings {
    fn snapshot(&self) -> AiRuntimeSettings {
        self.0.clone()
    }
}

/// 가짜 외부 AI — 네트워크 지연만 흉내(sleep)하고 결정적 결과를 돌려준다. 이미지는 열어 보지 않는다.
pub struct FakeExternal {
    pub latency: Duration,
    pub calls: AtomicU64,
}

impl ExternalAnalyzer for FakeExternal {
    fn provider(&self) -> String {
        "openai".into()
    }

    fn analyze(&self, req: &ExternalRequest) -> Result<ExternalResult, StepFailure> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        std::thread::sleep(self.latency);
        let a = EN_WORDS[(req.asset_id as usize) % EN_WORDS.len()];
        let b = KO_WORDS[(req.asset_id as usize / 7) % KO_WORDS.len()];
        Ok(ExternalResult {
            description: format!("{a} 장면 {b} synthetic description {}", req.asset_id),
            keywords: vec![a.to_string(), b.to_string(), "합성".to_string()],
            ocr_text: String::new(),
            summary: String::new(),
            language: "ko".into(),
            provider: "openai".into(),
            model: "fake-model".into(),
            usage: ExternalUsage {
                input_tokens: 800,
                output_tokens: 120,
                request_bytes: 40_000,
                images: u32::try_from(req.images.len()).unwrap_or(0),
                auth_mode: None,
            },
        })
    }
}

/// 가짜 STT — 로컬 엔진 대역(원본 음성을 어디로도 보내지 않는다). 짧은 지연 후 구간 3개.
pub struct FakeStt {
    pub latency: Duration,
    pub calls: AtomicU64,
}

impl SttRunner for FakeStt {
    fn transcribe(
        &self,
        req: &SttRequest,
        cancel: &dyn Fn() -> bool,
    ) -> Result<TranscriptRecord, StepFailure> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        std::thread::sleep(self.latency);
        if cancel() {
            return Err(StepFailure::new("cancelled", "cancelled"));
        }
        let w = KO_WORDS[(req.asset_id as usize) % KO_WORDS.len()];
        let seg = |i: u64, text: String| TranscriptSegment {
            start_ms: i * 1000,
            end_ms: i * 1000 + 900,
            text,
        };
        Ok(TranscriptRecord {
            language: "ko".into(),
            segments: vec![
                seg(0, format!("안녕하세요 {w} 입니다")),
                seg(1, "synthetic transcript segment".into()),
                seg(2, format!("감사합니다 {}", req.asset_id)),
            ],
            engine: "fake-whisper".into(),
            model: "fake".into(),
        })
    }
}

/// 이미 계산된 벡터를 돌려주는 인코더 (제품 `EmbeddingAdapter` 의 `Precomputed` 와 같은 방식 — 추론은 DB 쓰기 밖).
struct Pre {
    info: ModelInfo,
    out: Vec<Vec<f32>>,
}

impl VisualEncoder for Pre {
    fn model(&self) -> ModelInfo {
        self.info.clone()
    }
    fn encode_images(&self, _: &[PathBuf]) -> Result<Vec<Vec<f32>>, String> {
        Ok(self.out.clone())
    }
}

impl TextEncoder for Pre {
    fn model(&self) -> ModelInfo {
        self.info.clone()
    }
    fn encode_texts(&self, _: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(self.out.clone())
    }
}

fn info_of(key: &ModelKey) -> ModelInfo {
    ModelInfo {
        model_id: key.model_id.clone(),
        model_version: key.model_version.clone(),
        dimension: key.dimension,
    }
}

/// 임베딩 저장 지연 표본 (ms) — `EmbedStore::put` 포함 DB 쓰기 구간.
#[derive(Default)]
pub struct EmbedStats {
    pub visual_ms: Mutex<Vec<f64>>,
    pub semantic_ms: Mutex<Vec<f64>>,
    pub jobs: AtomicU64,
    pub failures: AtomicU64,
}

/// 합성 인코더 + 실제 저장 경로 임베딩 실행기.
pub struct SynthEmbedding {
    pub pool: Arc<DbPool>,
    pub clock: SharedClock,
    pub store: Arc<EmbedStore>,
    pub data_root: PathBuf,
    pub stats: Arc<EmbedStats>,
}

fn failure(e: &EmbeddingError) -> StepFailure {
    match e {
        EmbeddingError::Vector(_) | EmbeddingError::Db(_) => {
            StepFailure::new("io_transient", "embedding store failed")
        }
        _ => StepFailure::new("embedding_failed", "embedding failed"),
    }
}

impl EmbeddingRunner for SynthEmbedding {
    fn embed(&self, req: &EmbedRequest) -> Result<(), StepFailure> {
        let images: Vec<&mam_core::ai_jobs::ImageInput> = if req.mode == EmbedMode::SemanticOnly {
            Vec::new()
        } else {
            req.images.iter().collect()
        };
        // 1) 추론 흉내 (DB 쓰기 밖)
        let visual = (!images.is_empty()).then(|| {
            let out = images
                .iter()
                .map(|i| {
                    spin(VISUAL_SPIN);
                    let seed =
                        fnv(&i.path.to_string_lossy()) ^ i.frame.map_or(0, |f| u64::from(f.index));
                    unit_vector(seed, VISUAL_DIM)
                })
                .collect();
            Pre {
                info: info_of(&visual_key()),
                out,
            }
        });
        let parts: Vec<String> = req.text_parts.clone();
        let joined = parts
            .iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let semantic = {
            let out = if joined.is_empty() {
                Vec::new()
            } else {
                spin(SEMANTIC_SPIN);
                vec![unit_vector(
                    fnv(&format!("{PASSAGE_PREFIX}{joined}")),
                    SEMANTIC_DIM,
                )]
            };
            Pre {
                info: info_of(&semantic_key()),
                out,
            }
        };
        let sources: Vec<ImageSource> = images
            .iter()
            .map(|i| ImageSource {
                path: i.path.clone(),
                frame_index: i.frame_index(),
            })
            .collect();
        // 2) 저장 — 제품과 같은 `embed_visual` / `embed_semantic` (한 번의 쓰기 작업)
        let (store, root, asset_id, now) = (
            Arc::clone(&self.store),
            self.data_root.clone(),
            req.asset_id,
            mam_db::Clock::now_ms(&*self.clock),
        );
        let stats = Arc::clone(&self.stats);
        let stored = self
            .pool
            .write(Priority::Low, move |c| {
                let run = |c: &mut rusqlite::Connection| -> Result<(), EmbeddingError> {
                    if let Some(v) = &visual {
                        let t = Instant::now();
                        embed_visual(c, &store, v, &root, asset_id, &sources, now)?;
                        stats
                            .visual_ms
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(t.elapsed().as_secs_f64() * 1e3);
                    }
                    let t = Instant::now();
                    let text: Vec<&str> = parts.iter().map(String::as_str).collect();
                    embed_semantic(c, &store, &semantic, asset_id, &text, now)?;
                    stats
                        .semantic_ms
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(t.elapsed().as_secs_f64() * 1e3);
                    Ok(())
                };
                Ok(run(c))
            })
            .map_err(|_| StepFailure::new("io_transient", "embedding store failed"))?;
        match stored {
            Ok(()) => {
                self.stats.jobs.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(e) => {
                self.stats.failures.fetch_add(1, Ordering::Relaxed);
                Err(failure(&e))
            }
        }
    }
}

/// 합성 질의 임베더 — 질의 문자열 해시로 결정적 의미 벡터를 만든다 (실제 e5 질의 추론 약 10~13ms 는 포함하지 않는다).
pub struct SynthQuery;

impl QueryEmbedder for SynthQuery {
    fn semantic_key(&self) -> Option<ModelKey> {
        Some(semantic_key())
    }
    fn visual_key(&self) -> Option<ModelKey> {
        Some(visual_key())
    }
    fn embed_semantic_query(&self, text: &str) -> Result<Vec<f32>, String> {
        Ok(unit_vector(fnv(text), SEMANTIC_DIM))
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (결정성) / Business rule (질의 키가 저장 키와 같다)
    use super::*;

    #[test]
    fn t_p4_fake_01_query_embedder_matches_stored_model_key() {
        // Arrange
        let q = SynthQuery;
        // Act
        let a = q.embed_semantic_query("sunset beach").unwrap();
        let b = q.embed_semantic_query("sunset beach").unwrap();
        // Assert
        assert_eq!(a, b);
        assert_eq!(a.len(), SEMANTIC_DIM);
        assert_eq!(q.semantic_key().unwrap(), semantic_key());
    }

    #[test]
    fn t_p4_fake_02_spin_burns_at_least_the_requested_time() {
        let t = Instant::now();
        spin(Duration::from_millis(5));
        assert!(t.elapsed() >= Duration::from_millis(5));
    }
}
