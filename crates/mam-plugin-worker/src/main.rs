//! MAM 플러그인 워커 — QuickJS 런타임 1개로 플러그인 1개를 실행하는 자식 프로세스.
//! 호스트(`mam-plugin-host`)가 stdin 으로 `init`(플러그인 id·JS 소스 본문·자원 한도)을 보내면 워커가 소스를 평가하고 `ready` 를 돌려준다.
//! **워커는 경로로 파일을 열지 않는다** — 소스는 호스트가 검증해 본문으로 넘긴다(경로 탈출·TOCTOU 표면 제거).
//! 라이브러리 코드에서 `unwrap`/`expect` 를 쓰지 않는다. 종료 코드: 0 정상·호스트 소멸, 1 초기화 실패, 2 `init` 아님·버전 불일치, 3 프로토콜 위반.

mod engine;
mod io;
mod meter;

use std::process::ExitCode;
use std::rc::Rc;

use mam_plugin_host::protocol::{CODE_TOO_LARGE, ErrorBody, HostMsg, PROTOCOL_VERSION, WorkerMsg};

use engine::Engine;
use io::Io;

fn main() -> ExitCode {
    let io = Rc::new(Io::new());
    let Some(HostMsg::Init {
        v, source, limits, ..
    }) = io.read()
    else {
        return ExitCode::from(2);
    };
    if v != PROTOCOL_VERSION {
        let _ = io.send(&WorkerMsg::InitError {
            message: format!("unsupported protocol version {v}"),
        });
        return ExitCode::from(2);
    }
    io.set_max_line(usize::try_from(limits.max_line_bytes).unwrap_or(usize::MAX));
    let engine = match Engine::boot(&io, &source, &limits) {
        Ok(e) => e,
        Err(message) => {
            let _ = io.send(&WorkerMsg::InitError { message });
            return ExitCode::from(1);
        }
    };
    drop(source);
    let _ = io.send(&WorkerMsg::Ready {
        v: PROTOCOL_VERSION,
        commands: engine.commands.clone(),
        providers: engine.providers.clone(),
    });

    while let Some(msg) = io.read() {
        match msg {
            HostMsg::Invoke {
                id,
                kind,
                target,
                params,
            } => {
                let reply = match engine.invoke(kind, &target, &params) {
                    Ok(value) => WorkerMsg::Result {
                        id,
                        ok: true,
                        value: Some(value),
                        error: None,
                    },
                    Err(f) => WorkerMsg::Result {
                        id,
                        ok: false,
                        value: None,
                        error: Some(ErrorBody {
                            code: f.code.to_owned(),
                            message: f.message,
                        }),
                    },
                };
                // 결과가 줄 상한을 넘으면 호스트가 워커를 죽이기 전에 오류 응답으로 바꾼다
                if let Err(m) = io.send(&reply) {
                    let _ = io.send(&WorkerMsg::Result {
                        id,
                        ok: false,
                        value: None,
                        error: Some(ErrorBody {
                            code: CODE_TOO_LARGE.to_owned(),
                            message: m,
                        }),
                    });
                }
            }
            HostMsg::Quit => break,
            HostMsg::Init { .. } | HostMsg::HostResult { .. } => return ExitCode::from(3),
        }
    }
    ExitCode::SUCCESS
}
