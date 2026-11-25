//! 질의 지연 측정(타입별·값 없음·복합·텍스트), 정답 검증, filtered_asset_ids, 갱신 반영.

use std::path::Path;
use std::time::{Duration, Instant};

use mam_search::analyzer::{Kind, tokenize};
use tantivy::Order;
use tantivy::collector::{Collector, Count, SegmentCollector, TopDocs};
use tantivy::query::Query;
use tantivy::{DocId, Index, ReloadPolicy, Score, Searcher, SegmentReader, Term};

use crate::data::{Spec, gen_values, kw_value, option_id, specs};
use crate::q::{Fx, P, build_query, build_schema, make_doc};
use crate::rng::Rng;
use crate::stats::{Summary, summarize};
use crate::sysinfo::{mb, peak_rss_bytes};

const DAY: i64 = 86_400;
const T0: i64 = 1_420_070_400;

pub struct KindDef {
    pub name: &'static str,
    pub variants: Vec<(Vec<P>, Vec<String>)>,
}

fn two(r: &mut Rng, n: usize) -> Vec<usize> {
    let a = r.skewed(n);
    let mut b = r.skewed(n);
    if b == a {
        b = (a + 1) % n;
    }
    vec![a, b]
}
fn picks(r: &mut Rng, n: usize, k: usize) -> Vec<usize> {
    let mut v: Vec<usize> = Vec::new();
    while v.len() < k {
        let x = r.skewed(n);
        if !v.contains(&x) {
            v.push(x);
        }
    }
    v
}
fn opts(field: u32, ks: &[usize]) -> Vec<i64> {
    ks.iter().map(|k| option_id(field, *k)).collect()
}
fn month(r: &mut Rng) -> P {
    let (y, m) = (2015 + r.below(12) as i64, 1 + r.below(12) as i64);
    P::Date(10, y * 10_000 + m * 100 + 1, y * 10_000 + m * 100 + 31)
}
fn year(r: &mut Rng, id: u32) -> P {
    let y = 2015 + r.below(12) as i64;
    P::Date(id, y * 10_000 + 101, y * 10_000 + 1231)
}
fn years5(r: &mut Rng, id: u32) -> P {
    let y = 2015 + r.below(8) as i64;
    P::Date(id, y * 10_000 + 101, (y + 4) * 10_000 + 1231)
}
fn cap(r: &mut Rng, years: i64) -> P {
    let lo = T0 + r.below(((12 - years) * 365) as usize) as i64 * DAY;
    P::Captured(lo, lo + years * 365 * DAY)
}
fn media(r: &mut Rng) -> P {
    P::Media(["image", "video", "audio"][r.weighted(&[50, 40, 10])])
}
fn media_v(r: &mut Rng) -> P {
    let _ = r;
    P::Media("video")
}
fn words(r: &mut Rng, k: usize) -> Vec<String> {
    let mut out = Vec::new();
    for _ in 0..k {
        let w = kw_value(r.skewed(60));
        for (t, _, _) in tokenize(Kind::KoreanQuery, &w).unwrap() {
            out.push(t);
        }
    }
    out.dedup();
    out
}

pub fn kinds(seed: u64) -> Vec<KindDef> {
    let mut r = Rng::new(seed);
    let r = &mut r;
    let nv = 20;
    let mut out: Vec<KindDef> = Vec::new();
    macro_rules! kind {
        ($name:expr, |$i:ident| $body:expr) => {{
            let mut variants = Vec::new();
            for $i in 0..nv {
                let _ = $i;
                variants.push($body);
            }
            out.push(KindDef { name: $name, variants });
        }};
    }
    // 1. 타입별 단일 조건
    kind!("kw_exact f1(300종)", |_i| (vec![P::Kw(1, kw_value(r.skewed(300)))], vec![]));
    kind!("kw_exact f5(10만종)", |_i| (vec![P::Kw(5, kw_value(r.skewed(100_000)))], vec![]));
    kind!("kw_any(5) f2", |_i| (vec![P::KwAny(2, (0..5).map(|_| kw_value(r.skewed(5000))).collect())], vec![]));
    kind!("kw_any(50) f2 Bool", |_i| (vec![P::KwAny(2, (0..50).map(|_| kw_value(r.skewed(5000))).collect())], vec![]));
    kind!("kw_any(50) f2 TermSet", |_i| (vec![P::KwAnySet(2, (0..50).map(|_| kw_value(r.skewed(5000))).collect())], vec![]));
    kind!("num_range f6 ~5%", |_i| {
        let a = r.unit() * 950.0;
        (vec![P::Num(6, a, a + 50.0)], vec![])
    });
    kind!("num_range f6 ~50%", |_i| {
        let a = r.unit() * 500.0;
        (vec![P::Num(6, a, a + 500.0)], vec![])
    });
    kind!("num_range f7 정수값 [a,a+10]", |_i| {
        let a = r.below(80) as f64;
        (vec![P::Num(7, a, a + 10.0)], vec![])
    });
    kind!("num_range f8 ~50%", |_i| {
        let a = r.unit() * 500_000.0;
        (vec![P::Num(8, a, a + 500_000.0)], vec![])
    });
    kind!("date f10 1개월", |_i| (vec![month(r)], vec![]));
    kind!("date f10 1년", |_i| (vec![year(r, 10)], vec![]));
    kind!("date f10 5년", |_i| (vec![years5(r, 10)], vec![]));
    kind!("boolean f14=1", |_i| (vec![P::Bool(14, 1)], vec![]));
    kind!("enum_any(2) f16", |_i| (vec![P::EnumAny(16, opts(16, &two(r, 4)))], vec![]));
    kind!("enum_any(5) f18", |_i| (vec![P::EnumAny(18, opts(18, &picks(r, 50, 5)))], vec![]));
    kind!("multi_any(2) f19", |_i| (vec![P::MultiAny(19, opts(19, &two(r, 8)))], vec![]));
    kind!("multi_all(2) f19", |_i| (vec![P::MultiAll(19, opts(19, &two(r, 8)))], vec![]));
    kind!("multi_all(3) f20", |_i| (vec![P::MultiAll(20, opts(20, &picks(r, 30, 3)))], vec![]));
    // 2. 값 없음
    kind!("missing f3 단독", |_i| (vec![P::Missing(3)], vec![]));
    kind!("missing f9 단독", |_i| (vec![P::Missing(9)], vec![]));
    kind!("missing f3 + video + date f10 1년", |_i| (vec![P::Missing(3), media_v(r), year(r, 10)], vec![]));
    kind!("missing f9 + enum_any(2) f16 + bool f14=1", |_i| (vec![P::Missing(9), P::EnumAny(16, opts(16, &two(r, 4))), P::Bool(14, 1)], vec![]));
    kind!("missing f3 + missing f13 + media", |_i| (vec![P::Missing(3), P::Missing(13), media(r)], vec![]));
    // 3. 복합
    kind!("고정만: media + captured 5년 + rating>=1 (기준선)", |_i| (vec![media(r), cap(r, 5), P::RatingMin(1)], vec![]));
    kind!("복합3: video + date f10 1년 + enum f16", |_i| (vec![media_v(r), year(r, 10), P::EnumAny(16, opts(16, &two(r, 4)))], vec![]));
    kind!("복합4: +captured 2년 + num f6 50%", |_i| {
        let a = r.unit() * 500.0;
        (vec![media_v(r), year(r, 10), P::EnumAny(16, opts(16, &two(r, 4))), cap(r, 2), P::Num(6, a, a + 500.0)], vec![])
    });
    kind!("복합5 선택적: kw f1 + num f6 + date f11 5년 + enum f17 + bool f14", |_i| {
        let a = r.unit() * 500.0;
        (vec![P::Kw(1, kw_value(r.skewed(300))), P::Num(6, a, a + 500.0), years5(r, 11), P::EnumAny(17, opts(17, &picks(r, 12, 4))), P::Bool(14, 0)], vec![])
    });
    kind!("복합6 광범위: media + captured 5년 + date f10 5년 + num f8 50% + multi_any f19 + rating>=1", |_i| {
        let a = r.unit() * 500_000.0;
        (vec![media(r), cap(r, 5), years5(r, 10), P::Num(8, a, a + 500_000.0), P::MultiAny(19, opts(19, &two(r, 8))), P::RatingMin(1)], vec![])
    });
    // 4. umeta_text (BM25)
    kind!("text 단일어 (umeta_text)", |_i| (vec![], words(r, 1)));
    kind!("text 2단어 AND", |_i| (vec![], words(r, 2)));
    kind!("text + enum f16 + date f10 1년", |_i| (vec![P::EnumAny(16, opts(16, &two(r, 4))), year(r, 10)], words(r, 1)));
    kind!("text + video + kw f1 일치", |_i| (vec![media_v(r), P::Kw(1, kw_value(r.skewed(300)))], words(r, 1)));
    kind!("text + missing f3 + media", |_i| (vec![P::Missing(3), media(r)], words(r, 1)));
    out
}

pub struct Row {
    pub name: &'static str,
    pub hits_mean: f64,
    pub cold_ms: f64,
    pub count: Summary,
    pub top: Summary,
    pub counts: Vec<u64>,
    pub capped: bool,
}

fn time<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let v = f();
    (v, t.elapsed())
}

fn run_kind(s: &Searcher, fx: &Fx, k: &KindDef, reps: usize, cap_secs: u64) -> Row {
    let qs: Vec<Box<dyn Query>> = k.variants.iter().map(|(p, t)| build_query(fx, p, t)).collect();
    let text = !k.variants[0].1.is_empty();
    let (c0, cold) = time(|| s.search(&*qs[0], &Count).unwrap());
    let _ = c0;
    // 웜업 1회 + 개수 기록
    let counts: Vec<u64> = qs.iter().map(|q| s.search(&**q, &Count).unwrap() as u64).collect();
    let top = |q: &dyn Query| {
        if text {
            s.search(q, &TopDocs::with_limit(50).order_by_score()).map(|v| v.len()).unwrap()
        } else {
            s.search(q, &TopDocs::with_limit(50).order_by_fast_field::<i64>("captured_at", Order::Desc))
                .map(|v| v.len())
                .unwrap()
        }
    };
    for q in &qs {
        top(&**q);
    }
    let cap = Duration::from_secs(cap_secs);
    let (mut cs, mut ts) = (Vec::new(), Vec::new());
    let start = Instant::now();
    let mut capped = false;
    'a: for _ in 0..reps {
        for q in &qs {
            cs.push(time(|| s.search(&**q, &Count).unwrap()).1);
            if start.elapsed() > cap {
                capped = true;
                break 'a;
            }
        }
    }
    let start = Instant::now();
    'b: for _ in 0..reps {
        for q in &qs {
            ts.push(time(|| top(&**q)).1);
            if start.elapsed() > cap {
                capped = true;
                break 'b;
            }
        }
    }
    Row {
        name: k.name,
        hits_mean: counts.iter().sum::<u64>() as f64 / counts.len() as f64,
        cold_ms: cold.as_secs_f64() * 1000.0,
        count: summarize(&cs),
        top: summarize(&ts),
        counts,
        capped,
    }
}

struct AssetIdCollector;
struct AssetIdSegment {
    column: tantivy::columnar::Column<u64>,
    ids: Vec<u64>,
}
impl Collector for AssetIdCollector {
    type Fruit = Vec<u64>;
    type Child = AssetIdSegment;
    fn for_segment(&self, _o: u32, seg: &SegmentReader) -> tantivy::Result<AssetIdSegment> {
        Ok(AssetIdSegment { column: seg.fast_fields().u64("asset_id")?, ids: Vec::new() })
    }
    fn requires_scoring(&self) -> bool {
        false
    }
    fn merge_fruits(&self, fruits: Vec<Vec<u64>>) -> tantivy::Result<Vec<u64>> {
        Ok(fruits.into_iter().flatten().collect())
    }
}
impl SegmentCollector for AssetIdSegment {
    type Fruit = Vec<u64>;
    fn collect(&mut self, doc: DocId, _s: Score) {
        if let Some(id) = self.column.first(doc) {
            self.ids.push(id);
        }
    }
    fn harvest(self) -> Vec<u64> {
        self.ids
    }
}

fn open(dir: &Path) -> (Index, Fx) {
    mam_search::korean::ensure_ready().unwrap();
    let t = Instant::now();
    let index = Index::open_in_dir(dir).unwrap();
    mam_search::analyzer::register(&index);
    println!("index open: {:.1} ms, segments={}", t.elapsed().as_secs_f64() * 1000.0, index.searchable_segment_metas().unwrap().len());
    let (_, fx) = build_schema();
    (index, fx)
}

fn f(ms: f64) -> String {
    format!("{ms:.1}")
}

pub fn run(dir: &Path, n: u64, reps: usize, verify: bool, update: usize) {
    let (index, fx) = open(dir);
    let reader = index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into().unwrap();
    let searcher = reader.searcher();
    println!("docs={} segments={}", searcher.num_docs(), searcher.segment_readers().len());
    let ks = kinds(7);
    let mut rows = Vec::new();
    println!("\n| 질의 종류 | 평균 hit | 첫 실행(ms) | Count P50/P95/P99 (ms) | Top50 P50/P95/P99 (ms) | n |");
    println!("|---|---|---|---|---|---|");
    for k in &ks {
        let row = run_kind(&searcher, &fx, k, reps, 30);
        println!(
            "| {} | {:.0} | {} | {} / {} / {} | {} / {} / {} | {}{} |",
            row.name, row.hits_mean, f(row.cold_ms),
            f(row.count.p50_ms), f(row.count.p95_ms), f(row.count.p99_ms),
            f(row.top.p50_ms), f(row.top.p95_ms), f(row.top.p99_ms),
            row.count.n, if row.capped { " (시간 상한)" } else { "" }
        );
        rows.push(row);
    }
    println!("peak RSS so far: {:.0} MB", mb(peak_rss_bytes()));

    // 정답 검증: 직접 평가한 개수와 Count 비교 (text 종류 제외)
    if verify {
        let sp: Vec<Spec> = specs();
        let flat: Vec<(usize, usize)> = ks
            .iter()
            .enumerate()
            .filter(|(_, k)| k.variants[0].1.is_empty())
            .flat_map(|(ki, k)| (0..k.variants.len()).map(move |vi| (ki, vi)))
            .collect();
        let mut exp = vec![0u64; flat.len()];
        let t = Instant::now();
        for id in 1..=n {
            let vals = gen_values(id, &sp, 0);
            for (slot, (ki, vi)) in flat.iter().enumerate() {
                if ks[*ki].variants[*vi].0.iter().all(|p| p.eval(id, &vals)) {
                    exp[slot] += 1;
                }
            }
        }
        let mut bad = 0;
        let mut per_kind: std::collections::BTreeMap<&str, (usize, usize)> = Default::default();
        for (slot, (ki, vi)) in flat.iter().enumerate() {
            let got = rows[*ki].counts[*vi];
            let e = per_kind.entry(ks[*ki].name).or_default();
            e.0 += 1;
            if got != exp[slot] {
                bad += 1;
                e.1 += 1;
                if e.1 <= 2 {
                    println!("MISMATCH {} variant {vi}: tantivy={got} expected={} preds={:?}", ks[*ki].name, exp[slot], ks[*ki].variants[*vi].0);
                }
            }
        }
        println!("\n정답 검증({:.0}s): {} 변형 중 불일치 {bad}", t.elapsed().as_secs_f64(), flat.len());
        for (name, (tot, b)) in per_kind {
            println!("  {name}: {tot} 변형, 불일치 {b}");
        }
    }

    // filtered_asset_ids 식 전체 수집
    println!("\n| 전체 id 수집 (AssetIdCollector) | 건수 | P50 ms | P95 ms | max ms |");
    println!("|---|---|---|---|---|");
    let mut r = Rng::new(99);
    let broad: Vec<(&str, Vec<P>)> = vec![
        ("media=image (~50%)", vec![P::Media("image")]),
        ("date f10 5년 (~42%)", vec![P::Date(10, 20150101, 20191231)]),
        ("missing f3 (~20%)", vec![P::Missing(3)]),
        ("enum f16 any2 (~60%)", vec![P::EnumAny(16, opts(16, &[0, 1]))]),
        ("복합: image + cap 5년 + date f10 5년 + num f8 50% + multi f19", {
            let a = r.unit() * 500_000.0;
            vec![P::Media("image"), P::Captured(T0, T0 + 5 * 365 * DAY), P::Date(10, 20150101, 20191231), P::Num(8, a, a + 500_000.0), P::MultiAny(19, opts(19, &[0, 1]))]
        }),
        ("고정만: image + cap 5년 + rating>=1", vec![P::Media("image"), P::Captured(T0, T0 + 5 * 365 * DAY), P::RatingMin(1)]),
    ];
    for (name, preds) in broad {
        let q = build_query(&fx, &preds, &[]);
        let mut ts = Vec::new();
        let mut len = 0;
        for _ in 0..6 {
            let (ids, d) = time(|| searcher.search(&*q, &AssetIdCollector).unwrap());
            len = ids.len();
            ts.push(d);
        }
        let s = summarize(&ts[1..]); // 첫 회 제외
        println!("| {name} | {len} | {} | {} | {} |", f(s.p50_ms), f(s.p95_ms), f(s.max_ms));
    }

    // 갱신 반영: 임의 `update` 건 삭제 + 재추가, commit, reader reload
    if update > 0 {
        let mut w = index.writer_with_num_threads(2, 200_000_000).unwrap();
        let sp = specs();
        let mut r = Rng::new(1234);
        let mut ids: Vec<u64> = (0..update).map(|_| r.range(1, n)).collect();
        ids.sort_unstable();
        ids.dedup();
        let (_, d_add) = time(|| {
            for id in &ids {
                w.delete_term(Term::from_field_u64(fx.asset_id, *id));
                w.add_document(make_doc(&fx, *id, &gen_values(*id, &sp, 1), false)).unwrap();
            }
        });
        let (_, d_commit) = time(|| w.commit().unwrap());
        let (_, d_reload) = time(|| reader.reload().unwrap());
        println!(
            "\n갱신 {} 건(delete+add): 투입 {:.0} ms, commit {:.0} ms, reader.reload {:.0} ms, segments={}",
            ids.len(), d_add.as_secs_f64() * 1000.0, d_commit.as_secs_f64() * 1000.0, d_reload.as_secs_f64() * 1000.0,
            reader.searcher().segment_readers().len()
        );
        drop(w);
        let s2 = reader.searcher();
        println!("\n갱신 후 재측정 (삭제 표시 + 추가 세그먼트 존재)");
        println!("| 질의 종류 | Count P50/P95/P99 | Top50 P50/P95/P99 |");
        println!("|---|---|---|");
        for name in ["복합4: +captured 2년 + num f6 50%", "복합6 광범위: media + captured 5년 + date f10 5년 + num f8 50% + multi_any f19 + rating>=1", "missing f3 + video + date f10 1년"] {
            let k = ks.iter().find(|k| k.name == name).unwrap();
            let row = run_kind(&s2, &fx, k, 5, 20);
            println!("| {} | {} / {} / {} | {} / {} / {} |", row.name, f(row.count.p50_ms), f(row.count.p95_ms), f(row.count.p99_ms), f(row.top.p50_ms), f(row.top.p95_ms), f(row.top.p99_ms));
        }
    }
    println!("\npeak RSS: {:.0} MB", mb(peak_rss_bytes()));
}
