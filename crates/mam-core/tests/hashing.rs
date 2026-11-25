//! T-id-05~07: BLAKE3 스트리밍 해시.
//!
//! 분류: Happy / Boundary / Business rule / Concurrency

mod common;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use common::set_mtime;
use mam_core::hash::{CHUNK_SIZE, hash_file_observed};
use mam_core::{Blake3Hasher, ContentHasher, CoreError};
use tempfile::TempDir;

#[test]
fn t_id_05_matches_official_vectors_and_reference_hash() {
    // Arrange
    let dir = TempDir::new().unwrap();
    let never = AtomicBool::new(false);
    let empty = dir.path().join("empty.bin");
    fs::write(&empty, b"").unwrap();
    let one = dir.path().join("one.bin");
    fs::write(&one, [0u8]).unwrap();
    // 청크 경계를 넘는 크기 (3.5 청크)
    let big_bytes: Vec<u8> = (0..CHUNK_SIZE * 7 / 2).map(|i| (i % 251) as u8).collect();
    let big = dir.path().join("big.bin");
    fs::write(&big, &big_bytes).unwrap();

    // Act
    let h_empty = Blake3Hasher.hash_file(&empty, &never).unwrap();
    let h_one = Blake3Hasher.hash_file(&one, &never).unwrap();
    let h_big = Blake3Hasher.hash_file(&big, &never).unwrap();

    // Assert: 공식 테스트 벡터 (input_len 0 / 1) 와 blake3::hash 참조 구현
    assert_eq!(
        h_empty.hash.to_hex(),
        "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
    );
    assert_eq!(
        h_one.hash.to_hex(),
        "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"
    );
    assert_eq!(h_empty.size, 0);
    assert_eq!(h_big.size, big_bytes.len() as u64);
    assert_eq!(h_big.hash.as_bytes(), blake3::hash(&big_bytes).as_bytes());
}

#[test]
fn t_id_06_change_during_hash_is_discarded_size() {
    // Arrange: 첫 청크를 읽은 직후 파일 끝에 덧붙인다
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("grow.bin");
    fs::write(&f, vec![7u8; CHUNK_SIZE * 3]).unwrap();
    let never = AtomicBool::new(false);
    let mut appended = false;

    // Act
    let r = hash_file_observed(&f, &never, &mut |_| {
        if !appended {
            appended = true;
            let mut h = OpenOptions::new().append(true).open(&f).unwrap();
            h.write_all(b"tail").unwrap();
        }
    });

    // Assert
    assert!(matches!(r, Err(CoreError::ChangedDuringHash(_))), "{r:?}");
}

#[test]
fn t_id_06_change_during_hash_is_discarded_mtime_only() {
    // 크기는 같고 수정 시각만 바뀌어도 폐기한다
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("touch.bin");
    fs::write(&f, vec![1u8; CHUNK_SIZE * 2]).unwrap();
    set_mtime(&f, 1_600_000_000_000);
    let never = AtomicBool::new(false);
    let mut touched = false;

    let r = hash_file_observed(&f, &never, &mut |_| {
        if !touched {
            touched = true;
            set_mtime(&f, 1_650_000_000_000);
        }
    });

    assert!(matches!(r, Err(CoreError::ChangedDuringHash(_))), "{r:?}");
}

#[test]
fn t_id_06_stable_file_passes_observer_progress() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("ok.bin");
    fs::write(&f, vec![1u8; CHUNK_SIZE * 2 + 5]).unwrap();
    let never = AtomicBool::new(false);
    let mut seen = Vec::new();

    let r = hash_file_observed(&f, &never, &mut |n| seen.push(n)).unwrap();

    assert_eq!(r.size, (CHUNK_SIZE * 2 + 5) as u64);
    assert_eq!(*seen.last().unwrap(), r.size);
}

#[test]
fn t_id_07_cancel_flag_stops_without_result() {
    // Arrange: 첫 청크 후 취소
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("big.bin");
    fs::write(&f, vec![9u8; CHUNK_SIZE * 4]).unwrap();
    let cancel = AtomicBool::new(false);
    let mut chunks = 0;

    // Act
    let r = hash_file_observed(&f, &cancel, &mut |_| {
        chunks += 1;
        cancel.store(true, Ordering::Relaxed);
    });

    // Assert: 중단되고 부분 결과는 없다. 더 읽지 않았다
    assert!(matches!(r, Err(CoreError::Cancelled)));
    assert_eq!(chunks, 1);
}

#[test]
fn t_id_07_preset_cancel_never_reads() {
    let dir = TempDir::new().unwrap();
    let f = dir.path().join("a.bin");
    fs::write(&f, b"abc").unwrap();
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        Blake3Hasher.hash_file(&f, &cancel),
        Err(CoreError::Cancelled)
    ));
}
