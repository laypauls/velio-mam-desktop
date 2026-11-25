//! S1 — 압축 장시간 운용: 파일 추가·이름 변경·삭제 → 감시/재스캔 → 처리·AI → 태그·메모 편집 → 검색, 라운드 반복.
//!
//! 한 라운드 = 제품 주기 점검(30초) 한 번에 해당한다고 보고 240 라운드 ≈ 2시간으로 환산한다(주기 점검·디바운스 간격을 줄여
//! 압축). 실제 2시간 벽시계에서만 드러나는 누수(타이머·캐시 만료 등)는 일부만 드러난다.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use mam_core::jobs::JobKind;
use mam_core::userdata::UpdateTagsInput;

use super::bench_env::{Bench, Options, prepare_db};
use super::bg::Background;
use super::media_gen::{Templates, file_for, generate};
use super::metrics::proc_now;
use super::queries::{Ctx, Report, index_docs, run_fixed};
use super::report::{dir_bytes, gb, print_conditions, print_query_table, trend_line, wait_for};
use super::stats::{Lcg, summarize};
use super::synth::file_stem;

pub struct S1 {
    pub base_files: usize,
    pub rounds: usize,
    pub add: usize,
    pub rename: usize,
    pub delete: usize,
    pub edits: usize,
}

#[derive(Default, Clone, Copy)]
struct Row {
    rss: f64,
    commit: f64,
    handles: f64,
    threads: f64,
    settle_s: f64,
    drain_s: f64,
    query_p95: f64,
    round_s: f64,
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&d) {
            for e in rd.flatten() {
                match e.file_type() {
                    Ok(t) if t.is_dir() => stack.push(e.path()),
                    Ok(_) => out.push(e.path()),
                    Err(_) => {}
                }
            }
        }
    }
    out.sort();
    out
}

fn present(b: &Bench) -> i64 {
    b.count(
        "SELECT count(*) FROM file_locations WHERE detached_at IS NULL AND presence = 'present'",
    )
}

/// 스캔·메타·썸네일·해시 작업이 없다 (AI 작업은 백그라운드 백로그라 제외).
fn pipeline_idle(b: &Bench) -> bool {
    b.job_counts()
        .iter()
        .all(|(k, c)| *k == JobKind::Ai || c.pending + c.running == 0)
}

pub fn run(root: &Path, s: &S1, light: bool) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).expect("root");
    print_conditions(
        "S1 조건",
        &[
            ("기준 파일", format!("{} (실제 파일, 전 파이프라인 처리)", s.base_files)),
            ("라운드", format!("{} (240 = 2시간 상당), 라운드당 추가 {} / 이름 변경 {} / 삭제 {} / 편집 {}", s.rounds, s.add, s.rename, s.delete, s.edits)),
            ("압축 설정", "감시 디바운스 150ms, 주기 점검 250ms (제품 기본 2s / 30s), 4 라운드마다 명시적 재스캔".into()),
        ],
    );
    let templates = Templates::build(&root.join("scratch"), light);
    let lib = root.join("lib");
    generate(&lib, &templates, 0..s.base_files as u64, 21);
    let (_paths, conn) = prepare_db(root);
    drop(conn);
    let opts = Options {
        debounce_ms: 150,
        supervise_every: Duration::from_millis(250),
        ..Options::default()
    };
    let bench = Bench::start(root, &opts);
    let t0 = Instant::now();
    let view = bench
        .ingest
        .add_folder(&lib.to_string_lossy())
        .expect("add_folder");
    let want = s.base_files as i64;
    let ok = wait_for(Duration::from_secs(3600), || {
        present(&bench) == want && pipeline_idle(&bench) && bench.outbox_pending() == 0
    });
    println!(
        "- 초기 스캔·처리·색인: {} ({:.0}s), 자산 {}",
        if ok { "완료" } else { "시간 초과" },
        t0.elapsed().as_secs_f64(),
        present(&bench)
    );
    if !ok {
        bench.shutdown();
        return;
    }
    let floor = bench.count("SELECT max(asset_id) FROM media_assets");
    let ctx = Ctx {
        folder_id: view.folder_id,
    };
    let bg = Background::start(&bench, floor, true);
    let _ = run_fixed(&bench.search, &ctx, 1, 2, 5);

    let mut tracked = walk(&lib);
    let mut rng = Lcg(99);
    let mut next_id = s.base_files as u64;
    let mut rows: Vec<Row> = Vec::new();
    let mut queries = Report::new();
    let (mut unsettled, mut drain_timeouts, mut edit_errors) = (0u32, 0u32, 0u32);
    let started = Instant::now();
    let start_proc = proc_now();

    for r in 0..s.rounds {
        let t_round = Instant::now();
        // 1) 사용자가 폴더를 바꾼다 (MAM 이 아니라 외부 도구가 하는 일 — 원본 조작은 하네스가 한다)
        let dir = lib.join(format!("round{:04}", r / 20));
        fs::create_dir_all(&dir).expect("round dir");
        for _ in 0..s.add {
            let (ext, bytes) = file_for(&templates, next_id);
            let stem = file_stem(&mut rng, next_id as i64);
            let p = dir.join(format!("{stem}.{ext}"));
            fs::write(&p, bytes).expect("add file");
            tracked.push(p);
            next_id += 1;
        }
        for i in 0..s.rename.min(tracked.len()) {
            let k = rng.below(tracked.len());
            let from = tracked.swap_remove(k);
            let ext = from
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("jpg")
                .to_string();
            let to = from.with_file_name(format!("renamed_{r}_{i}.{ext}"));
            fs::rename(&from, &to).expect("rename file");
            tracked.push(to);
        }
        for _ in 0..s.delete.min(tracked.len()) {
            let k = rng.below(tracked.len());
            let p = tracked.swap_remove(k);
            fs::remove_file(&p).expect("delete file");
        }
        // 2) 감시가 따라온다 — 4 라운드마다 명시적 재스캔도 함께 (놓친 이벤트 재조정 경로)
        if r % 4 == 3 {
            let _ = bench.ingest.rescan_folder(view.folder_id);
        }
        let t_settle = Instant::now();
        let want = tracked.len() as i64;
        let settled = wait_for(Duration::from_secs(180), || {
            present(&bench) == want && pipeline_idle(&bench) && bench.outbox_pending() == 0
        });
        let settle_s = t_settle.elapsed().as_secs_f64();
        unsettled += u32::from(!settled);

        // 3) 사용자 편집 (태그·메모·즐겨찾기) — 색인은 priority 0 이벤트로 바로 따라와야 한다
        let ids: Vec<i64> = bench
            .pool
            .read(|c| {
                let mut st = c.prepare_cached(
                    "SELECT asset_id FROM file_locations WHERE detached_at IS NULL AND presence = 'present'
                     ORDER BY random() LIMIT ?1",
                )?;
                let rows = st.query_map([s.edits as i64], |r| r.get::<_, i64>(0))?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .unwrap_or_default();
        let tag = UpdateTagsInput {
            asset_ids: ids.clone(),
            add: vec![format!("soak-{}", r % 7)],
            remove: vec![],
        };
        edit_errors += u32::from(bench.user.update_asset_tags(tag).is_err());
        for id in ids.iter().take(s.edits / 2) {
            edit_errors += u32::from(
                bench
                    .user
                    .set_memo(*id, &format!("메모 라운드 {r} 번호 {id}"))
                    .is_err(),
            );
        }
        edit_errors += u32::from(
            bench
                .user
                .set_favorite(&ids[..ids.len().min(3)], true)
                .is_err(),
        );
        bench.search.wake();
        let t_drain = Instant::now();
        let drained = wait_for(Duration::from_secs(120), || bench.outbox_pending() == 0);
        let drain_s = t_drain.elapsed().as_secs_f64();
        drain_timeouts += u32::from(!drained);

        // 4) 검색 (종류당 2회)
        let q = run_fixed(&bench.search, &ctx, 2, 0, 1000 + r as u64);
        let q95 = q.all().p95;
        for (k, v) in q.samples.iter().enumerate() {
            queries.samples[k].extend_from_slice(v);
        }
        queries.errors += q.errors;
        queries.hybrid_total += q.hybrid_total;
        queries.hybrid_degraded += q.hybrid_degraded;
        queries.first_errors.extend(q.first_errors);

        let p = proc_now();
        rows.push(Row {
            rss: super::metrics::mb(p.rss),
            commit: super::metrics::mb(p.commit),
            handles: f64::from(p.handles),
            threads: f64::from(p.threads),
            settle_s,
            drain_s,
            query_p95: q95,
            round_s: t_round.elapsed().as_secs_f64(),
        });
        if r % 20 == 19 || r + 1 == s.rounds {
            let w = rows.last().copied().unwrap_or_default();
            println!(
                "라운드 {:>3}/{} | 경과 {:.0}s | RSS {:.0} MB 핸들 {} 스레드 {} | settle {:.2}s drain {:.2}s | 검색 P95 {:.1} ms | AI 대기 {}",
                r + 1,
                s.rounds,
                started.elapsed().as_secs_f64(),
                w.rss,
                w.handles,
                w.threads,
                w.settle_s,
                w.drain_s,
                w.query_p95,
                bench
                    .job_counts()
                    .iter()
                    .find(|(k, _)| *k == JobKind::Ai)
                    .map_or(0, |(_, c)| c.pending + c.running)
            );
        }
    }

    // 최종 소진 (AI 백로그 포함)
    let t_final = Instant::now();
    let _ = wait_for(Duration::from_secs(1800), || {
        bench.jobs_busy() == (0, 0)
            && bench.outbox_pending() == 0
            && bench.count(&format!(
                "SELECT count(*) FROM asset_derivatives d WHERE d.kind = 'thumb' AND d.state IN ('done','unsupported')
                 AND d.asset_id > {floor} AND NOT EXISTS (SELECT 1 FROM ai_task_state s WHERE s.asset_id = d.asset_id)"
            )) == 0
    });
    let final_s = t_final.elapsed().as_secs_f64();
    let bg_stats = bg.stop();
    let _ = wait_for(Duration::from_secs(120), || bench.outbox_pending() == 0);
    let end_proc = proc_now();

    // 보고
    let skip = rows.len() / 5; // 앞 20% 는 워밍업(캐시·세그먼트 형성)으로 보고 회귀에서 제외
    let tail = &rows[skip.min(rows.len().saturating_sub(2))..];
    let per_hour = s.rounds as f64 / 2.0; // 라운드 → 시간 환산 (240 라운드 = 2시간)
    println!(
        "\n### S1 추세 (라운드 {}~{} 회귀, 앞 {skip} 라운드 제외; 시간당 = 라운드 기울기 × {per_hour:.0})",
        skip + 1,
        rows.len()
    );
    println!(
        "| 지표 | 앞 25% 평균 | 뒤 25% 평균 | 기울기/라운드 | R² | 기울기/시간 상당 |\n|---|---:|---:|---:|---:|---:|"
    );
    let col = |f: fn(&Row) -> f64| tail.iter().map(f).collect::<Vec<f64>>();
    for (name, f) in [
        ("RSS MB", (|r: &Row| r.rss) as fn(&Row) -> f64),
        ("커밋 MB", |r: &Row| r.commit),
        ("핸들", |r: &Row| r.handles),
        ("스레드", |r: &Row| r.threads),
        ("settle s", |r: &Row| r.settle_s),
        ("outbox drain s", |r: &Row| r.drain_s),
        ("라운드 검색 P95 ms", |r: &Row| r.query_p95),
    ] {
        println!("{}", trend_line(name, &col(f), per_hour));
    }
    let settle = summarize(&rows.iter().map(|r| r.settle_s).collect::<Vec<_>>());
    let drain = summarize(&rows.iter().map(|r| r.drain_s).collect::<Vec<_>>());
    let round = summarize(&rows.iter().map(|r| r.round_s).collect::<Vec<_>>());
    println!(
        "\n- 변경 반영 시간(파일 변경 → 위치·처리·색인 수렴) P50/P95/P99/max = {:.2}/{:.2}/{:.2}/{:.2} s; 편집 → outbox 0 = {:.2}/{:.2}/{:.2}/{:.2} s; 라운드 길이 P50 {:.2} s",
        settle.p50,
        settle.p95,
        settle.p99,
        settle.max,
        drain.p50,
        drain.p95,
        drain.p99,
        drain.max,
        round.p50
    );
    println!(
        "- RSS 시작 {:.0} MB → 끝 {:.0} MB (최대 작업 집합 {:.0} MB); 핸들 {} → {}; 스레드 {} → {}",
        super::metrics::mb(start_proc.rss),
        super::metrics::mb(end_proc.rss),
        super::metrics::mb(end_proc.peak_rss),
        start_proc.handles,
        end_proc.handles,
        start_proc.threads,
        end_proc.threads
    );
    print_query_table("S1 전 라운드 검색 합산", &queries);

    let docs = index_docs(&bench.search);
    let active = bench.sqlite_active_docs();
    let outbox_failed = bench.count("SELECT count(*) FROM search_outbox WHERE status = 'failed'");
    let failed_jobs: u64 = bench.job_counts().iter().map(|(_, c)| c.failed).sum();
    println!("\n### S1 정합성");
    println!(
        "- 최종 소진 {final_s:.1}s 후 outbox pending {} / failed {outbox_failed}; 작업 pending/running {:?}, failed {failed_jobs}",
        bench.outbox_pending(),
        bench.jobs_busy()
    );
    println!(
        "- 인덱스 문서 {docs:?} vs SQLite 활성 자산(+활성 위치) {active} — {}; 디스크 파일 {} vs present 위치 {}",
        if docs == Some(active) {
            "일치"
        } else {
            "불일치"
        },
        tracked.len(),
        present(&bench)
    );
    println!(
        "- 오류: 수렴 시간 초과 {unsettled}, drain 시간 초과 {drain_timeouts}, 편집 오류 {edit_errors}, 질의 오류 {}, 스캔 오류(미해결) {}, AI 단계 failed {}, 파생물 failed {}, 임베딩 실패 {}",
        queries.errors,
        bench.count("SELECT count(*) FROM scan_errors WHERE resolved_at IS NULL"),
        bench.count("SELECT count(*) FROM ai_task_state WHERE state = 'failed'"),
        bench.count("SELECT count(*) FROM asset_derivatives WHERE state = 'failed'"),
        bench.embed_stats.failures.load(Ordering::Relaxed)
    );
    println!(
        "- AI: 자동 등록 {} 건(오류 {}), 외부 호출 {}, STT {}, 임베딩 작업 {}; 벡터 세대 동기화 {} 회(오류 {})",
        bg_stats.fed.load(Ordering::Relaxed),
        bg_stats.feed_errors.load(Ordering::Relaxed),
        bench.ext.calls.load(Ordering::Relaxed),
        bench.stt.calls.load(Ordering::Relaxed),
        bench.embed_stats.jobs.load(Ordering::Relaxed),
        bg_stats.sync_runs.load(Ordering::Relaxed),
        bg_stats.sync_errors.load(Ordering::Relaxed)
    );
    println!(
        "- 크기: SQLite {:.2} GB, Tantivy {:.3} GB, 벡터 {:.3} GB, 캐시 {:.3} GB",
        gb(fs::metadata(bench.paths.db()).map_or(0, |m| m.len())),
        gb(dir_bytes(&bench.paths.tantivy())),
        gb(dir_bytes(
            &bench.paths.root().join("indexes").join("vectors")
        )),
        gb(dir_bytes(&bench.paths.cache()))
    );
    bench.print_failure_reasons();
    super::logs::print_summary("S1");
    bench.shutdown();
}
