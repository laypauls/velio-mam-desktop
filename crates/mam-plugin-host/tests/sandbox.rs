//! 권한 밖 접근 3종(파일 읽기·쓰기 / 네트워크 연결 / 자식 프로세스) 차단 — F5 방식으로 **호스트가 직접** 확인한다.
//! 플러그인의 자기 보고는 보조 증거일 뿐: 비밀 파일 내용 유출 여부, 표식 파일 생성 여부, 호스트 소유 127.0.0.1 리스너 접속 여부를 본다.
//! 분류: Business rule(샌드박스) / Validation(없는 기능 호출)
mod common;

use std::net::TcpListener;

use common::*;
use serde_json::{Value, json};

const ATTACK: &str = r#"
const g = globalThis;
mam.commands.register('attack', (a) => {
  const r = { types: {} };
  for (const n of ['require', 'std', 'os', 'fetch', 'XMLHttpRequest', 'WebSocket', 'process', 'Deno', 'Bun', 'scriptArgs', 'module', 'Worker', 'WebAssembly', 'console']) r.types[n] = typeof g[n];
  const reach = (label, f) => { try { r[label] = { ok: true, value: String(f()) }; } catch (e) { r[label] = { ok: false, error: String(e && e.message || e) }; } };
  reach('read_node', () => g.require('fs').readFileSync(a.secretPath, 'utf8'));
  reach('read_std', () => g.std.loadFile(a.secretPath));
  reach('write_node', () => g.require('fs').writeFileSync(a.writePath, 'pwned'));
  reach('write_std', () => g.std.open(a.writePath, 'w').puts('pwned'));
  reach('net_node', () => g.require('net').connect(a.port, '127.0.0.1'));
  reach('fetch', () => g.fetch('http://127.0.0.1:' + a.port + '/'));
  reach('spawn_node', () => g.require('child_process').spawnSync('cmd.exe', ['/c', 'echo pwned> ' + a.markerPath]));
  reach('spawn_os', () => g.os.exec(['cmd.exe', '/c', 'echo pwned> ' + a.markerPath]));
  reach('eval_escape', () => (0, eval)('typeof require + typeof std + typeof os'));
  reach('mam_mutation', () => { 'use strict'; g.mam.host = {}; });
  g.__imp = 'pending';
  import(a.modulePath).then(() => { g.__imp = 'loaded'; }, () => { g.__imp = 'rejected'; });
  return r;
});
mam.commands.register('after', () => ({ imp: g.__imp, evil: typeof g.__evil }));
"#;

#[test]
fn t_phost_sbx_01_file_network_and_process_access_are_impossible_verified_by_host() {
    // Arrange: 비밀 파일, 쓰기 대상·표식 경로, 호스트 소유 리스너, 로드되면 흔적을 남길 모듈
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("secret.txt");
    std::fs::write(&secret, "TOPSECRET-d41d8cd9").unwrap();
    let (write_path, marker) = (
        dir.path().join("written.txt"),
        dir.path().join("marker.txt"),
    );
    let module = dir.path().join("evil.js");
    std::fs::write(&module, "globalThis.__evil = 'loaded';").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (host, _, _) = new_host(ATTACK, |_| {});

    // Act
    let report = run(&host, "attack", json!({
        "secretPath": secret.to_string_lossy(), "writePath": write_path.to_string_lossy(), "markerPath": marker.to_string_lossy(),
        "modulePath": module.to_string_lossy(), "port": port,
    })).unwrap();
    let after = run(&host, "after", json!(null)).unwrap();

    // Assert (호스트 직접 확인): 유출 없음 / 파일·표식 미생성 / 접속 없음
    assert!(
        !report.to_string().contains("TOPSECRET"),
        "비밀 파일 내용이 보고에 나타남: {report}"
    );
    assert!(!write_path.exists(), "쓰기 성공");
    assert!(!marker.exists(), "자식 프로세스 실행됨");
    assert!(
        matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
        "호스트 리스너에 접속됨"
    );
    assert_eq!(
        std::fs::read_to_string(&secret).unwrap(),
        "TOPSECRET-d41d8cd9",
        "원본 불변"
    );
    // Assert (보조): 모듈 import 는 거부, 위험 전역은 없음, 모든 시도가 실패
    assert_eq!(after, json!({"imp": "rejected", "evil": "undefined"}));
    for name in [
        "require",
        "std",
        "os",
        "fetch",
        "XMLHttpRequest",
        "WebSocket",
        "process",
        "Deno",
        "Bun",
        "scriptArgs",
        "module",
        "Worker",
        "WebAssembly",
        "console",
    ] {
        assert_eq!(report["types"][name], "undefined", "전역 {name} 이 존재함");
    }
    for k in [
        "read_node",
        "read_std",
        "write_node",
        "write_std",
        "net_node",
        "fetch",
        "spawn_node",
        "spawn_os",
        "mam_mutation",
    ] {
        assert_eq!(report[k]["ok"], Value::Bool(false), "{k}: {}", report[k]);
    }
}

#[test]
fn t_phost_sbx_02_control_group_proves_the_attack_code_is_valid() {
    // 대조군: 같은 호스트 쪽 검사가 "접속·파일 생성" 을 실제로 감지한다 (공격이 아니라 검사 수단의 유효성 확인)
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    let _client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    // 접속이 있으면 accept 가 성공한다 — 위 sbx_01 의 "WouldBlock" 단언이 의미가 있다
    let accepted = listener.accept().ok();
    assert!(accepted.is_some());
    let f = dir.path().join("m.txt");
    std::process::Command::new("cmd.exe")
        .args(["/c", &format!("echo x> {}", f.display())])
        .status()
        .unwrap();
    assert!(f.exists(), "표식 검사 수단이 파일 생성을 감지한다");
}

#[test]
fn t_phost_sbx_03_mam_global_is_frozen_and_worker_cannot_open_files_by_path() {
    // 워커는 `init` 의 소스 본문만 쓴다 — 경로를 인자로 받는 입력이 없다. 소스에서 mam 을 바꾸려는 시도는 실패
    let src = "try { mam.host.call = () => 'hijacked'; } catch (_) {} mam.commands.register('t', () => mam.host.call('echo', 5));";
    let (host, _, _) = new_host(src, |_| {});
    assert_eq!(run(&host, "t", json!(null)).unwrap(), json!(5));
}
