//! 부하 단계 — 신규 폴더 스캔 + 메타·썸네일·해시 + AI 작업을 돌리면서 같은 검색 질의를 반복하고, 끝나면 색인 소진·정합성을 확인한다.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mam_core::jobs::JobKind;

use super::bench_env::Bench;
use super::bg::Background;
use super::media_gen::fingerprint;
use super::metrics::{Sampler, logical_cpus};
use super::queries::{Ctx, Report, index_docs, run_fixed, run_until};
use super::report::{dir_bytes, gb, print_query_table, print_timeline, wait_for};
use super::scen_m12::{counts_line, derivative_done, print_compare};
use super::stats::summarize;

pub struct Phase<'a> {
    pub bench: &'a Arc<Bench>,
    pub ctx: &'a Ctx,
    pub new_dir: &'a Path,
    pub fp_before: (u64, u64, u128, u128),
    /// 시작 전 라이브러리의 마지막 asset_id (이보다 큰 자산이 신규)
    pub floor: i64,
    pub window: Duration,
    pub per_kind: usize,
    pub label: &'a str,
}

/// 부하 단계 실행. `m1` 은 같은 규모 유휴 기준선.
pub fn load_phase(p: &Phase<'_>, m1: &Report) {
    let (bench, ctx, new_dir, floor) = (p.bench, p.ctx, p.new_dir, p.floor);
    let s_base = floor;
    let fp_before = p.fp_before;
    let folder_id = ctx.folder_id;
    let new_files_hint = p.label;
    // 5) M2: 신규 폴더 스캔 + 처리 + AI + 검색 부하
    let floor = s_base;
    let probe_bench = Arc::clone(bench);
    let sampler = Sampler::start(
        Duration::from_secs(5),
        Arc::new(move || {
            let (p, r) = probe_bench.jobs_busy();
            (probe_bench.outbox_pending(), p, r)
        }),
    );
    let bg = Background::start(bench, floor, true);
    let stop = Arc::new(AtomicBool::new(false));
    let q_thread = {
        let (b, stop) = (Arc::clone(bench), Arc::clone(&stop));
        std::thread::Builder::new()
            .name("p4-queries".into())
            .spawn(move || {
                run_until(
                    &b.search,
                    &Ctx { folder_id },
                    &stop,
                    Duration::from_millis(10),
                    43,
                )
            })
            .expect("spawn query thread")
    };
    let t_load = Instant::now();
    let view = bench
        .ingest
        .add_folder(&new_dir.to_string_lossy())
        .expect("add_folder");
    println!(
        "\n### M2 진행 (경과 s | 자산 | 메타/썸네일/해시 완료(신규) | 작업 pending/running/failed | outbox | AI 호출)"
    );
    let mut scan_done_s: Option<f64> = None;
    let mut milestones: Vec<(f64, i64, i64, i64, i64)> = Vec::new();
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let el = t_load.elapsed();
        let counts = bench.job_counts();
        let scan_busy = counts
            .iter()
            .find(|(k, _)| *k == JobKind::Scan)
            .is_some_and(|(_, c)| c.pending + c.running > 0);
        if !scan_busy && scan_done_s.is_none() {
            scan_done_s = Some(el.as_secs_f64());
        }
        let assets = bench.count("SELECT count(*) FROM media_assets");
        let (meta, thumb) = (
            derivative_done(bench, "meta", floor),
            derivative_done(bench, "thumb", floor),
        );
        let hashed = bench.count(&format!(
            "SELECT count(*) FROM media_assets WHERE asset_id > {floor} AND hash_state = 'done'"
        ));
        let ai_done = bench.count(&format!(
            "SELECT count(DISTINCT asset_id) FROM ai_task_state WHERE asset_id > {floor} AND state = 'done'"
        ));
        let outbox = bench.outbox_pending();
        println!(
            "| {:.0} | {assets} | {meta}/{thumb}/{hashed} | {} | {outbox} | ext {} embed {} |",
            el.as_secs_f64(),
            counts_line(bench),
            bench.ext.calls.load(Ordering::Relaxed),
            bench.embed_stats.jobs.load(Ordering::Relaxed)
        );
        if (el.as_secs() % 30) < 5 {
            milestones.push((el.as_secs_f64(), meta, thumb, hashed, ai_done));
        }
        let (pend, run_n) = bench.jobs_busy();
        let unfed = bench.count(&format!(
            "SELECT count(*) FROM asset_derivatives d WHERE d.kind = 'thumb' AND d.state IN ('done','unsupported')
             AND d.asset_id > {floor} AND NOT EXISTS (SELECT 1 FROM ai_task_state s WHERE s.asset_id = d.asset_id)"
        ));
        if el >= p.window {
            println!("(창 상한 도달 — 처리 백로그가 남아 있으면 일시정지한다)");
            break;
        }
        if scan_done_s.is_some() && pend == 0 && run_n == 0 && outbox == 0 && unfed == 0 {
            println!("(모든 작업 소진)");
            break;
        }
    }
    let load_secs = t_load.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    let m2 = q_thread.join().expect("query thread");
    let (pend_end, run_end) = bench.jobs_busy();
    let backlog_at_end = counts_line(bench);
    let new_assets = bench.count("SELECT count(*) FROM media_assets") - s_base;
    let rates = (
        derivative_done(bench, "meta", floor),
        derivative_done(bench, "thumb", floor),
        bench.count(&format!(
            "SELECT count(*) FROM media_assets WHERE asset_id > {floor} AND hash_state = 'done'"
        )),
        bench.embed_stats.jobs.load(Ordering::Relaxed),
    );
    let snaps = sampler.finish();
    let bg_stats = bg.stop();

    // 6) 백로그를 멈추고 색인이 따라잡는 시간 (유휴 재측정을 위해)
    bench.ingest.pause_jobs(None).expect("pause");
    let paused_ok = wait_for(Duration::from_secs(180), || bench.jobs_busy().1 == 0);
    let t_drain = Instant::now();
    let drained = wait_for(Duration::from_secs(900), || bench.outbox_pending() == 0);
    let drain_s = t_drain.elapsed().as_secs_f64();
    let _ = wait_for(Duration::from_secs(60), || {
        index_docs(&bench.search) == Some(bench.sqlite_active_docs())
    });

    println!(
        "\n### M2 결과 — 부하 {load_secs:.0}s 동안 검색 (일시정지·재개 없이 실행 중 작업과 동시)"
    );
    print_query_table(
        "M2 부하 중 검색 지연 (색인은 증분 갱신 중이며 문서 수가 계속 늘어난다)",
        &m2,
    );
    if let Some((start, p95, n)) = m2.worst_window_p95(30.0, 20) {
        println!("- 가장 느린 30초 구간(시작 {start:.0}s, n={n}) P95 = {p95:.1} ms");
    }
    print_compare("M1", m1, "M2", &m2);
    println!("\n### M2 자원 시계열 (5초 표본, 30초마다 출력)");
    print_timeline(&snaps, 6);
    println!("\n### M2 작업 처리량 (신규 폴더 {} 파일)", new_files_hint);
    println!(
        "- 스캔 완료: {} / 등록된 신규 자산 {new_assets}",
        scan_done_s.map_or("창 안에 못 끝남".into(), |v| format!("{v:.0}s"))
    );
    println!(
        "- {load_secs:.0}s 시점 완료: 메타 {} / 썸네일 {} / 해시 {} / AI(임베딩) {} 작업 — 평균 {:.1} / {:.1} / {:.1} / {:.2} 건/s",
        rates.0,
        rates.1,
        rates.2,
        rates.3,
        rates.0 as f64 / load_secs,
        rates.1 as f64 / load_secs,
        rates.2 as f64 / load_secs,
        rates.3 as f64 / load_secs
    );
    println!("- 종료 시점 대기 작업 pending={pend_end} running={run_end} ({backlog_at_end})");
    println!("- 30초 간격 누적 완료 (경과 s, 메타, 썸네일, 해시, AI done): {milestones:?}");
    let vis = summarize(
        &bench
            .embed_stats
            .visual_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner()),
    );
    let sem = summarize(
        &bench
            .embed_stats
            .semantic_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner()),
    );
    println!(
        "- 임베딩 저장(`EmbedStore::put` 포함 쓰기 구간): 시각 n={} P50 {:.1} / P95 {:.1} / max {:.1} ms, 의미 n={} P50 {:.1} / P95 {:.1} / max {:.1} ms",
        vis.n, vis.p50, vis.p95, vis.max, sem.n, sem.p50, sem.p95, sem.max
    );
    let ht = |v: &std::sync::Mutex<Vec<f64>>| {
        let v = v.lock().unwrap_or_else(|e| e.into_inner());
        let k = (v.len() / 5).max(1).min(v.len());
        if v.is_empty() {
            (0.0, 0.0)
        } else {
            (summarize(&v[..k]).p50, summarize(&v[v.len() - k..]).p50)
        }
    };
    let ((vh, vt), (sh, st)) = (
        ht(&bench.embed_stats.visual_ms),
        ht(&bench.embed_stats.semantic_ms),
    );
    println!(
        "- 임베딩 저장 지연 추이 (처음 20% P50 -> 마지막 20% P50): 시각 {vh:.1} -> {vt:.1} ms, 의미 {sh:.1} -> {st:.1} ms"
    );
    println!(        "- put 의 MAX(raw_offset) 질의 계획: {:?}",        bench.rows("EXPLAIN QUERY PLAN SELECT MAX(raw_offset) FROM embeddings WHERE vector_kind = 'semantic' AND model_id = 'synthetic-e5' AND model_version = 'p4' AND dimension = 384")    );
    let sync = summarize(&bg_stats.sync_ms.lock().unwrap_or_else(|e| e.into_inner()));
    println!(
        "- 벡터 인덱스 세대 동기화(5초 틱): 실행 {} 회, 오류 {}, P50 {:.0} / P95 {:.0} / max {:.0} ms; AI 자동 등록 {} 건 (오류 {})",
        bg_stats.sync_runs.load(Ordering::Relaxed),
        bg_stats.sync_errors.load(Ordering::Relaxed),
        sync.p50,
        sync.p95,
        sync.max,
        bg_stats.fed.load(Ordering::Relaxed),
        bg_stats.feed_errors.load(Ordering::Relaxed)
    );

    // 7) 사후: 백로그 일시정지 상태의 유휴 측정 (문서 수 증가 효과 분리)
    let docs_now = index_docs(&bench.search);
    let post = run_fixed(&bench.search, ctx, p.per_kind, 20, 44);
    print_query_table(
        &format!(
            "M2 직후 유휴 재측정 — 색인 {docs_now:?} 문서, 작업 일시정지(백로그 {backlog_at_end}), outbox 소진 {drained}"
        ),
        &post,
    );
    print_compare("M1", m1, "M2 후 유휴", &post);

    // 8) 정합성
    let active = bench.sqlite_active_docs();
    let failed_jobs: u64 = bench.job_counts().iter().map(|(_, c)| c.failed).sum();
    let outbox_failed = bench.count("SELECT count(*) FROM search_outbox WHERE status = 'failed'");
    let scan_errors = bench.count("SELECT count(*) FROM scan_errors WHERE resolved_at IS NULL");
    let ai_failed = bench.count("SELECT count(*) FROM ai_task_state WHERE state = 'failed'");
    let thumb_failed = bench.count("SELECT count(*) FROM asset_derivatives WHERE state = 'failed'");
    let fp_after = fingerprint(new_dir);
    println!("\n### M2 정합성");
    println!(
        "- 인덱스 문서 수 {docs_now:?} vs SQLite 활성 자산(+활성 위치) {active} — {}",
        if docs_now == Some(active) {
            "일치"
        } else {
            "불일치"
        }
    );
    println!(
        "- outbox pending {} / failed {outbox_failed} / 소진 대기 {drain_s:.1}s ({drained}); 일시정지 확인 {paused_ok}",
        bench.outbox_pending()
    );
    println!(
        "- 실패 작업 {failed_jobs}, 스캔 오류(미해결) {scan_errors}, AI 단계 failed {ai_failed}, 파생물 failed {thumb_failed}, 질의 오류 M1 {} / M2 {} / 사후 {}",
        m1.errors, m2.errors, post.errors
    );
    println!(
        "- 원본 지문(파일 수, 바이트, 수정시각 합, 최대): 전 {fp_before:?} / 후 {fp_after:?} — {}",
        if fp_before == fp_after {
            "불변"
        } else {
            "변경됨!"
        }
    );
    println!(
        "- 데이터 크기: SQLite {:.2} GB, Tantivy {:.2} GB, 벡터 {:.2} GB, 캐시 {:.2} GB; 폴더 뷰 {:?}",
        gb(std::fs::metadata(bench.paths.db()).map_or(0, |m| m.len())),
        gb(dir_bytes(&bench.paths.tantivy())),
        gb(dir_bytes(
            &bench.paths.root().join("indexes").join("vectors")
        )),
        gb(dir_bytes(&bench.paths.cache())),
        view.folder_id
    );
    println!("- 논리 CPU {}", logical_cpus());
    bench.print_failure_reasons();
    super::logs::print_summary("M2/M3 부하 단계");
}
