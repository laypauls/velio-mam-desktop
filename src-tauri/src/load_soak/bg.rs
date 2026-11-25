//! AI 백그라운드 틱 대역 — 제품 `AiRunner`(신규 자산 자동 분석 + 벡터 인덱스 세대 동기화, 5초 주기)를 같은 호출로 재현한다.
//!
//! `AiService`(Tauri 상태·모델 관리 포함)는 올리지 않고, 스케줄러(`AiScheduler`)·벡터 세대(`Generations`)를 직접 부른다.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mam_core::jobs::JobKind;
use mam_vector::{Generations, ModelKey, OpenMode, sync_incremental};

use super::bench_env::Bench;
use super::synth::{semantic_key, visual_key};

/// 대기 중 AI 작업이 이만큼 이상이면 더 등록하지 않는다 (백그라운드 스윕의 `sweep_batch` 와 같은 값)
const AI_BACKLOG_CAP: u64 = 200;
const FEED_BATCH: i64 = 100;

#[derive(Default)]
pub struct BgStats {
    pub fed: AtomicU64,
    pub feed_errors: AtomicU64,
    pub sync_runs: AtomicU64,
    pub sync_errors: AtomicU64,
    /// 인덱스 세대 동기화 한 번의 시간 (ms)
    pub sync_ms: Mutex<Vec<f64>>,
}

pub struct Background {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    pub stats: Arc<BgStats>,
}

/// 썸네일이 끝났고(음성은 미지원) 아직 AI 단계가 없는 자산을 등록한다. 등록한 수.
pub fn feed_once(b: &Bench, floor: i64) -> Result<usize, String> {
    let ai_pending = b
        .job_counts()
        .iter()
        .find(|(k, _)| *k == JobKind::Ai)
        .map_or(0, |(_, c)| c.pending);
    if ai_pending >= AI_BACKLOG_CAP {
        return Ok(0);
    }
    let ids: Vec<i64> = b
        .pool
        .read(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT d.asset_id FROM asset_derivatives d
                 WHERE d.kind = 'thumb' AND d.state IN ('done', 'unsupported') AND d.asset_id > ?1
                   AND NOT EXISTS (SELECT 1 FROM ai_task_state s WHERE s.asset_id = d.asset_id)
                 ORDER BY d.asset_id LIMIT ?2",
            )?;
            let rows = stmt.query_map([floor, FEED_BATCH], |r| r.get::<_, i64>(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
        .map_err(|e| e.to_string())?;
    if ids.is_empty() {
        return Ok(0);
    }
    let r = b
        .scheduler
        .analyze_assets(&ids, None, false, &b.rt)
        .map_err(|e| e.to_string())?;
    Ok(usize::try_from(r.queued).unwrap_or(0))
}

/// 제품 `AiService::sync_key` 와 같은 순서: 활성 세대를 열거나 재구축 → 증분 추가 → 새 벡터가 있으면 저장.
pub fn sync_key(b: &Bench, key: &ModelKey, last_max: &mut i64) -> Result<bool, String> {
    let (kind, id, ver) = (
        key.kind.as_str(),
        key.model_id.clone(),
        key.model_version.clone(),
    );
    let max: i64 = b
        .pool
        .read(move |c| {
            Ok(c.query_row(
                "SELECT COALESCE(MAX(embedding_id), 0) FROM embeddings
                 WHERE vector_kind = ?1 AND model_id = ?2 AND model_version = ?3 AND superseded_at IS NULL",
                rusqlite::params![kind, id, ver],
                |r| r.get(0),
            )?)
        })
        .map_err(|e| e.to_string())?;
    if max == 0 || max == *last_max {
        return Ok(false);
    }
    let (store, k2) = (Arc::clone(&b.store), key.clone());
    let r = b
        .pool
        .read(move |c| {
            Ok((|| -> mam_vector::Result<()> {
                let gens = Generations::new(&store, &k2)?;
                let (idx, rebuilt) = gens.open_or_rebuild(c, &store, &k2, OpenMode::Load)?;
                let report = sync_incremental(c, &store, &k2, &idx)?;
                if rebuilt.is_some() || report.read > 0 {
                    gens.persist(&k2, &idx)?;
                }
                Ok(())
            })())
        })
        .map_err(|e| e.to_string())?;
    r.map_err(|e| e.to_string())?;
    *last_max = max;
    Ok(true)
}

impl Background {
    /// `floor` 이하 asset_id(시작 전 라이브러리)는 자동 분석 대상이 아니다. `feed` 가 꺼져 있으면 벡터 동기화만 한다.
    pub fn start(b: &Arc<Bench>, floor: i64, feed: bool) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(BgStats::default());
        let mut threads = Vec::new();
        if feed {
            let (b, stop, stats) = (Arc::clone(b), Arc::clone(&stop), Arc::clone(&stats));
            threads.push(
                std::thread::Builder::new()
                    .name("p4-ai-feed".into())
                    .spawn(move || {
                        while !stop.load(Ordering::Relaxed) {
                            match feed_once(&b, floor) {
                                Ok(n) => {
                                    stats.fed.fetch_add(n as u64, Ordering::Relaxed);
                                }
                                Err(_) => {
                                    stats.feed_errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    })
                    .expect("spawn feeder"),
            );
        }
        {
            let (b, stop, stats) = (Arc::clone(b), Arc::clone(&stop), Arc::clone(&stats));
            threads.push(
                std::thread::Builder::new()
                    .name("p4-vec-sync".into())
                    .spawn(move || {
                        let (mut m_sem, mut m_vis) = (0i64, 0i64);
                        while !stop.load(Ordering::Relaxed) {
                            for (key, last) in
                                [(semantic_key(), &mut m_sem), (visual_key(), &mut m_vis)]
                            {
                                let t = Instant::now();
                                match sync_key(&b, &key, last) {
                                    Ok(true) => {
                                        stats.sync_runs.fetch_add(1, Ordering::Relaxed);
                                        stats
                                            .sync_ms
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner())
                                            .push(t.elapsed().as_secs_f64() * 1e3);
                                    }
                                    Ok(false) => {}
                                    Err(_) => {
                                        stats.sync_errors.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            // 제품 틱 간격 5초 (종료 신호에 빨리 반응하도록 쪼갠다)
                            for _ in 0..50 {
                                if stop.load(Ordering::Relaxed) {
                                    break;
                                }
                                std::thread::sleep(Duration::from_millis(100));
                            }
                        }
                    })
                    .expect("spawn vector sync"),
            );
        }
        Self {
            stop,
            threads,
            stats,
        }
    }

    pub fn stop(self) -> Arc<BgStats> {
        self.stop.store(true, Ordering::Relaxed);
        for t in self.threads {
            let _ = t.join();
        }
        self.stats
    }
}
