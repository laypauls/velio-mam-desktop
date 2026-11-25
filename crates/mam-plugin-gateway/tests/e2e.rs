//! 실제 `mam-plugin-worker`(QuickJS) 의 JS 플러그인이 실제 게이트웨이·SQLite 를 부르는 종단 시험 (T-pgw-38 ~ 42).
//!
//! 워커 위치: 이 시험 실행 파일은 `<target>/<profile>/deps/` 에 있고 워커는 `<target>/<profile>/` 에 있다. 시험 안에서 한 번
//! `cargo build -p mam-plugin-worker`(현재 프로필)를 돌려 최신을 보장한다. `MAM_PLUGIN_WORKER_EXE` 로 직접 줄 수도 있다.
//! 분류: Happy / Validation(JS 예외 코드) / Concurrency(철회) / Boundary(폭주) / External(쓰기 중 워커 종료)

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use common::*;
use mam_db::Priority;
use mam_plugin_gateway::Gateway;
use mam_plugin_host::{CallCx, HostApi, HostConfig, HostError, HostState, InvokeError, InvokeKind, InvokeScope, LogLevel, PluginHost, SystemClock};
use mam_plugin_manifest::{Permission as P, PluginId};
use serde_json::{Value, json};

fn worker_exe() -> PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        if let Some(p) = std::env::var_os("MAM_PLUGIN_WORKER_EXE") {
            return PathBuf::from(p);
        }
        let test_exe = std::env::current_exe().expect("test exe path");
        let profile_dir = test_exe.parent().and_then(Path::parent).expect("profile dir").to_path_buf();
        let mut cmd = std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
        cmd.args(["build", "-p", "mam-plugin-worker", "--quiet"]);
        if profile_dir.file_name().is_some_and(|n| n == "release") {
            cmd.arg("--release");
        }
        assert!(cmd.status().expect("run cargo build for the worker").success(), "building mam-plugin-worker failed");
        let exe = profile_dir.join("mam-plugin-worker.exe");
        assert!(exe.exists(), "worker not found at {}", exe.display());
        exe
    })
    .clone()
}

/// 호출 수를 세는 `HostApi` — 시험이 "호출 중" 시점을 잡는 데 쓴다
struct Counting {
    inner: Arc<Gateway>,
    calls: AtomicUsize,
}

impl HostApi for Counting {
    fn call(&self, cx: &CallCx<'_>, method: &str, params: Value) -> Result<Value, HostError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        self.inner.call(cx, method, params)
    }
    fn log(&self, cx: &CallCx<'_>, level: LogLevel, message: &str) {
        self.inner.log(cx, level, message);
    }
}

fn host(fx: &Fx, pid: &PluginId, source: &str, tweak: impl FnOnce(&mut HostConfig)) -> (PluginHost, Arc<Counting>) {
    let api = Arc::new(Counting { inner: Arc::clone(&fx.gw), calls: AtomicUsize::new(0) });
    let mut cfg = HostConfig::new(worker_exe(), source.to_owned());
    tweak(&mut cfg);
    (PluginHost::new(pid.clone(), cfg, api.clone(), Arc::new(SystemClock)), api)
}

fn run(h: &PluginHost, cmd: &str, assets: &[i64], params: Value) -> Result<Value, InvokeError> {
    h.invoke(InvokeKind::Command, cmd, params, InvokeScope::new(assets.to_vec()).unwrap())
}

const FULL_PLUGIN: &str = r#"
mam.commands.register('run', (p) => {
  const a = mam.host.call('asset.get', { assetId: p.id });
  mam.host.call('kv.set', { key: 'seen', value: { name: a.fileName, 한글: [1, 2] } });
  mam.host.call('kv.set', { key: 'per-asset', value: p.id, assetId: p.id });
  const got = mam.host.call('kv.get', { key: 'seen' });
  const list = mam.host.call('kv.list', {});
  const tag = mam.host.call('tag.add', { assetId: p.id, tags: ['e2e', '가나'] });
  const d = mam.host.call('asset.getDerived', { assetId: p.id, kind: 'thumbnail' });
  return { a, got, list, tag, mime: d.mime, len: d.byteLength, b64: typeof d.dataBase64 };
});
"#;

#[test]
fn t_pgw_38_js_plugin_uses_asset_kv_tag_and_derived_through_the_real_worker_and_db() {
    // Arrange
    let fx = Fx::new();
    let pid = fx.full_plugin("com.example.e2e");
    let id = fx.add_asset("image", "lib/photo.jpg");
    fx.thumbs(id);
    let before = snapshot(&fx.lib);
    let (h, _) = host(&fx, &pid, FULL_PLUGIN, |_| {});
    // Act
    let out = run(&h, "run", &[id], json!({"id": id})).unwrap();
    // Assert: 반환값
    assert_eq!(out["a"]["fileName"], "photo.jpg");
    assert_eq!(out["got"], json!({"found": true, "value": {"name": "photo.jpg", "한글": [1, 2]}}));
    assert_eq!(out["list"], json!({"keys": ["seen"], "nextCursor": null}));
    assert_eq!(out["tag"], json!({"added": 2}));
    assert_eq!((out["mime"].clone(), out["b64"].clone()), (json!("image/webp"), json!("string")));
    assert!(out["len"].as_u64().unwrap() > 0);
    // Assert: DB 상태 — 플러그인 네임스페이스 키 2개(전역·자산), 태그 2개, revision 1회, outbox 우선순위 1
    assert_eq!(fx.count("SELECT COUNT(*) FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = 'com.example.e2e'"), 2);
    assert_eq!(fx.count(&format!("SELECT COUNT(*) FROM asset_tags WHERE asset_id = {id}")), 2);
    assert_eq!(fx.count(&format!("SELECT revision FROM media_assets WHERE asset_id = {id}")), 2, "초기 1 + 태그 추가 1");
    assert_eq!(fx.count(&format!("SELECT priority FROM search_outbox WHERE asset_id = {id} ORDER BY event_id DESC LIMIT 1")), 1);
    assert_eq!(snapshot(&fx.lib), before, "원본 불변");
    h.shutdown();
}

#[test]
fn t_pgw_39_js_sees_error_codes_for_denied_permissions_forged_ids_and_extra_params() {
    // Arrange: asset.read.metadata 만 허용
    let fx = Fx::new();
    let pid = fx.plugin("com.example.deny", &P::ALL, Some(&granted_json(&[P::AssetReadMetadata])), Some("true"));
    let (id, other) = (fx.add_asset("image", "a/x.jpg"), fx.add_asset("image", "a/y.jpg"));
    let src = r#"
      const code = (m, p) => { try { mam.host.call(m, p); return 'no error'; } catch (e) { return [e.code, e.message]; } };
      mam.commands.register('probe', (p) => ({
        denied: code('kv.get', { key: 'k' }),
        deniedTag: code('tag.add', { assetId: p.id, tags: ['x'] }),
        forged: code('asset.get', { assetId: p.other }),
        pluginId: code('asset.get', { assetId: p.id, pluginId: 'com.example.other' }),
        unknown: code('asset.getDerived2', {}),
        badId: code('asset.get', { assetId: '1' }),
        ok: code('asset.get', { assetId: p.id }),
      }));
    "#;
    let (h, _) = host(&fx, &pid, src, |_| {});
    // Act
    let out = run(&h, "probe", &[id], json!({"id": id, "other": other})).unwrap();
    // Assert: JS 예외의 .code 와 정적 메시지
    assert_eq!(out["denied"], json!(["permission_denied", "permission denied: kv.read"]));
    assert_eq!(out["deniedTag"], json!(["permission_denied", "permission denied: tag.write"]));
    assert_eq!(out["forged"], json!(["not_found", "asset not found"]));
    assert_eq!(out["pluginId"], json!(["bad_request", "invalid params"]));
    assert_eq!(out["unknown"], json!(["unknown_method", "unknown method"]));
    assert_eq!(out["badId"], json!(["bad_request", "invalid asset id"]));
    assert_eq!(out["ok"], json!("no error"));
    assert_eq!(fx.count("SELECT COUNT(*) FROM tags"), 0);
    h.shutdown();
}

#[test]
fn t_pgw_40_revocation_during_a_running_call_takes_effect_without_deadlock() {
    // Arrange: 권한이 있는 동안 계속 kv.get 을 부르는 명령
    let fx = Arc::new(Fx::new());
    let pid = fx.plugin("com.example.revoke", &P::ALL, Some(&granted_json(&[P::KvRead])), Some("true"));
    let src = r#"
      mam.commands.register('loop', () => {
        for (let i = 0; i < 9000; i++) {
          try { mam.host.call('kv.get', { key: 'k' }); } catch (e) { return { i, code: e.code }; }
        }
        return { i: 9000, code: 'never' };
      });
    "#;
    let (h, api) = host(&fx, &pid, src, |_| {});
    // Act: 호출 수가 50 을 넘으면 다른 스레드가 철회한다 (실행 중인 호출 한가운데)
    let revoker = {
        let (fx, api, pid) = (Arc::clone(&fx), Arc::clone(&api), pid.clone());
        std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(30);
            while api.calls.load(Ordering::Acquire) < 50 && Instant::now() < until {
                std::thread::yield_now();
            }
            fx.set_settings("com.example.revoke", Some("true"), Some("[]"));
            fx.gw.grants().refresh(&fx.pool, &pid, soon()).unwrap();
        })
    };
    let out = run(&h, "loop", &[], json!(null)).unwrap();
    revoker.join().unwrap();
    // Assert
    assert_eq!(out["code"], "permission_denied");
    assert!(out["i"].as_u64().unwrap() >= 50, "철회 전에는 허용됐다: {out}");
    h.shutdown();
}

#[test]
fn t_pgw_41_ten_thousand_host_calls_hit_the_message_cap_and_kv_stays_consistent() {
    // Arrange: 호출당 메시지 상한 10,000 — 쓰기 20,000 번을 시도하는 플러그인
    let fx = Fx::new();
    let pid = fx.full_plugin("com.example.flood");
    let src = "mam.commands.register('flood', () => { for (let i = 0; i < 20000; i++) mam.host.call('kv.set', { key: 'k' + (i % 10), value: i }); return 'done'; });\n\
               mam.commands.register('echo', () => 'alive');";
    // 이 테스트는 메시지 상한(Flood)을 검증한다 — 부하 시 기본 CPU 상한(2 s)이 먼저 걸리지 않도록 CPU 상한도 올린다
    let (h, _) = host(&fx, &pid, src, |c| {
        c.call_timeout = Duration::from_secs(120);
        c.limits.cpu_ms = 120_000;
    });
    // Act
    let e = run(&h, "flood", &[], json!(null)).unwrap_err();
    // Assert: 폭주로 워커 종료, 부분 쓰기 없음 — 10 개 키 모두 완전한 JSON 정수이고 키 번호와 값이 맞는다
    assert_eq!(e, InvokeError::Flood);
    assert_eq!(h.state(), HostState::Idle);
    let rows: Vec<(String, String)> = fx.read(|c| c.prepare("SELECT key, value FROM plugin_kv ORDER BY key").unwrap().query_map([], |r| Ok((r.get(0).unwrap(), r.get(1).unwrap()))).unwrap().map(Result::unwrap).collect());
    assert_eq!(rows.len(), 10);
    for (k, v) in rows {
        let n: i64 = v.parse().expect("정수 JSON");
        assert_eq!(format!("k{}", n % 10), k);
        assert!(n >= 9_990, "10,000 번째 메시지까지 처리됐다: {k}={n}");
    }
    // 새 호출이 성공하고 워커는 다시 시작된다
    assert_eq!(run(&h, "echo", &[], json!(null)).unwrap(), json!("alive"));
    assert_eq!(h.starts(), 2);
    h.shutdown();
}

#[test]
fn t_pgw_42_worker_killed_mid_write_leaves_db_consistent_and_the_retry_is_idempotent() {
    // Arrange: 워커를 먼저 띄워 PID 를 얻고(호출 중에는 상태 조회가 감독자 락에 막힌다), 쓰기 스레드를 막는다
    let fx = Arc::new(Fx::new());
    let pid = fx.full_plugin("com.example.kill");
    let id = fx.add_asset("image", "a/x.jpg");
    let src = r#"
      mam.commands.register('warm', () => 1);
      mam.commands.register('work', (p) => {
        mam.host.call('tag.add', { assetId: p.id, tags: ['once'] });
        mam.host.call('kv.set', { key: 'done', value: 1 });
        return 'ok';
      });
    "#;
    let (h, _) = host(&fx, &pid, src, |_| {});
    let h = Arc::new(h);
    run(&h, "warm", &[], json!(null)).unwrap();
    let HostState::Running { pid: worker_pid } = h.state() else { panic!("worker should be running") };
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let blocker = {
        let pool = Arc::clone(&fx.pool);
        std::thread::spawn(move || {
            pool.write(Priority::High, move |_| {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(20)).unwrap();
                Ok(())
            })
            .unwrap();
        })
    };
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let caller = {
        let h = Arc::clone(&h);
        std::thread::spawn(move || run(&h, "work", &[id], json!({"id": id})))
    };
    // tag.add 의 쓰기가 막힌 큐에 올라올 때까지 (시간 상한 있는 양보 대기 — 셸 루프 아님)
    let until = Instant::now() + Duration::from_secs(10);
    while fx.pool.pending_writes() < 1 && Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(fx.pool.pending_writes() >= 1, "쓰기가 큐에 올라와 있어야 한다");
    // Act: 워커를 PID 로만 종료하고 쓰기 스레드를 푼다
    let st = std::process::Command::new("taskkill").args(["/PID", &worker_pid.to_string(), "/F"]).stdout(std::process::Stdio::null()).status().unwrap();
    assert!(st.success());
    release_tx.send(()).unwrap();
    blocker.join().unwrap();
    let first = caller.join().unwrap();
    // Assert: 호출은 실패(워커 종료)하지만 이미 큐에 있던 쓰기는 하나의 완전한 트랜잭션으로 끝났다
    assert!(first.is_err(), "{first:?}");
    assert_eq!(fx.count(&format!("SELECT COUNT(*) FROM asset_tags WHERE asset_id = {id}")), 1);
    assert_eq!(fx.count("SELECT COUNT(*) FROM plugin_kv"), 0, "죽은 워커는 두 번째 호출을 보내지 못했다");
    let (rev, outbox) = (fx.count(&format!("SELECT revision FROM media_assets WHERE asset_id = {id}")), fx.count("SELECT COUNT(*) FROM search_outbox"));
    // Act 2: 재시도 — 새 워커, 같은 요청
    assert_eq!(run(&h, "work", &[id], json!({"id": id})).unwrap(), json!("ok"));
    // Assert 2: 태그는 그대로 1개(no-op, revision·outbox 불변), KV 는 이제 완성
    assert_eq!(fx.count(&format!("SELECT COUNT(*) FROM asset_tags WHERE asset_id = {id}")), 1);
    assert_eq!(fx.count(&format!("SELECT revision FROM media_assets WHERE asset_id = {id}")), rev);
    assert_eq!(fx.count("SELECT COUNT(*) FROM search_outbox"), outbox);
    assert_eq!(fx.count("SELECT COUNT(*) FROM plugin_kv"), 1);
    h.shutdown();
}
