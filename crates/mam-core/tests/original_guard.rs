//! T-id-17: 원본 보호 가드 — 스캔·해시·상태 전환을 모두 거친 뒤에도 픽스처의 내용·수정 시각·목록이 불변.
//!
//! 분류: Business rule
#![cfg(windows)]

mod common;

use common::*;
use mam_core::{VolumeKind, WindowsFileIdentity, set_volume_online};

const ID: WindowsFileIdentity = WindowsFileIdentity;

#[test]
fn t_id_17_full_pipeline_never_modifies_originals() {
    // Arrange: 여러 크기·폴더의 픽스처 (임시 디렉터리, 실제 사용자 폴더 아님)
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let files = [
        ("a.jpg", vec![1u8; 10]),
        ("sub/b.mp4", vec![2u8; 3 * 1024 * 1024 + 7]),
        ("sub/deep/c.wav", vec![]),
    ];
    let paths: Vec<_> = files.iter().map(|(n, b)| vol.write(n, b)).collect();
    set_mtime(&paths[0], 1_300_000_000_000);
    let guard = Snapshot::take(vol.dir.path());

    // Act: 스캔(신규) -> 해시 -> 재스캔 -> 완료 -> 오프라인/온라인 -> 재스캔
    let mut s = w.begin(&vol);
    let ids: Vec<i64> = paths
        .iter()
        .map(|p| created_id(w.scan(&vol, &mut s, &ID, p).unwrap()))
        .collect();
    {
        let rec = reconciler(&w.clock, &w.cancel);
        for (id, p) in ids.iter().zip(&paths) {
            rec.hash_pending_asset(&mut w.db.conn, *id, p).unwrap();
        }
    }
    w.finish(&s);
    set_volume_online(&w.db.conn, &w.clock, vol.id, false).unwrap();
    set_volume_online(&w.db.conn, &w.clock, vol.id, true).unwrap();
    let mut s2 = w.begin(&vol);
    for p in &paths {
        w.scan(&vol, &mut s2, &ID, p).unwrap();
    }
    w.finish(&s2);

    // Assert
    assert_eq!(guard, Snapshot::take(vol.dir.path()));
}
