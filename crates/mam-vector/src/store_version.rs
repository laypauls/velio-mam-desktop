//! 활성 세대 디렉터리의 **제자리 갱신 없는** 저장 — 데이터 파일 버전 + `DATA` 포인터.
//!
//! 세대 디렉터리의 메타(`mam-vector.meta`)는 만든 뒤 다시 쓰지 않는다. 증분 저장([`save_index_version`])은
//! 새 파일 `index.<n>.flat` 을 임시 파일 + fsync + rename 으로 완성한 뒤 `DATA` 포인터(임시 파일 + rename)를
//! 새 번호로 바꾼다. 열려 있는 파일은 건드리지 않으므로 읽기 쪽(View 매핑)은 이전 버전을 계속 쓰고,
//! 다음에 열 때 새 버전을 본다. 포인터가 없으면 번호 0 = 구축이 만든 `index.flat` 이다.
//!
//! 정리: 포인터 교체 뒤 활성 + 직전 버전만 남기고 지운다. Windows 에서 매핑 중인 파일은 지워지지 않고 남으며
//! (최선 노력), 다음 저장이 다시 지운다. 포인터 교체 전에 죽어 남은 파일(고아)도 같은 경로로 지워진다.
//! 쓰는 쪽은 세대당 하나라고 가정한다 (앱의 AI 틱 하나).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{Result, VectorError};
use crate::meta;
use crate::store::{Engine, IndexSpec};
use crate::types::{AnnIndex, OpenMode};

pub(crate) const DATA_PTR: &str = "DATA";

/// 버전 `v` 의 데이터 파일 경로 (0 = 구축 직후의 `index.flat`).
pub(crate) fn versioned_path(engine: Engine, dir: &Path, v: u64) -> PathBuf {
    if v == 0 {
        engine.data_path(dir)
    } else {
        dir.join(format!("index.{v}.flat"))
    }
}

/// 디렉터리의 현재 데이터 버전. 포인터가 없으면 0, 내용이 숫자가 아니면 `Corrupt` (호출자가 재구축).
pub(crate) fn current_version(dir: &Path) -> Result<u64> {
    match std::fs::read_to_string(dir.join(DATA_PTR)) {
        Ok(s) => s
            .trim()
            .parse::<u64>()
            .map_err(|_| VectorError::Corrupt(format!("bad {DATA_PTR} pointer '{}'", s.trim()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Err(VectorError::Corrupt(
            format!("{DATA_PTR} pointer is not UTF-8"),
        )),
        Err(e) => Err(e.into()),
    }
}

/// 현재 버전의 데이터 파일 경로.
pub(crate) fn current_data_path(engine: Engine, dir: &Path) -> Result<PathBuf> {
    Ok(versioned_path(engine, dir, current_version(dir)?))
}

/// 현재 버전의 데이터 파일을 연다. 포인터를 읽은 직후 쓰는 쪽이 버전을 올리고 옛 파일을 지우는 경합에서
/// `NotFound` 가 나면 포인터를 다시 읽어 한 번 더 시도한다 (직전 버전은 항상 남겨 두므로 한 번이면 충분하다).
pub(crate) fn open_current(
    dir: &Path,
    spec: &IndexSpec,
    mode: OpenMode,
) -> Result<crate::flat::FlatIndex> {
    let mut last = None;
    for _ in 0..3 {
        let v = current_version(dir)?;
        let path = versioned_path(spec.engine, dir, v);
        match crate::flat::FlatIndex::open(&path, spec.dimension, spec.quantization, mode) {
            Err(VectorError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                if current_version(dir)? == v {
                    return Err(VectorError::Io(e)); // 포인터는 그대로인데 파일이 없다 -> 진짜 없음
                }
                last = Some(VectorError::Io(e));
            }
            other => return other,
        }
    }
    Err(last.unwrap_or_else(|| VectorError::Corrupt("data pointer kept moving".into())))
}

/// `index[.<n>].flat` / `.tmp` 파일의 (버전, 경로) 목록. 알 수 없는 파일은 무시한다.
fn data_files(dir: &Path) -> Vec<(u64, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let rest = name.strip_prefix("index")?;
            let mid = rest
                .strip_suffix(".flat")
                .or_else(|| rest.strip_suffix(".tmp"))?;
            let v = if mid.is_empty() {
                0
            } else {
                mid.strip_prefix('.')?.parse().ok()?
            };
            Some((v, e.path()))
        })
        .collect()
}

fn write_pointer(dir: &Path, v: u64) -> Result<()> {
    let tmp = dir.join(format!("{DATA_PTR}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(v.to_string().as_bytes())?;
        f.sync_all()?;
    }
    // Windows: 읽는 쪽이 포인터를 막 열고 있으면 순간적으로 거부될 수 있다 -> 짧게 재시도
    let target = dir.join(DATA_PTR);
    let mut attempt = 0;
    loop {
        match std::fs::rename(&tmp, &target) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < 20 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e.into());
            }
        }
    }
}

/// 완성된(메타 있는) 디렉터리에 새 데이터 버전을 쓰고 포인터를 바꾼다. 새 버전 번호를 돌려준다.
/// 중간에 죽으면 이전 버전이 그대로 활성이다.
pub fn save_index_version(dir: &Path, index: &dyn AnnIndex, spec: &IndexSpec) -> Result<u64> {
    if !meta::check(dir, spec)? {
        return Err(VectorError::IndexMismatch(format!(
            "cannot save a version into a directory without {}",
            meta::FILE
        )));
    }
    let prev = current_version(dir)?;
    let next = data_files(dir)
        .iter()
        .map(|(v, _)| *v)
        .max()
        .unwrap_or(0)
        .max(prev)
        + 1;
    index.save_file(&versioned_path(spec.engine, dir, next))?;
    write_pointer(dir, next)?;
    prune(dir, next, prev);
    Ok(next)
}

/// 활성(`keep`)과 직전(`prev`) 외의 데이터 파일을 지운다 (실패 무시 — 매핑 중인 Windows 파일).
fn prune(dir: &Path, keep: u64, prev: u64) {
    for (v, p) in data_files(dir) {
        if v != keep && v != prev {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// 현재 데이터 파일이 `min_age` 보다 오래됐는지 (저장 주기 제한용). 알 수 없으면 `true`.
pub(crate) fn is_older_than(dir: &Path, engine: Engine, min_age: Duration) -> bool {
    current_data_path(engine, dir)
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .is_none_or(|age| age >= min_age)
}

/// 디렉터리에 남은 데이터 파일 버전 수 (시험·진단용).
pub fn data_file_versions(dir: &Path) -> Vec<u64> {
    let mut v: Vec<u64> = data_files(dir)
        .into_iter()
        .filter(|(_, p)| p.extension().is_some_and(|e| e == "flat"))
        .map(|(v, _)| v)
        .collect();
    v.sort_unstable();
    v
}
