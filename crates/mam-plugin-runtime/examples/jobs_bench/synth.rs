//! 합성 DB — 실제 마이그레이션(V1..V26)을 적용한 뒤 SQL(재귀 CTE) 청크로 대량 행을 넣는다. 합성 메타데이터뿐이다
//! (실제 파일시스템·코덱·Tantivy·UI 성능이 아니다).

use std::path::{Path, PathBuf};
use std::time::Instant;

use mam_db::{SystemClock, migrate, open};
use rusqlite::{Connection, params};

use crate::util::say;

const CHUNK: u64 = 100_000;

pub const PLUGIN_GONE: &str = "com.example.gone";

/// 빈 DB(마이그레이션 적용)를 만든다. `data_root/mam.sqlite`
pub fn empty_db(data_root: &Path) -> PathBuf {
    std::fs::create_dir_all(data_root).expect("create data root");
    let db = data_root.join("mam.sqlite");
    let mut conn = open(&db).expect("open db");
    migrate(&mut conn, &data_root.join("backups"), &SystemClock).expect("migrate");
    conn.close().expect("close");
    db
}

/// `1..=total` 을 `CHUNK` 단위 트랜잭션으로 `body(conn, lo, hi)` 에 넘긴다 (빌더 연결은 synchronous=OFF — 적재 속도용, 측정 대상이 아니다)
fn chunked(conn: &mut Connection, total: u64, label: &str, body: impl Fn(&Connection, i64, i64) -> rusqlite::Result<()>) {
    let t = Instant::now();
    let mut lo = 1;
    while lo <= total {
        let hi = (lo + CHUNK - 1).min(total);
        let tx = conn.transaction().expect("tx");
        body(&tx, lo as i64, hi as i64).unwrap_or_else(|e| panic!("{label} {lo}..{hi}: {e}"));
        tx.commit().expect("commit");
        lo = hi + 1;
    }
    say(format!("  synth {label}: {total} rows in {:.1}s", t.elapsed().as_secs_f64()));
}

const SERIES: &str = "WITH RECURSIVE c(n) AS (SELECT ?1 UNION ALL SELECT n + 1 FROM c WHERE n < ?2)";

/// 활성 이미지 자산 `assets` 개 (+ 활성 위치 + `outbox` 개의 `upsert` 이벤트). 볼륨 1개.
/// 모든 자산이 `image` 이다 — 이미지 provider 1개의 run-all 이 자산 수만큼 행을 만드는 최악 경우.
pub fn build(data_root: &Path, assets: u64, locations: bool, outbox: u64) -> PathBuf {
    let db = empty_db(data_root);
    let mut conn = open(&db).expect("open db");
    conn.pragma_update(None, "synchronous", "OFF").expect("pragma");
    conn.execute(
        "INSERT INTO volumes (kind, volume_key, label, last_mount_point, last_seen_at) VALUES ('fixed', 'vol-bench', 'Bench', 'Z:\\', 1)",
        [],
    )
    .expect("volume");
    chunked(&mut conn, assets, "media_assets", |c, lo, hi| {
        c.execute(
            &format!(
                "{SERIES} INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
                 SELECT n, 'image', 20000 + n % 5000, 1700000000000 + n * 1000, 1700000000000, 1700000000000 FROM c"
            ),
            params![lo, hi],
        )
        .map(drop)
    });
    if locations {
        chunked(&mut conn, assets, "file_locations", |c, lo, hi| {
            c.execute(
                &format!(
                    "{SERIES} INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
                     SELECT n, 1, 'album_' || (n / 50) || '/f_' || n || '.jpg', 'album_' || (n / 50) || '/f_' || n || '.jpg', 1 FROM c"
                ),
                params![lo, hi],
            )
            .map(drop)
        });
    }
    if outbox > 0 {
        chunked(&mut conn, outbox, "search_outbox", |c, lo, hi| {
            c.execute(
                &format!(
                    "{SERIES} INSERT INTO search_outbox (asset_id, revision, event_type, priority, status, created_at)
                     SELECT ((n - 1) % ?3) + 1, 1, 'upsert', 1, 'done', 1700000000000 FROM c"
                ),
                params![lo, hi, assets.max(1) as i64],
            )
            .map(drop)
        });
    }
    finish(conn);
    db
}

/// M2: 설치되지 않은 플러그인 `PLUGIN_GONE` 의 KV 행 `rows` 개(자산당 1키)와 `plugin_tasks` 완료 행 `rows` 개
pub fn seed_orphans(db: &Path, rows: u64) {
    let mut conn = open(db).expect("open db");
    conn.pragma_update(None, "synchronous", "OFF").expect("pragma");
    let plugin_ref = mam_db::plugin_registry::ensure(&conn, PLUGIN_GONE).expect("registry");
    let provider_ref = mam_db::plugin_registry::ensure_provider(&conn, plugin_ref, "meta.extract").expect("registry provider");
    chunked(&mut conn, rows, "plugin_kv", |c, lo, hi| {
        c.execute(
            &format!(
                "{SERIES} INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at)
                 SELECT {plugin_ref}, n, 'fileName', '\"f_' || n || '.jpg\"', 1 FROM c"
            ),
            params![lo, hi],
        )
        .map(drop)
    });
    chunked(&mut conn, rows, "plugin_tasks", |c, lo, hi| {
        c.execute(
            &format!(
                "{SERIES} INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, done_revision, status, updated_at)
                 SELECT {plugin_ref}, {provider_ref}, n, 1, 1, 'done', 1 FROM c"
            ),
            params![lo, hi],
        )
        .map(drop)
    });
    finish(conn);
}

fn finish(conn: Connection) {
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).expect("checkpoint");
    conn.close().map_err(|(_, e)| e).expect("close");
}
