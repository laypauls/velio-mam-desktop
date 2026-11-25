//! MAMData 위치 결정 — 부트스트랩 파일 `location.json` 과 NSIS 훅용 미러 `data-dir.txt`.
//!
//! 부트스트랩 디렉터리는 `%LOCALAPPDATA%\MAMDesktop` — 위치를 바꿔도 이 파일들은 이동하지 않는다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::StorageError;

const LOCATION_FILE: &str = "location.json";
const MIRROR_FILE: &str = "data-dir.txt";

/// 경로가 네트워크 드라이브(매핑된 드라이브 포함)에 있는지 판정한다 — 시험에서는 가짜로 대체한다.
pub trait DriveProbe {
    /// `drive_root` 는 `C:\` 형태.
    fn is_remote_drive(&self, drive_root: &str) -> bool;
}

/// OS(`GetDriveTypeW`)에 묻는 실제 구현. Windows 가 아니면 항상 로컬로 본다.
pub struct SystemDriveProbe;

impl DriveProbe for SystemDriveProbe {
    #[cfg(windows)]
    fn is_remote_drive(&self, drive_root: &str) -> bool {
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        const DRIVE_REMOTE: u32 = 4;
        let wide: Vec<u16> = drive_root.encode_utf16().chain(Some(0)).collect();
        // SAFETY: `wide` 는 NUL 로 끝나는 유효한 UTF-16 버퍼이고 호출 동안 살아 있다.
        unsafe { GetDriveTypeW(wide.as_ptr()) == DRIVE_REMOTE }
    }
    #[cfg(not(windows))]
    fn is_remote_drive(&self, _drive_root: &str) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationSource {
    Env,
    File,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLocation {
    pub data_dir: PathBuf,
    pub source: LocationSource,
    /// 사용자가 알아야 하지만 기동은 막지 않는 문제(손상된 부트스트랩 파일 등). 호출자가 로그에 남긴다.
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocationFile {
    data_dir: PathBuf,
}

/// `%LOCALAPPDATA%\MAMDesktop`.
pub fn bootstrap_dir() -> Result<PathBuf, StorageError> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or(StorageError::NoLocalAppData)?;
    Ok(PathBuf::from(base).join("MAMDesktop"))
}

/// 위치가 MAMData 로 쓸 수 있는 형태인지 검사한다: 절대 경로 + 로컬 드라이브 (파일시스템은 건드리지 않는다).
pub fn validate_location(path: &Path, probe: &dyn DriveProbe) -> Result<(), StorageError> {
    if !path.is_absolute() {
        return Err(StorageError::NotAbsolute(path.to_path_buf()));
    }
    let text = path.to_string_lossy().replace('/', "\\");
    let is_unc = text.starts_with("\\\\?\\UNC\\")
        || (text.starts_with("\\\\")
            && !text.starts_with("\\\\?\\")
            && !text.starts_with("\\\\.\\"));
    if is_unc {
        return Err(StorageError::NetworkLocation(path.to_path_buf()));
    }
    let text = text.strip_prefix("\\\\?\\").unwrap_or(&text);
    let mut chars = text.chars();
    if let (Some(letter), Some(':')) = (chars.next(), chars.next())
        && letter.is_ascii_alphabetic()
        && probe.is_remote_drive(&format!("{}:\\", letter.to_ascii_uppercase()))
    {
        return Err(StorageError::NetworkLocation(path.to_path_buf()));
    }
    Ok(())
}

/// 우선순위: `env`(`MAM_DATA_DIR`) > `location.json` > 기본 위치. 손상·부적합한 부트스트랩 파일은
/// 기본 위치를 쓰고 경고를 남긴다(기동을 막지 않는다). 환경 변수는 시험·진단용이라 로컬 검증을 하지 않는다.
pub fn resolve_location(
    env: Option<OsString>,
    bootstrap_dir: &Path,
    probe: &dyn DriveProbe,
) -> ResolvedLocation {
    let default = bootstrap_dir.join("MAMData");
    let mut warnings = Vec::new();
    if let Some(dir) = env.filter(|d| !d.is_empty()) {
        let dir = PathBuf::from(dir);
        if dir.is_absolute() {
            return ResolvedLocation {
                data_dir: dir,
                source: LocationSource::Env,
                warnings,
            };
        }
        warnings.push(format!("MAM_DATA_DIR ignored (not absolute): {dir:?}"));
    }
    match read_location_file(bootstrap_dir) {
        Ok(None) => {}
        Ok(Some(dir)) => match validate_location(&dir, probe) {
            Ok(()) => {
                return ResolvedLocation {
                    data_dir: dir,
                    source: LocationSource::File,
                    warnings,
                };
            }
            Err(e) => warnings.push(format!("{LOCATION_FILE} rejected, using default: {e}")),
        },
        Err(e) => warnings.push(format!("{LOCATION_FILE} unreadable, using default: {e}")),
    }
    ResolvedLocation {
        data_dir: default,
        source: LocationSource::Default,
        warnings,
    }
}

fn read_location_file(bootstrap_dir: &Path) -> Result<Option<PathBuf>, String> {
    let path = bootstrap_dir.join(LOCATION_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    serde_json::from_slice::<LocationFile>(&bytes)
        .map(|f| Some(f.data_dir))
        .map_err(|e| e.to_string())
}

/// `location.json` 에 적혀 있는 값(검증 없이). UI 가 "다음 실행부터 적용될 위치"를 보여 줄 때 쓴다.
pub fn pending_location(bootstrap_dir: &Path) -> Option<PathBuf> {
    read_location_file(bootstrap_dir).ok().flatten()
}

/// 새 위치를 `location.json` 에 기록한다 (다음 실행부터 적용). 검증 -> 디렉터리 생성·쓰기 확인 -> 원자적 기록.
/// 기존 데이터를 옮기지 않는다 — 새 위치에 MAMData 가 없으면 다음 실행은 빈 상태로 시작한다.
pub fn write_location(
    bootstrap_dir: &Path,
    new_dir: &Path,
    probe: &dyn DriveProbe,
) -> Result<(), StorageError> {
    validate_location(new_dir, probe)?;
    std::fs::create_dir_all(new_dir).map_err(|e| StorageError::io(new_dir, e))?;
    let probe_file = new_dir.join(".write-probe");
    std::fs::write(&probe_file, b"")
        .map_err(|_| StorageError::NotWritable(new_dir.to_path_buf()))?;
    let _ = std::fs::remove_file(&probe_file);

    let json = serde_json::to_vec_pretty(&LocationFile {
        data_dir: new_dir.to_path_buf(),
    })
    .map_err(|e| StorageError::io(bootstrap_dir, std::io::Error::other(e)))?;
    atomic_write(&bootstrap_dir.join(LOCATION_FILE), &json)
}

/// `location.json` 을 지워 기본 위치로 되돌린다 (다음 실행부터).
pub fn clear_location(bootstrap_dir: &Path) -> Result<(), StorageError> {
    let path = bootstrap_dir.join(LOCATION_FILE);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StorageError::io(&path, e)),
    }
}

pub fn mirror_file(bootstrap_dir: &Path) -> PathBuf {
    bootstrap_dir.join(MIRROR_FILE)
}

/// NSIS 훅이 읽는 "현재 MAMData 위치" 사본. NSIS 는 JSON 을 못 읽으므로 UTF-16LE(BOM) 한 줄(개행 없음)로 쓴다.
/// 내용이 같으면 다시 쓰지 않는다.
pub fn write_mirror(bootstrap_dir: &Path, data_dir: &Path) -> Result<(), StorageError> {
    if read_mirror(bootstrap_dir).as_deref() == Some(data_dir) {
        return Ok(());
    }
    let mut bytes = vec![0xFF, 0xFE];
    for unit in data_dir.to_string_lossy().encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    atomic_write(&mirror_file(bootstrap_dir), &bytes)
}

pub fn read_mirror(bootstrap_dir: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(mirror_file(bootstrap_dir)).ok()?;
    let body = bytes.strip_prefix(&[0xFF, 0xFE])?;
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).ok().map(PathBuf::from)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| StorageError::io(parent, e))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|e| StorageError::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        StorageError::io(path, e)
    })
}
