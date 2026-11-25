//! 내보내기 대상 경로 검증과 원자적 쓰기. 원본 보호: 관리 폴더(원본)·MAMData·백업 폴더와 겹치는 대상은 거부한다
//! (`storage::paths_overlap` — 대소문자·정션·8.3 이름·`..` 정규화). 쓰기 직전에 다시 확인한다.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use mam_core::scan::{FolderRow, load_all};
use mam_core::storage::paths_overlap;
use mam_plugin_manifest::{export_extension_allowed, is_reserved_name};

use super::core::Core;
use super::export::ExportError;

const IN_MANAGED: &str = "destPath is inside a managed folder";
const IN_PROTECTED: &str = "destPath is inside the app data folder";

/// 대상 경로의 모양(절대·파일 이름·확장자·부모 폴더)과 겹침을 모두 확인한다 — provider 호출 전
pub(super) fn check_dest(core: &Core, dest: &str, ext: &str) -> Result<PathBuf, ExportError> {
    let path = check_shape(dest, ext)?;
    check_overlap(&path, &managed_roots(core)?, &core.protected_paths())?;
    Ok(path)
}

/// 쓰기: 겹침 재확인 -> 같은 폴더의 임시 파일 -> `sync_all` -> `rename`. 실패하면 임시 파일을 지운다
pub(super) fn write_atomic(core: &Core, dest: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let (roots, protected) = (managed_roots(core)?, core.protected_paths());
    check_overlap(dest, &roots, &protected)?;
    write_temp_rename(dest, bytes, &|real| check_overlap(real, &roots, &protected))
}

fn managed_roots(core: &Core) -> Result<Vec<PathBuf>, ExportError> {
    let rows = core.read(|c| Ok(load_all(c)))?;
    let rows: Vec<FolderRow> = rows.map_err(|e| ExportError::Folders(e.to_string()))?;
    Ok(rows.iter().map(FolderRow::root_path).collect())
}

fn invalid(m: &'static str) -> ExportError {
    ExportError::InvalidInput(m)
}

pub(super) fn check_shape(dest: &str, ext: &str) -> Result<PathBuf, ExportError> {
    // 매니페스트 검증과 같은 허용 목록을 한 번 더 — 실행·스크립트 형식은 쓰지 않는다
    if !export_extension_allowed(ext) {
        return Err(invalid(
            "provider file extension is not an allowed export format",
        ));
    }
    if dest.is_empty() || dest.contains('\0') {
        return Err(invalid("destPath is empty or contains NUL"));
    }
    let path = PathBuf::from(dest);
    if !path.is_absolute() {
        return Err(invalid("destPath must be absolute"));
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(invalid("destPath has no file name"))?;
    // 대체 데이터 스트림(`a:b`)·와일드카드·제어 문자 이름은 거부한다
    if name
        .chars()
        .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(invalid("destPath file name has invalid characters"));
    }
    if is_reserved_device_name(name) {
        return Err(invalid("destPath file name is a reserved device name"));
    }
    if !path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
    {
        return Err(invalid("destPath extension does not match the provider"));
    }
    let parent_ok = path
        .parent()
        .is_some_and(|p| fs::metadata(p).is_ok_and(|m| m.is_dir()));
    if !parent_ok {
        return Err(invalid("destPath parent folder does not exist"));
    }
    if path.is_dir() {
        return Err(invalid("destPath is a directory"));
    }
    Ok(path)
}

/// Windows 예약 장치 이름(`NUL.csv`, `COM1.txt` …) — 확장자가 있어도 장치로 열린다
fn is_reserved_device_name(name: &str) -> bool {
    is_reserved_name(name.split('.').next().unwrap_or(""))
}

/// 관리 폴더 루트·보호 경로(MAMData·백업) 어느 것과도 같거나 포함 관계면 거부
pub(super) fn check_overlap(
    dest: &Path,
    roots: &[PathBuf],
    protected: &[PathBuf],
) -> Result<(), ExportError> {
    if roots.iter().any(|r| paths_overlap(dest, r)) {
        return Err(invalid(IN_MANAGED));
    }
    if protected.iter().any(|p| paths_overlap(dest, p)) {
        return Err(invalid(IN_PROTECTED));
    }
    Ok(())
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_path(dest: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tag = format!(
        "{:x}{:x}{:x}",
        nanos,
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    dest.with_file_name(format!("{name}.{tag}.mamtmp"))
}

/// 임시 파일을 지우는 가드 — 성공하면 `disarm`
struct TempGuard<'a> {
    path: &'a Path,
    armed: bool,
}

impl Drop for TempGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(self.path);
        }
    }
}

/// 열린 파일의 실제 위치 — 핸들로 조회하므로 경로 해석 뒤에 폴더가 정션으로 바뀌어도 실제로 만들어진 곳을 돌려준다
#[cfg(windows)]
fn final_path(file: &fs::File, _fallback: &Path) -> io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };
    let handle = file.as_raw_handle();
    let mut buf = vec![0u16; 512];
    loop {
        // SAFETY: `handle` 는 `file` 이 살아 있는 동안 유효하고, `buf` 길이를 함께 넘긴다
        let n = unsafe {
            GetFinalPathNameByHandleW(
                handle as _,
                buf.as_mut_ptr(),
                buf.len() as u32,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        } as usize;
        if n == 0 {
            return Err(io::Error::last_os_error());
        }
        if n < buf.len() {
            buf.truncate(n);
            return Ok(PathBuf::from(std::ffi::OsString::from_wide(&buf)));
        }
        buf.resize(n + 1, 0);
    }
}

#[cfg(not(windows))]
fn final_path(_file: &fs::File, fallback: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(fallback)
}

/// 임시 파일을 만든 직후 그 실제 위치로 `check_real` 을 다시 돈다(TOCTOU) — 거부하면 임시 파일을 지우고 쓰지 않는다
pub(super) fn write_temp_rename(
    dest: &Path,
    bytes: &[u8],
    check_real: &dyn Fn(&Path) -> Result<(), ExportError>,
) -> Result<(), ExportError> {
    let mut last = io::Error::other("temp file name collision");
    for _ in 0..5 {
        let temp = temp_path(dest);
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                last = e;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let mut guard = TempGuard {
            path: &temp,
            armed: true,
        };
        check_real(&final_path(&file, &temp)?)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        // 쓰는 동안 위치가 바뀌었을 수 있으니 rename 직전에 열린 핸들로 한 번 더 확인한다 — 거부하면 guard 가 임시 파일을 지운다
        check_real(&final_path(&file, &temp)?)?;
        drop(file);
        fs::rename(&temp, dest)?;
        guard.armed = false;
        return Ok(());
    }
    Err(last.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Validation(경로 모양) / Business rule(원본·보호 경로 겹침) / Happy·External(원자 쓰기, 임시 파일 정리)

    fn msg(r: Result<PathBuf, ExportError>) -> String {
        match r {
            Err(ExportError::InvalidInput(m)) => m.to_owned(),
            other => format!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn t_pjob_exportfs_01_shape_validation() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let ok = dir.join("out.CSV");
        assert!(
            check_shape(ok.to_str().unwrap(), "csv").is_ok(),
            "case-insensitive extension"
        );
        assert!(msg(check_shape("", "csv")).contains("empty"));
        assert!(msg(check_shape("out.csv", "csv")).contains("absolute"));
        assert!(
            msg(check_shape(dir.join("out.txt").to_str().unwrap(), "csv")).contains("extension")
        );
        assert!(
            msg(check_shape(
                dir.join("missing").join("out.csv").to_str().unwrap(),
                "csv"
            ))
            .contains("parent")
        );
        let ads = format!(r"{}\a:b.csv", dir.display()); // `join` would treat `a:` as a drive prefix
        let m = msg(check_shape(&ads, "csv"));
        assert!(m.contains("invalid characters"), "{m}");
        assert!(msg(check_shape(dir.join("a\0b.csv").to_str().unwrap(), "csv")).contains("NUL"));
        fs::create_dir(dir.join("d.csv")).unwrap();
        assert!(msg(check_shape(dir.join("d.csv").to_str().unwrap(), "csv")).contains("directory"));
    }

    #[test]
    fn t_pjob_exportfs_02_overlap_with_managed_and_protected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("media");
        let data = tmp.path().join("MAMData");
        let free = tmp.path().join("free");
        for d in [&root, &data, &free] {
            fs::create_dir(d).unwrap();
        }
        let (roots, prot) = (vec![root.clone()], vec![data.clone()]);
        assert!(check_overlap(&free.join("o.csv"), &roots, &prot).is_ok());
        assert!(
            matches!(check_overlap(&root.join("o.csv"), &roots, &prot), Err(ExportError::InvalidInput(m)) if m == IN_MANAGED)
        );
        assert!(
            matches!(check_overlap(&root.join("sub").join("o.csv"), &roots, &prot), Err(ExportError::InvalidInput(m)) if m == IN_MANAGED)
        );
        // 대소문자 변형
        let upper = PathBuf::from(root.to_string_lossy().to_uppercase()).join("o.csv");
        assert!(check_overlap(&upper, &roots, &prot).is_err());
        assert!(
            matches!(check_overlap(&data.join("o.csv"), &roots, &prot), Err(ExportError::InvalidInput(m)) if m == IN_PROTECTED)
        );
    }

    #[test]
    fn t_pjob_exportfs_03_atomic_write_replaces_and_leaves_no_temp() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out.csv");
        fs::write(&dest, b"old").unwrap();
        write_temp_rename(&dest, b"new,data", &|_| Ok(())).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new,data");
        let leftovers: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".mamtmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn t_pjob_exportfs_04_failed_rename_removes_temp() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out.csv");
        fs::create_dir(&dest).unwrap(); // rename onto a directory fails
        assert!(write_temp_rename(&dest, b"x", &|_| Ok(())).is_err());
        let leftovers = fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".mamtmp"))
            .count();
        assert_eq!(leftovers, 0, "temp file cleaned after a write failure");
    }

    fn temp_leftovers(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".mamtmp"))
            .count()
    }

    #[test]
    fn t_pjob_exportfs_05_final_path_rejection_deletes_temp_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out.csv");
        let r = write_temp_rename(&dest, b"x", &|_| Err(invalid(IN_MANAGED)));
        assert!(matches!(r, Err(ExportError::InvalidInput(m)) if m == IN_MANAGED));
        assert!(!dest.exists());
        assert_eq!(temp_leftovers(tmp.path()), 0);
    }

    #[test]
    fn t_pjob_exportfs_08_second_final_path_check_before_rename_rejects_and_deletes_temp() {
        // Arrange — 첫 확인은 통과, 쓰기 뒤 두 번째 확인(rename 직전)에서 거부
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out.csv");
        fs::write(&dest, b"old").unwrap();
        let calls = std::cell::Cell::new(0);

        // Act
        let r = write_temp_rename(&dest, b"new", &|_| {
            calls.set(calls.get() + 1);
            if calls.get() >= 2 {
                Err(invalid(IN_MANAGED))
            } else {
                Ok(())
            }
        });

        // Assert — 기존 대상은 그대로, 임시 파일은 없음
        assert!(matches!(r, Err(ExportError::InvalidInput(m)) if m == IN_MANAGED));
        assert_eq!(calls.get(), 2);
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert_eq!(temp_leftovers(tmp.path()), 0);
    }

    #[test]
    fn t_pjob_exportfs_06_reserved_names_incl_superscript_and_extension_allowlist() {
        for name in [
            "COM1.csv",
            "lpt9.csv",
            "COM\u{b9}.csv",
            "COM\u{b2}.csv",
            "LPT\u{b3}.csv",
            "CONIN$.csv",
            "conout$.csv",
            "nul.csv",
        ] {
            assert!(is_reserved_device_name(name), "{name}");
        }
        for name in [
            "COM0.csv",
            "COM10.csv",
            "COMA.csv",
            "console.csv",
            "out.csv",
        ] {
            assert!(!is_reserved_device_name(name), "{name}");
        }
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("evil.exe");
        assert!(msg(check_shape(exe.to_str().unwrap(), "exe")).contains("allowed export format"));
        let bat = tmp.path().join("evil.bat");
        assert!(msg(check_shape(bat.to_str().unwrap(), "bat")).contains("allowed export format"));
    }

    /// 사전 검사를 통과한 뒤 폴더가 정션으로 바뀌는 TOCTOU 를 흉내 낸다: 사전 검사 없이 정션 경로로 쓰면 핸들의 실제 위치가 관리 폴더로 나온다
    #[cfg(windows)]
    #[test]
    fn t_pjob_exportfs_07_junction_swap_is_caught_by_final_path_check() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("media");
        let link = tmp.path().join("link");
        fs::create_dir(&root).unwrap();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&root)
            .output()
            .unwrap();
        assert!(status.status.success(), "mklink /J failed");
        let roots = vec![root.clone()];
        let dest = link.join("out.csv");
        let r = write_temp_rename(&dest, b"x", &|real| check_overlap(real, &roots, &[]));
        assert!(
            matches!(r, Err(ExportError::InvalidInput(m)) if m == IN_MANAGED),
            "{r:?}"
        );
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            0,
            "temp file removed from the original folder"
        );
        assert!(!root.join("out.csv").exists());
        // 정션이 아닌 일반 폴더는 같은 검사를 통과한다
        let free = tmp.path().join("free");
        fs::create_dir(&free).unwrap();
        assert!(
            write_temp_rename(&free.join("out.csv"), b"ok", &|real| check_overlap(
                real,
                &roots,
                &[]
            ))
            .is_ok()
        );
        let _ = fs::remove_dir(&link); // 정션만 지운다 (대상은 유지)
    }
}
