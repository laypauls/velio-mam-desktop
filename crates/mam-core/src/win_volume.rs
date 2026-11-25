//! Windows 볼륨 구현 — GetVolumePathNameW / GetVolumeNameForVolumeMountPointW /
//! GetVolumeInformationW / GetDriveTypeW (+ 매핑된 네트워크 드라이브는 WNetGetConnectionW).

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE, NO_ERROR,
};
use windows_sys::Win32::NetworkManagement::WNet::WNetGetConnectionW;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, FindFirstVolumeW, FindNextVolumeW,
    FindVolumeClose, GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationW,
    GetVolumeNameForVolumeMountPointW, GetVolumePathNameW, GetVolumePathNamesForVolumeNameW,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

use crate::error::CoreError;
use crate::volume::{VolumeInfo, VolumeKind, VolumeProvider, strip_verbatim};

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;

/// Windows API 기반 볼륨 제공자.
///
/// 외장 USB HDD 는 `GetDriveTypeW` 가 `DRIVE_FIXED` 로 보고하므로 `Fixed` 로 분류된다
/// (kind 는 표시용이며 오프라인 판정에는 쓰이지 않는다).
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsVolumes;

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn api_err(api: &str) -> CoreError {
    // SAFETY: 스레드 로컬 오류 코드를 읽을 뿐이다.
    let code = unsafe { GetLastError() };
    CoreError::Volume(format!("{api} failed (error {code})"))
}

/// `\\server\share[\..]` -> (`//server/share`, `\\server\share\`)
fn unc_root(norm: &str) -> Result<(String, String), CoreError> {
    let mut parts = norm
        .trim_start_matches('\\')
        .split(['\\', '/'])
        .filter(|s| !s.is_empty());
    let (Some(server), Some(share)) = (parts.next(), parts.next()) else {
        return Err(CoreError::Volume(format!("invalid UNC path: {norm}")));
    };
    Ok((
        format!("//{}/{}", server.to_lowercase(), share.to_lowercase()),
        format!(r"\\{server}\{share}\"),
    ))
}

fn network_info(key: String, mount_point: String) -> VolumeInfo {
    VolumeInfo {
        key,
        kind: VolumeKind::Network,
        label: None,
        mount_point,
    }
}

struct FsInfo {
    label: Option<String>,
    serial: u32,
    fs_name: String,
}

fn volume_information(root: &str) -> Result<FsInfo, CoreError> {
    let root_w = wide(root);
    let mut label = [0u16; 261];
    let mut fs = [0u16; 261];
    let mut serial = 0u32;
    // SAFETY: 모든 버퍼는 길이를 함께 넘기며 root_w 는 NUL 종료 문자열이다.
    let ok = unsafe {
        GetVolumeInformationW(
            root_w.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            &mut serial,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    if ok == 0 {
        return Err(api_err("GetVolumeInformationW"));
    }
    let label = from_wide(&label);
    Ok(FsInfo {
        label: (!label.is_empty()).then_some(label),
        serial,
        fs_name: from_wide(&fs),
    })
}

fn kind_of(drive_type: u32) -> VolumeKind {
    if drive_type == DRIVE_REMOVABLE {
        VolumeKind::Removable
    } else {
        VolumeKind::Fixed
    }
}

/// 마운트 지점(`C:\`)의 볼륨 GUID 경로.
fn volume_guid(mount: &str) -> Option<String> {
    let mount_w = wide(mount);
    let mut out = [0u16; 64];
    // SAFETY: 버퍼 길이를 함께 넘기며 mount_w 는 NUL 종료.
    let ok = unsafe {
        GetVolumeNameForVolumeMountPointW(mount_w.as_ptr(), out.as_mut_ptr(), out.len() as u32)
    };
    (ok != 0).then(|| from_wide(&out))
}

fn mount_point_of(norm: &str) -> Result<String, CoreError> {
    let verbatim = wide(&format!(r"\\?\{norm}"));
    let mut out = vec![0u16; norm.encode_utf16().count() + 16];
    // SAFETY: 출력 버퍼 길이를 함께 넘긴다.
    let ok = unsafe { GetVolumePathNameW(verbatim.as_ptr(), out.as_mut_ptr(), out.len() as u32) };
    if ok == 0 {
        return Err(api_err("GetVolumePathNameW"));
    }
    Ok(strip_verbatim(&from_wide(&out)))
}

/// 매핑된 네트워크 드라이브(`Z:`) -> `\\server\share[\sub]`
fn mapped_remote(mount: &str) -> Result<String, CoreError> {
    let local = wide(mount.trim_end_matches('\\'));
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: 버퍼와 길이 포인터가 유효하다.
    let rc = unsafe { WNetGetConnectionW(local.as_ptr(), buf.as_mut_ptr(), &mut len) };
    if rc != NO_ERROR {
        return Err(CoreError::Volume(format!(
            "WNetGetConnectionW failed (error {rc})"
        )));
    }
    Ok(from_wide(&buf))
}

/// IOCTL_STORAGE_QUERY_PROPERTY (windows-sys 의 Ioctl 기능을 켜지 않으려고 값을 직접 둔다)
const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
/// 꺼낼 수 있는 버스: 1394, USB, SD, MMC
const EJECTABLE_BUSES: [u32; 4] = [4, 7, 0x0C, 0x0D];

/// 볼륨 장치의 버스 종류·이동식 여부. 질의 핸들은 접근 권한 없이(0) 열고 바로 닫는다 - 꺼내기를 막지 않는다. 조회 실패는 `None`.
fn query_ejectable(mount: &str) -> Option<bool> {
    if unsafe { GetDriveTypeW(wide(mount).as_ptr()) } == DRIVE_REMOVABLE {
        return Some(true);
    }
    let trimmed = mount.trim_end_matches('\\');
    let dev = if trimmed.len() == 2 && trimmed.ends_with(':') {
        format!(r"\\.\{trimmed}")
    } else {
        volume_guid(mount)?.trim_end_matches('\\').to_string()
    };
    let path = wide(&dev);
    // SAFETY: path 는 NUL 종료. 접근 권한 0 + 읽기·쓰기 공유로 열어 다른 핸들을 막지 않는다.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    // STORAGE_PROPERTY_QUERY { PropertyId = StorageDeviceProperty(0), QueryType = PropertyStandardQuery(0), AdditionalParameters[1] }
    let query = [0u8; 12];
    let mut out = [0u8; 1024];
    let mut returned = 0u32;
    // SAFETY: 입력·출력 버퍼와 길이가 유효하다.
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            query.as_ptr().cast(),
            query.len() as u32,
            out.as_mut_ptr().cast(),
            out.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    // SAFETY: 위에서 연 핸들 - 질의가 끝나면 즉시 닫는다.
    unsafe { CloseHandle(h) };
    if ok == 0 || returned < 32 {
        return None;
    }
    // STORAGE_DEVICE_DESCRIPTOR: RemovableMedia 는 오프셋 10, BusType 은 오프셋 28
    let bus = u32::from_le_bytes([out[28], out[29], out[30], out[31]]);
    Some(out[10] != 0 || EJECTABLE_BUSES.contains(&bus))
}

impl VolumeProvider for WindowsVolumes {
    fn is_ejectable_device(&self, mount_point: &str) -> Option<bool> {
        query_ejectable(mount_point)
    }

    fn total_bytes(&self, mount_point: &str) -> Option<u64> {
        let root = wide(mount_point);
        let mut total = 0u64;
        // SAFETY: root 는 NUL 종료 문자열이고 출력 포인터는 유효하다 (사용하지 않는 출력은 NULL).
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                root.as_ptr(),
                std::ptr::null_mut(),
                &mut total,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some(total)
    }

    fn volume_of(&self, path: &Path) -> Result<VolumeInfo, CoreError> {
        let abs = std::path::absolute(path).map_err(|e| CoreError::io(path, e))?;
        let norm = strip_verbatim(&abs.to_string_lossy());
        if norm.starts_with(r"\\") {
            let (key, mount) = unc_root(&norm)?;
            return Ok(network_info(key, mount));
        }
        let mount = mount_point_of(&norm)?;
        let mount_w = wide(&mount);
        // SAFETY: NUL 종료 문자열.
        let drive_type = unsafe { GetDriveTypeW(mount_w.as_ptr()) };
        if drive_type == DRIVE_REMOTE {
            // 매핑 드라이브: 하위 폴더 매핑도 구분되도록 원격 전체 경로를 키로 쓴다
            let remote = mapped_remote(&mount)?;
            let key = format!(
                "//{}",
                remote.trim_matches('\\').replace('\\', "/").to_lowercase()
            );
            return Ok(network_info(key, mount));
        }
        let fs = volume_information(&mount)?;
        let key = volume_guid(&mount)
            .unwrap_or_else(|| format!("serial:{:08x}:{}", fs.serial, fs.fs_name.to_lowercase()));
        Ok(VolumeInfo {
            key,
            kind: kind_of(drive_type),
            label: fs.label,
            mount_point: mount,
        })
    }

    fn list_volumes(&self) -> Result<Vec<VolumeInfo>, CoreError> {
        let mut out = Vec::new();
        let mut name = [0u16; 64];
        // SAFETY: 버퍼 길이를 함께 넘기고, 핸들은 아래에서 FindVolumeClose 로 닫는다.
        let handle = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(api_err("FindFirstVolumeW"));
        }
        loop {
            if let Some(info) = describe_guid_volume(&from_wide(&name)) {
                out.push(info);
            }
            // SAFETY: handle 은 유효한 열거 핸들이다.
            let more = unsafe { FindNextVolumeW(handle, name.as_mut_ptr(), name.len() as u32) };
            if more == 0 {
                // SAFETY: 오류 코드 조회 후 열거 핸들을 닫는다.
                let err = unsafe { GetLastError() };
                unsafe { FindVolumeClose(handle) };
                if err != ERROR_NO_MORE_FILES {
                    return Err(CoreError::Volume(format!(
                        "FindNextVolumeW failed (error {err})"
                    )));
                }
                return Ok(out);
            }
        }
    }
}

/// 연결된(마운트 지점이 있고 읽을 수 있는) 로컬 볼륨만 `Some`.
fn describe_guid_volume(guid: &str) -> Option<VolumeInfo> {
    let guid_w = wide(guid);
    // SAFETY: NUL 종료 문자열.
    let drive_type = unsafe { GetDriveTypeW(guid_w.as_ptr()) };
    if drive_type != DRIVE_FIXED && drive_type != DRIVE_REMOVABLE {
        return None;
    }
    let fs = volume_information(guid).ok()?;
    let mut names = vec![0u16; 1024];
    let mut ret = 0u32;
    // SAFETY: 버퍼 길이를 함께 넘긴다.
    let ok = unsafe {
        GetVolumePathNamesForVolumeNameW(
            guid_w.as_ptr(),
            names.as_mut_ptr(),
            names.len() as u32,
            &mut ret,
        )
    };
    if ok == 0 {
        return None;
    }
    let mount = from_wide(&names); // multi-sz 의 첫 항목
    if mount.is_empty() {
        return None;
    }
    Some(VolumeInfo {
        key: guid.to_string(),
        kind: kind_of(drive_type),
        label: fs.label,
        mount_point: mount,
    })
}
