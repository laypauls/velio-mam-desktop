//! M2 — 고아 데이터 정리 비용 (1M `plugin_kv` + 1M `plugin_tasks`, 설치되지 않은 플러그인).
//! (a) 복원 경로 `purge_uninstalled` — 부팅을 막는다. 별도 연결(`mam_db::open`)에서 한 번에 실행해 총 시간만 잰다.
//! (b) 제거 `!keepData` 루프 — `PluginRuntime::uninstall` 과 같은 Low 쓰기 1건/배치(`c.transaction()` + `delete_plugin_batch(…, 5000)`),
//!     동시에 실제 High 쓰기(`UserData::update_asset_tags`) 지연.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use mam_db::plugin_data::{BATCH, data_plugin_ids, delete_plugin_batch, purge_uninstalled};
use mam_db::{DbPool, Priority, open};

use crate::env::Env;
use crate::synth::{self, PLUGIN_GONE};
use crate::util::{Lat, Load, db_files_bytes, mib, say};

fn copy_db(from: &Path, to_dir: &Path) -> std::path::PathBuf {
    std::fs::create_dir_all(to_dir).expect("dir");
    let to = to_dir.join("mam.sqlite");
    std::fs::copy(from, &to).expect("copy db");
    to
}

fn counts(db: &Path) -> (i64, i64) {
    let c = open(db).expect("open");
    let q = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).expect("count");
    (
        q("SELECT COUNT(*) FROM plugin_kv"),
        q("SELECT COUNT(*) FROM plugin_tasks"),
    )
}

pub fn run(scratch: &Path, rows: u64) {
    let base = scratch.join("m2");
    let _ = std::fs::remove_dir_all(&base);
    let t = Instant::now();
    let src = synth::build(&base.join("base"), rows, false, 0);
    synth::seed_orphans(&src, rows);
    say(format!(
        "M2 dataset: {rows} assets + {rows} plugin_kv + {rows} plugin_tasks (plugin {PLUGIN_GONE}, not installed), built in {:.1}s; db {:.1} MiB",
        t.elapsed().as_secs_f64(),
        mib(db_files_bytes(&src))
    ));

    // (a) purge_uninstalled — 복원 경로(부팅 차단). 복사 직후라 OS 캐시는 따뜻하다(콜드 캐시 아님)
    let db_a = copy_db(&src, &base.join("a"));
    let c = open(&db_a).expect("open");
    let t = Instant::now();
    let ids = data_plugin_ids(&c).expect("ids");
    say(format!(
        "  (a) data_plugin_ids skip-scan: {:.1} ms -> {ids:?}",
        t.elapsed().as_secs_f64() * 1000.0
    ));
    let t = Instant::now();
    let removed = purge_uninstalled(&c, &BTreeSet::new()).expect("purge");
    let total = t.elapsed();
    drop(c);
    say(format!(
        "  (a) purge_uninstalled(batch {BATCH}/table): removed {removed} rows in {:.2}s ({:.0} rows/s) => boot blocked {:.2}s",
        total.as_secs_f64(),
        removed as f64 / total.as_secs_f64(),
        total.as_secs_f64()
    ));
    say(format!("  (a) rows left (kv, tasks) = {:?}", counts(&db_a)));
    let _ = std::fs::remove_dir_all(base.join("a"));

    // (b) 제거 !keepData 루프 + 동시 High 쓰기
    let db_b = copy_db(&src, &base.join("b").join("MAMData"));
    let root = db_b.parent().expect("root").to_path_buf();
    let env = Env::with_pool(
        std::sync::Arc::new(DbPool::open(&db_b, 4).expect("pool")),
        &root,
        None,
        None,
    );
    let w = Load::spawn(Duration::from_millis(5), env.editor(rows, 11));
    std::thread::sleep(Duration::from_secs(5));
    say(w
        .finish()
        .0
        .line("  (b) High write idle baseline 5s (no delete loop)"));
    let w = Load::spawn(Duration::from_millis(5), env.editor(rows, 12));
    let (mut batches, mut removed) = (Lat::default(), 0usize);
    let t = Instant::now();
    loop {
        let key = PLUGIN_GONE.to_owned();
        let s = Instant::now();
        let n = env
            .pool
            .write(Priority::Low, move |c| {
                let tx = c.transaction()?;
                let n = delete_plugin_batch(&tx, &key, BATCH)?;
                tx.commit()?;
                Ok(n)
            })
            .expect("batch");
        batches.push(s.elapsed());
        if n == 0 {
            break;
        }
        removed += n;
    }
    let total = t.elapsed();
    let (high, failed) = w.finish();
    say(format!(
        "  (b) uninstall loop: removed {removed} rows in {:.2}s, {} batches (last is the empty check) ({:.0} rows/s)",
        total.as_secs_f64(),
        batches.0.len(),
        removed as f64 / total.as_secs_f64()
    ));
    say(batches.line("  (b) per-batch Low tx (5,000 rows/table, kv-asset + kv-global + tasks)"));
    say(high.line(&format!(
        "  (b) High write DURING delete loop, real update_asset_tags (fail {failed})"
    )));
    say(format!(
        "  (b) writer occupancy by delete batches = {:.1}% of wall",
        100.0 * batches.sum_ms() / (total.as_secs_f64() * 1000.0)
    ));
    env.shutdown();
    drop(env);
    say(format!(
        "  (b) plugin rows left (kv, tasks) = {:?}",
        counts(&db_b)
    ));
    let _ = std::fs::remove_dir_all(&base);
    say("M2 cleanup: dataset directory deleted");
}
