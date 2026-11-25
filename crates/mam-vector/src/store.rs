//! 인덱스 생성·저장·열기 — 엔진 선택, 디렉터리 배치, 메타 대조.
//!
//! 엔진은 전수 비교(flat) 하나다 (P1-3 결론; USearch / hnsw_rs 는 P3-5 에서 제거).
//! 디렉터리 배치: `mam-vector.meta` + `index.flat`.

use std::path::{Path, PathBuf};

use crate::error::{Result, VectorError};
use crate::flat::FlatIndex;
use crate::meta;
use crate::types::{AnnIndex, OpenMode, Quantization};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// 전수 비교
    Flat,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        "flat"
    }

    pub fn parse(s: &str) -> Option<Self> {
        (s == "flat").then_some(Self::Flat)
    }

    /// 메타에 기록하는 엔진 버전. 다르면 재구축 대상이다.
    pub fn version(self) -> String {
        "flat-1".to_string()
    }

    /// 디렉터리 안의 엔진 데이터 경로.
    pub fn data_path(self, dir: &Path) -> PathBuf {
        dir.join("index.flat")
    }
}

/// 인덱스가 만들어지는/열리는 기대 설정. 메타 파일과 1:1 이다.
#[derive(Clone, Debug)]
pub struct IndexSpec {
    pub engine: Engine,
    pub model_id: String,
    pub model_version: String,
    pub dimension: usize,
    pub quantization: Quantization,
}

/// 빈 인덱스를 메모리에 만든다. `capacity_hint` 는 예상 건수(재할당·용량 확장 최소화).
pub fn create_index(spec: &IndexSpec, capacity_hint: usize) -> Result<Box<dyn AnnIndex>> {
    if spec.dimension == 0 {
        return Err(VectorError::InvalidInput("dimension must be > 0".into()));
    }
    Ok(Box::new(FlatIndex::with_capacity(
        spec.dimension,
        spec.quantization,
        capacity_hint,
    )))
}

/// 인덱스를 `dir` 에 저장한다: 데이터 파일 -> 메타 순서. 메타가 있으면 완전한 인덱스다.
///
/// **제자리 덮어쓰기다** — 다른 핸들이 열어 둔 디렉터리(활성 세대)에는 쓰지 말 것.
/// 활성 세대의 증분 저장은 [`crate::save_index_version`] 을 쓴다.
pub fn save_index(dir: &Path, index: &dyn AnnIndex, spec: &IndexSpec) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    // 이전 저장의 메타를 먼저 지워 중간 크래시 시 "메타는 있는데 데이터는 새것" 상태를 막는다
    let _ = std::fs::remove_file(dir.join(meta::FILE));
    // 버전 포인터가 남아 있으면 새로 쓴 index.flat 을 가리지 않도록 지운다
    let _ = std::fs::remove_file(dir.join(crate::store_version::DATA_PTR));
    index.save_file(&spec.engine.data_path(dir))?;
    meta::write(dir, spec)
}

/// 저장된 인덱스를 연다. 메타 불일치 -> `IndexMismatch`, 메타 없음 + 데이터 있음 -> `IndexMismatch`,
/// 아무것도 없음 -> `Io(NotFound)`, 데이터 손상 -> `Corrupt`.
pub fn open_index(dir: &Path, spec: &IndexSpec, mode: OpenMode) -> Result<Box<dyn AnnIndex>> {
    if !meta::check(dir, spec)? {
        let data = crate::store_version::current_data_path(spec.engine, dir)?;
        return Err(if data.exists() {
            VectorError::IndexMismatch(format!("index exists without {}", meta::FILE))
        } else {
            VectorError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no index in directory",
            ))
        });
    }
    Ok(Box::new(crate::store_version::open_current(
        dir, spec, mode,
    )?))
}
