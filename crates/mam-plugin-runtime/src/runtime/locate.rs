//! 워커 실행 파일 탐색 — 환경변수 `MAM_PLUGIN_WORKER_EXE`(파일) → 실행 파일 폴더 → 그 폴더가 `deps` 면 한 단계 위(시험 바이너리).
//! 잘못 설정된 환경변수는 무시하고 다음 후보로 간다. 못 찾으면 `None` — 플러그인 호출은 `Start` 오류가 되고 `workerAvailable=false`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const WORKER_ENV: &str = "MAM_PLUGIN_WORKER_EXE";
pub const WORKER_EXE: &str = "mam-plugin-worker.exe";

/// 순수 함수 — 환경변수 값과 실행 파일 폴더를 인자로 받는다
pub fn locate_worker(env: Option<OsString>, exe_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(v) = env.filter(|v| !v.is_empty()) {
        let p = PathBuf::from(v);
        if p.is_file() {
            return Some(p);
        }
    }
    let dir = exe_dir?;
    let mut candidates = vec![dir.to_path_buf()];
    if dir.file_name().is_some_and(|n| n == "deps")
        && let Some(parent) = dir.parent()
    {
        candidates.push(parent.to_path_buf());
    }
    candidates.into_iter().map(|d| d.join(WORKER_EXE)).find(|p| p.is_file())
}

/// 실제 프로세스 환경 기준. **환경변수 재정의는 디버그 빌드 전용** — 릴리스에서는 같은 사용자 환경의 다른 프로그램·셸이 심은 변수로
/// 임의 실행 파일을 워커(= 플러그인 코드 실행기)로 쓰게 되므로 무시하고 실행 파일 폴더만 본다
pub fn locate_worker_default() -> Option<PathBuf> {
    locate_worker_with(cfg!(debug_assertions))
}

fn locate_worker_with(allow_env_override: bool) -> Option<PathBuf> {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let env = if allow_env_override { std::env::var_os(WORKER_ENV) } else { None };
    locate_worker(env, exe_dir.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T-prt-loc-01 (Business rule): 재정의 허용 시 환경변수 파일이 이기고, 불허(릴리스 동작) 시 무시되어 실행 파일 폴더 후보만 쓴다
    #[test]
    fn t_prt_loc_01_env_override_is_honored_only_when_allowed() {
        // Arrange: 환경변수는 건드리지 않고 순수 부분(`locate_worker`)으로 두 분기를 재현
        let t = std::env::temp_dir().join(format!("mam-loc-{}", std::process::id()));
        let (env_dir, exe_dir) = (t.join("env"), t.join("exe"));
        std::fs::create_dir_all(&env_dir).unwrap();
        std::fs::create_dir_all(&exe_dir).unwrap();
        let planted = env_dir.join("evil.exe");
        std::fs::write(&planted, "x").unwrap();
        let pick = |allow: bool| locate_worker(allow.then(|| planted.clone().into_os_string()), Some(&exe_dir));
        // Act / Assert
        assert_eq!(pick(true), Some(planted.clone()));
        assert_eq!(pick(false), None, "릴리스 분기: 환경변수 무시, exe 폴더에 워커 없음 -> None");
        std::fs::write(exe_dir.join(WORKER_EXE), "x").unwrap();
        assert_eq!(pick(false), Some(exe_dir.join(WORKER_EXE)));
        let _ = std::fs::remove_dir_all(&t);
    }

    /// T-prt-loc-02 (Business rule): 기본 함수의 허용 여부는 빌드 프로필(`debug_assertions`)과 같다 — 릴리스 빌드에서는 항상 false
    #[test]
    fn t_prt_loc_02_default_gate_follows_the_build_profile() {
        // 불허 경로는 환경변수가 설정돼 있어도 결과가 환경변수와 무관하다
        let without_env = locate_worker_with(false);
        assert_eq!(without_env, locate_worker(None, std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).as_deref()));
    }
}
