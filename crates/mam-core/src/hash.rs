//! 전체 내용 해시 (BLAKE3 스트리밍).
//!
//! 완전 중복 판정의 유일한 근거는 이 전체 해시다 (크기·부분 해시는 후보 축소용, 확정 근거가 아니다).
//! 엣지 시그니처([`EdgeSig`]) = BLAKE3(크기 ‖ 앞 4K ‖ 뒤 4K) 는 "중복 아님" 을 빨리 확정하는 후보 필터로만 쓴다.
//! 전체 해시는 크기와 엣지 시그니처가 모두 같은 그룹에 대해서만 계산한다 (나머지는 해시 없이 유일 자산).
//! 원본은 읽기 전용으로만 연다. 해시 전후 크기·수정 시각을 비교해 도중 변경을 감지하고 (D5),
//! 취소 플래그를 청크마다 확인한다. 취소·변경 시 부분 결과는 돌려주지 않는다.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::CoreError;
use crate::identity::basic_stat as stat;

/// 읽기 버퍼 크기. 취소·변경 감시 주기이기도 하다.
pub const CHUNK_SIZE: usize = 1 << 20;

/// BLAKE3 digest (32 바이트) — `media_assets.content_hash` 와 같은 표현.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// 해시와, 해시 시점에 확인된 파일 상태 (호출자가 스캔 시점 메타와 대조한다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashedFile {
    pub hash: ContentHash,
    pub size: u64,
    pub modified_ms: i64,
}

/// 파일 내용 해시 계산기 — reconcile 이 주입받는다.
pub trait ContentHasher {
    fn hash_file(&self, path: &Path, cancel: &AtomicBool) -> Result<HashedFile, CoreError>;

    /// [`Self::hash_file`] + 읽은 바이트가 늘 때마다 `on_bytes(누적)` 호출 (스캔 멈춤 감시용). 기본 구현은 진행을 알리지 않는다.
    fn hash_file_progress(
        &self,
        path: &Path,
        cancel: &AtomicBool,
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<HashedFile, CoreError> {
        let _ = on_bytes;
        self.hash_file(path, cancel)
    }

    /// 앞뒤 4K 체크섬 (후보 축소용). 기본 구현은 실제 파일을 읽는다 — 시험은 재정의해 주입한다.
    fn edge_signature(&self, path: &Path, cancel: &AtomicBool) -> Result<EdgeFile, CoreError> {
        edge_signature(path, cancel)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Blake3Hasher;

impl ContentHasher for Blake3Hasher {
    fn hash_file(&self, path: &Path, cancel: &AtomicBool) -> Result<HashedFile, CoreError> {
        hash_file_observed(path, cancel, &mut |_| {})
    }

    fn hash_file_progress(
        &self,
        path: &Path,
        cancel: &AtomicBool,
        on_bytes: &mut dyn FnMut(u64),
    ) -> Result<HashedFile, CoreError> {
        hash_file_observed(path, cancel, on_bytes)
    }
}

/// `on_chunk(누적 읽은 바이트)` 를 청크마다 호출하는 해시. 진행률 표시와 도중 변경 시험에 쓴다.
pub fn hash_file_observed(
    path: &Path,
    cancel: &AtomicBool,
    on_chunk: &mut dyn FnMut(u64),
) -> Result<HashedFile, CoreError> {
    let (size_before, mtime_before) = stat(path)?;
    let mut file = File::open(path).map_err(|e| CoreError::io(path, e))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut total: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(CoreError::Cancelled);
        }
        let n = file.read(&mut buf).map_err(|e| CoreError::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
        on_chunk(total);
    }
    let (size_after, mtime_after) = stat(path)?;
    if (size_before, mtime_before) != (size_after, mtime_after) || total != size_before {
        return Err(CoreError::ChangedDuringHash(path.to_path_buf()));
    }
    Ok(HashedFile {
        hash: ContentHash(*hasher.finalize().as_bytes()),
        size: size_after,
        modified_ms: mtime_after,
    })
}

/// 앞뒤 가장자리 길이. 파일이 `2 * EDGE_LEN` 보다 작으면 전체를 읽는다 (앞뒤 겹침 방지).
pub const EDGE_LEN: usize = 4096;

/// 앞 4K + 뒤 4K 체크섬 (BLAKE3(크기 ‖ 앞 ‖ 뒤), 32 바이트) — `media_assets.edge_sig` 와 같은 표현.
///
/// **후보 축소용**: 서로 다르면 내용이 다르다는 뜻이지만, 같다고 같은 파일은 아니다 (가운데만 다른 파일).
/// 완전 중복 판정은 [`ContentHash`] 로만 한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeSig([u8; 32]);

impl EdgeSig {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// 가장자리 체크섬과, 계산 시점에 확인된 파일 상태 (호출자가 DB·스캔 시점 값과 대조한다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EdgeFile {
    pub sig: EdgeSig,
    pub size: u64,
    pub modified_ms: i64,
}

/// 앞 4K + 뒤 4K 체크섬을 계산한다 (읽기 전용). 전후 크기·수정 시각이 달라지거나 읽은 길이가 맞지 않으면
/// `ChangedDuringHash` — 부분 결과는 돌려주지 않는다.
/// `between` 은 앞 구간을 읽은 직후 호출된다 (도중 변경 시험용).
pub fn edge_signature_observed(
    path: &Path,
    cancel: &AtomicBool,
    between: &mut dyn FnMut(),
) -> Result<EdgeFile, CoreError> {
    let (size_before, mtime_before) = stat(path)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(CoreError::Cancelled);
    }
    let changed = || CoreError::ChangedDuringHash(path.to_path_buf());
    let mut file = File::open(path).map_err(|e| CoreError::io(path, e))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(&size_before.to_le_bytes());

    let whole = size_before < (2 * EDGE_LEN) as u64;
    let head_len = if whole {
        size_before as usize
    } else {
        EDGE_LEN
    };
    let mut buf = vec![0u8; head_len];
    read_full(&mut file, &mut buf, path, &changed)?;
    hasher.update(&buf);
    between();
    if !whole {
        file.seek(SeekFrom::Start(size_before - EDGE_LEN as u64))
            .map_err(|e| CoreError::io(path, e))?;
        read_full(&mut file, &mut buf, path, &changed)?;
        hasher.update(&buf);
    }
    let (size_after, mtime_after) = stat(path)?;
    if (size_before, mtime_before) != (size_after, mtime_after) {
        return Err(changed());
    }
    Ok(EdgeFile {
        sig: EdgeSig(*hasher.finalize().as_bytes()),
        size: size_after,
        modified_ms: mtime_after,
    })
}

/// `buf` 를 가득 채운다. 파일이 예상보다 짧으면(읽는 도중 잘림) 변경으로 본다.
fn read_full(
    file: &mut File,
    buf: &mut [u8],
    path: &Path,
    changed: &dyn Fn() -> CoreError,
) -> Result<(), CoreError> {
    match file.read_exact(buf) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Err(changed()),
        Err(e) => Err(CoreError::io(path, e)),
    }
}

/// 가장자리 체크섬 계산 (도중 변경 감시 없는 일반 호출).
pub fn edge_signature(path: &Path, cancel: &AtomicBool) -> Result<EdgeFile, CoreError> {
    edge_signature_observed(path, cancel, &mut || {})
}
