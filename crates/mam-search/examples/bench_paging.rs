//! 위치 seek(`at`)·역방향 커서·구간 집계(`section_index`) 지연 측정 — 합성 문서를 메모리에서 만들어 색인한다.
//!
//! 실행: `cargo run --release -p mam-search --example bench_paging -- --size 1000000`
//! 옵션: `--runs 30`(위치당 측정 횟수), `--limit 100`, `--out <dir>`(기본 `target/bench-paging/<size>`), `--keep`(끝나도 색인을 지우지 않는다)
//!
//! 끝나면 색인 디렉터리를 지운다(`--keep` 이 없을 때) — 수십 GB 가 아니라도 측정 데이터는 남기지 않는다.
//! 첫 호출(콜드에 가까움, OS 파일 캐시는 비우지 않음)과 이후 반복(웜)을 분리해 보고한다.

#[path = "common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use common::rng::{Rng, civil_ms};
use common::stats::summarize;
use common::vocab::{FN_EN, FN_KO};
use common::{args, sysinfo};
use mam_search::coordinator::{
    CoordRequest, Direction, SearchHandle, SortField, SortSpec, local_utc_offset_secs,
};
use mam_search::{
    AssetDoc, FilenameStrategy, Filters, IndexOptions, MediaType, Presence, SearchIndex,
};

fn make_doc(id: u64, rng: &mut Rng) -> AssetDoc {
    let r = rng.below(100);
    let name = match r {
        0..=54 => format!("{}_{}.jpg", rng.pick(FN_EN), rng.range(1, 9999)),
        55..=84 => format!("{}_{}.mp4", rng.pick(FN_KO), rng.range(1, 9999)),
        85..=94 => format!("IMG_{:04}.jpg", rng.range(1, 9999)),
        95..=97 => format!("é{}_{}.png", rng.pick(FN_EN), rng.range(1, 99)),
        _ => format!("カメラ{}.jpg", rng.range(1, 999)),
    };
    // 촬영일 85% 존재, 2012~2025 균등 (유닉스 초) / 수정일 2010~2026 (ms)
    let captured = rng.chance(0.85).then(|| {
        civil_ms(2012, 1, 1, 0, 0, 0) / 1000
            + rng.range(0, 14 * 365 * 86_400) as i64
    });
    AssetDoc {
        asset_id: id,
        revision: 1,
        file_name: name,
        user_tags: vec![],
        ai_keywords: vec![],
        ai_description: String::new(),
        user_memo: String::new(),
        title: String::new(),
        transcript: String::new(),
        media_type: match rng.below(10) {
            0..=6 => MediaType::Image,
            7..=8 => MediaType::Video,
            _ => MediaType::Audio,
        },
        volume_id: 1,
        presence: Presence::Present,
        captured_at: captured,
        file_size: rng.range(10_000, 5_000_000_000),
        duration_ms: rng.chance(0.4).then(|| rng.range(1_000, 7_200_000)),
        favorite: false,
        rating: 0,
        collection_ids: vec![],
        tag_ids: vec![],
        modified_at: civil_ms(2010, 1, 1, 0, 0, 0) + rng.range(0, 16 * 365 * 86_400_000) as i64,
        folder_id: None,
        dir_key: String::new(),
        user_fields: vec![],
    }
}

fn req(field: SortField, desc: bool, limit: usize) -> CoordRequest {
    CoordRequest {
        text: String::new(),
        strategy: FilenameStrategy::B,
        filters: Filters::default(),
        sort: SortSpec {
            field,
            descending: desc,
        },
        limit,
        cursor: None,
        direction: Direction::Forward,
        at: None,
        collection_order: None,
    }
}

fn measure(runs: usize, mut f: impl FnMut()) -> (f64, common::stats::Summary) {
    let t = Instant::now();
    f();
    let first = t.elapsed().as_secs_f64() * 1000.0;
    let mut samples: Vec<Duration> = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t = Instant::now();
        f();
        samples.push(t.elapsed());
    }
    (first, summarize(&samples))
}

fn row(label: &str, first: f64, s: &common::stats::Summary) {
    println!(
        "| {label} | {first:.1} | {:.1} | {:.1} | {:.1} | {:.1} |",
        s.p50_ms, s.p95_ms, s.p99_ms, s.max_ms
    );
}

fn main() {
    let a = args::Args::parse();
    let size: u64 = a.required("size");
    let runs: usize = a.num("runs", 30);
    let limit: usize = a.num("limit", 100);
    let dir = a.path("out").unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/bench-paging")
            .join(size.to_string())
    });
    let _ = std::fs::remove_dir_all(&dir);

    // 색인 구축
    let t_build = Instant::now();
    let opts = IndexOptions::strategy_b();
    let index = SearchIndex::open_or_create(&dir, opts).expect("open index");
    let mut rng = Rng::new(42);
    for id in 1..=size {
        index
            .upsert_unguarded(&make_doc(id, &mut rng))
            .expect("upsert");
    }
    index.commit().expect("commit");
    let (_, timed_out) = common::wait_for_merges(&dir, Duration::from_secs(3), Duration::from_secs(1800));
    let build_s = t_build.elapsed().as_secs_f64();
    let segments = common::segment_count(&dir);
    let bytes = common::dir_size(&dir);
    let rss_build = sysinfo::peak_rss_bytes();
    let handle = SearchHandle::for_index(&index);

    let cond = sysinfo::conditions();
    println!("\n== bench_paging ==");
    common::print_conditions(&cond);
    println!(
        "Dataset: {size} synthetic docs (seed 42; names 55% EN / 30% KO / 10% IMG_nnnn / 5% accented+kana; captured_at present 85% over 2012-2025; modified_at 100% over 2010-2026), strategy B, filters default"
    );
    println!(
        "Index state: segments={segments}, merge_timed_out={timed_out}, dir={:.0} MB, build {build_s:.0}s, peak RSS after build {:.0} MB; limit={limit}, runs={runs} (+1 first call)",
        sysinfo::mb(bytes),
        sysinfo::mb(rss_build)
    );

    let total = handle.search(&req(SortField::Name, false, 1)).unwrap().total;
    assert_eq!(total, size, "every document is searchable");
    let positions: [(&str, u64); 5] = [
        ("0", 0),
        ("10%", total / 10),
        ("50%", total / 2),
        ("90%", total * 9 / 10),
        ("last page", total - limit as u64),
    ];

    println!("\n| call | first ms | P50 ms | P95 ms | P99 ms | max ms |\n|---|---:|---:|---:|---:|---:|");
    let sorts = [
        ("capturedAt desc", SortField::CapturedAt, true),
        ("name asc", SortField::Name, false),
        ("modifiedAt desc", SortField::ModifiedAt, true),
    ];
    for (name, field, desc) in sorts {
        // 기준: 커서 없는 첫 페이지 (전체 수집 패스 1회)
        let r0 = req(field, desc, limit);
        let (f, s) = measure(runs, || {
            handle.search(&r0).unwrap();
        });
        row(&format!("{name} / first page (no at)"), f, &s);
        for (pos_name, pos) in positions {
            let mut r = req(field, desc, limit);
            let (f, s) = measure(runs, || {
                r.at = Some(pos as usize);
                let p = handle.search(&r).unwrap();
                assert_eq!(p.start_offset, pos);
                assert_eq!(p.ids.len(), limit);
            });
            row(&format!("{name} / at={pos_name}"), f, &s);
        }
        // 중간 위치에서 이어지는 커서 페이지 (순방향·역방향) — startOffset 계산 포함
        let mut mid = req(field, desc, limit);
        mid.at = Some((total / 2) as usize);
        let page = handle.search(&mid).unwrap();
        let mut fwd = req(field, desc, limit);
        fwd.cursor = page.next_cursor.clone();
        let (f, s) = measure(runs, || {
            let p = handle.search(&fwd).unwrap();
            assert_eq!(p.start_offset, total / 2 + limit as u64);
        });
        row(&format!("{name} / forward cursor @50%"), f, &s);
        let mut back = req(field, desc, limit);
        back.cursor = page.prev_cursor.clone();
        back.direction = Direction::Backward;
        let (f, s) = measure(runs, || {
            let p = handle.search(&back).unwrap();
            assert_eq!(p.start_offset, total / 2 - limit as u64);
        });
        row(&format!("{name} / backward cursor @50%"), f, &s);
    }

    println!("\n| section_index | first ms | P50 ms | P95 ms | P99 ms | max ms |\n|---|---:|---:|---:|---:|---:|");
    for (name, field, desc) in [
        ("capturedAt (month)", SortField::CapturedAt, true),
        ("modifiedAt (month)", SortField::ModifiedAt, true),
        ("name (initial)", SortField::Name, false),
        ("size (none)", SortField::Size, false),
    ] {
        let r = req(field, desc, 1);
        let mut shape = (0, 0);
        let (f, s) = measure(runs, || {
            let idx = handle.section_index(&r, &local_utc_offset_secs).unwrap();
            assert_eq!(idx.total, size);
            shape = (idx.sections.len(), idx.sections.iter().map(|x| x.count).sum::<u64>() as usize);
        });
        row(&format!("{name} [{} sections]", shape.0), f, &s);
    }
    println!(
        "\npeak RSS after all measurements: {:.0} MB",
        sysinfo::mb(sysinfo::peak_rss_bytes())
    );

    drop(handle);
    drop(index);
    if !a.flag("keep") {
        let _ = std::fs::remove_dir_all(&dir);
        println!("(deleted {})", dir.display());
    }
}
