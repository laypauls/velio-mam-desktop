//! 실행 파일 탐색 — 셸 없이 직접 실행할 수 있는 경로를 찾는다.
//!
//! Windows 에서 npm 이 설치한 `claude` 는 `claude.cmd` 래퍼(shim)이고 `CreateProcess` 는 이를 실행하지 못한다.
//! `shell: true`/`cmd /c` 는 명령 주입 위험이라 쓰지 않고, 래퍼가 가리키는 `.exe` 를 읽어 직접 실행한다.
//! 순서: PATH 의 `claude.exe` -> PATH 의 `claude.cmd` 가 가리키는 `.exe` -> `<홈>/.local/bin/claude.exe`(공식 설치 위치).
//! CLI 의 자격 증명 파일은 읽지 않는다(경로 탐색만).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 환경 접근 — 시험이 PATH·파일 시스템·환경변수를 흉내 낸다.
pub trait CliEnv: Send + Sync {
    fn path_var(&self) -> Option<OsString>;
    fn home_dir(&self) -> Option<PathBuf>;
    fn is_file(&self, p: &Path) -> bool;
    fn read_to_string(&self, p: &Path) -> Option<String>;
    /// 환경변수 존재 여부만 본다 — **값은 읽지 않는다**.
    fn has_var(&self, name: &str) -> bool;
    fn is_windows(&self) -> bool {
        cfg!(windows)
    }
}

pub struct SystemEnv;

impl CliEnv for SystemEnv {
    fn path_var(&self) -> Option<OsString> {
        std::env::var_os("PATH")
    }

    fn home_dir(&self) -> Option<PathBuf> {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
    }

    fn is_file(&self, p: &Path) -> bool {
        p.is_file()
    }

    fn read_to_string(&self, p: &Path) -> Option<String> {
        // 래퍼 스크립트는 작다 — 비정상적으로 큰 파일은 읽지 않는다
        let meta = std::fs::metadata(p).ok()?;
        if meta.len() > 64 * 1024 {
            return None;
        }
        std::fs::read_to_string(p).ok()
    }

    fn has_var(&self, name: &str) -> bool {
        std::env::var_os(name).is_some_and(|v| !v.is_empty())
    }
}

/// `claude` 실행 파일을 찾는다. 없으면 `None`.
pub fn resolve_claude(env: &dyn CliEnv) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = env
        .path_var()
        .map(|p| {
            std::env::split_paths(&p)
                .filter(|d| !d.as_os_str().is_empty())
                .collect()
        })
        .unwrap_or_default();
    if env.is_windows() {
        // 1) 직접 실행 가능한 .exe
        for d in &dirs {
            let exe = d.join("claude.exe");
            if env.is_file(&exe) {
                return Some(exe);
            }
        }
        // 2) npm 래퍼(.cmd) -> 가리키는 .exe
        for d in &dirs {
            let shim = d.join("claude.cmd");
            if env.is_file(&shim)
                && let Some(t) = shim_target(env, &shim)
            {
                return Some(t);
            }
        }
        // 3) 공식 설치 위치 (PATH 에 없을 수 있다)
        let home = env
            .home_dir()?
            .join(".local")
            .join("bin")
            .join("claude.exe");
        return env.is_file(&home).then_some(home);
    }
    for d in &dirs {
        let bin = d.join("claude");
        if env.is_file(&bin) {
            return Some(bin);
        }
    }
    let home = env.home_dir()?.join(".local").join("bin").join("claude");
    env.is_file(&home).then_some(home)
}

/// npm 이 만든 `.cmd` 래퍼에서 `"%dp0%\...\claude.exe"` 를 찾아 실제 경로로 풀어 준다. 그 밖의 형태(`node ...js`)는 `None`.
pub(crate) fn shim_target(env: &dyn CliEnv, shim: &Path) -> Option<PathBuf> {
    let text = env.read_to_string(shim)?;
    let dir = shim
        .parent()?
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_string();
    for quoted in text.split('"').skip(1).step_by(2) {
        let lower = quoted.to_ascii_lowercase();
        if !(lower.contains("%dp0%") && lower.ends_with(".exe")) {
            continue;
        }
        let idx = lower.find("%dp0%")?;
        let tail = &quoted[idx + "%dp0%".len()..];
        let tail = tail.trim_start_matches(['\\', '/']);
        let target = PathBuf::from(format!("{dir}\\{tail}"));
        return env.is_file(&target).then_some(target);
    }
    None
}
