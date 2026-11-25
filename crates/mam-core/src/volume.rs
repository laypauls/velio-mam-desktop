//! 볼륨 식별 모델 (D2) — 드라이브 문자가 바뀌어도 같은 볼륨을 알아본다.
//!
//! 볼륨 키: 로컬·USB 는 볼륨 GUID 경로(`\\?\Volume{GUID}\`), 실패 시 `serial:<hex>:<파일시스템>`,
//! NAS 는 정규화한 UNC 공유 루트(`//server/share`). 드라이브 문자는 `mount_point` 로만 기록한다.
//! OS 호출은 [`VolumeProvider`] 뒤에 숨겨 reconcile 로직을 가짜 구현으로 시험한다.

use std::path::Path;

use crate::error::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VolumeKind {
    Fixed,
    Removable,
    Network,
}

impl VolumeKind {
    /// `volumes.kind` CHECK 값
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Removable => "removable",
            Self::Network => "network",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fixed" => Some(Self::Fixed),
            "removable" => Some(Self::Removable),
            "network" => Some(Self::Network),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    /// `volumes.volume_key`
    pub key: String,
    pub kind: VolumeKind,
    pub label: Option<String>,
    /// 현재 마운트 지점 (`C:\`, `\\server\share\`). 상대 경로의 기준
    pub mount_point: String,
}

/// 볼륨 조회 — Windows 구현은 `WindowsVolumes`, 시험은 가짜 구현.
pub trait VolumeProvider {
    /// 경로가 속한 볼륨 (경로가 존재하지 않아도 볼륨은 알 수 있다).
    fn volume_of(&self, path: &Path) -> Result<VolumeInfo, CoreError>;
    /// 지금 연결돼 있고 마운트 지점이 있는 로컬(고정·이동식) 볼륨 목록. 네트워크 볼륨은 열거하지 않는다.
    fn list_volumes(&self) -> Result<Vec<VolumeInfo>, CoreError>;
    /// 마운트 지점의 전체 용량(바이트). 알 수 없으면 `None` (새 디스크 안내의 표시용 — 실패해도 흐름을 막지 않는다).
    fn total_bytes(&self, _mount_point: &str) -> Option<u64> {
        None
    }
    /// 마운트 지점의 장치가 "꺼낼 수 있는 종류"(USB·1394·SD·MMC 버스 또는 이동식 미디어)인가. 조회 실패는 `None`
    /// (호출자가 꺼낼 수 있는 것으로 취급한다). 시스템·MAMData 볼륨 제외는 호출자의 몫이다. 기본 구현은 `Some(true)`.
    fn is_ejectable_device(&self, _mount_point: &str) -> Option<bool> {
        Some(true)
    }
}

const SEPARATORS: [char; 2] = ['\\', '/'];

/// `\\?\` 확장 길이 접두어를 일반 표기로 되돌린다 (`\\?\UNC\a\b` -> `\\a\b`).
pub fn strip_verbatim(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

/// 마운트 지점 기준 상대 경로 (`file_locations.rel_path`). 대소문자는 구분하지 않고 비교하며
/// 반환 값은 원래 표기를 유지한다. 볼륨 루트 자체나 볼륨 밖 경로는 오류.
pub fn relative_path(mount_point: &str, abs_path: &Path) -> Result<String, CoreError> {
    let abs = strip_verbatim(&abs_path.to_string_lossy());
    let mount = strip_verbatim(mount_point);
    let mount_trim = mount.trim_end_matches(SEPARATORS);
    let outside = || CoreError::OutsideVolume {
        path: abs.clone(),
        mount: mount.clone(),
    };
    let head: String = abs.chars().take(mount_trim.chars().count()).collect();
    if head.to_lowercase() != mount_trim.to_lowercase() {
        return Err(outside());
    }
    let rest = &abs[head.len()..];
    if !rest.is_empty() && !rest.starts_with(SEPARATORS) {
        return Err(outside()); // "C:\foo" 가 "C:\fo" 마운트에 속하는 것으로 오인되는 것 방지
    }
    let rel = rest.trim_matches(SEPARATORS);
    if rel.is_empty() {
        return Err(outside());
    }
    Ok(rel.to_string())
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary / Validation

    use super::*;

    #[test]
    fn t_id_rel_01_strips_mount_and_keeps_case() {
        let rel = relative_path(r"D:\", Path::new(r"d:\Photos\IMG.JPG")).unwrap();
        assert_eq!(rel, r"Photos\IMG.JPG");
    }

    #[test]
    fn t_id_rel_02_verbatim_and_unc() {
        assert_eq!(
            relative_path(r"C:\", Path::new(r"\\?\C:\a\b.jpg")).unwrap(),
            r"a\b.jpg"
        );
        assert_eq!(
            relative_path(r"\\nas\share\", Path::new(r"\\?\UNC\NAS\Share\x\y.mp4")).unwrap(),
            r"x\y.mp4"
        );
    }

    #[test]
    fn t_id_rel_03_sibling_prefix_and_root_rejected() {
        assert!(relative_path(r"C:\fo", Path::new(r"C:\foo\a.jpg")).is_err());
        assert!(relative_path(r"C:\", Path::new(r"C:\")).is_err());
        assert!(relative_path(r"D:\", Path::new(r"C:\a.jpg")).is_err());
    }
}
