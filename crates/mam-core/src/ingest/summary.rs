//! 작업 요약 (`job_summary` 반환·`jobs://progress` 페이로드).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::{Ingest, lock};
use crate::jobs::{JobCounts, JobKind};
use crate::scan::IngestError;

/// 자산 수 집계 캐시 수명 (진행률 이벤트가 4Hz 로 나가도 100만 건 COUNT 를 매번 하지 않는다)
const TOTAL_TTL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgressView {
    pub folder_id: i64,
    pub folder_path: String,
    pub processed: u64,
    /// 전체 파일 수를 아직 모르면 null (사전 계수는 하지 않는다 — 디스크를 두 번 읽게 된다)
    pub total: Option<u64>,
}

/// 프론트 계약 `JobSummary`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub paused: bool,
    pub paused_kinds: Vec<JobKind>,
    pub counts: BTreeMap<JobKind, JobCounts>,
    pub scan: Option<ScanProgressView>,
    pub total_assets: i64,
}

impl Ingest {
    pub fn summary(&self) -> Result<JobSummary, IngestError> {
        let counts = self.jobs.counts()?.into_iter().collect();
        let scan = self.visible_scan()?;
        Ok(JobSummary {
            paused: self.jobs.paused_all(),
            paused_kinds: self.jobs.paused_kinds(),
            counts,
            scan,
            total_assets: self.total_assets()?,
        })
    }

    /// 진행 중인 스캔 중 숨김 폴더가 아닌 첫 번째 — 숨김 폴더의 스캔은 상태 표시줄에 싣지 않는다.
    /// 진행 중인 스캔이 없으면 DB 를 읽지 않는다 (진행률 이벤트가 4Hz 로 돌아도 한가할 때는 비용 0).
    fn visible_scan(&self) -> Result<Option<ScanProgressView>, IngestError> {
        let running = self.env.progress.all();
        if running.is_empty() {
            return Ok(None);
        }
        let hidden = self
            .env
            .pool
            .read(|c| Ok(crate::scan::hidden_folder_ids(c)))??;
        Ok(running
            .into_iter()
            .find(|p| !hidden.contains(&(p.folder_id as u64)))
            .map(|p| ScanProgressView {
                folder_id: p.folder_id,
                folder_path: p.folder_path,
                processed: p.processed,
                total: None,
            }))
    }

    fn total_assets(&self) -> Result<i64, IngestError> {
        let mut cache = lock(&self.totals);
        let generation = self.jobs.generation();
        // 작업 상태가 바뀌면(스캔 완료 등) 캐시를 버린다 — 마지막 이벤트가 오래된 자산 수를 싣고 끝나지 않게
        if let Some((at, g, n)) = *cache
            && g == generation
            && at.elapsed() < TOTAL_TTL
        {
            return Ok(n);
        }
        // 별점 개수와 같은 기준(활성 위치 + present + 활성 자산 + 숨김 폴더 밖) — 고아 자산은 세지 않는다
        let n: i64 = self
            .env
            .pool
            .read(|c| Ok(c.query_row(crate::scan::TOTAL_COUNT_SQL, [], |r| r.get(0))))??;
        *cache = Some((Instant::now(), generation, n));
        Ok(n)
    }

    /// 상태 변화 번호 — 이 값이 바뀌었을 때만 진행률 이벤트를 만들면 된다.
    pub fn generation(&self) -> u64 {
        self.jobs.generation()
    }

    pub fn pause_jobs(&self, kind: Option<JobKind>) -> Result<JobSummary, IngestError> {
        self.jobs.pause(kind)?;
        self.summary()
    }

    pub fn resume_jobs(&self, kind: Option<JobKind>) -> Result<JobSummary, IngestError> {
        self.jobs.resume(kind)?;
        self.summary()
    }
}
