//! 검색 품질 평가 — 정답은 SQLite 에서 계산한다 (검색 결과로 정하지 않는다).
//!
//! 실행: `cargo run --release -p mam-search --example eval_quality -- --size 10000 --strategy a`
//! 질의 파일 생성(한 번, 체크인용): `... --example eval_quality -- --write-queries`
//!
//! 질의 파일 `examples/data/quality_queries.tsv` 열: query, kind, notes, author, truth_terms.
//! - kind `fn_*`/`tricky_fn`  : 정답 = 정규화(NFKC+소문자)한 파일명(확장자 포함)이 truth_terms 를 모두 부분 문자열로 포함
//! - kind `ft_*`/`tricky_ft`  : 정답 = 태그·AI 키워드·설명·메모를 이은 텍스트가 truth_terms(표제어)를 모두 포함
//! - kind `tricky_any`        : 단어마다 (파일명 또는 텍스트)에 포함, 단어끼리는 AND
//!   author=agent 인 30개는 **사람이 아니라 에이전트가 작성**했다 (사람 검토 전).
//!
//! 지표: Recall@50 = |top50 ∩ 정답| / min(50, |정답|), Precision@10 = |top10 ∩ 정답| / min(10, |정답|).

#[path = "common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use common::vocab::{ALL_PARTICLES, FN_KO, FN_PLACES, Josa, TEXT_EN, TEXT_KO, with_josa};
use common::{args, json_str, load, sysinfo};
use mam_search::preprocess::normalize;
use mam_search::{SearchIndex, SearchRequest};

struct Q {
    query: String,
    kind: String,
    notes: String,
    author: String,
    terms: Vec<String>,
}

fn queries_path(a: &args::Args) -> PathBuf {
    a.path("queries").unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/data/quality_queries.tsv")
    })
}

fn gen_q(query: &str, kind: &str, notes: &str, author: &str, terms: &str) -> String {
    format!("{query}\t{kind}\t{notes}\t{author}\t{terms}")
}

/// 생성 질의 120개 + 에이전트 작성 30개. 데이터셋과 무관하게 결정적이다.
fn build_query_lines() -> Vec<String> {
    let g = "generated";
    let mut out = vec!["query\tkind\tnotes\tauthor\ttruth_terms".to_string()];
    let low = |s: &str| normalize(s).to_lowercase();

    // fn_ko2: 2음절 한국어 (지명 18 + 일반 2)
    for w in FN_PLACES.iter().chain(["가족", "친구"].iter()) {
        out.push(gen_q(w, "fn_ko2", "2-syllable Korean filename word", g, w));
    }
    // fn_ko_long: 3음절 이상 단어·복합명사
    for w in [
        "가족사진",
        "여행사진",
        "졸업사진",
        "증명사진",
        "결혼사진",
        "여행영상",
        "프로젝트",
        "발표자료",
        "동창회",
        "회의록",
    ] {
        out.push(gen_q(
            w,
            "fn_ko_long",
            "3+ syllable word / compound noun",
            g,
            w,
        ));
    }
    // fn_ko_inner: 단어 중간·형태소 경계를 가로지르는 조각
    for (w, n) in [
        (
            "사진",
            "inside compounds 여행사진/가족사진 (ko-dic keeps compound as one token)",
        ),
        ("행사", "inside 여행사진 and the word 행사"),
        ("족사진", "cross-morpheme inside 가족사진"),
        ("업사", "cross-morpheme inside 졸업사진"),
        ("명사", "cross-morpheme inside 증명사진"),
        ("혼사", "cross-morpheme inside 결혼사진"),
        ("표자", "inside 발표자료"),
        ("젝트", "inside 프로젝트"),
        ("창회", "inside 동창회"),
        ("의록", "inside 회의록"),
    ] {
        out.push(gen_q(w, "fn_ko_inner", n, g, w));
    }
    // fn_en: 영어 단어와 단어 중간 조각
    for (w, n) in [
        ("trip", "whole word"),
        ("beach", "whole word"),
        ("vacat", "prefix of vacation"),
        ("ummer", "inside summer (also camelCase pieces)"),
        ("report", "whole word"),
        ("eport", "inside report"),
        ("design", "whole word"),
        ("ogo", "inside logo"),
        ("backup", "whole word"),
        ("draft", "whole word"),
        ("hiking", "whole word"),
        ("elfie", "inside selfie"),
        ("memo", "whole word"),
        ("ample", "inside sample"),
        ("ecture", "inside lecture"),
    ] {
        out.push(gen_q(w, "fn_en", n, g, w));
    }
    // fn_digits: 날짜·번호 조각
    for (w, n) in [
        ("0815", "MMDD inside date names"),
        ("1225", "MMDD"),
        ("0101", "MMDD"),
        ("0504", "MMDD"),
        ("0630", "MMDD"),
        ("0928", "MMDD"),
        ("1111", "repeated digits"),
        ("2023", "year"),
        ("2019", "year"),
        ("2015", "year"),
        ("4821", "camera counter"),
        ("1234", "camera counter"),
        ("202408", "year+month inside date"),
        ("20240815", "full date"),
        ("143052", "time part"),
    ] {
        out.push(gen_q(w, "fn_digits", n, g, w));
    }
    // ft_plain: 표제어 (인기 하위 구간 위주 = 선택적)
    for i in (30..TEXT_KO.len()).step_by(5).take(14) {
        out.push(gen_q(
            TEXT_KO[i],
            "ft_plain",
            "Korean lemma (less popular half)",
            g,
            TEXT_KO[i],
        ));
    }
    for i in (1..TEXT_EN.len()).step_by(4).take(6) {
        out.push(gen_q(
            TEXT_EN[i],
            "ft_plain",
            "English lemma",
            g,
            TEXT_EN[i],
        ));
    }
    // ft_particle: 조사 변형 질의 -> 표제어 문서
    let kinds = [Josa::Subject, Josa::Object, Josa::With, Josa::Topic];
    for (n, i) in (20..TEXT_KO.len()).step_by(4).take(20).enumerate() {
        let l = TEXT_KO[i];
        let q = if n % 5 == 4 {
            format!("{l}{}", ["에서", "도", "만", "의"][n / 5 % 4])
        } else {
            with_josa(l, kinds[n % 4])
        };
        out.push(gen_q(&q, "ft_particle", "lemma + particle", g, l));
    }
    // ft_multi: 두 단어 AND
    for (q, t) in [
        ("하늘 노을", "하늘|노을"),
        ("해변 파도", "해변|파도"),
        ("강아지 공원", "강아지|공원"),
        ("고양이 거실", "고양이|거실"),
        ("커피 카페", "커피|카페"),
        ("강아지가 바다", "강아지|바다"),
        ("케이크 생일을", "케이크|생일"),
        ("등산 단풍", "등산|단풍"),
        ("고양이 sunset", "고양이|sunset"),
        ("coffee 카페에서", "coffee|카페"),
    ] {
        out.push(gen_q(q, "ft_multi", "two words (AND)", g, t));
    }
    let _ = (ALL_PARTICLES, FN_KO);

    // 에이전트 작성 30개 (사람이 아님)
    let ag = "agent";
    for (q, n) in [
        (
            "사진",
            "compound-noun split: inside 여행사진/가족사진; A expected to miss",
        ),
        ("여행", "word and compound part (부산여행, 여행사진)"),
        ("행사", "cross-morpheme inside 여행사진"),
        ("부산여", "crosses the 부산|여행 morpheme boundary"),
        (
            "제주",
            "proper noun (place); also prefix of text tag 제주도",
        ),
        ("부산trip", "Korean-English adjacency in one piece pair"),
        (
            "summervacation",
            "lowercase joined query vs camelCase SummerVacation filename; ngram/regex are per-piece so expected to miss",
        ),
        (
            "SummerVacation",
            "camelCase typed as in filename; splits into summer+vacation",
        ),
        (
            "0815",
            "digits inside 8-digit date 20240815; A cannot match inside token",
        ),
        ("240815", "6-digit tail of a date"),
        ("img", "camera prefix, huge truth set"),
        ("스크린샷", "Korean screenshot prefix"),
        ("Screenshot", "capitalised English prefix"),
        ("jpg", "extension typed as a word"),
        ("4821", "camera counter digits"),
        ("녹음", "Korean recording prefix/word"),
        ("가", "single Hangul syllable; 1-char policy: B/C ignore"),
    ] {
        out.push(gen_q(q, "tricky_fn", n, ag, &low(q)));
    }
    for (q, t, n) in [
        ("강아지를", "강아지", "particle on very popular lemma"),
        (
            "아이스크림을",
            "아이스크림",
            "foreign-origin compound noun + particle",
        ),
        ("해운대", "해운대", "proper noun tag; ko-dic may split"),
        (
            "에버랜드",
            "에버랜드",
            "proper noun tag (brand); likely OOV",
        ),
        ("한라산을", "한라산", "proper noun + particle"),
        ("sunset 바다", "sunset|바다", "Korean-English mixed AND"),
        ("Sunset", "sunset", "case folding"),
        ("떡볶이가", "떡볶이", "noun ending in 이 + subject particle"),
    ] {
        out.push(gen_q(q, "tricky_ft", n, ag, t));
    }
    for (q, t, n) in [
        (
            "제주",
            "제주",
            "place in filenames AND prefix of text tag 제주도 (substring truth includes both)",
        ),
        ("광주", "광주", "place name in filenames; text has no 광주"),
        ("파티", "파티", "text lemma; no filename use"),
        ("경복궁에서", "경복궁", "proper-noun tag + particle"),
        (
            "부산 해변",
            "부산|해변",
            "word 1 in filename, word 2 in tags/AI text (cross-field AND)",
        ),
    ] {
        out.push(gen_q(q, "tricky_any", n, ag, t));
    }
    out
}

fn read_queries(path: &PathBuf) -> Vec<Q> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| {
            args::fail(&format!(
                "{}: {e} (run with --write-queries)",
                path.display()
            ))
        })
        .replace("\r\n", "\n");
    text.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() != 5 {
                args::fail(&format!("bad TSV line (need 5 columns): {l}"));
            }
            Q {
                query: c[0].to_string(),
                kind: c[1].to_string(),
                notes: c[2].to_string(),
                author: c[3].to_string(),
                terms: c[4]
                    .split('|')
                    .map(|t| normalize(t).to_lowercase())
                    .collect(),
            }
        })
        .collect()
}

struct Corpus {
    ids: Vec<u64>,
    names: Vec<String>,
    blobs: Vec<String>,
    idx: HashMap<u64, usize>,
}

fn load_corpus(conn: &rusqlite::Connection) -> Corpus {
    let mut stmt = conn.prepare(load::ALL_DOCS_SQL).unwrap();
    let mut rows = stmt.query([]).unwrap();
    let mut c = Corpus {
        ids: vec![],
        names: vec![],
        blobs: vec![],
        idx: HashMap::new(),
    };
    while let Some(r) = rows.next().unwrap() {
        let d = load::doc_from_row(r).unwrap();
        let blob = [
            d.user_tags.join(" "),
            d.ai_keywords.join(" "),
            d.ai_description.clone(),
            d.user_memo.clone(),
        ]
        .join(" ");
        c.idx.insert(d.asset_id, c.ids.len());
        c.ids.push(d.asset_id);
        c.names.push(normalize(&d.file_name).to_lowercase());
        c.blobs.push(normalize(&blob).to_lowercase());
    }
    c
}

fn is_truth(c: &Corpus, q: &Q, i: usize) -> bool {
    q.terms.iter().all(|t| {
        let in_name = c.names[i].contains(t.as_str());
        let in_text = c.blobs[i].contains(t.as_str());
        match q.kind.as_str() {
            k if k.starts_with("fn_") || k == "tricky_fn" => in_name,
            k if k.starts_with("ft_") || k == "tricky_ft" => in_text,
            _ => in_name || in_text,
        }
    })
}

struct Outcome {
    q: usize,
    truth: usize,
    hits: usize,
    recall50: f64,
    p10: f64,
    evidence: String,
}

fn main() {
    let a = args::Args::parse();
    let qpath = queries_path(&a);
    if a.flag("write-queries") {
        std::fs::create_dir_all(qpath.parent().unwrap()).unwrap();
        let lines = build_query_lines();
        std::fs::write(&qpath, lines.join("\n") + "\n").unwrap();
        println!("wrote {} queries to {}", lines.len() - 1, qpath.display());
        return;
    }
    let size: u64 = a.required("size");
    let (letter, strategy, opts) = common::parse_strategy(a.get("strategy").unwrap_or("a"));
    let layout = common::Layout::new(size, a.path("out"));
    let conn = common::open_db(&layout.db());
    let ds_seed = common::setting(&conn, "poc.seed").unwrap_or_default();
    let queries = read_queries(&qpath);
    let corpus = load_corpus(&conn);
    let index_dir = layout.index(letter);
    if !index_dir.exists() {
        args::fail("index not found - run build_index first");
    }
    let index = SearchIndex::open_or_create(&index_dir, opts).expect("open index");
    let segments = common::segment_count(&index_dir);

    let mut outcomes = Vec::new();
    let mut skipped = Vec::new();
    for (qi, q) in queries.iter().enumerate() {
        let truth = (0..corpus.ids.len())
            .filter(|&i| is_truth(&corpus, q, i))
            .count();
        if truth == 0 {
            skipped.push(qi);
            continue;
        }
        let req = SearchRequest {
            text: q.query.clone(),
            strategy,
            limit: 50,
            ..Default::default()
        };
        let hits = index.search(&req).expect("search");
        let ok = |h: &mam_search::Hit| {
            corpus
                .idx
                .get(&h.asset_id)
                .is_some_and(|&i| is_truth(&corpus, q, i))
        };
        let in50 = hits.iter().filter(|h| ok(h)).count();
        let in10 = hits.iter().take(10).filter(|h| ok(h)).count();
        let evidence = if hits.is_empty() {
            "engine returned no hits".to_string()
        } else {
            let wrong = hits
                .iter()
                .find(|h| !ok(h))
                .and_then(|h| corpus.idx.get(&h.asset_id))
                .map(|&i| corpus.names[i].clone());
            // 정답이 50건 이하일 때만 "놓친 정답" 예시가 의미 있다 (그보다 많으면 top50 밖은 정상)
            let missed = (truth <= 50)
                .then(|| {
                    (0..corpus.ids.len())
                        .find(|&i| {
                            is_truth(&corpus, q, i)
                                && !hits.iter().any(|h| h.asset_id == corpus.ids[i])
                        })
                        .map(|i| corpus.names[i].clone())
                })
                .flatten();
            let mut parts = Vec::new();
            if let Some(w) = wrong {
                parts.push(format!("wrong hit e.g. {w:?}"));
            }
            if let Some(m) = missed {
                parts.push(format!("missed truth e.g. {m:?}"));
            }
            if parts.is_empty() {
                "none".to_string()
            } else {
                parts.join("; ")
            }
        };
        outcomes.push(Outcome {
            q: qi,
            truth,
            hits: hits.len(),
            recall50: in50 as f64 / truth.min(50) as f64,
            p10: in10 as f64 / truth.min(10) as f64,
            evidence,
        });
    }

    let cond = sysinfo::conditions();
    println!("\n== eval_quality ==");
    common::print_conditions(&cond);
    println!(
        "Dataset: size={size} dataset_seed={ds_seed} strategy={} index segments={segments}; queries={} (skipped for empty ground truth: {})",
        letter.to_ascii_uppercase(),
        queries.len(),
        skipped.len()
    );
    println!(
        "NOTE: the 30 `tricky_*` queries (author=agent) were written by an AI agent, not a human; review before relying on them."
    );
    println!(
        "Recall@50 = |top50 ∩ truth| / min(50,|truth|); P@10 = |top10 ∩ truth| / min(10,|truth|). Ground truth from SQLite substring/lemma rules, not from search results.\n"
    );
    println!(
        "| kind | author | queries | truth size (min/median/max) | Recall@50 | P@10 | zero-hit queries |"
    );
    println!("|---|---|---:|---|---:|---:|---:|");
    let mut kinds: Vec<(String, String)> = Vec::new();
    for o in &outcomes {
        let k = (queries[o.q].kind.clone(), queries[o.q].author.clone());
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    let mut kind_json = Vec::new();
    for (kind, author) in &kinds {
        let sel: Vec<&Outcome> = outcomes
            .iter()
            .filter(|o| queries[o.q].kind == *kind)
            .collect();
        let mut sizes: Vec<usize> = sel.iter().map(|o| o.truth).collect();
        sizes.sort_unstable();
        let n = sel.len() as f64;
        let (r, p) = (
            sel.iter().map(|o| o.recall50).sum::<f64>() / n,
            sel.iter().map(|o| o.p10).sum::<f64>() / n,
        );
        let zero = sel.iter().filter(|o| o.hits == 0).count();
        println!(
            "| {kind} | {author} | {} | {}/{}/{} | {r:.3} | {p:.3} | {zero} |",
            sel.len(),
            sizes[0],
            sizes[sizes.len() / 2],
            sizes[sizes.len() - 1]
        );
        kind_json.push(format!("{{\"kind\":{},\"author\":{},\"n\":{},\"recall50\":{r:.4},\"p10\":{p:.4},\"zero_hit\":{zero}}}", json_str(kind), json_str(author), sel.len()));
    }
    // 합격 기준(초기, PoC 한정): 생성 질의의 파일명 Recall@50 >= 0.95, 풀텍스트 Recall@50 >= 0.9
    let agg = |pred: &dyn Fn(&Q) -> bool| -> (f64, usize) {
        let s: Vec<&Outcome> = outcomes.iter().filter(|o| pred(&queries[o.q])).collect();
        (
            s.iter().map(|o| o.recall50).sum::<f64>() / s.len().max(1) as f64,
            s.len(),
        )
    };
    let (fn_r, fn_n) = agg(&|q| q.author == "generated" && q.kind.starts_with("fn_"));
    let (ft_r, ft_n) = agg(&|q| q.author == "generated" && q.kind.starts_with("ft_"));
    let (tfn_r, tfn_n) = agg(&|q| q.kind == "tricky_fn");
    let (tft_r, tft_n) = agg(&|q| q.kind == "tricky_ft" || q.kind == "tricky_any");
    println!(
        "\nGenerated filename-partial Recall@50 = {fn_r:.3} over {fn_n} queries (criterion >= 0.95: {})",
        if fn_r >= 0.95 { "PASS" } else { "FAIL" }
    );
    println!(
        "Generated full-text Recall@50       = {ft_r:.3} over {ft_n} queries (criterion >= 0.90: {})",
        if ft_r >= 0.90 { "PASS" } else { "FAIL" }
    );
    println!(
        "Agent-authored tricky filename      = {tfn_r:.3} over {tfn_n}; tricky text/any = {tft_r:.3} over {tft_n}"
    );

    let mut worst: Vec<&Outcome> = outcomes
        .iter()
        .filter(|o| o.recall50 < 1.0 || o.p10 < 1.0)
        .collect();
    worst.sort_by(|x, y| {
        x.recall50
            .partial_cmp(&y.recall50)
            .unwrap()
            .then(x.p10.partial_cmp(&y.p10).unwrap())
    });
    println!("\n### Worst 10 queries (only those with Recall@50 < 1 or P@10 < 1)\n");
    println!(
        "| query | kind | author | truth | hits | Recall@50 | P@10 | notes / evidence |\n|---|---|---|---:|---:|---:|---:|---|"
    );
    for o in worst.iter().take(10) {
        let q = &queries[o.q];
        println!(
            "| {} | {} | {} | {} | {} | {:.3} | {:.3} | {} - {} |",
            q.query, q.kind, q.author, o.truth, o.hits, o.recall50, o.p10, q.notes, o.evidence
        );
    }
    if !skipped.is_empty() {
        println!(
            "\nSkipped (empty ground truth at this size): {}",
            skipped
                .iter()
                .map(|&i| queries[i].query.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    println!(
        "JSON: {{\"tool\":\"eval_quality\",\"size\":{size},\"strategy\":\"{}\",\"dataset_seed\":{},\"segments\":{segments},\"queries\":{},\"skipped\":{},\"generated_fn_recall50\":{fn_r:.4},\"generated_ft_recall50\":{ft_r:.4},\"tricky_fn_recall50\":{tfn_r:.4},\"tricky_ft_any_recall50\":{tft_r:.4},\"kinds\":[{}],\"conditions\":{}}}",
        letter,
        json_str(&ds_seed),
        queries.len(),
        skipped.len(),
        kind_json.join(","),
        common::conditions_json(&cond)
    );
}
