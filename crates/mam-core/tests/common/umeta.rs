//! 사용자 메타데이터 필드 시험 공용 환경 — 실제 SQLite(마이그레이션 적용) + 직접 INSERT 한 자산 (스캔 없이 대량 가능).
#![allow(dead_code)]

use std::sync::Arc;

use mam_core::userdata::{FieldPatch, FieldValue, MetadataField, UserData, ValueType};

use super::env::Env;

pub struct Fx {
    pub env: Env,
    pub ud: UserData,
    pub ids: Vec<i64>,
}

/// 자산 `n` 개 (asset_id 1..=n) 를 한 트랜잭션으로 넣는다.
pub fn fixture(n: usize) -> Fx {
    let env = Env::new();
    env.pool
        .write(mam_db::Priority::High, move |c| {
            let tx = c.transaction()?;
            {
                let mut s = tx.prepare(
                    "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
                     VALUES ('image', 10, 0, 0, 0)",
                )?;
                for _ in 0..n {
                    s.execute([])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    let ids = (1..=n as i64).collect();
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());
    Fx { env, ud, ids }
}

impl Fx {
    pub fn field(&self, name: &str, vt: ValueType, opts: &[&str]) -> MetadataField {
        let opts: Vec<String> = opts.iter().map(|s| s.to_string()).collect();
        self.ud.create_metadata_field(name, vt, &opts).unwrap()
    }

    pub fn revision(&self, id: i64) -> i64 {
        self.env.count(&format!(
            "SELECT revision FROM media_assets WHERE asset_id = {id}"
        ))
    }

    /// 자산의 outbox 이벤트 `(revision, priority)` 목록 (event_id 순)
    pub fn outbox(&self, id: i64) -> Vec<(i64, i64)> {
        self.env.read(|c| {
            let mut s = c
                .prepare("SELECT revision, priority FROM search_outbox WHERE asset_id = ?1 ORDER BY event_id")
                .unwrap();
            s.query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        })
    }

    pub fn outbox_total(&self) -> i64 {
        self.env.count("SELECT count(*) FROM search_outbox")
    }

    pub fn value_rows(&self) -> i64 {
        self.env.count("SELECT count(*) FROM asset_metadata_values")
    }

    pub fn set(&self, asset: i64, field: i64, v: FieldValue) {
        self.ud
            .set_metadata_values(asset, &[(field, FieldPatch::Set(v))])
            .unwrap();
    }

    pub fn get(&self, asset: i64, field: i64) -> Option<FieldValue> {
        self.ud
            .get_metadata_values(asset)
            .unwrap()
            .into_iter()
            .find(|v| v.field_id == field)
            .map(|v| v.value)
    }
}
