//! SSH 구현 시험 (T-remote-ssh-01 ~ 09). **loopback(127.0.0.1) 의 임시 포트**에 이 프로세스 안의 `russh` 서버를 띄워 실제 핸드셰이크·인증·exec 를 시험한다
//! (외부 네트워크·실제 장비 없음). 아래 키는 이 시험 전용으로 만든 **버려도 되는 키**다 — 어디에도 등록되지 않았다.
//!
//! 분류: Happy (01 비밀번호·06 키 인증·04 지문 조회) / Validation (02 지문 미확인·03 지문 불일치·05 인증 거부) /
//! External (07 종료 코드·08 접속 거부·제한 시간) / Business rule (09 오류 문구에 비밀 없음, 02·03 은 인증 시도 0건).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use russh::keys::PrivateKey;
use russh::server::{self, Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;

use crate::secret::Secret;
use crate::watch::remote::runner::{RemoteConnector, RemoteRunner, RunError};
use crate::watch::remote::ssh::SshConnector;
use crate::watch::remote::tail::list_sizes;
use crate::watch::remote::{Auth as SrcAuth, RemoteSource, SourceKind};

// 시험 전용 버려도 되는 키 (ed25519) — 서버 호스트 키
const HOST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACAd7u+zgoadBEOqxFwslW/gLaIB0Kuregp5droUpT1vzgAAAJCYyaA9mMmg
PQAAAAtzc2gtZWQyNTUxOQAAACAd7u+zgoadBEOqxFwslW/gLaIB0Kuregp5droUpT1vzg
AAAEBJs1WnxdpDOq9UkeBSgLBALRj4hYBkCp2POUeaFhHZmh3u77OChp0EQ6rEXCyVb+At
ogHQq6t6Cnl2uhSlPW/OAAAADW1hbS10ZXN0LWhvc3Q=
-----END OPENSSH PRIVATE KEY-----
";
const HOST_FP: &str = "SHA256:Ig4rZ7Z7ET3zVOTrfedvfKbmxZCYcYRmG9eCLLjzATs";
// 시험 전용 버려도 되는 키 (ed25519) — 클라이언트 인증 키 (서버는 어떤 공개 키든 받는다)
const CLIENT_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACDBslYXqw31IW6hZgjDHBQxoI8UQuMcfdFb2UElZ2DBtgAAAJgAVWPIAFVj
yAAAAAtzc2gtZWQyNTUxOQAAACDBslYXqw31IW6hZgjDHBQxoI8UQuMcfdFb2UElZ2DBtg
AAAECsum0y+hgUhZCVX246dA2GrYhX5HT0v7SHiFiMOvfXrcGyVherDfUhbqFmCMMcFDGg
jxRC4xx90VvZQSVnYMG2AAAAD21hbS10ZXN0LWNsaWVudAECAwQFBg==
-----END OPENSSH PRIVATE KEY-----
";
const WRONG_FP: &str = "SHA256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Clone)]
struct Srv {
    auth_attempts: Arc<AtomicU32>,
}

impl server::Server for Srv {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for Srv {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.auth_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "audit" && password == "pw-ok" { Auth::Accept } else { Auth::reject() })
    }

    async fn auth_publickey(&mut self, _user: &str, _key: server::VerifiedPublicKey<'_>) -> Result<Auth, Self::Error> {
        self.auth_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(Auth::Accept)
    }

    async fn channel_open_session(&mut self, _c: Channel<Msg>, reply: server::ChannelOpenHandle, _s: &mut Session) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(&mut self, ch: ChannelId, data: &[u8], s: &mut Session) -> Result<(), Self::Error> {
        s.channel_success(ch)?;
        let cmd = String::from_utf8_lossy(data).to_string();
        let (out, code): (&str, u32) = if cmd.contains("audit-log.*") {
            ("12 audit-log.0\n5 audit-log.1\n", 0)
        } else if cmd.starts_with("tail -c +") {
            ("hello\n", 0)
        } else if cmd == "exit9" {
            ("", 9)
        } else if cmd == "slow" {
            return Ok(()); // 영영 답하지 않는다
        } else {
            ("", 3)
        };
        s.data(ch, out.as_bytes().to_vec())?;
        s.exit_status_request(ch, code)?;
        s.eof(ch)?;
        s.close(ch)?;
        Ok(())
    }
}

struct Fx {
    rt: Runtime,
    port: u16,
    attempts: Arc<AtomicU32>,
}

fn serve() -> Fx {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let attempts = Arc::new(AtomicU32::new(0));
    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let port = listener.local_addr().unwrap().port();
    let cfg = Arc::new(server::Config {
        keys: vec![PrivateKey::from_openssh(HOST_KEY).unwrap()],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..server::Config::default()
    });
    let srv = Srv { auth_attempts: Arc::clone(&attempts) };
    rt.spawn(async move {
        let mut srv = srv;
        while let Ok((sock, peer)) = listener.accept().await {
            let (cfg, h) = (Arc::clone(&cfg), srv.new_client(Some(peer)));
            tokio::spawn(async move {
                if let Ok(s) = server::run_stream(cfg, sock, h).await {
                    let _ = s.await;
                }
            });
        }
    });
    Fx { rt, port, attempts }
}

fn src(port: u16, host_key: Option<&str>, auth: SrcAuth) -> RemoteSource {
    RemoteSource {
        source_id: 1,
        kind: SourceKind::DdpAudit,
        name: "t".into(),
        host: "127.0.0.1".into(),
        port,
        user: "audit".into(),
        auth,
        host_key: host_key.map(str::to_string),
        log_dir: Some("/tmp/audit".into()),
        db_path: None,
        poll_sec: 1,
        path_map: Vec::new(),
    }
}

fn pw(p: &str) -> SrcAuth {
    SrcAuth::Password(Secret::new(p.into()))
}

fn conn() -> SshConnector {
    SshConnector { timeout: Duration::from_secs(3) }
}

fn run_err(r: Result<Box<dyn RemoteRunner>, RunError>) -> RunError {
    match r {
        Err(e) => e,
        Ok(_) => panic!("expected an error"),
    }
}

#[test]
fn t_remote_ssh_01_pinned_host_key_and_password_auth_run_read_commands_over_a_real_ssh_session() {
    let fx = serve();
    let mut run = conn().connect(&src(fx.port, Some(HOST_FP), pw("pw-ok"))).unwrap();
    let sizes = list_sizes(run.as_mut(), "/tmp/audit").unwrap();
    assert_eq!(sizes.get("audit-log.0"), Some(&12));
    assert_eq!(sizes.get("audit-log.1"), Some(&5));
    assert_eq!(run.run("tail -c +1 '/tmp/audit/audit-log.0'").unwrap(), b"hello\n");
    assert_eq!(fx.attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn t_remote_ssh_02_an_unconfirmed_host_key_is_refused_before_any_credential_is_sent_and_reports_the_fingerprint() {
    let fx = serve();
    let e = run_err(conn().connect(&src(fx.port, None, pw("pw-ok"))));
    assert_eq!(e, RunError::HostKeyUnconfirmed { actual: HOST_FP.into() });
    assert_eq!(e.fingerprint(), Some(HOST_FP));
    assert_eq!(fx.attempts.load(Ordering::SeqCst), 0, "no authentication attempt reached the server");
}

#[test]
fn t_remote_ssh_03_a_different_host_key_is_a_distinct_error_and_never_silently_accepted() {
    let fx = serve();
    let e = run_err(conn().connect(&src(fx.port, Some(WRONG_FP), pw("pw-ok"))));
    assert_eq!(e, RunError::HostKeyMismatch { expected: WRONG_FP.into(), actual: HOST_FP.into() });
    assert_eq!(e.key(), "host_key_mismatch");
    assert_eq!(fx.attempts.load(Ordering::SeqCst), 0);
    // 대소문자 하나만 달라도 불일치 (정확히 같아야 한다)
    let almost = HOST_FP.to_lowercase();
    assert!(matches!(conn().connect(&src(fx.port, Some(&almost), pw("pw-ok"))), Err(RunError::HostKeyMismatch { .. })));
}

#[test]
fn t_remote_ssh_04_fingerprint_lookup_returns_the_server_key_without_authenticating() {
    let fx = serve();
    assert_eq!(conn().fingerprint("127.0.0.1", fx.port).unwrap(), HOST_FP);
    assert_eq!(fx.attempts.load(Ordering::SeqCst), 0);
}

#[test]
fn t_remote_ssh_05_a_wrong_password_is_an_auth_error() {
    let fx = serve();
    let e = run_err(conn().connect(&src(fx.port, Some(HOST_FP), pw("pw-wrong"))));
    assert_eq!(e, RunError::Auth);
}

#[test]
fn t_remote_ssh_06_private_key_auth_works_and_an_unusable_key_is_a_bad_key_error_without_its_content() {
    let fx = serve();
    let ok = conn().connect(&src(fx.port, Some(HOST_FP), SrcAuth::Key(Secret::new(CLIENT_KEY.into()))));
    assert!(ok.is_ok(), "{:?}", ok.err());
    let e = run_err(conn().connect(&src(fx.port, Some(HOST_FP), SrcAuth::Key(Secret::new("-----BEGIN GARBAGE-----zzz".into())))));
    assert_eq!(e, RunError::BadKey);
    assert!(!format!("{e}{e:?}").contains("GARBAGE"));
}

#[test]
fn t_remote_ssh_07_exit_codes_map_to_no_log_dir_and_exec_errors_with_the_status() {
    let fx = serve();
    let mut run = conn().connect(&src(fx.port, Some(HOST_FP), pw("pw-ok"))).unwrap();
    assert_eq!(run.run("exit9").unwrap_err(), RunError::NoLogDir);
    match run.run("whatever") {
        Err(RunError::Exec(m)) => assert!(m.contains("exit status 3"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn t_remote_ssh_08_a_refused_port_and_a_silent_command_are_errors_not_hangs() {
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    assert!(matches!(conn().connect(&src(dead, Some(HOST_FP), pw("x"))), Err(RunError::Connect(_))));
    let fx = serve();
    let quick = SshConnector { timeout: Duration::from_millis(500) };
    let mut run = quick.connect(&src(fx.port, Some(HOST_FP), pw("pw-ok"))).unwrap();
    assert_eq!(run.run("slow").unwrap_err(), RunError::Timeout);
}

#[test]
fn t_remote_ssh_09_error_text_never_contains_the_password_or_key() {
    let fx = serve();
    let secret = "pw-super-secret-zzz";
    let errs = [
        run_err(conn().connect(&src(fx.port, Some(HOST_FP), pw(secret)))),
        run_err(conn().connect(&src(fx.port, None, pw(secret)))),
        run_err(conn().connect(&src(fx.port, Some(WRONG_FP), pw(secret)))),
        run_err(conn().connect(&src(1, Some(HOST_FP), pw(secret)))),
    ];
    for e in errs {
        assert!(!format!("{e}{e:?}").contains(secret), "{e}");
    }
    drop(fx.rt);
}
