//! 모델 교체 시 인덱스 교체 (D7): 새 디렉터리에 구축 -> 메타 검증 -> 활성 포인터 교체 -> 기존 해제 후 삭제.
//!
//! `root/ACTIVE` 텍스트 파일이 활성 세대 디렉터리 이름을 가리킨다. 포인터는 임시 파일 + rename 으로 바꾸므로
//! 어느 시점에 죽어도 이전 또는 새 세대 중 하나만 활성이다. 구축이 실패하면 새 디렉터리를 지우고 포인터는 그대로다.
//!
//! **Windows**: 메모리 매핑(view) 중인 파일이 있는 디렉터리는 삭제·이름 변경이 실패한다.
//! 호출자는 기존 인덱스 핸들을 drop 한 뒤에 [`IndexRoot::retire`] 해야 한다.

use std::path::{Path, PathBuf};

use crate::error::{Result, VectorError};
use crate::meta;
use crate::store::IndexSpec;
use crate::types::AnnIndex;

const ACTIVE: &str = "ACTIVE";

pub struct IndexRoot {
    root: PathBuf,
}

impl IndexRoot {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// 세대 디렉터리 경로 (이름은 `v<번호>` 등 호출자 규칙).
    pub fn dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// 현재 활성 세대 이름. 없으면 `None`.
    /// 포인터가 UTF-8 이 아니거나(`InvalidData`) 세대 이름으로 쓸 수 없는 문자(경로 구분자·`..`)를 담고 있으면
    /// 손상으로 보고 "활성 없음" 으로 취급한다 — 인덱스는 파생 데이터이므로 호출자가 재구축하면 된다.
    /// (그렇지 않으면 재구축의 `activate` 가 같은 바이트에서 영구히 실패한다.)
    pub fn active(&self) -> Result<Option<String>> {
        match std::fs::read_to_string(self.root.join(ACTIVE)) {
            Ok(s) => {
                let s = s.trim();
                let sane =
                    !s.is_empty() && s != ".." && s != "." && !s.contains(['/', '\\', '\0', ':']);
                Ok(sane.then(|| s.to_string()))
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// `name` 세대를 `build` 로 새 디렉터리에 구축한다. 실패하면 디렉터리를 지우고 포인터는 건드리지 않는다.
    pub fn build_generation(
        &self,
        name: &str,
        build: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<()> {
        let dir = self.dir(name);
        if dir.exists() {
            return Err(VectorError::InvalidInput(format!(
                "generation '{name}' already exists"
            )));
        }
        std::fs::create_dir_all(&dir)?;
        if let Err(e) = build(&dir) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        Ok(())
    }

    /// 새 세대의 메타가 기대 설정과 같음을 확인한 뒤 활성 포인터를 바꾼다. 이전 활성 이름을 돌려준다.
    pub fn activate(&self, name: &str, spec: &IndexSpec) -> Result<Option<String>> {
        if !meta::check(&self.dir(name), spec)? {
            return Err(VectorError::IndexMismatch(format!(
                "generation '{name}' has no completed index (missing meta)"
            )));
        }
        let previous = self.active()?;
        let tmp = self.root.join(format!("{ACTIVE}.tmp"));
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(name.as_bytes())?;
            f.sync_all()?; // 이름이 디스크에 닿은 뒤에 rename 으로 공개한다
        }
        std::fs::rename(&tmp, self.root.join(ACTIVE))?;
        Ok(previous)
    }

    /// 이전 세대 디렉터리를 삭제한다. 활성 세대는 지우지 않는다.
    /// view 로 연 핸들이 남아 있으면 Windows 에서 실패한다 (`Io`) — 핸들을 drop 한 뒤 다시 호출한다.
    pub fn retire(&self, name: &str) -> Result<()> {
        if self.active()?.as_deref() == Some(name) {
            return Err(VectorError::InvalidInput(
                "cannot retire the active generation".into(),
            ));
        }
        match std::fs::remove_dir_all(self.dir(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// 인덱스를 빌려 쓰는 쪽이 어느 세대를 열었는지 로그에 남기기 위한 간단한 이름표.
pub fn describe(index: &dyn AnnIndex) -> String {
    format!(
        "{} dim={} {} n={}",
        index.engine_name(),
        index.dimension(),
        index.quantization().as_str(),
        index.len()
    )
}
