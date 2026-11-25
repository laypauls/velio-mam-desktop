//! AI 분석 작업(P3-1/P3-3) 통합 테스트 환경 - 실제 SQLite(마이그레이션 + 제안 DDL) + 가짜 외부 AI·STT·임베딩.
//! 테스트 파일은 `mod common;` 뒤에 `#[path = "common/ai_env.rs"] mod ai_env;` 로 포함한다.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::common::env::Env;
use mam_core::ai_jobs::*;
use mam_core::ai_store::{TranscriptRecord, TranscriptSegment};
use mam_core::jobs::{Job, JobError, JobKind, JobOutcome, JobQueue};
use mam_core::{SharedClock, VolumeInfo, VolumeKind, register_volume};
use mam_db::Priority;
use mam_media::{ThumbSize, strip_path, thumbnail_path};

pub type Log = Arc<Mutex<Vec<String>>>;

pub struct FakeSettings(pub Mutex<AiRuntimeSettings>);
impl AiSettingsSource for FakeSettings {
    fn snapshot(&self) -> AiRuntimeSettings {
        self.0.lock().unwrap().clone()
    }
}

pub fn ok_external() -> ExternalResult {
    ExternalResult {
        description: "a red car".into(),
        keywords: vec!["car".into(), "red".into()],
        ocr_text: "STOP".into(),
        summary: "car photo".into(),
        language: "ko".into(),
        provider: "openai".into(),
        model: "gpt-x".into(),
        usage: ExternalUsage {
            input_tokens: 100,
            output_tokens: 20,
            request_bytes: 5000,
            images: 1,
            auth_mode: None,
        },
    }
}

pub fn transcript() -> TranscriptRecord {
    TranscriptRecord {
        language: "ko".into(),
        segments: vec![TranscriptSegment {
            start_ms: 0,
            end_ms: 1000,
            text: "hello world".into(),
        }],
        engine: "whisper.cpp".into(),
        model: "ggml-base".into(),
    }
}

pub struct FakeExternal {
    pub script: Mutex<VecDeque<Result<ExternalResult, StepFailure>>>,
    pub calls: Mutex<Vec<ExternalRequest>>,
    pub log: Log,
    /// `ExternalAnalyzer::auth_mode()` 가 돌려줄 값 (기본 None)
    pub auth_mode: Mutex<Option<String>>,
    /// `ExternalAnalyzer::paused_until_ms()` 가 돌려줄 값 (기본 None)
    pub paused_until: Mutex<Option<i64>>,
}
impl ExternalAnalyzer for FakeExternal {
    fn paused_until_ms(&self) -> Option<i64> {
        *self.paused_until.lock().unwrap()
    }
    fn provider(&self) -> String {
        "openai".into()
    }
    fn auth_mode(&self) -> Option<String> {
        self.auth_mode.lock().unwrap().clone()
    }
    fn analyze(&self, req: &ExternalRequest) -> Result<ExternalResult, StepFailure> {
        self.log.lock().unwrap().push("external".into());
        self.calls.lock().unwrap().push(req.clone());
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(ok_external()))
    }
}

pub struct FakeStt {
    pub script: Mutex<VecDeque<Result<TranscriptRecord, StepFailure>>>,
    pub calls: Mutex<Vec<SttRequest>>,
    pub log: Log,
}
impl SttRunner for FakeStt {
    fn transcribe(
        &self,
        req: &SttRequest,
        _cancel: &dyn Fn() -> bool,
    ) -> Result<TranscriptRecord, StepFailure> {
        self.log.lock().unwrap().push("stt".into());
        self.calls.lock().unwrap().push(req.clone());
        self.script
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(transcript()))
    }
}

pub struct FakeEmb {
    pub script: Mutex<VecDeque<Result<(), StepFailure>>>,
    pub calls: Mutex<Vec<EmbedRequest>>,
    pub log: Log,
    /// 켜면(`AiEnv::enable_vectors`) 실제 벡터 저장소에 쓴다
    pub real: Mutex<Option<Arc<RealEmb>>>,
}
impl EmbeddingRunner for FakeEmb {
    fn embed(&self, req: &EmbedRequest) -> Result<(), StepFailure> {
        self.log.lock().unwrap().push("embedding".into());
        self.calls.lock().unwrap().push(req.clone());
        let scripted = self.script.lock().unwrap().pop_front().unwrap_or(Ok(()));
        let real = self.real.lock().unwrap().clone();
        match (scripted, real) {
            (Ok(()), Some(r)) => r.run(req).map_err(|m| StepFailure::new("invalid_input", m)),
            (s, _) => s,
        }
    }
}

pub struct AiEnv {
    pub env: Env,
    pub cache: PathBuf,
    pub settings: Arc<FakeSettings>,
    pub ext: Arc<FakeExternal>,
    pub stt: Arc<FakeStt>,
    pub emb: Arc<FakeEmb>,
    pub log: Log,
    pub exec: AiExecutor,
    pub queue: Arc<JobQueue>,
    pub sched: AiScheduler,
    volume_id: i64,
    n: Mutex<u32>,
}

/// 동의함 + 외부 종류 전부 + STT·의미 검색 켬 (대부분의 시험이 쓰는 기본).
pub fn full_settings() -> AiRuntimeSettings {
    AiRuntimeSettings {
        consent_given: true,
        external_kinds: vec![
            StepKind::Describe,
            StepKind::Keywords,
            StepKind::Ocr,
            StepKind::Summary,
        ],
        semantic_enabled: true,
        ..AiRuntimeSettings::default()
    }
}

impl AiEnv {
    pub fn new() -> Self {
        let env = Env::new();
        let cache = env.data.join("cache");
        fs::create_dir_all(&cache).unwrap();
        let clock: SharedClock = env.clock.clone();
        let vinfo = VolumeInfo {
            key: "ai-vol".into(),
            kind: VolumeKind::Fixed,
            label: None,
            mount_point: env.vol.to_string_lossy().into_owned(),
        };
        let volume_id = env
            .pool
            .write(Priority::Low, {
                let clock = clock.clone();
                move |c| Ok(register_volume(c, &*clock, &vinfo).unwrap())
            })
            .unwrap();
        let log: Log = Arc::default();
        let settings = Arc::new(FakeSettings(Mutex::new(full_settings())));
        let ext = Arc::new(FakeExternal {
            script: Mutex::default(),
            calls: Mutex::default(),
            log: log.clone(),
            auth_mode: Mutex::default(),
            paused_until: Mutex::default(),
        });
        let stt = Arc::new(FakeStt {
            script: Mutex::default(),
            calls: Mutex::default(),
            log: log.clone(),
        });
        let emb = Arc::new(FakeEmb {
            script: Mutex::default(),
            calls: Mutex::default(),
            log: log.clone(),
            real: Mutex::default(),
        });
        let cfg = AiJobsConfig::new(cache.clone());
        let queue =
            JobQueue::with_concurrency(Arc::clone(&env.pool), clock.clone(), cfg.queue_caps());
        let exec = AiExecutor::new(
            AiDeps {
                pool: Arc::clone(&env.pool),
                clock: clock.clone(),
                settings: settings.clone(),
                external: ext.clone(),
                stt: stt.clone(),
                embedding: emb.clone(),
            },
            cfg,
        );
        let sched = AiScheduler::new(Arc::clone(&env.pool), Arc::clone(&queue), clock, 200);
        Self {
            env,
            cache,
            settings,
            ext,
            stt,
            emb,
            log,
            exec,
            queue,
            sched,
            volume_id,
            n: Mutex::new(0),
        }
    }

    /// DB 쓰기 스레드에서 `&mut Connection` + 시계로 실행한다.
    pub fn w<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut rusqlite::Connection, &dyn mam_db::Clock) -> T + Send + 'static,
    ) -> T {
        let clk = self.env.clock.clone();
        self.env
            .pool
            .write(Priority::Low, move |c| {
                Ok::<_, mam_db::DbError>(f(c, &*clk))
            })
            .unwrap()
    }

    pub fn set(&self, f: impl FnOnce(&mut AiRuntimeSettings)) {
        f(&mut self.settings.0.lock().unwrap());
    }

    /// 임베딩 어댑터를 실제 벡터 저장소(`embeddings` + `EmbedStore`)에 연결한다.
    pub fn enable_vectors(&self) {
        let store = Arc::new(
            mam_vector::EmbedStore::new(self.env.data.join("indexes").join("vectors")).unwrap(),
        );
        *self.emb.real.lock().unwrap() = Some(Arc::new(RealEmb {
            pool: Arc::clone(&self.env.pool),
            store,
            data_root: self.env.data.clone(),
        }));
    }

    /// 활성(대체되지 않은) 벡터의 `embedding_id` 목록. `kind` = visual | semantic.
    pub fn vector_ids(&self, asset: i64, kind: &str) -> Vec<i64> {
        self.env.read(|c| {
            let mut s = c
                .prepare(
                    "SELECT embedding_id FROM embeddings
                     WHERE asset_id = ?1 AND vector_kind = ?2 AND superseded_at IS NULL ORDER BY embedding_id",
                )
                .unwrap();
            s.query_map((asset, kind), |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        })
    }

    /// 활성 시각 벡터의 (source, frame_index) 목록.
    pub fn visual_sources(&self, asset: i64) -> Vec<(String, Option<i64>)> {
        self.env.read(|c| {
            let mut s = c
                .prepare(
                    "SELECT source, frame_index FROM embeddings
                     WHERE asset_id = ?1 AND vector_kind = 'visual' AND superseded_at IS NULL ORDER BY embedding_id",
                )
                .unwrap();
            s.query_map([asset], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        })
    }

    /// 활성 자산 + 원본 파일 + (영상·음성) 길이 메타 + (이미지·영상) 큰 썸네일.
    pub fn add_asset(&self, media: &str) -> i64 {
        let idx = {
            let mut n = self.n.lock().unwrap();
            *n += 1;
            *n
        };
        let rel = format!("ai_{idx}.bin");
        fs::write(self.env.vol.join(&rel), [idx as u8; 16]).unwrap();
        let (media_s, vol) = (media.to_string(), self.volume_id);
        let id: i64 = self
            .env
            .pool
            .write(Priority::Low, move |c| {
                let id: i64 = c.query_row(
                    "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
                     VALUES (?1, 16, 1, 1, 1) RETURNING asset_id",
                    [&media_s],
                    |r| r.get(0),
                )?;
                c.execute(
                    "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
                     VALUES (?1, ?2, ?3, ?3, 1)",
                    rusqlite::params![id, vol, rel],
                )?;
                if media_s != "image" {
                    c.execute(
                        "INSERT INTO media_metadata (asset_id, duration_ms) VALUES (?1, 60000)",
                        [id],
                    )?;
                }
                Ok(id)
            })
            .unwrap();
        if media != "audio" {
            self.put_thumb(id);
        }
        if media == "video" {
            self.set_strip(id, Some("done"), true);
        }
        id
    }

    pub fn put_thumb(&self, id: i64) {
        let p = thumbnail_path(&self.cache, id, ThumbSize::Large);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"webp").unwrap();
    }

    pub fn remove_thumb(&self, id: i64) {
        let _ = fs::remove_file(thumbnail_path(&self.cache, id, ThumbSize::Large));
    }

    /// `asset_derivatives(thumb)` 상태 (`None` 이면 행 삭제 = 아직 안 만들어짐). 파일은 건드리지 않는다.
    pub fn set_thumb_state(&self, id: i64, state: Option<&'static str>) {
        self.w(move |c, _| {
            c.execute(
                "DELETE FROM asset_derivatives WHERE asset_id = ?1 AND kind = 'thumb'",
                [id],
            )
            .unwrap();
            if let Some(s) = state {
                c.execute(
                    "INSERT INTO asset_derivatives (asset_id, kind, state, updated_at)
                     VALUES (?1, 'thumb', ?2, 1)",
                    rusqlite::params![id, s],
                )
                .unwrap();
            }
        });
    }

    /// 영상 프레임 스트립 파일 + `asset_derivatives(strip)` 상태. `state` 가 `None` 이면 행을 지운다 (아직 안 만들어짐).
    pub fn set_strip(&self, id: i64, state: Option<&'static str>, with_file: bool) {
        let p = strip_path(&self.cache, id);
        if with_file {
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, b"webp-strip").unwrap();
        } else {
            let _ = fs::remove_file(&p);
        }
        self.w(move |c, _| {
            c.execute(
                "DELETE FROM asset_derivatives WHERE asset_id = ?1 AND kind = 'strip'",
                [id],
            )
            .unwrap();
            if let Some(s) = state {
                c.execute(
                    "INSERT INTO asset_derivatives (asset_id, kind, state, updated_at)
                     VALUES (?1, 'strip', ?2, 1)",
                    rusqlite::params![id, s],
                )
                .unwrap();
            }
        });
    }

    /// 대기열에서 AI 작업 1건을 꺼내 실행하고 결과를 큐에 반영한다. 대기 작업이 없으면 `None`.
    pub fn run_next(&self, stop: &dyn Fn() -> bool) -> Option<Result<JobOutcome, JobError>> {
        let job: Job = self.queue.claim_next(&[JobKind::Ai]).unwrap()?;
        let res = self.exec.run(&job, stop);
        self.queue.finish(&job, res.clone()).unwrap();
        Some(res)
    }

    pub fn step(&self, asset: i64, kind: &str) -> Option<(String, Option<String>)> {
        self.env.read(|c| {
            c.query_row(
                "SELECT state, error_code FROM ai_task_state WHERE asset_id = ?1 AND kind = ?2",
                (asset, kind),
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
        })
    }

    pub fn st(&self, asset: i64, kind: &str) -> String {
        self.step(asset, kind).map_or("none".into(), |s| s.0)
    }

    pub fn order(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// 자산에 모든 단계를 등록하고 작업을 큐에 넣는다.
    pub fn queue_all(&self, asset: i64, kinds: Option<&[StepKind]>, force: bool) -> QueueResult {
        let st = self.settings.snapshot();
        self.sched
            .analyze_assets(&[asset], kinds, force, &st)
            .unwrap()
    }
}

/// 실제 `embedding::embed_visual` / `embed_semantic` + `EmbedStore` 위에서 동작하는 임베딩 어댑터 (결정적 가짜 인코더).
/// 시각: `Full` 일 때 이미지(프레임)마다 벡터 하나. 의미: 항상 다시 만든다.
pub struct RealEmb {
    pool: Arc<mam_db::DbPool>,
    store: Arc<mam_vector::EmbedStore>,
    data_root: PathBuf,
}

struct EncV;
struct EncT;
fn vec16(seed: u8) -> Vec<f32> {
    (0..16).map(|i| f32::from(seed) + 1.0 + i as f32).collect()
}
impl mam_core::embedding::VisualEncoder for EncV {
    fn model(&self) -> mam_core::embedding::ModelInfo {
        mam_core::embedding::ModelInfo {
            model_id: "clip-fake".into(),
            model_version: "1".into(),
            dimension: 16,
        }
    }
    fn encode_images(&self, paths: &[PathBuf]) -> Result<Vec<Vec<f32>>, String> {
        Ok(paths.iter().map(|_| vec16(1)).collect())
    }
}
impl mam_core::embedding::TextEncoder for EncT {
    fn model(&self) -> mam_core::embedding::ModelInfo {
        mam_core::embedding::ModelInfo {
            model_id: "e5-fake".into(),
            model_version: "1".into(),
            dimension: 16,
        }
    }
    fn encode_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(texts.iter().map(|t| vec16(t.len() as u8)).collect())
    }
}

impl RealEmb {
    fn run(&self, req: &EmbedRequest) -> Result<(), String> {
        use mam_core::embedding::{ImageSource, embed_semantic, embed_visual};
        let (id, mode) = (req.asset_id, req.mode);
        let images: Vec<ImageSource> = req
            .images
            .iter()
            .map(|i| ImageSource {
                path: i.path.clone(),
                frame_index: i.frame_index(),
            })
            .collect();
        let (parts, store, root) = (
            req.text_parts.clone(),
            Arc::clone(&self.store),
            self.data_root.clone(),
        );
        self.pool
            .write(Priority::Low, move |c| {
                let now = 1_700_000_000_000;
                let r = (|| -> Result<(), String> {
                    if mode == EmbedMode::Full && !images.is_empty() {
                        embed_visual(c, &store, &EncV, &root, id, &images, now)
                            .map_err(|e| e.to_string())?;
                    }
                    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
                    embed_semantic(c, &store, &EncT, id, &refs, now).map_err(|e| e.to_string())?;
                    Ok(())
                })();
                Ok::<_, mam_db::DbError>(r)
            })
            .map_err(|e| e.to_string())?
    }
}
