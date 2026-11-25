//! 개발 전용 자동 등록 (`pnpm dev:temp`): 환경 변수 `MAM_DEV_SEED_FOLDER` 의 폴더를 관리 폴더가 하나도 없을 때만 등록한다.
//! 모듈 전체가 `cfg(debug_assertions)` 뒤에 있어 release 빌드에는 포함되지 않는다. 등록은 `add_folder` command 와
//! 같은 `Ingest::add_folder` 를 쓰므로 MAMData·백업 폴더 겹침 거부 등 검증이 그대로 적용된다. 실패는 경고만 남긴다.

use std::path::Path;
use std::sync::Arc;

use mam_core::ingest::Ingest;

pub const ENV_KEY: &str = "MAM_DEV_SEED_FOLDER";

#[derive(Debug, PartialEq, Eq)]
pub enum SeedOutcome {
    /// 환경 변수가 없거나 비어 있다
    Disabled,
    /// 등록했다 (정규화된 표시 경로)
    Registered(String),
    /// 이미 관리 폴더가 있어 건드리지 않았다
    SkippedHasFolders,
    /// 절대 경로가 아니거나 존재하는 폴더가 아니다
    InvalidPath,
    /// `add_folder` 검증 실패 (겹침·권한 등) — 오류 종류만 담는다
    Rejected(String),
}

/// 환경 변수 값 하나를 처리한다 (환경에 의존하지 않아 시험할 수 있다).
pub fn seed_folder(ingest: &Ingest, raw: Option<&str>) -> SeedOutcome {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return SeedOutcome::Disabled;
    };
    let path = Path::new(raw);
    if !path.is_absolute() || !path.is_dir() {
        return SeedOutcome::InvalidPath;
    }
    match ingest.list_folders() {
        Ok(folders) if !folders.is_empty() => return SeedOutcome::SkippedHasFolders,
        Ok(_) => {}
        Err(e) => return SeedOutcome::Rejected(format!("{e}")),
    }
    match ingest.add_folder(raw) {
        Ok(view) => SeedOutcome::Registered(view.path),
        Err(e) => SeedOutcome::Rejected(format!("{e}")),
    }
}

/// 서비스가 시작된 뒤 호출한다. 기동을 막지 않도록 별도 스레드에서 실행하고 결과를 로그로 남긴다.
pub fn spawn_from_env(ingest: &Arc<Ingest>) {
    let Ok(raw) = std::env::var(ENV_KEY) else {
        return;
    };
    let ingest = Arc::clone(ingest);
    let spawned = std::thread::Builder::new()
        .name("mam-dev-seed".into())
        .spawn(move || match seed_folder(&ingest, Some(&raw)) {
            SeedOutcome::Disabled => {}
            SeedOutcome::Registered(path) => {
                tracing::info!(%path, "dev seed: registered managed folder from {ENV_KEY}");
            }
            SeedOutcome::SkippedHasFolders => {
                tracing::info!("dev seed: skipped, managed folders already exist");
            }
            SeedOutcome::InvalidPath => {
                tracing::warn!(
                    "dev seed: {ENV_KEY} is not an absolute existing directory; ignored"
                );
            }
            SeedOutcome::Rejected(why) => {
                tracing::warn!(%why, "dev seed: cannot register {ENV_KEY}");
            }
        });
    if let Err(e) = spawned {
        tracing::warn!(%e, "dev seed: cannot start thread");
    }
}

#[cfg(test)]
#[path = "dev_seed_tests.rs"]
mod tests;
