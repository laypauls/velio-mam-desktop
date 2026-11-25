//! 플레이어 워커 — libmpv 를 별도 프로세스에서 호스트하고 Named Pipe(JSON 줄)로 제어한다.
//!
//! 실행: `mam-player-worker --pipe mam-player-<32hex> --mpv <libmpv 폴더> [--log]` (호스트가 띄운다. 직접 실행하지 않는다)
//! 개발·시험용 환경변수: `MAM_PLAYER_AO`(예: `null` — 소리 끔), `MAM_PLAYER_RENDER_API`(`sw`|`d3d11`), `MAM_PLAYER_LOG`(mpv 경고를 stderr 로)
//!
//! 원본 보호: 원본 파일은 읽기 전용으로만 열고, mpv 의 쓰기 기능(재생 위치 저장·스크린샷·디스크 캐시·셰이더 캐시)은 옵션으로 막는다.
//! 영상은 `vo=libmpv` 렌더 API 로 호스트가 복제해 준 SharedBuffer 섹션에 직접 그린다 (자체 창 없음).

mod bank;
mod cache;
mod d3d;
mod events;
mod mpv_api;
mod player;
mod render;
mod session;

use mam_player::host::pipe::{self, ServerPipes};
use mam_player::protocol::{
    Command, Message, PROTOCOL_VERSION, Request, WorkerErrorCode, decode_line, encode_line,
    read_bounded_line,
};
use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc;

struct Args {
    pipe: String,
    mpv_dir: PathBuf,
    log: bool,
}

fn parse_args(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let (mut pipe, mut dir, mut log) = (
        None,
        std::env::var_os("MAM_LIBMPV_DIR").map(PathBuf::from),
        std::env::var_os("MAM_PLAYER_LOG").is_some(),
    );
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pipe" => pipe = Some(it.next().ok_or("--pipe value missing")?),
            "--mpv" => dir = Some(PathBuf::from(it.next().ok_or("--mpv value missing")?)),
            "--log" => log = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let pipe = pipe.ok_or("--pipe is required")?;
    if !pipe::is_valid_pipe_name(&pipe) {
        return Err("invalid pipe name".into());
    }
    Ok(Args {
        pipe,
        mpv_dir: dir.ok_or("--mpv DIR or MAM_LIBMPV_DIR is required")?,
        log,
    })
}

fn fail(code: i32, msg: &str) -> ! {
    eprintln!("mam-player-worker: {msg}");
    std::process::exit(code)
}

fn main() {
    let args = parse_args(std::env::args().skip(1)).unwrap_or_else(|e| fail(64, &e));
    // 파이프를 먼저 열어 호스트가 연결되게 한다 — 이후 libmpv 로드 실패도 오류 메시지로 알릴 수 있다
    let server = ServerPipes::create(&args.pipe)
        .unwrap_or_else(|e| fail(65, &format!("pipe create failed: {e}")));
    let (cmd_pipe, mut evt_pipe) = server
        .accept()
        .unwrap_or_else(|e| fail(65, &format!("pipe connect failed: {e}")));
    let (tx, rx) = mpsc::channel::<Message>();
    // 이벤트 쓰기 전용 스레드 — 모든 스레드는 채널로만 보낸다
    let writer = std::thread::spawn(move || {
        for m in rx {
            let Ok(line) = encode_line(&m) else { continue };
            if evt_pipe.write_all(line.as_bytes()).is_err() {
                break;
            }
        }
    });

    let opts = player::Options {
        log: args.log,
        ao: std::env::var("MAM_PLAYER_AO")
            .ok()
            .filter(|v| !v.is_empty()),
        force_sw: match std::env::var("MAM_PLAYER_RENDER_API").as_deref() {
            Ok("sw") => Some(true),
            Ok("d3d11") => Some(false),
            _ => None,
        },
    };
    let started = mpv_api::load_api(&args.mpv_dir)
        .and_then(|api| player::Player::start(api, &opts, tx.clone()));
    let player = match started {
        Ok(p) => p,
        Err(e) => {
            // 호스트에는 분류된 문구만 — 경로는 stderr 로만
            eprintln!("mam-player-worker: start failed: {e}");
            let _ = tx.send(Message::Error {
                code: WorkerErrorCode::Internal,
                msg: "player engine failed to start".into(),
            });
            drop(tx);
            let _ = writer.join();
            std::process::exit(2);
        }
    };
    let _ = tx.send(Message::Ready {
        mpv_api: player.api_version(),
        protocol: PROTOCOL_VERSION,
        render_path: player.inner.render_path,
    });

    let mut reader = BufReader::new(cmd_pipe);
    let mut line = String::new();
    loop {
        line.clear();
        match read_bounded_line(&mut reader, &mut line, 256 * 1024) {
            Ok(0) | Err(_) => break, // 호스트가 닫음/죽음 — 정리 후 종료
            Ok(_) => {}
        }
        let req: Request = match decode_line(&line) {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Message::Error {
                    code: WorkerErrorCode::InvalidArgument,
                    msg: format!("bad command: {e}"),
                });
                continue;
            }
        };
        let quit = req.command == Command::Quit;
        let result = player.handle(req.command);
        // id=0 은 응답하지 않는 고빈도 명령(Release) — 실패해도 오류 응답 대신 통계(ignored)로만 센다
        if req.id != 0 {
            let reply = match result {
                Ok(data) => Message::Reply {
                    id: req.id,
                    ok: true,
                    code: None,
                    error: None,
                    data,
                },
                Err((code, error)) => Message::Reply {
                    id: req.id,
                    ok: false,
                    code: Some(code),
                    error: Some(error),
                    data: None,
                },
            };
            let _ = tx.send(reply);
        }
        if quit {
            break;
        }
    }
    player.shutdown();
    drop(tx);
    let _ = writer.join();
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Validation(인자) / 보안(파이프 이름)

    fn args(v: &[&str]) -> impl Iterator<Item = String> {
        v.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    const NAME: &str = "mam-player-0123456789abcdef0123456789abcdef";

    #[test]
    fn t_player_worker_args_01_valid_and_invalid() {
        let a = parse_args(args(&["--pipe", NAME, "--mpv", r"C:\libmpv"])).unwrap();
        assert_eq!(
            (a.pipe.as_str(), a.mpv_dir),
            (NAME, PathBuf::from(r"C:\libmpv"))
        );
        assert!(parse_args(args(&["--mpv", r"C:\x"])).is_err(), "pipe 필수");
        assert!(
            parse_args(args(&["--pipe", "evil", "--mpv", "x"])).is_err(),
            "pipe 이름 형식"
        );
        assert!(parse_args(args(&["--pipe", NAME, "--bogus"])).is_err());
        assert!(parse_args(args(&["--pipe"])).is_err());
    }
}
