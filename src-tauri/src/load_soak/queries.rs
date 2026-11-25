//! 고정 시드 질의 집합과 실행기 — `SearchService::search` 는 `search_assets` command 가 부르는 바로 그 진입점이다.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mam_core::query::{
    FolderScope, MediaKind, SearchFilters, SearchMode, SearchRequest, SortDirection, SortField,
    SortSpec,
};

use super::stats::{Lcg, Summary, summarize};
use super::synth::{EN_WORDS, KO_WORDS, TAG_NAMES};
use crate::search_service::SearchService;

pub const KINDS: [&str; 12] = [
    "filename partial (en)",
    "filename partial (ko)",
    "korean word",
    "tag as text",
    "tag filter, modified desc",
    "filter combo (video+date+online+size, captured desc)",
    "folder scope + keyword, name asc",
    "browse, captured desc (default gallery)",
    "keyword, relevance sort",
    "keyword, captured-desc sort (same words)",
    "hybrid (synthetic e5 query vector)",
    "browse page 2 via cursor",
];
pub const HYBRID: usize = 10;
const PAGE2: usize = 11;
/// 2021-01-01 .. 2025-01-01 (ms)
const FROM_2021: i64 = 1_609_459_200_000;
const TO_2025: i64 = 1_735_689_600_000;

pub struct Ctx {
    pub folder_id: i64,
}

fn base_filters() -> SearchFilters {
    SearchFilters {
        media_types: vec![],
        include_offline: true,
        include_missing: false,
        captured_from: None,
        captured_to: None,
        size_min: None,
        size_max: None,
        duration_min_ms: None,
        duration_max_ms: None,
        folder: None,
        favorites_only: false,
        collection_id: None,
        tag_ids: vec![],
        rating_min: None,
        rating_max: None,
        user_fields: vec![],
    }
}

fn request(
    text: &str,
    filters: SearchFilters,
    field: SortField,
    dir: SortDirection,
) -> SearchRequest {
    SearchRequest {
        query: text.to_string(),
        filters,
        sort: SortSpec {
            field,
            direction: dir,
        },
        limit: 50,
        cursor: None,
        mode: None,
        direction: None,
        at: None,
    }
}

fn pick_str(rng: &mut Lcg, v: &[&'static str]) -> &'static str {
    v[rng.below(v.len())]
}
fn word(rng: &mut Lcg) -> String {
    if rng.below(2) == 0 {
        pick_str(rng, &EN_WORDS).to_string()
    } else {
        pick_str(rng, &KO_WORDS).to_string()
    }
}

/// 종류 `kind` 의 요청 하나 (시드 `rng` 에서 결정적).
pub fn build(kind: usize, rng: &mut Lcg, ctx: &Ctx) -> SearchRequest {
    use SortDirection::{Asc, Desc};
    use SortField::{CapturedAt, ModifiedAt, Name, Relevance};
    match kind {
        0 => {
            let w = pick_str(rng, &EN_WORDS);
            let len = 3 + rng.below(3);
            let start = rng.below(w.len().saturating_sub(len) + 1);
            request(
                &w[start..(start + len).min(w.len())],
                base_filters(),
                Relevance,
                Desc,
            )
        }
        1 => {
            let w = pick_str(rng, &KO_WORDS);
            let chars: Vec<char> = w.chars().collect();
            let start = rng.below(chars.len().saturating_sub(2) + 1);
            let part: String = chars.iter().skip(start).take(2).collect();
            request(&part, base_filters(), Relevance, Desc)
        }
        2 => request(pick_str(rng, &KO_WORDS), base_filters(), Relevance, Desc),
        3 => request(pick_str(rng, &TAG_NAMES), base_filters(), Relevance, Desc),
        4 => {
            let mut f = base_filters();
            f.tag_ids = vec![1 + rng.below(TAG_NAMES.len()) as i64];
            request("", f, ModifiedAt, Desc)
        }
        5 => {
            let mut f = base_filters();
            f.media_types = vec![MediaKind::Video];
            f.include_offline = false;
            f.captured_from = Some(FROM_2021);
            f.captured_to = Some(TO_2025);
            f.size_min = Some(1_000_000);
            request(pick_str(rng, &EN_WORDS), f, CapturedAt, Desc)
        }
        6 => {
            let mut f = base_filters();
            f.folder = Some(FolderScope {
                folder_id: ctx.folder_id,
                rel_path: Some(format!("{}/{:02}", 2018 + rng.below(9), 1 + rng.below(12))),
                recursive: true,
            });
            request(pick_str(rng, &EN_WORDS), f, Name, Asc)
        }
        7 | PAGE2 => request("", base_filters(), CapturedAt, Desc),
        8 => request(
            &format!("{} {}", rng.pick(&EN_WORDS), rng.pick(&KO_WORDS)),
            base_filters(),
            Relevance,
            Desc,
        ),
        9 => request(
            &format!("{} {}", rng.pick(&EN_WORDS), rng.pick(&KO_WORDS)),
            base_filters(),
            CapturedAt,
            Desc,
        ),
        _ => {
            let mut r = request(&word(rng), base_filters(), Relevance, Desc);
            r.query = format!("{} {}", r.query, word(rng));
            r.mode = Some(SearchMode::Hybrid);
            r
        }
    }
}

/// 측정 결과 묶음.
#[derive(Default)]
pub struct Report {
    pub samples: Vec<Vec<f64>>,
    /// (경과 초, 종류, ms)
    pub timeline: Vec<(f64, usize, f64)>,
    pub errors: u64,
    pub first_errors: Vec<String>,
    /// 하이브리드 요청이 키워드로 되돌아간 횟수 (`degraded`) — 0 이 아니면 하이브리드 수치는 무효
    pub hybrid_degraded: u64,
    pub hybrid_total: u64,
    pub wall_s: f64,
}

impl Report {
    pub fn new() -> Self {
        Self {
            samples: vec![Vec::new(); KINDS.len()],
            ..Self::default()
        }
    }

    pub fn summary(&self, kind: usize) -> Summary {
        summarize(&self.samples[kind])
    }

    pub fn all(&self) -> Summary {
        let v: Vec<f64> = self.samples.iter().flatten().copied().collect();
        summarize(&v)
    }

    pub fn total(&self) -> usize {
        self.samples.iter().map(Vec::len).sum()
    }

    fn note_error(&mut self, e: String) {
        self.errors += 1;
        if self.first_errors.len() < 5 && !self.first_errors.contains(&e) {
            self.first_errors.push(e);
        }
    }

    /// 가장 느린 `window_s` 초 구간의 P95 (구간 안 표본이 `min_n` 이상일 때만).
    pub fn worst_window_p95(&self, window_s: f64, min_n: usize) -> Option<(f64, f64, usize)> {
        let end = self.timeline.last()?.0;
        let mut best: Option<(f64, f64, usize)> = None;
        let mut start = 0.0;
        while start < end {
            let v: Vec<f64> = self
                .timeline
                .iter()
                .filter(|(t, _, _)| *t >= start && *t < start + window_s)
                .map(|(_, _, ms)| *ms)
                .collect();
            if v.len() >= min_n {
                let p95 = summarize(&v).p95;
                if best.is_none_or(|b| p95 > b.1) {
                    best = Some((start, p95, v.len()));
                }
            }
            start += window_s;
        }
        best
    }
}

/// 요청 1건을 실행하고 지연(ms)을 기록한다. 하이브리드가 키워드로 되돌아가면 센다.
fn run_one(
    search: &SearchService,
    kind: usize,
    req: &SearchRequest,
    rep: &mut Report,
    t0: Instant,
) {
    let timed = |req: &SearchRequest| {
        let t = Instant::now();
        let r = search.search(req);
        (t.elapsed().as_secs_f64() * 1e3, r)
    };
    let (ms, res) = if kind == PAGE2 {
        match search.search(req) {
            Ok(first) => {
                let mut next = req.clone();
                next.cursor = first.next_cursor;
                timed(&next)
            }
            Err(e) => (0.0, Err(e)),
        }
    } else {
        timed(req)
    };
    match res {
        Ok(page) => {
            if kind == HYBRID {
                rep.hybrid_total += 1;
                // 키워드로 되돌아간 응답은 하이브리드 지연이 아니다 — 표본에서 빼고 횟수만 센다
                if page.degraded.is_some() {
                    rep.hybrid_degraded += 1;
                    return;
                }
            }
            rep.samples[kind].push(ms);
            rep.timeline.push((t0.elapsed().as_secs_f64(), kind, ms));
        }
        Err(e) => rep.note_error(format!("kind {kind}: {e:?}")),
    }
}

/// 종류마다 `warmup` + `per_kind` 회, 종류를 번갈아 실행한다 (유휴 상태 측정).
pub fn run_fixed(
    search: &SearchService,
    ctx: &Ctx,
    per_kind: usize,
    warmup: usize,
    seed: u64,
) -> Report {
    let mut rng = Lcg(seed);
    let mut warm = Report::new();
    let t0 = Instant::now();
    for _ in 0..warmup {
        for k in 0..KINDS.len() {
            let req = build(k, &mut rng, ctx);
            run_one(search, k, &req, &mut warm, t0);
        }
    }
    let mut rep = Report::new();
    let t0 = Instant::now();
    for _ in 0..per_kind {
        for k in 0..KINDS.len() {
            let req = build(k, &mut rng, ctx);
            run_one(search, k, &req, &mut rep, t0);
        }
    }
    rep.wall_s = t0.elapsed().as_secs_f64();
    rep.errors += warm.errors;
    rep.first_errors.extend(warm.first_errors);
    rep
}

/// `stop` 이 켜질 때까지 종류를 번갈아 실행한다 (부하 중 측정; `think` = 질의 사이 대기, UI 입력 간격 흉내).
pub fn run_until(
    search: &SearchService,
    ctx: &Ctx,
    stop: &AtomicBool,
    think: Duration,
    seed: u64,
) -> Report {
    let mut rng = Lcg(seed);
    let mut rep = Report::new();
    let t0 = Instant::now();
    let mut k = 0;
    while !stop.load(Ordering::Relaxed) {
        let req = build(k % KINDS.len(), &mut rng, ctx);
        run_one(search, k % KINDS.len(), &req, &mut rep, t0);
        k += 1;
        std::thread::sleep(think);
    }
    rep.wall_s = t0.elapsed().as_secs_f64();
    rep
}

/// 색인 문서 수 (파일 없음·오프라인 포함 전부).
pub fn index_docs(search: &SearchService) -> Option<i64> {
    let mut f = base_filters();
    f.include_missing = true;
    search
        .search(&SearchRequest {
            limit: 1,
            ..request("", f, SortField::Name, SortDirection::Asc)
        })
        .ok()
        .map(|p| p.total)
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (결정적 질의) / Boundary (구간 P95 최소 표본)
    use super::*;

    #[test]
    fn t_p4_query_01_same_seed_builds_same_requests() {
        // Arrange
        let ctx = Ctx { folder_id: 1 };
        // Act
        let a: Vec<String> = (0..KINDS.len())
            .map(|k| format!("{:?}", build(k, &mut Lcg(5 + k as u64), &ctx).query))
            .collect();
        let b: Vec<String> = (0..KINDS.len())
            .map(|k| format!("{:?}", build(k, &mut Lcg(5 + k as u64), &ctx).query))
            .collect();
        // Assert
        assert_eq!(a, b);
        assert!(build(HYBRID, &mut Lcg(1), &ctx).mode == Some(SearchMode::Hybrid));
        assert!(!build(0, &mut Lcg(2), &ctx).query.is_empty());
    }

    #[test]
    fn t_p4_query_02_worst_window_needs_enough_samples() {
        let mut r = Report::new();
        r.timeline = vec![(0.1, 0, 5.0), (0.2, 0, 50.0), (1.1, 0, 7.0)];
        assert_eq!(r.worst_window_p95(1.0, 3), None);
        let w = r.worst_window_p95(1.0, 2).unwrap();
        assert_eq!(w.1, 50.0);
    }
}
