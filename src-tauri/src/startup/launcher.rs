//! 이전 버전 설치기 실행 — 시험에서는 가짜로 대체한다.

use std::path::Path;
use std::process::Command;

pub trait Launcher {
    fn launch(&self, installer: &Path) -> std::io::Result<()>;
}

/// 이전 설치기를 조용히(`/S`) 실행하고, 끝나면 앱을 다시 띄우게(`/UPDATE /R`) 한다.
/// 호출한 앱이 곧 종료되어도 설치기가 계속 돌도록 분리된 프로세스로 띄운다.
pub struct ProcessLauncher;

impl Launcher for ProcessLauncher {
    fn launch(&self, installer: &Path) -> std::io::Result<()> {
        if !installer.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("previous installer not found: {installer:?}"),
            ));
        }
        let mut cmd = Command::new(installer);
        cmd.args(["/S", "/UPDATE", "/R"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        }
        cmd.spawn().map(|_| ())
    }
}
