//! 합성 데이터셋 생성기 — mam-db V1 스키마에 실제로 적재한 SQLite 파일을 만든다.
//!
//! 실행: `cargo run --release -p mam-search --example gen_dataset -- --size 10000 --seed 42`
//! 옵션: `--out <dir>` (기본 target/bench-p1-2/<size>/), `--ai-fraction 0.30`
//!
//! D5: `poc_asset_text` 는 이 측정 DB 에만 만든다 (마이그레이션 아님).

#[path = "common/mod.rs"]
mod common;

use std::collections::HashSet;
use std::time::Instant;

use common::rng::Rng;
use common::synth::{self, NOW_MS, VOLUMES};
use common::{args, json_str, sysinfo, vocab};
use mam_search::preprocess::normalize;
use rusqlite::{Connection, params};

const BATCH: u64 = 50_000;
const FILES_PER_FOLDER: u64 = 100;

fn main() {
    let a = args::Args::parse();
    let size: u64 = a.required("size");
    let seed: u64 = a.num("seed", 42);
    let ai_fraction: f64 = a.num("ai-fraction", 0.30);
    let layout = common::Layout::new(size, a.path("out"));

    let bad = vocab::self_check();
    if !bad.is_empty() {
        for b in &bad {
            eprintln!("vocab collision: {b}");
        }
        args::fail("vocabulary self-check failed");
    }

    std::fs::create_dir_all(&layout.dir).expect("create output dir");
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", layout.db().display()));
    }

    let started = Instant::now();
    let mut conn = common::open_db(&layout.db());
    mam_db::migrate(&mut conn, &layout.dir.join("backup"), &mam_db::SystemClock)
        .expect("migrate V1");
    // 이 측정 DB 는 매번 새로 만든다 — 생성 중에만 fsync 를 끈다 (연결 단위 설정)
    conn.pragma_update(None, "synchronous", "OFF").unwrap();
    conn.execute_batch(
        "CREATE TABLE poc_asset_text (
            asset_id       INTEGER PRIMARY KEY REFERENCES media_assets (asset_id),
            ai_description TEXT,
            ai_keywords    TEXT,
            user_memo      TEXT
         ) STRICT;",
    )
    .unwrap();

    let tags = synth::tag_list();
    insert_reference_rows(&mut conn, &tags, seed, size, ai_fraction);

    let mut rng = Rng::new(seed);
    let vol_weights = synth::volume_weights();
    let mut used: HashSet<String> = HashSet::new();
    let mut id = 1u64;
    while id <= size {
        let end = (id + BATCH - 1).min(size);
        let tx = conn.transaction().unwrap();
        {
            let mut s_asset = tx.prepare_cached(
                "INSERT INTO media_assets (asset_id, media_type, content_hash, hash_algo, hash_state,
                    file_size, modified_at, revision, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)").unwrap();
            let mut s_loc = tx.prepare_cached(
                "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, presence, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)").unwrap();
            let mut s_meta = tx.prepare_cached(
                "INSERT INTO media_metadata (asset_id, width, height, duration_ms, codec, captured_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)").unwrap();
            let mut s_tag = tx
                .prepare_cached(
                    "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, ?2, ?3)",
                )
                .unwrap();
            let mut s_text = tx
                .prepare_cached(
                    "INSERT INTO poc_asset_text (asset_id, ai_description, ai_keywords, user_memo)
                 VALUES (?1, ?2, ?3, ?4)",
                )
                .unwrap();
            for i in id..=end {
                if (i - 1) % FILES_PER_FOLDER == 0 {
                    used.clear();
                }
                let n = synth::gen_asset(&mut rng);
                let vol = &VOLUMES[rng.weighted(&vol_weights)];
                let name = synth::dedupe_name(&mut used, n.root, n.file_name.clone());
                let rel_path = format!("{}/b{:06}/{}", n.root, (i - 1) / FILES_PER_FOLDER, name);
                let key = mam_db::path_key(&rel_path).expect("path_key");
                let hash: Option<Vec<u8>> = n
                    .hashed
                    .then(|| (0..4).flat_map(|_| rng.next_u64().to_le_bytes()).collect());
                s_asset
                    .execute(params![
                        i as i64,
                        n.media_type,
                        hash,
                        n.hashed.then_some("blake3"),
                        if n.hashed { "done" } else { "pending" },
                        n.file_size,
                        n.modified_at,
                        n.modified_at + 1000
                    ])
                    .unwrap();
                let presence = if n.missing && vol.online {
                    "missing"
                } else {
                    "present"
                };
                s_loc
                    .execute(params![i as i64, vol.id, rel_path, key, presence, NOW_MS])
                    .unwrap();
                s_meta
                    .execute(params![
                        i as i64,
                        n.width,
                        n.height,
                        n.duration_ms,
                        n.codec,
                        n.captured_at
                    ])
                    .unwrap();
                for t in synth::gen_tags(&mut rng, tags.len()) {
                    s_tag
                        .execute(params![i as i64, t as i64 + 1, n.modified_at])
                        .unwrap();
                }
                if let Some(t) = synth::gen_text(&mut rng, ai_fraction) {
                    s_text
                        .execute(params![i as i64, t.description, t.keywords, t.memo])
                        .unwrap();
                }
            }
        }
        tx.commit().unwrap();
        eprintln!("  inserted {end}/{size}");
        id = end + 1;
    }
    let insert_secs = started.elapsed().as_secs_f64();

    // 무결성 검사 — 깨끗하지 않으면 큰 소리로 실패
    let t = Instant::now();
    let fk_violations = fk_check(&conn);
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    if fk_violations != 0 || integrity != "ok" {
        eprintln!(
            "FATAL: foreign_key_check violations={fk_violations}, integrity_check={integrity}"
        );
        std::process::exit(1);
    }
    let check_secs = t.elapsed().as_secs_f64();
    let t = Instant::now();
    conn.execute_batch("ANALYZE").unwrap();
    let analyze_secs = t.elapsed().as_secs_f64();
    conn.pragma_update(None, "synchronous", "FULL").unwrap();
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
        .unwrap();

    report(
        &conn,
        &a,
        size,
        seed,
        ai_fraction,
        insert_secs,
        check_secs,
        analyze_secs,
        &layout,
    );
}

fn fk_check(conn: &Connection) -> usize {
    let mut stmt = conn.prepare("PRAGMA foreign_key_check").unwrap();
    stmt.query_map([], |_| Ok(())).unwrap().count()
}

fn insert_reference_rows(conn: &mut Connection, tags: &[&str], seed: u64, size: u64, ai: f64) {
    let tx = conn.transaction().unwrap();
    for v in &VOLUMES {
        tx.execute(
            "INSERT INTO volumes (volume_id, kind, volume_key, label, last_mount_point, availability, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                v.id,
                v.kind,
                format!("VOL-{:08}", v.id * 7919),
                v.label,
                v.mount,
                if v.online { "online" } else { "offline" },
                NOW_MS
            ],
        )
        .unwrap();
    }
    for (i, t) in tags.iter().enumerate() {
        tx.execute(
            "INSERT INTO tags (tag_id, name, name_key) VALUES (?1, ?2, ?3)",
            params![i as i64 + 1, t, normalize(t).to_lowercase()],
        )
        .unwrap();
    }
    for (k, v) in [
        ("poc.seed", seed.to_string()),
        ("poc.size", size.to_string()),
        ("poc.ai_fraction", ai.to_string()),
    ] {
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)",
            params![k, v],
        )
        .unwrap();
    }
    tx.commit().unwrap();
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn report(
    conn: &Connection,
    _a: &args::Args,
    size: u64,
    seed: u64,
    ai_fraction: f64,
    insert_secs: f64,
    check_secs: f64,
    analyze_secs: f64,
    layout: &common::Layout,
) {
    let assets = count(conn, "SELECT count(*) FROM media_assets");
    let by_type = |t: &str| {
        count(
            conn,
            &format!("SELECT count(*) FROM media_assets WHERE media_type='{t}'"),
        )
    };
    let (img, vid, aud) = (by_type("image"), by_type("video"), by_type("audio"));
    let tag_rows = count(conn, "SELECT count(*) FROM asset_tags");
    let zero_tag = count(
        conn,
        "SELECT count(*) FROM media_assets a WHERE NOT EXISTS (SELECT 1 FROM asset_tags t WHERE t.asset_id=a.asset_id)",
    );
    let with_ai = count(
        conn,
        "SELECT count(*) FROM poc_asset_text WHERE ai_description IS NOT NULL",
    );
    let with_memo = count(
        conn,
        "SELECT count(*) FROM poc_asset_text WHERE user_memo IS NOT NULL",
    );
    let on_offline = count(
        conn,
        "SELECT count(*) FROM file_locations l JOIN volumes v USING (volume_id) WHERE v.availability='offline'",
    );
    let missing = count(
        conn,
        "SELECT count(*) FROM file_locations WHERE presence='missing'",
    );
    let no_capture = count(
        conn,
        "SELECT count(*) FROM media_metadata WHERE captured_at IS NULL",
    );
    let in_2024 = count(
        conn,
        "SELECT count(*) FROM media_metadata WHERE captured_at >= 1704067200000 AND captured_at < 1735689600000",
    );
    let hashed = count(
        conn,
        "SELECT count(*) FROM media_assets WHERE hash_state='done'",
    );
    let db_bytes = std::fs::metadata(layout.db()).map(|m| m.len()).unwrap_or(0);
    let pct = |n: i64| 100.0 * n as f64 / assets.max(1) as f64;

    let cond = sysinfo::conditions();
    println!("\n== gen_dataset ==");
    common::print_conditions(&cond);
    println!("Dataset: size={size} seed={seed} ai_fraction(target)={ai_fraction}");
    println!(
        "DB: {} ({:.1} MB)",
        layout.db().display(),
        sysinfo::mb(db_bytes)
    );
    println!("| step | seconds |\n|---|---:|");
    println!("| migrate + insert | {insert_secs:.2} |");
    println!("| foreign_key_check + integrity_check | {check_secs:.2} |");
    println!("| ANALYZE | {analyze_secs:.2} |");
    println!(
        "| assets/sec (insert) | {:.0} |",
        assets as f64 / insert_secs
    );
    println!("\n| distribution | count | % |\n|---|---:|---:|");
    for (k, v) in [
        ("image", img),
        ("video", vid),
        ("audio", aud),
        ("assets with 0 tags", zero_tag),
        ("assets with AI description+keywords", with_ai),
        ("assets with user memo", with_memo),
        ("locations on offline volumes", on_offline),
        ("locations presence=missing", missing),
        ("captured_at NULL", no_capture),
        ("captured_at in 2024", in_2024),
        ("content hash done", hashed),
    ] {
        println!("| {k} | {v} | {:.1} |", pct(v));
    }
    println!(
        "\nasset_tags rows: {tag_rows} (avg {:.2}/asset), tags: {}, volumes: {}",
        tag_rows as f64 / assets.max(1) as f64,
        vocab::text_lemmas().len(),
        VOLUMES.len()
    );
    println!(
        "JSON: {{\"tool\":\"gen_dataset\",\"size\":{size},\"seed\":{seed},\"ai_fraction\":{ai_fraction},\"insert_secs\":{insert_secs:.2},\"check_secs\":{check_secs:.2},\"analyze_secs\":{analyze_secs:.2},\"db_mb\":{:.1},\"with_ai\":{with_ai},\"with_memo\":{with_memo},\"tag_rows\":{tag_rows},\"offline_locations\":{on_offline},\"conditions\":{},\"db\":{}}}",
        sysinfo::mb(db_bytes),
        common::conditions_json(&cond),
        json_str(&layout.db().display().to_string())
    );
}
