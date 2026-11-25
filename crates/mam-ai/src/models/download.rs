//! 이어받기 다운로드 + SHA-256 검증 + 원자적 이동.
//!
//! 흐름: `<dest>.part` 에 받는다(있으면 `Range` 로 이어받기) -> 크기 확인 -> 전체 SHA-256 검증 -> `rename`.
//! 해시가 다르면 부분 파일을 지운다(손상 데이터를 이어받지 않도록). 연결 끊김은 부분 파일을 남긴다.
//! 쓰기 실패(디스크 부족)는 부분 파일을 지워 공간을 돌려준다. 네트워크 접근은 이 모듈에서만 일어난다.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

use super::manifest::ModelFile;
use super::space::SpaceProbe;
use crate::error::{AiError, Result};

/// 연결 제한 시간 (오프라인이면 빨리 실패해 기능을 비활성으로 돌린다).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 응답 수신이 이 시간 동안 멈추면 끊긴 것으로 본다.
const RECV_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK: usize = 256 * 1024;

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .timeout_recv_body(Some(RECV_TIMEOUT))
        .timeout_recv_response(Some(RECV_TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into()
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|r| r.split('/').next())
        .unwrap_or(url)
        .to_string()
}

/// 연결 실패 계열은 `Offline`, 그 밖의 전송 오류는 `Interrupted`.
fn map_net(url: &str, e: ureq::Error) -> AiError {
    use ureq::Error as E;
    let offline = || AiError::Offline { host: host_of(url) };
    match e {
        E::ConnectionFailed | E::HostNotFound | E::Timeout(_) => offline(),
        E::Io(ref io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::NotConnected
            ) =>
        {
            offline()
        }
        other => AiError::Interrupted {
            file: format!("{url} ({other})"),
            received: 0,
            expected: 0,
        },
    }
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = File::open(path).map_err(|e| AiError::io(path, e))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf).map_err(|e| AiError::io(path, e))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// `file` 을 `dest` 로 내려받는다. 이미 `dest` 가 해시까지 맞으면 아무것도 하지 않는다.
/// `progress(받은 바이트, 전체 바이트)` 는 청크마다 호출된다.
pub fn download_file(
    file: &ModelFile,
    dest: &Path,
    space: &dyn SpaceProbe,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    if dest.is_file() && fs::metadata(dest).map(|m| m.len()).ok() == Some(file.size) {
        if sha256_file(dest)? == file.sha256 {
            return Ok(());
        }
        // 크기는 같지만 내용이 다른 파일 — 덮어쓰기 위해 지운다.
        fs::remove_file(dest).map_err(|e| AiError::io(dest, e))?;
    }
    let dir = dest.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir).map_err(|e| AiError::io(dir, e))?;
    let part = part_path(dest);

    let mut offset = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if offset > file.size {
        let _ = fs::remove_file(&part);
        offset = 0;
    }
    let needed = file.size - offset;
    if let Some(available) = space.available(dir)
        && available < needed
    {
        return Err(AiError::InsufficientSpace {
            dir: dir.to_path_buf(),
            needed,
            available,
        });
    }

    if offset < file.size {
        fetch_into_part(file, &part, offset, progress)?;
    }

    let actual = sha256_file(&part)?;
    if actual != file.sha256 {
        let _ = fs::remove_file(&part);
        return Err(AiError::HashMismatch {
            file: file.path.clone(),
            expected: file.sha256.clone(),
            actual,
        });
    }
    fs::rename(&part, dest).map_err(|e| AiError::io(dest, e))
}

fn fetch_into_part(
    file: &ModelFile,
    part: &Path,
    mut offset: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<()> {
    let agent = agent();
    let mut req = agent.get(&file.url);
    if offset > 0 {
        req = req.header("Range", format!("bytes={offset}-"));
    }
    let resp = req.call().map_err(|e| map_net(&file.url, e))?;
    match resp.status().as_u16() {
        206 => {}
        200 => offset = 0, // 서버가 Range 를 무시 — 처음부터 다시 받는다.
        416 => {
            // 부분 파일이 서버 파일과 맞지 않는다 — 버리고 처음부터.
            let _ = fs::remove_file(part);
            return fetch_into_part(file, part, 0, progress);
        }
        status => {
            return Err(AiError::Http {
                url: file.url.clone(),
                status,
            });
        }
    }

    let mut out = if offset == 0 {
        File::create(part)
    } else {
        OpenOptions::new().append(true).open(part)
    }
    .map_err(|e| AiError::io(part, e))?;

    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; CHUNK];
    let mut received = offset;
    let interrupted = |received| AiError::Interrupted {
        file: file.path.clone(),
        received,
        expected: file.size,
    };
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            // 수신 중 끊김 — 부분 파일은 남긴다(다음 시도에서 이어받기).
            Err(_) => {
                let _ = out.flush();
                return Err(interrupted(received));
            }
        };
        if let Err(e) = out.write_all(&buf[..n]) {
            // 디스크 부족 등 쓰기 실패 — 공간을 돌려주려고 부분 파일을 지운다.
            drop(out);
            let _ = fs::remove_file(part);
            return Err(AiError::io(part, e));
        }
        received += n as u64;
        progress(received, file.size);
        if received > file.size {
            drop(out);
            let _ = fs::remove_file(part);
            return Err(AiError::HashMismatch {
                file: file.path.clone(),
                expected: format!("{} bytes", file.size),
                actual: format!(">{} bytes", file.size),
            });
        }
    }
    out.flush().map_err(|e| AiError::io(part, e))?;
    if received != file.size {
        return Err(interrupted(received));
    }
    Ok(())
}
