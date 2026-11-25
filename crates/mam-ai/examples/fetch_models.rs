//! 내장 매니페스트의 모델을 실제 서버에서 내려받는다 (URL·SHA-256 고정값·이어받기 실검증용 수동 도구).
//!
//! `fetch_models --root <MAMData/models> [--id <model_id> ...] [--verify]` (id 생략 시 전체)

#[path = "common/mod.rs"]
mod common;

use std::time::Instant;

use common::{Args, fail};
use mam_ai::models::{Manifest, ModelStore};

fn main() {
    let a = Args::parse();
    let manifest = Manifest::builtin().unwrap_or_else(|e| fail(&e.to_string()));
    let ids: Vec<String> = std::env::args()
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| w[0] == "--id")
        .map(|w| w[1].clone())
        .collect();
    let ids = if ids.is_empty() {
        manifest.models.iter().map(|m| m.id.clone()).collect()
    } else {
        ids
    };
    let store = ModelStore::new(a.require("root"), manifest);
    for id in ids {
        let t = Instant::now();
        let mut last_pct = u64::MAX;
        let r = store.ensure(&id, &mut |file, got, total| {
            let pct = got * 100 / total.max(1);
            if pct / 10 != last_pct / 10 {
                eprintln!("  {id}/{file}: {pct}%");
                last_pct = pct;
            }
        });
        match r {
            Ok(p) => {
                let bad = if a.has("verify") {
                    store.verify(&id).unwrap_or_else(|e| fail(&e.to_string()))
                } else {
                    Vec::new()
                };
                println!(
                    "{id}: ready at {} ({:.1}s) verify-failures={bad:?}",
                    p.dir.display(),
                    t.elapsed().as_secs_f64()
                );
            }
            Err(e) => println!("{id}: FAILED {e}"),
        }
    }
}
