//! JSON 숫자 타입 일관성 — 작은 RAM 인덱스로 정수처럼 보이는 f64·i64/f64 혼합·특수값의 질의 누락을 확인한다.

use std::collections::BTreeMap;

use tantivy::collector::DocSetCollector;
use tantivy::query::{Query, QueryParser, TermQuery};
use tantivy::schema::{IndexRecordOption, OwnedValue, Schema};
use tantivy::{Index, TantivyDocument, Term};

use crate::data::sortable_i64;
use crate::q::{
    json_options, range_f64_only, range_i64, range_num_sortable, range_num_union, t_f64, t_i64,
    t_str,
};

fn obj(k: &str, v: OwnedValue) -> BTreeMap<String, OwnedValue> {
    BTreeMap::from([(k.to_string(), v)])
}

fn nofast_demo() {
    use tantivy::collector::Count;
    let mut b = Schema::builder();
    let u = b.add_json_field("umeta", crate::q::json_options_nofast());
    let index = Index::create_in_ram(b.build());
    let mut w = index.writer_with_num_threads(1, 20_000_000).unwrap();
    let mut d = TantivyDocument::default();
    d.add_object(u, obj("f1", OwnedValue::I64(5)));
    w.add_document(d).unwrap();
    w.commit().unwrap();
    let s = index.reader().unwrap().searcher();
    let r = s.search(&*range_i64(u, "f1", 0, 10), &Count);
    println!("JSON INDEXED(비 FAST) 범위 질의 결과: {r:?}");
    let r = s.search(
        &TermQuery::new(t_i64(u, "f1", 5), IndexRecordOption::Basic),
        &Count,
    );
    println!("JSON INDEXED(비 FAST) 일치 질의 결과: {r:?}");
}

pub fn run() {
    nofast_demo();
    let mut b = Schema::builder();
    let name = b.add_text_field("name", tantivy::schema::STRING | tantivy::schema::STORED);
    let umeta = b.add_json_field("umeta", json_options());
    let schema = b.build();
    let index = Index::create_in_ram(schema.clone());
    let mut w = index.writer_with_num_threads(1, 20_000_000).unwrap();
    let add = |w: &mut tantivy::IndexWriter,
               label: &str,
               k: &str,
               v: OwnedValue,
               extra: Option<(&str, OwnedValue)>| {
        let mut d = TantivyDocument::default();
        d.add_text(name, label);
        let mut m = obj(k, v);
        if let Some((k2, v2)) = extra {
            m.insert(k2.to_string(), v2);
        }
        d.add_object(umeta, m);
        w.add_document(d).unwrap();
    };
    // number 필드(f1): F64 로 넣은 값들. n1 은 정렬 가능 i64 인코딩 대안
    let nums = [
        ("n=3.0", 3.0),
        ("n=4.5", 4.5),
        ("n=10.0", 10.0),
        ("n=-2.0", -2.0),
        ("n=0.0", 0.0),
        ("n=-0.0", -0.0),
        ("n=7.25", 7.25),
        ("n=-3.5", -3.5),
        ("n=1e19", 1e19),
        (
            "n=9.007199254740993e15(2^53+1 근처)",
            9_007_199_254_740_993.0,
        ),
        ("n=2.5e300", 2.5e300),
    ];
    for (l, x) in nums {
        add(
            &mut w,
            l,
            "f1",
            OwnedValue::F64(x),
            Some(("n1", OwnedValue::I64(sortable_i64(x)))),
        );
    }
    // 동일 경로에 정수(I64) 가 섞이는 경우 (금지해야 할 입력): date 값을 number 경로에 넣은 것
    add(&mut w, "mixed I64(5)", "f1", OwnedValue::I64(5), None);
    add(
        &mut w,
        "mixed U64 > i64::MAX",
        "f1",
        OwnedValue::U64(u64::MAX - 5),
        None,
    );
    // boolean: 의도(i64 0/1)와 JSON bool 의 혼동
    add(&mut w, "bool as I64(1) f2", "f2", OwnedValue::I64(1), None);
    add(
        &mut w,
        "bool as Bool(true) f2",
        "f2",
        OwnedValue::Bool(true),
        None,
    );
    // keyword 특수값
    add(
        &mut w,
        "kw 날짜형 문자열",
        "f3",
        OwnedValue::Str("2026-11-20T00:00:00Z".into()),
        None,
    );
    add(
        &mut w,
        "kw 숫자형 문자열",
        "f3",
        OwnedValue::Str("123".into()),
        None,
    );
    add(
        &mut w,
        "kw 대문자 혼합(소문자화 전)",
        "f3",
        OwnedValue::Str("ABC def".into()),
        None,
    );
    add(
        &mut w,
        "kw 70000자",
        "f3",
        OwnedValue::Str("가".repeat(70_000 / 3 + 10)),
        None,
    );
    add(
        &mut w,
        "kw 200자",
        "f3",
        OwnedValue::Str("나".repeat(200)),
        None,
    );
    add(
        &mut w,
        "kw 빈 문자열",
        "f3",
        OwnedValue::Str(String::new()),
        None,
    );
    // parse_json 경로
    for (l, js) in [
        ("json \"3\"", r#"{"umeta":{"f4":3}}"#),
        ("json \"3.0\"", r#"{"umeta":{"f4":3.0}}"#),
        ("json \"4.5\"", r#"{"umeta":{"f4":4.5}}"#),
    ] {
        let mut d = TantivyDocument::parse_json(&schema, js).unwrap();
        d.add_text(name, l);
        w.add_document(d).unwrap();
    }
    // multi 값 배열
    add(
        &mut w,
        "multi [1,2,3] f5",
        "f5",
        OwnedValue::Array(vec![
            OwnedValue::I64(1),
            OwnedValue::I64(2),
            OwnedValue::I64(3),
        ]),
        None,
    );
    w.commit().unwrap();
    let reader = index.reader().unwrap();
    let s = reader.searcher();
    let labels = |q: &dyn Query| -> String {
        let mut v: Vec<String> = s
            .search(q, &DocSetCollector)
            .unwrap()
            .into_iter()
            .map(|a| {
                let d: TantivyDocument = s.doc(a).unwrap();
                use tantivy::schema::Value;
                d.get_first(name)
                    .and_then(|x| x.as_str())
                    .unwrap_or("?")
                    .to_string()
            })
            .collect();
        v.sort();
        if v.is_empty() {
            "(없음)".into()
        } else {
            v.join(", ")
        }
    };
    let f = umeta;
    println!("\n| 질의 | 일치 문서 |");
    println!("|---|---|");
    let show = |t: &str, q: Box<dyn Query>| println!("| {t} | {} |", labels(&*q));
    show(
        "f64 범위만 [0.0, 10.0] (f64 term)",
        range_f64_only(f, "f1", 0.0, 10.0),
    );
    show("i64 범위만 [0, 10] (i64 term)", range_i64(f, "f1", 0, 10));
    show(
        "합집합 i64∪f64 [0.0, 10.0]",
        range_num_union(f, "f1", 0.0, 10.0),
    );
    show(
        "합집합 [-5.0, 5.0] (음수 포함)",
        range_num_union(f, "f1", -5.0, 5.0),
    );
    show(
        "합집합 [1e18, 1e20] (i64 상한 초과)",
        range_num_union(f, "f1", 1e18, 1e20),
    );
    show(
        "정렬 i64 대안 [0.0, 10.0] (n1)",
        range_num_sortable(f, 1, 0.0, 10.0),
    );
    show(
        "정렬 i64 대안 [-5.0, 5.0]",
        range_num_sortable(f, 1, -5.0, 5.0),
    );
    show(
        "정렬 i64 대안 [1e18, 1e20]",
        range_num_sortable(f, 1, 1e18, 1e20),
    );
    show(
        "f64 term 일치 3.0 (정규화 없이)",
        Box::new(TermQuery::new(
            t_f64(f, "f1", 3.0),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "i64 term 일치 3",
        Box::new(TermQuery::new(t_i64(f, "f1", 3), IndexRecordOption::Basic)),
    );
    show(
        "f64 term 일치 4.5",
        Box::new(TermQuery::new(
            t_f64(f, "f1", 4.5),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "i64 term 일치 5 (혼합 i64 문서)",
        Box::new(TermQuery::new(t_i64(f, "f1", 5), IndexRecordOption::Basic)),
    );
    show(
        "f64 term 일치 5.0 (혼합 i64 문서)",
        Box::new(TermQuery::new(
            t_f64(f, "f1", 5.0),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "합집합 [4.0, 6.0] (혼합 I64(5) 문서)",
        range_num_union(f, "f1", 4.0, 6.0),
    );
    show("합집합 일치 -0.0..0.0", range_num_union(f, "f1", -0.0, 0.0));
    show(
        "boolean: i64 term 1",
        Box::new(TermQuery::new(t_i64(f, "f2", 1), IndexRecordOption::Basic)),
    );
    {
        let mut t = Term::from_field_json_path(f, "f2", false);
        t.append_type_and_fast_value(true);
        show(
            "boolean: bool term true",
            Box::new(TermQuery::new(t, IndexRecordOption::Basic)),
        );
    }
    show(
        "keyword 날짜형 문자열 str term",
        Box::new(TermQuery::new(
            t_str(f, "f3", "2026-11-20T00:00:00Z"),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "keyword \"123\" str term",
        Box::new(TermQuery::new(
            t_str(f, "f3", "123"),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "keyword \"123\" i64 term",
        Box::new(TermQuery::new(
            t_i64(f, "f3", 123),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "keyword \"ABC def\" 그대로",
        Box::new(TermQuery::new(
            t_str(f, "f3", "ABC def"),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "keyword \"abc def\" (소문자 질의)",
        Box::new(TermQuery::new(
            t_str(f, "f3", "abc def"),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "keyword 70000자 값",
        Box::new(TermQuery::new(
            t_str(f, "f3", &"가".repeat(70_000 / 3 + 10)),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "keyword 200자 값",
        Box::new(TermQuery::new(
            t_str(f, "f3", &"나".repeat(200)),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "keyword 빈 문자열 값",
        Box::new(TermQuery::new(t_str(f, "f3", ""), IndexRecordOption::Basic)),
    );
    show(
        "parse_json f4: i64 term 3",
        Box::new(TermQuery::new(t_i64(f, "f4", 3), IndexRecordOption::Basic)),
    );
    show(
        "parse_json f4: f64 term 3.0",
        Box::new(TermQuery::new(
            t_f64(f, "f4", 3.0),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "parse_json f4: f64 term 4.5",
        Box::new(TermQuery::new(
            t_f64(f, "f4", 4.5),
            IndexRecordOption::Basic,
        )),
    );
    show(
        "multi f5 i64 term 2",
        Box::new(TermQuery::new(t_i64(f, "f5", 2), IndexRecordOption::Basic)),
    );
    // QueryParser 가 만드는 질의 (참고: 질의 파서는 i64/f64 모두 시도하는지)
    let qp = QueryParser::for_index(&index, vec![]);
    for text in [
        "umeta.f1:[0 TO 10]",
        "umeta.f1:[0.0 TO 10.0]",
        "umeta.f1:3",
        "umeta.f1:3.0",
    ] {
        match qp.parse_query(text) {
            Ok(q) => println!("| QueryParser `{text}` | {} |", labels(&*q)),
            Err(e) => println!("| QueryParser `{text}` | 오류: {e} |"),
        }
    }
}
