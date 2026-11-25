//! P1-3 측정 도구 공용 코드 — `#[path = "common/mod.rs"] mod common;` 로 가져다 쓴다.
//! 도구마다 쓰는 함수가 달라 미사용 경고를 끈다.
#![allow(dead_code)]

pub mod args;
pub mod data;
pub mod model;
pub mod rng;
pub mod stats;
pub mod sysinfo;

use mam_vector::{Engine, IndexSpec, Quantization};

pub use data::json_str;

pub fn open_db(path: &std::path::Path) -> rusqlite::Connection {
    mam_db::open(path).unwrap_or_else(|e| args::fail(&format!("open {}: {e}", path.display())))
}

/// 모델 ID·버전은 합성 데이터라 고정 문자열이다 (P1-6 에서 실제 모델로 대체).
pub const MODEL_ID: &str = "synthetic-gaussian-mixture";
pub const MODEL_VERSION: &str = "1";

/// `--engine flat --quant f32|f16|i8` -> IndexSpec.
pub fn spec_from_args(a: &args::Args, dim: usize) -> IndexSpec {
    let engine = Engine::parse(a.get("engine").unwrap_or("flat"))
        .unwrap_or_else(|| args::fail("--engine must be flat"));
    let quant = Quantization::parse(a.get("quant").unwrap_or("f16"))
        .unwrap_or_else(|| args::fail("--quant must be f32|f16|i8"));
    IndexSpec {
        engine,
        model_id: MODEL_ID.into(),
        model_version: MODEL_VERSION.into(),
        dimension: dim,
        quantization: quant,
    }
}

/// 인덱스 폴더 이름: `flat-f16`.
pub fn index_name(s: &IndexSpec) -> String {
    format!("{}-{}", s.engine.as_str(), s.quantization.as_str())
}

pub fn print_conditions() -> sysinfo::Conditions {
    let c = sysinfo::conditions();
    println!("CPU: {}", c.cpu);
    println!("RAM: {} GB", c.ram_gb);
    println!("OS: {}", c.os);
    println!("Power plan: {}", c.power_plan);
    println!("Disks: {}", c.disks);
    c
}

pub fn conditions_json(c: &sysinfo::Conditions) -> String {
    format!(
        "{{\"cpu\":{},\"ram_gb\":{},\"os\":{},\"power_plan\":{}}}",
        json_str(&c.cpu),
        json_str(&c.ram_gb),
        json_str(&c.os),
        json_str(&c.power_plan)
    )
}

pub fn gb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}
