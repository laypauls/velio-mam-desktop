//! 게이트웨이 호스트 함수 지연 측정 (수동 — CI 단언 없음, 일반 `cargo test` 에서 실행되지 않는다).
//!
//! 실행: `GW_BENCH_DIR=<빈 폴더> cargo run --release -p mam-plugin-gateway --example gateway_bench`
//! (Windows 에서 친화도를 고정하려면 `cmd /c start /affinity FCF /B /WAIT <exe>`). 데이터셋은 `GW_BENCH_DIR` 에 만들고 끝나면 지운다.
//! 데이터: 자산 10만(위치·메타데이터 포함) + `plugin_kv` 100만 행(행 비용 합 ≈ 256 MiB 이상, 전역 40% / 자산 60%) + 실제 WebP 썸네일 1쌍.
//! 측정: 직접 호출(`Gateway::call`)과 실제 워커 경유(`PluginHost::invoke`) × 유휴 / Low 쓰기 스캔 부하 중. 제안 목표(미측정): 읽기 P95 < 5 ms, 쓰기 P95 < 20 ms(유휴).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mam_db::{DbPool, Priority, migrate, open};
use mam_plugin_gateway::{Gateway, GatewayConfig};
use mam_plugin_host::{CallCx, HostApi, HostConfig, InvokeKind, InvokeScope, PluginHost, SystemClock};
use mam_plugin_manifest::{Permission, PluginId, parse_manifest};
use rusqlite::params;
use serde_json::{Value, json};

const ASSETS: i64 = 100_000;
const KV_ROWS: i64 = 1_000_000;
const SCOPE_MAX: i64 = 10_000;
const PLUGIN: &str = "com.bench.kv";

struct Clock;
impl mam_db::Clock for Clock {
    fn now_ms(&self) -> i64 {
        1_700_000_000_000
    }
}

fn pct(sorted: &[Duration], p: f64) -> f64 {
    let i = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[i].as_secs_f64() * 1000.0
}

fn report(label: &str, mut v: Vec<Duration>) {
    v.sort();
    eprintln!("BENCH {label}: n={} P50={:.3}ms P95={:.3}ms P99={:.3}ms max={:.3}ms", v.len(), pct(&v, 0.50), pct(&v, 0.95), pct(&v, 0.99), v.last().map_or(0.0, |d| d.as_secs_f64() * 1000.0));
}

fn plugin_id() -> PluginId {
    parse_manifest(&format!(r#"{{"apiVersion":1,"id":"{PLUGIN}","version":"0.1.0","engines":{{"mam":">=0.1.0"}},"main":"main.js","displayName":"x"}}"#)).expect("manifest").id
}

fn seed(dir: &Path) -> PathBuf {
    let db_path = dir.join("mam.sqlite");
    let mut conn = open(&db_path).expect("open");
    migrate(&mut conn, &dir.join("backups"), &Clock).expect("migrate");
    let t = Instant::now();
    let tx = conn.transaction().expect("tx");
    tx.execute("INSERT INTO volumes (kind, volume_key, last_seen_at) VALUES ('fixed', 'bench', 1)", []).expect("volume");
    {
        let mut a = tx.prepare("INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at) VALUES ('image', 1, 1, 1, 1)").expect("prep");
        let mut l = tx.prepare("INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at) VALUES (?1, 1, ?2, ?2, 1)").expect("prep");
        let mut m = tx.prepare("INSERT INTO media_metadata (asset_id, width, height, duration_ms, captured_at) VALUES (?1, 4000, 3000, NULL, 1700000000000)").expect("prep");
        for i in 1..=ASSETS {
            a.execute([]).expect("asset");
            let rel = format!("bench/dir{}/file{i}.jpg", i % 100);
            l.execute(params![i, rel]).expect("loc");
            m.execute([i]).expect("meta");
        }
    }
    tx.commit().expect("commit assets");
    let tx = conn.transaction().expect("tx");
    {
        let plugin_ref = mam_db::plugin_registry::ensure(&tx, PLUGIN).expect("registry");
        let mut kv = tx.prepare("INSERT INTO plugin_kv (plugin_ref, asset_id, key, value, updated_at) VALUES (?1, ?2, ?3, ?4, 1)").expect("prep");
        let value = format!("\"{}\"", "x".repeat(200));
        for i in 0..KV_ROWS {
            if i % 10 < 4 {
                kv.execute(params![plugin_ref, Option::<i64>::None, format!("g{i:07}"), value]).expect("kv");
            } else {
                kv.execute(params![plugin_ref, 1 + (i % ASSETS), format!("k{i:07}"), value]).expect("kv");
            }
        }
    }
    tx.commit().expect("commit kv");
    conn.execute_batch("CREATE TABLE bench_load (a INTEGER NOT NULL, b TEXT NOT NULL)").expect("load table");
    eprintln!("SEED {ASSETS} assets + {KV_ROWS} plugin_kv rows in {:.1}s", t.elapsed().as_secs_f64());
    db_path
}

fn thumbs(dir: &Path, cache: &Path) {
    let src = dir.join("src.png");
    image::RgbImage::from_fn(1500, 900, |x, _| if x < 750 { image::Rgb([255, 0, 0]) } else { image::Rgb([0, 0, 255]) }).save(&src).expect("png");
    let tools = mam_media::tool::MediaTools::with_paths(None, None, Arc::new(mam_media::tool::ProcessRunner));
    mam_media::thumbnail::create_thumbnails(&tools, &src, 1, cache, &mam_media::thumbnail::ThumbnailOptions::default()).expect("thumbnails");
}

fn worker_exe() -> PathBuf {
    let exe = std::env::current_exe().expect("exe");
    let profile = exe.parent().and_then(Path::parent).expect("profile dir"); // examples/ 의 부모
    let name = if cfg!(windows) { "mam-plugin-worker.exe" } else { "mam-plugin-worker" };
    let p = profile.join(name);
    assert!(p.exists(), "build the worker first: cargo build --release -p mam-plugin-worker ({})", p.display());
    p
}

struct Env {
    gw: Arc<Gateway>,
    pid: PluginId,
}

impl Env {
    fn call(&self, assets: &[i64], method: &str, params: Value) -> Value {
        let scope = InvokeScope::new(assets.to_vec()).expect("scope").with_deadline(Instant::now() + Duration::from_secs(30));
        self.gw.call(&CallCx { plugin: &self.pid, scope: &scope }, method, params).unwrap_or_else(|e| panic!("{method}: {e:?}"))
    }
}

fn time<F: FnMut(usize)>(n: usize, warm: usize, mut f: F) -> Vec<Duration> {
    for i in 0..warm {
        f(i);
    }
    (0..n)
        .map(|i| {
            let t = Instant::now();
            f(warm + i);
            t.elapsed()
        })
        .collect()
}

/// 직접 호출 측정. `tag.add` 는 호출마다 다른 자산(`base` 이후)이라 매번 실제 부착이다
fn direct(env: &Env, label: &str, base: i64) {
    let scope: Vec<i64> = (1..=SCOPE_MAX).collect();
    report(&format!("{label} asset.get"), time(2000, 200, |i| drop(env.call(&scope, "asset.get", json!({"assetId": 1 + (i as i64 % SCOPE_MAX)})))));
    report(&format!("{label} asset.getDerived small"), time(1000, 100, |_| drop(env.call(&[1], "asset.getDerived", json!({"assetId": 1, "kind": "thumbnail"})))));
    report(&format!("{label} kv.get global hit"), time(2000, 200, |i| drop(env.call(&[], "kv.get", json!({"key": format!("g{:07}", (i % 100_000) * 10)})))));
    report(&format!("{label} kv.get asset hit"), time(2000, 200, |i| drop(env.call(&scope, "kv.get", json!({"key": format!("k{:07}", (i % 1000) * 10 + 4), "assetId": 1 + ((i % 1000) * 10 + 4) as i64})))));
    report(&format!("{label} kv.list limit 50"), time(1000, 100, |i| drop(env.call(&[], "kv.list", json!({"prefix": "g", "cursor": format!("g{:07}", (i % 50_000) * 10), "limit": 50})))));
    report(&format!("{label} kv.set overwrite"), time(1000, 100, |i| drop(env.call(&[], "kv.set", json!({"key": format!("g{:07}", (i % 100_000) * 10), "value": "y".repeat(200)})))));
    report(&format!("{label} kv.delete+set (delete timed)"), {
        let mut v = Vec::new();
        for i in 0..500usize {
            let key = format!("d{label_len}{i:05}", label_len = label.len());
            env.call(&[], "kv.set", json!({"key": key, "value": 1}));
            let t = Instant::now();
            env.call(&[], "kv.delete", json!({"key": key}));
            v.push(t.elapsed());
        }
        v
    });
    report(&format!("{label} tag.add new attachment"), time(1000, 0, |i| drop(env.call(&[base + i as i64], "tag.add", json!({"assetId": base + i as i64, "tags": ["bench", "t2"]})))));
}

const JS: &str = r#"
mam.commands.register('echo', () => 1);
mam.commands.register('asset_get', (p) => mam.host.call('asset.get', { assetId: p.id }).assetId);
mam.commands.register('kv_get', (p) => mam.host.call('kv.get', { key: p.key }).found);
mam.commands.register('kv_set', (p) => { mam.host.call('kv.set', { key: p.key, value: p.v }); return 1; });
mam.commands.register('tag_add', (p) => mam.host.call('tag.add', { assetId: p.id, tags: ['bench', 't2'] }).added);
"#;

fn via_worker(env: &Env, label: &str, base: i64) {
    let api: Arc<dyn HostApi> = env.gw.clone();
    let host = PluginHost::new(env.pid.clone(), HostConfig::new(worker_exe(), JS.to_owned()), api, Arc::new(SystemClock));
    let scope = |assets: Vec<i64>| InvokeScope::new(assets).expect("scope");
    let ids: Vec<i64> = (1..=SCOPE_MAX).collect();
    let run = |cmd: &str, assets: Vec<i64>, p: Value| host.invoke(InvokeKind::Command, cmd, p, scope(assets)).unwrap_or_else(|e| panic!("{cmd}: {e:?}"));
    report(&format!("{label} worker echo (no host call)"), time(2000, 200, |_| drop(run("echo", vec![], json!(null)))));
    report(&format!("{label} worker asset.get"), time(2000, 200, |i| drop(run("asset_get", ids.clone(), json!({"id": 1 + (i as i64 % SCOPE_MAX)})))));
    report(&format!("{label} worker kv.get"), time(2000, 200, |i| drop(run("kv_get", vec![], json!({"key": format!("g{:07}", (i % 100_000) * 10)})))));
    report(&format!("{label} worker kv.set overwrite"), time(1000, 100, |i| drop(run("kv_set", vec![], json!({"key": format!("g{:07}", (i % 100_000) * 10), "v": "z".repeat(200)})))));
    report(&format!("{label} worker tag.add new attachment"), time(1000, 0, |i| drop(run("tag_add", vec![base + i as i64], json!({"id": base + i as i64})))));
    host.shutdown();
}

/// Low 우선순위 쓰기를 연속으로 넣는 스캔 모사 부하 — 스레드 2개, 트랜잭션마다 500 행
fn load(pool: &Arc<DbPool>) -> (Arc<AtomicBool>, Vec<std::thread::JoinHandle<()>>) {
    let stop = Arc::new(AtomicBool::new(false));
    let hs = (0..2)
        .map(|_| {
            let (pool, stop) = (Arc::clone(pool), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    let _ = pool.write(Priority::Low, |c| {
                        let tx = c.transaction()?;
                        {
                            let mut s = tx.prepare_cached("INSERT INTO bench_load (a, b) VALUES (?1, ?2)")?;
                            for i in 0..500 {
                                s.execute(params![i, "scan-load-row-padding-padding-padding"])?;
                            }
                        }
                        tx.commit()?;
                        Ok(())
                    });
                }
            })
        })
        .collect();
    (stop, hs)
}

fn main() {
    let dir = PathBuf::from(std::env::var("GW_BENCH_DIR").expect("set GW_BENCH_DIR to an empty scratch folder"));
    std::fs::create_dir_all(&dir).expect("dir");
    let cache = dir.join("cache");
    let db_path = seed(&dir);
    thumbs(&dir, &cache);
    eprintln!("SIZE sqlite file = {:.1} MiB", std::fs::metadata(&db_path).map_or(0, |m| m.len()) as f64 / 1048576.0);
    let pool = Arc::new(DbPool::open(&db_path, 4).expect("pool"));
    let mk = || {
        let mut cfg = GatewayConfig::new(cache.clone());
        cfg.kv_quota_bytes = 1 << 30; // 데이터셋(≈ 261 MiB)이 상한에 걸리지 않게
        let gw = Arc::new(Gateway::new(Arc::clone(&pool), Arc::new(Clock), cfg));
        let pid = plugin_id();
        gw.grants().register(&pid, &Permission::ALL.iter().copied().collect());
        pool.write(Priority::High, |c| {
            mam_db::plugin_settings::put(c, "plugin_enabled", PLUGIN, "true").expect("put");
            mam_db::plugin_settings::put(c, "plugin_granted", PLUGIN, &json!(Permission::ALL.map(|p| p.as_str())).to_string()).expect("put");
            Ok(())
        })
        .expect("settings");
        gw.grants().refresh(&pool, &pid, Instant::now() + Duration::from_secs(5)).expect("refresh");
        Env { gw, pid }
    };
    // 첫 kv.set = 계량기 초기화(SUM 스캔) — 새 게이트웨이 3회
    let mut init = Vec::new();
    for _ in 0..3 {
        let env = mk();
        let t = Instant::now();
        env.call(&[], "kv.set", json!({"key": "g0000000", "value": "x"}));
        init.push(t.elapsed());
    }
    eprintln!("BENCH meter init (first kv.set, SUM over {KV_ROWS} rows): {:?}", init);
    let env = mk();
    env.call(&[], "kv.set", json!({"key": "g0000000", "value": "x"}));
    direct(&env, "idle", 20_000);
    via_worker(&env, "idle", 30_000);
    let (stop, hs) = load(&pool);
    direct(&env, "load", 40_000);
    via_worker(&env, "load", 50_000);
    stop.store(true, Ordering::Release);
    for h in hs {
        h.join().expect("load thread");
    }
    pool.shutdown();
    drop(pool);
    let _ = std::fs::remove_dir_all(&dir);
    eprintln!("CLEANED {}", dir.display());
}
