//! 앞뒤 4K 체크섬(`edge_signature`) 단위 시험 — T-hash-edge-01~04 (경계값), 05 의 전제(가운데만 다르면 같은 sig),
//! 09 (계산 도중 변경 폐기), 취소·원본 보호.
//!
//! 분류: Boundary (0B / 4K / 8K-1 / 8K / 8K+1) / Business rule (sig 의 의미) / External (도중 변경·없는 파일) / Concurrency (취소)
//! 임시 디렉터리의 실제 파일만 쓴다.

mod common;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use common::set_mtime;
use mam_core::hash::{EDGE_LEN, edge_signature_observed};
use mam_core::{ContentHasher, CoreError, edge_signature};
use tempfile::TempDir;

fn sig_of(path: &Path) -> [u8; 32] {
    *edge_signature(path, &AtomicBool::new(false))
        .unwrap()
        .sig
        .as_bytes()
}

fn expected(parts: &[&[u8]], size: u64) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(&size.to_le_bytes());
    for p in parts {
        h.update(p);
    }
    *h.finalize().as_bytes()
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn write(dir: &TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.path().join(name);
    fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn t_hash_edge_01_empty_file_signs_only_the_size() {
    // Arrange
    let dir = TempDir::new().unwrap();
    let p = write(&dir, "e.bin", b"");

    // Act
    let r = edge_signature(&p, &AtomicBool::new(false)).unwrap();

    // Assert
    assert_eq!(r.size, 0);
    assert_eq!(*r.sig.as_bytes(), expected(&[], 0));
}

#[test]
fn t_hash_edge_02_files_under_8k_are_signed_whole() {
    // Arrange: 4K (앞뒤가 겹치는 경계) / 8K-1 (겹침 방지 경계)
    let dir = TempDir::new().unwrap();
    for len in [1, EDGE_LEN, 2 * EDGE_LEN - 1] {
        let bytes = pattern(len);
        let p = write(&dir, &format!("f{len}.bin"), &bytes);

        // Act / Assert
        assert_eq!(sig_of(&p), expected(&[&bytes], len as u64), "len {len}");
    }
}

#[test]
fn t_hash_edge_03_exactly_8k_is_head_plus_tail() {
    // Arrange
    let dir = TempDir::new().unwrap();
    let bytes = pattern(2 * EDGE_LEN);
    let p = write(&dir, "f.bin", &bytes);

    // Act / Assert: 앞 4K + 뒤 4K = 전체
    assert_eq!(
        sig_of(&p),
        expected(&[&bytes[..EDGE_LEN], &bytes[EDGE_LEN..]], bytes.len() as u64)
    );
}

#[test]
fn t_hash_edge_04_over_8k_signs_only_head_and_tail() {
    // Arrange: 8K+1 과 큰 파일
    let dir = TempDir::new().unwrap();
    for len in [2 * EDGE_LEN + 1, 1 << 20] {
        let bytes = pattern(len);
        let p = write(&dir, &format!("f{len}.bin"), &bytes);

        // Act / Assert
        assert_eq!(
            sig_of(&p),
            expected(
                &[&bytes[..EDGE_LEN], &bytes[len - EDGE_LEN..]],
                len as u64
            ),
            "len {len}"
        );
    }
}

#[test]
fn t_hash_edge_04b_sig_is_a_candidate_filter_middle_only_difference_is_the_same_sig() {
    // Arrange: 크기·앞 4K·뒤 4K 가 같고 가운데만 다른 두 파일 (비압축 TIFF·WAV 의 부분 편집과 같은 상황)
    let dir = TempDir::new().unwrap();
    let a = pattern(100_000);
    let mut b = a.clone();
    b[50_000] ^= 0xFF;
    let (pa, pb) = (write(&dir, "a.bin", &a), write(&dir, "b.bin", &b));
    // 앞 / 뒤 / 크기가 다르면 sig 도 다르다
    let mut front = a.clone();
    front[10] ^= 0xFF;
    let mut back = a.clone();
    back[99_990] ^= 0xFF;
    let mut longer = a.clone();
    longer.push(0);

    // Act / Assert
    assert_eq!(sig_of(&pa), sig_of(&pb), "middle-only difference is invisible");
    assert_ne!(sig_of(&pa), sig_of(&write(&dir, "f.bin", &front)));
    assert_ne!(sig_of(&pa), sig_of(&write(&dir, "k.bin", &back)));
    assert_ne!(sig_of(&pa), sig_of(&write(&dir, "l.bin", &longer)));
    // 완전한 복사본은 같다
    assert_eq!(sig_of(&pa), sig_of(&write(&dir, "copy.bin", &a)));
}

#[test]
fn t_hash_edge_09_change_during_read_is_discarded() {
    // Arrange: 앞 구간을 읽은 직후 파일을 늘리고 수정 시각을 바꾼다
    let dir = TempDir::new().unwrap();
    let p = write(&dir, "g.bin", &pattern(50_000));
    let mut touched = false;

    // Act
    let r = edge_signature_observed(&p, &AtomicBool::new(false), &mut || {
        if !touched {
            touched = true;
            let mut bytes = pattern(50_000);
            bytes.extend_from_slice(b"appended");
            fs::write(&p, bytes).unwrap();
            set_mtime(&p, 1_500_000_000_000);
        }
    });

    // Assert: 부분 결과를 돌려주지 않는다
    assert!(matches!(r, Err(CoreError::ChangedDuringHash(_))), "{r:?}");
}

#[test]
fn t_hash_edge_09b_truncated_while_reading_is_a_change_not_an_io_error() {
    // Arrange: 앞 구간을 읽은 뒤 파일이 잘린다 (뒤 4K 를 읽을 수 없다)
    let dir = TempDir::new().unwrap();
    let p = write(&dir, "t.bin", &pattern(50_000));

    // Act
    let r = edge_signature_observed(&p, &AtomicBool::new(false), &mut || {
        fs::write(&p, b"tiny").unwrap();
    });

    // Assert
    assert!(matches!(r, Err(CoreError::ChangedDuringHash(_))), "{r:?}");
}

#[test]
fn t_hash_edge_09c_cancel_and_missing_file_return_errors_without_a_result() {
    // Arrange
    let dir = TempDir::new().unwrap();
    let p = write(&dir, "c.bin", &pattern(20_000));
    let cancel = AtomicBool::new(true);

    // Act / Assert
    assert!(matches!(
        edge_signature(&p, &cancel),
        Err(CoreError::Cancelled)
    ));
    cancel.store(false, Ordering::Relaxed);
    let missing = dir.path().join("nope.bin");
    assert!(matches!(
        edge_signature(&missing, &cancel),
        Err(CoreError::Io { .. })
    ));
}

#[test]
fn t_hash_edge_00_original_is_untouched_and_the_trait_default_reads_the_real_file() {
    // Arrange
    let dir = TempDir::new().unwrap();
    let bytes = pattern(30_000);
    let p = write(&dir, "o.bin", &bytes);
    set_mtime(&p, 1_600_000_000_000);
    let before = (fs::read(&p).unwrap(), fs::metadata(&p).unwrap().modified().unwrap());

    // Act: 트레이트 기본 구현 (주입 가능한 경로)
    let r = mam_core::Blake3Hasher
        .edge_signature(&p, &AtomicBool::new(false))
        .unwrap();

    // Assert
    assert_eq!(*r.sig.as_bytes(), sig_of(&p));
    assert_eq!(r.modified_ms, 1_600_000_000_000);
    assert_eq!(
        before,
        (fs::read(&p).unwrap(), fs::metadata(&p).unwrap().modified().unwrap())
    );
}
