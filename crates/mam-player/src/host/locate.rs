//! 워커 실행 파일·libmpv 폴더 탐색 (계약 §5.2).
//!
//! - 워커: 환경변수 `MAM_PLAYER_WORKER`(파일) → `<exe 폴더>` → 디버그 빌드의 target 폴더 → 개발 후보
//! - libmpv: 환경변수 `MAM_LIBMPV_DIR`(폴더) → `<exe 폴더>/libmpv/` → 개발 후보(저장소 폴더)
//!
//! 잘못 설정된 환경변수가 설치본의 재생을 막지 않도록, 후보에 파일이 없으면 무시하고 다음 후보로 간다.
//! 개발 후보(`dev_dirs`)는 디버그 빌드만 넘긴다 (릴리스는 항상 빈 목록 — 컴파일 시점 경로를 담지 않는다).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub const WORKER_ENV: &str = "MAM_PLAYER_WORKER";
pub const LIBMPV_ENV: &str = "MAM_LIBMPV_DIR";
pub const WORKER_EXE: &str = "mam-player-worker.exe";
pub const LIBMPV_SUBDIR: &str = "libmpv";
pub const LIBMPV_DLL: &str = "libmpv-2.dll";

fn is_worker(p: &Path) -> bool {
    p.is_file()
}

fn has_mpv(dir: &Path) -> bool {
    dir.join(LIBMPV_DLL).is_file()
}

/// 테스트 바이너리는 `target/debug/deps/` 에서 돌고 워커는 `target/debug/` 에 있다 — 한 단계 위도 후보로 본다
fn exe_dir_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![exe_dir.to_path_buf()];
    if exe_dir.file_name().is_some_and(|n| n == "deps")
        && let Some(parent) = exe_dir.parent()
    {
        v.push(parent.to_path_buf());
    }
    v
}

/// 순수 함수 — 환경변수 값·exe 폴더·개발 후보를 인자로 받는다
pub fn locate_worker(
    env: Option<OsString>,
    exe_dir: Option<&Path>,
    dev_dirs: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(v) = env.filter(|v| !v.is_empty()) {
        let p = PathBuf::from(v);
        if is_worker(&p) {
            return Some(p);
        }
    }
    let from_exe = exe_dir.map(exe_dir_candidates).unwrap_or_default();
    from_exe
        .iter()
        .chain(dev_dirs.iter())
        .map(|d| d.join(WORKER_EXE))
        .find(|p| is_worker(p))
}

/// 순수 함수 — `libmpv-2.dll` 이 있는 폴더
pub fn locate_libmpv_dir(
    env: Option<OsString>,
    exe_dir: Option<&Path>,
    dev_dirs: &[PathBuf],
) -> Option<PathBuf> {
    if let Some(v) = env.filter(|v| !v.is_empty()) {
        let d = PathBuf::from(v);
        if has_mpv(&d) {
            return Some(d);
        }
    }
    exe_dir
        .map(|d| d.join(LIBMPV_SUBDIR))
        .into_iter()
        .chain(dev_dirs.iter().cloned())
        .find(|d| has_mpv(d))
}

fn current_exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// 실제 프로세스 환경 기준
pub fn locate_worker_default(dev_dirs: &[PathBuf]) -> Option<PathBuf> {
    locate_worker(
        std::env::var_os(WORKER_ENV),
        current_exe_dir().as_deref(),
        dev_dirs,
    )
}

pub fn locate_libmpv_default(dev_dirs: &[PathBuf]) -> Option<PathBuf> {
    locate_libmpv_dir(
        std::env::var_os(LIBMPV_ENV),
        current_exe_dir().as_deref(),
        dev_dirs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Boundary(빈 값·불완전) / Business rule(우선순위·폴백)

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn t_player_locate_01_worker_env_wins_then_exe_dir_then_dev() {
        let t = tempfile::tempdir().unwrap();
        let (env, exe, dev) = (
            t.path().join("env").join("w.exe"),
            t.path().join("exe"),
            t.path().join("dev"),
        );
        touch(&env);
        touch(&exe.join(WORKER_EXE));
        touch(&dev.join(WORKER_EXE));
        assert_eq!(
            locate_worker(
                Some(env.clone().into()),
                Some(&exe),
                std::slice::from_ref(&dev)
            ),
            Some(env)
        );
        assert_eq!(
            locate_worker(None, Some(&exe), std::slice::from_ref(&dev)),
            Some(exe.join(WORKER_EXE))
        );
        assert_eq!(
            locate_worker(None, None, std::slice::from_ref(&dev)),
            Some(dev.join(WORKER_EXE))
        );
    }

    #[test]
    fn t_player_locate_02_bad_env_falls_back_and_nothing_found_is_none() {
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("exe");
        touch(&exe.join(WORKER_EXE));
        assert_eq!(
            locate_worker(Some(t.path().join("missing.exe").into()), Some(&exe), &[]),
            Some(exe.join(WORKER_EXE))
        );
        assert_eq!(
            locate_worker(Some(OsString::new()), Some(&exe), &[]),
            Some(exe.join(WORKER_EXE))
        );
        assert_eq!(
            locate_worker(None, Some(&t.path().join("empty")), &[]),
            None
        );
        assert_eq!(locate_worker(None, None, &[]), None);
    }

    #[test]
    fn t_player_locate_03_test_binaries_in_deps_find_the_worker_in_the_parent() {
        let t = tempfile::tempdir().unwrap();
        let deps = t.path().join("debug").join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        touch(&t.path().join("debug").join(WORKER_EXE));
        assert_eq!(
            locate_worker(None, Some(&deps), &[]),
            Some(t.path().join("debug").join(WORKER_EXE))
        );
    }

    #[test]
    fn t_player_locate_04_libmpv_env_exe_subdir_and_dev_candidates() {
        let t = tempfile::tempdir().unwrap();
        let (env, exe, dev) = (
            t.path().join("env"),
            t.path().join("exe"),
            t.path().join("dev"),
        );
        touch(&env.join(LIBMPV_DLL));
        touch(&exe.join(LIBMPV_SUBDIR).join(LIBMPV_DLL));
        touch(&dev.join(LIBMPV_DLL));
        assert_eq!(
            locate_libmpv_dir(
                Some(env.clone().into()),
                Some(&exe),
                std::slice::from_ref(&dev)
            ),
            Some(env)
        );
        assert_eq!(
            locate_libmpv_dir(None, Some(&exe), std::slice::from_ref(&dev)),
            Some(exe.join(LIBMPV_SUBDIR))
        );
        assert_eq!(
            locate_libmpv_dir(None, None, std::slice::from_ref(&dev)),
            Some(dev.clone())
        );
        // 잘못된 환경변수(DLL 없는 폴더)는 폴백
        assert_eq!(
            locate_libmpv_dir(
                Some(t.path().join("nothing").into()),
                None,
                std::slice::from_ref(&dev)
            ),
            Some(dev)
        );
    }

    #[test]
    fn t_player_locate_05_libmpv_dir_without_the_dll_is_none() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("libmpv")).unwrap();
        assert_eq!(locate_libmpv_dir(None, Some(t.path()), &[]), None);
        assert_eq!(
            locate_libmpv_dir(None, None, &[t.path().join("missing")]),
            None
        );
    }
}
