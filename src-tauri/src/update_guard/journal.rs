//! journal 파일 형식과 안전한 읽기·쓰기.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 계약 변경 시 `docs`/`DEVELOPER.md` 의 journal 설명도 함께 갱신한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub from_version: String,
    pub to_version: String,
    pub previous_installer: PathBuf,
    pub db_backup: Option<PathBuf>,
    pub state: State,
    /// 정상 기동(healthy)에 도달하지 못한 채 시작된 횟수
    pub failed_starts: u32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Pending,
    Healthy,
    RollingBack,
    RolledBack,
}

pub(super) enum ReadOutcome {
    Missing,
    Valid(Journal),
    /// 파싱 실패 — 원본은 `preserved` 로 옮겨 보존했다
    #[allow(dead_code)] // 경로는 테스트와 로그용
    Broken {
        preserved: PathBuf,
    },
}

pub(super) fn read_journal(path: &Path, now_ms: i64) -> std::io::Result<ReadOutcome> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ReadOutcome::Missing),
        Err(e) => return Err(e),
    };
    match serde_json::from_slice::<Journal>(&bytes) {
        Ok(j) => Ok(ReadOutcome::Valid(j)),
        Err(_) => {
            let preserved = with_suffix(path, &format!(".broken-{now_ms}"));
            std::fs::rename(path, &preserved)?;
            Ok(ReadOutcome::Broken { preserved })
        }
    }
}

/// 임시 파일에 쓴 뒤 교체한다 — 쓰는 도중 죽어도 반쯤 쓰인 journal 이 남지 않는다.
pub(super) fn write_journal(path: &Path, journal: &Journal) -> std::io::Result<()> {
    let tmp = with_suffix(path, ".tmp");
    let json = serde_json::to_vec_pretty(journal).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

pub(super) fn delete_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
