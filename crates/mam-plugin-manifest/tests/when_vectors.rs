//! 분류: Happy / Validation / Boundary (공용 벡터). External·Concurrency 는 순수 crate 라 해당 없음.
//! 벡터 파일은 Rust·TS(P8) 간 계약이다.

use mam_plugin_manifest::when::{IDENTIFIERS, IdentType};
use mam_plugin_manifest::{WhenContext, WhenErrorKind, WhenExpr, WhenValue};
use serde_json::Value;

const VECTORS: &str = include_str!("../vectors/when.json");

fn load() -> Value {
    // Arrange: 소스가 CRLF 로 체크아웃돼도 JSON 은 영향 없지만 정규화해 둔다
    serde_json::from_str(&VECTORS.replace("\r\n", "\n")).expect("vectors/when.json")
}

fn context(v: &Value) -> WhenContext {
    let mut ctx = WhenContext::new();
    for (k, val) in v.as_object().expect("context object") {
        let wv = match val {
            Value::Bool(b) => WhenValue::Bool(*b),
            Value::Number(n) => WhenValue::Num(n.as_f64().expect("f64")),
            Value::String(s) => WhenValue::Str(s.clone()),
            _ => WhenValue::Null,
        };
        ctx.insert(k, wv);
    }
    ctx
}

fn kind_name(k: WhenErrorKind) -> &'static str {
    match k {
        WhenErrorKind::Syntax => "syntax",
        WhenErrorKind::UnknownIdentifier => "unknownIdentifier",
        WhenErrorKind::TypeMismatch => "typeMismatch",
        WhenErrorKind::TooLong => "tooLong",
        WhenErrorKind::TooDeep => "tooDeep",
    }
}

#[test]
fn t_pmanifest_08_all_vectors_pass() {
    let doc = load();
    let cases = doc["cases"].as_array().expect("cases");
    assert!(cases.len() >= 40, "need >= 40 vectors, got {}", cases.len());

    for c in cases {
        let id = c["id"].as_str().expect("id");
        let expr = c["expr"].as_str().expect("expr");
        let parsed = WhenExpr::parse(expr);
        match &c["expect"] {
            Value::Bool(want) => {
                let e = parsed.unwrap_or_else(|err| panic!("{id}: unexpected {err:?}"));
                assert_eq!(e.eval(&context(&c["context"])), *want, "{id}");
            }
            exp => {
                let want = exp["error"].as_str().expect("error kind");
                let err = parsed.expect_err(id);
                assert_eq!(kind_name(err.kind), want, "{id}");
            }
        }
    }
}

#[test]
fn t_pmanifest_08_identifier_table_matches_vectors() {
    let doc = load();
    let table = doc["identifiers"].as_object().expect("identifiers");
    assert_eq!(table.len(), IDENTIFIERS.len());
    for (name, ty) in IDENTIFIERS {
        let want = &table[*name];
        match ty {
            IdentType::Number => assert_eq!(want, "number"),
            IdentType::Bool => assert_eq!(want, "bool"),
            IdentType::Enum(values) => {
                let got: Vec<&str> = want["enum"]
                    .as_array()
                    .expect("enum")
                    .iter()
                    .filter_map(Value::as_str)
                    .collect();
                assert_eq!(&got[..], *values);
            }
        }
    }
}

#[test]
fn t_pmanifest_08_runtime_type_error_is_fail_closed() {
    let e = WhenExpr::parse("asset.offline").expect("parse");
    let ctx = WhenContext::new().with("asset.offline", WhenValue::Str("yes".into()));
    assert!(e.try_eval(&ctx).is_err());
    assert!(!e.eval(&ctx));
}

#[test]
fn t_pmanifest_08_when_serializes_as_source_string() {
    let e: WhenExpr = serde_json::from_str("\"asset.offline\"").expect("de");
    assert_eq!(serde_json::to_string(&e).expect("ser"), "\"asset.offline\"");
    assert!(serde_json::from_str::<WhenExpr>("\"nope.x\"").is_err());
}
