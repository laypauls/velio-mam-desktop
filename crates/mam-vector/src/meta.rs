//! 인덱스 디렉터리의 `mam-vector.meta` — 인덱스를 만든 엔진·버전·모델·차원·거리·양자화·파라미터.
//!
//! 다른 모델·차원·양자화의 벡터를 같은 인덱스에 섞지 않는다(db.md §2). 열 때 기대 설정과 대조해
//! 하나라도 다르면 [`VectorError::IndexMismatch`] 를 반환하고 호출자는 새 디렉터리에 재구축한다.
//! 데이터 파일을 먼저 쓰고 메타를 마지막에 쓴다 — 메타가 있으면 저장이 끝난 인덱스다.

use std::path::Path;

use crate::error::{Result, VectorError};
use crate::store::IndexSpec;

/// 메타 파일·디렉터리 배치 형식 버전.
pub const FORMAT_VERSION: u32 = 1;

pub(crate) const FILE: &str = "mam-vector.meta";

pub(crate) fn render(spec: &IndexSpec) -> String {
    format!(
        "format_version={FORMAT_VERSION}\nengine={}\nengine_version={}\nmodel_id={}\nmodel_version={}\ndimension={}\nmetric=cosine\nquantization={}\n",
        spec.engine.as_str(),
        spec.engine.version(),
        spec.model_id,
        spec.model_version,
        spec.dimension,
        spec.quantization.as_str(),
    )
}

/// 디렉터리의 메타를 기대값과 대조한다. 메타가 없으면 `Ok(false)`, 같으면 `Ok(true)`.
pub(crate) fn check(dir: &Path, spec: &IndexSpec) -> Result<bool> {
    let expected = render(spec);
    match std::fs::read_to_string(dir.join(FILE)) {
        Ok(found) => {
            let found = found.replace("\r\n", "\n");
            if found != expected {
                return Err(VectorError::IndexMismatch(format!(
                    "found [{}] expected [{}]",
                    found.trim().replace('\n', ", "),
                    expected.trim().replace('\n', ", ")
                )));
            }
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        // UTF-8 이 아닌 메타 = 손상 -> 다른 설정과 같은 취급 (호출자가 새 세대에 재구축)
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Err(VectorError::IndexMismatch(
            "meta file is not valid UTF-8".into(),
        )),
        Err(e) => Err(e.into()),
    }
}

/// 임시 파일에 쓰고 이름을 바꿔 쓴다.
pub(crate) fn write(dir: &Path, spec: &IndexSpec) -> Result<()> {
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, render(spec))?;
    std::fs::rename(&tmp, dir.join(FILE))?;
    Ok(())
}
