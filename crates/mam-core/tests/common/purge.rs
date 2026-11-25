//! 폴더 제거·연결 데이터 정리 시험 공용 — SQL 로 자산·위치·연결 데이터를 직접 심고 남은 행을 센다 (실제 SQLite).
#![allow(dead_code)]

use mam_core::purge::LINKED_TABLES;
use mam_db::Priority;

use super::env::Env;

pub fn exec(env: &Env, sql: &str) {
    let sql = sql.to_string();
    env.pool
        .write(Priority::High, move |c| Ok(c.execute_batch(&sql)))
        .unwrap()
        .unwrap();
}

pub fn volume_id(env: &Env, key: &str) -> i64 {
    env.count(&format!(
        "SELECT volume_id FROM volumes WHERE volume_key = '{key}'"
    ))
}

pub fn ids_sql(ids: &[i64]) -> String {
    ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
}

/// `vol_key` 볼륨의 `dir` 아래에 활성 위치를 가진 활성 자산 `n` 개 (경로 `dir/f<i>.jpg`)
pub fn insert_assets(env: &Env, vol_key: &str, dir: &str, n: usize, media_type: &str) -> Vec<i64> {
    let vid = volume_id(env, vol_key);
    let (dir, media_type) = (dir.to_string(), media_type.to_string());
    env.pool
        .write(Priority::High, move |c| {
            let tx = c.transaction()?;
            let mut ids = Vec::with_capacity(n);
            {
                let mut a = tx.prepare(
                    "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
                     VALUES (?1, 1, 1, 1, 1)",
                )?;
                let mut l = tx.prepare(
                    "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
                     VALUES (?1, ?2, ?3, ?4, 1)",
                )?;
                for i in 0..n {
                    a.execute([&media_type])?;
                    let id = tx.last_insert_rowid();
                    let rel = format!("{dir}/f{id}_{i}.jpg");
                    l.execute(rusqlite::params![id, vid, rel, mam_db::path_key(&rel).unwrap()])?;
                    ids.push(id);
                }
            }
            tx.commit()?;
            Ok(ids)
        })
        .unwrap()
}

/// 자산마다 모든 연결 테이블에 한 행 이상 + 지우지 않을 `ai_usage` 이력 한 행을 심는다.
/// 태그·컬렉션은 이름이 고정이라 재사용한다 (`seed-tag`, `seed-collection`).
pub fn seed_linked(env: &Env, ids: &[i64]) {
    let ids = ids.to_vec();
    env.pool
        .write(Priority::High, move |c| {
            let tx = c.transaction()?;
            tx.execute("INSERT OR IGNORE INTO tags (name, name_key) VALUES ('seed-tag', 'seed-tag')", [])?;
            tx.execute(
                "INSERT OR IGNORE INTO collections (name, name_key, created_at) VALUES ('seed-collection', 'seed-collection', 1)",
                [],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO metadata_fields (name, name_key, value_type, position, created_at, updated_at) VALUES ('seed-field', 'seed-field', 'number', 0, 1, 1)",
                [],
            )?;
            let fld: i64 = tx.query_row("SELECT field_id FROM metadata_fields WHERE name_key = 'seed-field'", [], |r| r.get(0))?;
            let tag: i64 = tx.query_row("SELECT tag_id FROM tags WHERE name_key = 'seed-tag'", [], |r| r.get(0))?;
            let col: i64 = tx.query_row("SELECT collection_id FROM collections WHERE name_key = 'seed-collection'", [], |r| r.get(0))?;
            // 플러그인 레지스트리 (정수 키 — 행은 지우지 않으므로 자산 정리 뒤에도 남는다)
            tx.execute("INSERT OR IGNORE INTO plugins (plugin_id) VALUES ('seed-plugin')", [])?;
            tx.execute("INSERT OR IGNORE INTO plugin_providers (plugin_ref, name) SELECT plugin_ref, 'prov' FROM plugins WHERE plugin_id = 'seed-plugin'", [])?;
            // 플러그인 전역 KV (asset_id NULL) — 자산 정리 뒤에도 남아야 한다 (`global_plugin_kv_rows`)
            tx.execute(
                "INSERT OR IGNORE INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) SELECT plugin_ref, NULL, 'global', '1', 1 FROM plugins WHERE plugin_id = 'seed-plugin'",
                [],
            )?;
            for id in ids {
                tx.execute_batch(&format!(
                    "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES ({id}, {tag}, 1);
                     INSERT INTO collection_assets (collection_id, asset_id, added_at) VALUES ({col}, {id}, 1);
                     INSERT INTO asset_user_data (asset_id, memo, favorite, updated_at) VALUES ({id}, 'note', 1, 1);
                     INSERT INTO asset_play_ranges (asset_id, start_ms, end_ms) VALUES ({id}, 0, 10);
                     INSERT INTO asset_resume_positions (asset_id, position_ms, duration_ms, updated_at) VALUES ({id}, 60000, 600000, 1);
                     INSERT INTO asset_scene_cuts (asset_id, state, updated_at) VALUES ({id}, 'pending', 1);
                     INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_real, updated_at) VALUES ({id}, {fld}, 0, 1.5, 1);
                     INSERT OR REPLACE INTO media_metadata (asset_id, width, height) VALUES ({id}, 1, 1);
                     INSERT OR REPLACE INTO asset_derivatives (asset_id, kind, state, updated_at) VALUES ({id}, 'thumb', 'done', 1);
                     INSERT INTO transcripts (asset_id, language, text, engine, model, created_at) VALUES ({id}, 'ko', 't', 'e', 'm', 1);
                     INSERT INTO transcript_segments (transcript_id, seq, start_ms, end_ms, text)
                       VALUES ((SELECT transcript_id FROM transcripts WHERE asset_id = {id}), 0, 0, 1, 's');
                     INSERT INTO embeddings (asset_id, vector_kind, source, model_id, model_version, dimension, raw_offset, created_at)
                       VALUES ({id}, 'visual', 'image', 'm', '1', 4, 0, 1);
                     INSERT INTO ai_analyses (asset_id, provider, model, analyzed_at, updated_at) VALUES ({id}, 'p', 'm', 1, 1);
                     INSERT INTO ai_analysis_replaced (asset_id, description, keywords, ocr_text, summary, language, replaced_at)
                       VALUES ({id}, '', '[]', '', '', '', 1);
                     INSERT INTO ai_task_state (asset_id, kind, state, updated_at) VALUES ({id}, 'describe', 'done', 1);
                     INSERT INTO analysis_runs (asset_id, kind, provider, ok, attempts, at) VALUES ({id}, 'describe', 'p', 1, 1, 1);
                     INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) SELECT plugin_ref, {id}, 'k', '1', 1 FROM plugins WHERE plugin_id = 'seed-plugin';
                     INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, updated_at)
                       SELECT plugin_ref, provider_ref, {id}, 1, 1 FROM plugin_providers WHERE name = 'prov' AND plugin_ref = (SELECT plugin_ref FROM plugins WHERE plugin_id = 'seed-plugin');
                     INSERT INTO jobs (kind, asset_id, created_at, updated_at) VALUES ('ai', {id}, 1, 1);
                     INSERT INTO ai_usage (provider, model, kind, asset_id, ok, at) VALUES ('p', 'm', 'describe', {id}, 1, 1);"
                ))?;
            }
            tx.commit()?;
            Ok(())
        })
        .unwrap();
}

/// 이 자산들에 매인 연결 데이터 행 수 (테이블 이름, 행 수) — 모두 0 이어야 정리가 끝난 것
pub fn linked_rows(env: &Env, ids: &[i64]) -> Vec<(String, i64)> {
    let list = ids_sql(ids);
    let mut tables: Vec<String> = LINKED_TABLES.iter().map(|t| (*t).to_string()).collect();
    tables.push("transcripts".into());
    let mut out: Vec<(String, i64)> = tables
        .into_iter()
        .map(|t| {
            let n = env.count(&format!(
                "SELECT COUNT(*) FROM {t} WHERE asset_id IN ({list})"
            ));
            (t, n)
        })
        .collect();
    let seg = env.count(&format!(
        "SELECT COUNT(*) FROM transcript_segments WHERE transcript_id IN
           (SELECT transcript_id FROM transcripts WHERE asset_id IN ({list}))"
    ));
    out.push(("transcript_segments".into(), seg));
    out
}

/// 정리 대상이 아닌 플러그인 전역 KV 행 수 (`seed_linked` 가 1행 심는다)
pub fn global_plugin_kv_rows(env: &Env) -> i64 {
    env.count("SELECT COUNT(*) FROM plugin_kv WHERE asset_id IS NULL")
}

pub fn total_linked(env: &Env, ids: &[i64]) -> i64 {
    linked_rows(env, ids).iter().map(|(_, n)| n).sum()
}

pub fn status_counts(env: &Env, ids: &[i64], status: &str) -> i64 {
    env.count(&format!(
        "SELECT COUNT(*) FROM media_assets WHERE status = '{status}' AND asset_id IN ({})",
        ids_sql(ids)
    ))
}
