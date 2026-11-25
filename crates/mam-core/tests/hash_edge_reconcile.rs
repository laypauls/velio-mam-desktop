//! 전체 해시가 없는 자산의 재연결·제자리 변경 판정 — T-hash-edge-12 (R1/R2/R3 유지), 13 (보수적 신규 자산),
//! 14 (해시 있는 자산 터치), 15 (edge_sig 없는 V11 이전 자산).
//!
//! 분류: Business rule (PRD "내용 변경 시 신규 자산" 회귀 가드) / Boundary (크기 다름·edge 다름·수정 시각만 다름·edge 없음) / External (도중 변경)
//! 실제 SQLite(마이그레이션 적용) + 임시 디렉터리 + 주입 시계. 스캔 판정만 본다 (작업 실행기 없음).
#![cfg(windows)]

mod common;

use std::fs;

use common::*;
use mam_core::{Blake3Hasher, Outcome, PendingHash, Reconciler, VolumeKind, WindowsFileIdentity};

const ID: WindowsFileIdentity = WindowsFileIdentity;

fn body(seed: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8 ^ seed).collect()
}

/// 전체 해시 없이 스캔만 한 자산 (edge_sig 도 NULL = V11 이전과 같은 상태). 자산 id 와 경로.
fn unhashed(w: &mut World, vol: &Vol, rel: &str, bytes: &[u8]) -> (std::path::PathBuf, i64) {
    let p = vol.write(rel, bytes);
    let mut s = w.begin(vol);
    let id = created_id(w.scan(vol, &mut s, &ID, &p).unwrap());
    (p, id)
}

fn edge_is_set(w: &World, id: i64) -> bool {
    count(
        &w.db.conn,
        &format!(
            "SELECT count(*) FROM media_assets WHERE asset_id = {id} AND edge_sig IS NOT NULL"
        ),
    ) == 1
}

fn assert_superseded_with_relation(w: &World, old: i64, outcome: Outcome) -> i64 {
    let Outcome::ContentChanged {
        old_asset_id,
        new_asset_id,
    } = outcome
    else {
        panic!("expected ContentChanged, got {outcome:?}");
    };
    assert_eq!(old_asset_id, old);
    assert_eq!(asset_state(&w.db.conn, old).status, "superseded");
    let n = asset_state(&w.db.conn, new_asset_id);
    assert_eq!(
        (n.status.as_str(), n.hash_state.as_str()),
        ("active", "pending")
    );
    assert_eq!(
        count(
            &w.db.conn,
            &format!(
                "SELECT count(*) FROM asset_relations WHERE from_asset_id = {old} AND to_asset_id = {new_asset_id} AND relation = 'content_changed'"
            )
        ),
        1
    );
    // 옛 자산의 위치는 떼어지고 새 자산이 그 경로를 갖는다
    assert_eq!(
        count(
            &w.db.conn,
            &format!(
                "SELECT count(*) FROM file_locations WHERE asset_id = {old} AND detached_at IS NULL"
            )
        ),
        0
    );
    assert!(
        edge_is_set(w, new_asset_id),
        "scan fills the new asset's edge_sig"
    );
    new_asset_id
}

#[test]
fn t_hash_edge_13a_unhashed_asset_with_a_different_size_becomes_a_new_asset() {
    // Arrange
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, old) = unhashed(&mut w, &vol, "a.jpg", &body(1, 30_000));
    fs::write(&p, body(1, 30_500)).unwrap();
    let guard = Snapshot::take(vol.dir.path());

    // Act
    let mut s = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    // Assert
    assert_superseded_with_relation(&w, old, outcome);
    assert_eq!(
        guard,
        Snapshot::take(vol.dir.path()),
        "original must be untouched"
    );
}

#[test]
fn t_hash_edge_13b_same_size_different_edge_becomes_a_new_asset() {
    // Arrange: 크기는 같고 앞 4K 가 다르다
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, old) = unhashed(&mut w, &vol, "a.jpg", &body(1, 30_000));
    let mut edited = body(1, 30_000);
    edited[5] ^= 0xFF;
    fs::write(&p, &edited).unwrap();
    set_mtime(&p, 1_500_000_000_000);

    // Act
    let mut s = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    // Assert
    assert_superseded_with_relation(&w, old, outcome);
}

#[test]
fn t_hash_edge_13c_same_size_and_edge_with_only_a_new_mtime_is_still_a_new_asset() {
    // Arrange: 가운데만 편집됐을 수 있고 옛 내용은 사라져 확인할 방법이 없다 -> 보수적으로 신규 자산 (터치 오탐 수용)
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let bytes = body(1, 30_000);
    let (p, old) = unhashed(&mut w, &vol, "a.jpg", &bytes);
    let mut middle = bytes.clone();
    middle[15_000] ^= 0xFF;
    fs::write(&p, &middle).unwrap();
    set_mtime(&p, 1_500_000_000_000);

    // Act
    let mut s = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    // Assert
    assert_superseded_with_relation(&w, old, outcome);
}

#[test]
fn t_hash_edge_13d_user_data_stays_with_the_superseded_asset() {
    // Arrange: 태그가 붙은 전체 해시 없는 자산이 제자리 편집된다
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, old) = unhashed(&mut w, &vol, "a.jpg", &body(2, 20_000));
    w.db.conn
        .execute_batch(&format!(
            "INSERT INTO tags (tag_id, name, name_key) VALUES (1, 'x', 'x');
             INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES ({old}, 1, 0)"
        ))
        .unwrap();
    fs::write(&p, body(3, 20_001)).unwrap();

    // Act
    let mut s = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    // Assert: 옛 내용의 태그가 새 내용에 붙지 않는다
    let new_id = assert_superseded_with_relation(&w, old, outcome);
    assert_eq!(
        count(
            &w.db.conn,
            &format!("SELECT count(*) FROM asset_tags WHERE asset_id = {new_id}")
        ),
        0
    );
    assert_eq!(
        count(
            &w.db.conn,
            &format!("SELECT count(*) FROM asset_tags WHERE asset_id = {old}")
        ),
        1
    );
}

#[test]
fn t_hash_edge_13e_file_changed_while_signing_defers_without_touching_the_db() {
    // Arrange: 앞뒤 체크섬 계산기가 "도중 변경" 을 보고한다
    struct Changing;
    impl mam_core::ContentHasher for Changing {
        fn hash_file(
            &self,
            path: &std::path::Path,
            cancel: &std::sync::atomic::AtomicBool,
        ) -> Result<mam_core::HashedFile, mam_core::CoreError> {
            Blake3Hasher.hash_file(path, cancel)
        }
        fn edge_signature(
            &self,
            path: &std::path::Path,
            _: &std::sync::atomic::AtomicBool,
        ) -> Result<mam_core::EdgeFile, mam_core::CoreError> {
            Err(mam_core::CoreError::ChangedDuringHash(path.to_path_buf()))
        }
    }
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, old) = unhashed(&mut w, &vol, "a.jpg", &body(1, 30_000));
    fs::write(&p, body(1, 31_000)).unwrap();
    let before = (
        asset_state(&w.db.conn, old).revision,
        count(&w.db.conn, "SELECT count(*) FROM media_assets"),
        outbox(&w.db.conn).len(),
    );
    let mut s = w.begin(&vol);
    let rec = Reconciler {
        clock: &w.clock,
        hasher: &Changing,
        cancel: &w.cancel,
    };

    // Act
    let outcome = scan(&mut w.db.conn, &rec, &mut s, &ID, &vol, &p).unwrap();

    // Assert: 결과를 폐기하고 다음 스캔에서 다시 본다 (D5)
    assert_eq!(outcome, Outcome::HashDeferred);
    assert_eq!(
        before,
        (
            asset_state(&w.db.conn, old).revision,
            count(&w.db.conn, "SELECT count(*) FROM media_assets"),
            outbox(&w.db.conn).len(),
        )
    );
    assert_eq!(asset_state(&w.db.conn, old).status, "active");
}

#[test]
fn t_hash_edge_14_hashed_asset_touch_keeps_the_same_asset_and_content_change_still_compares_hashes()
{
    // Arrange: 전체 해시가 있는 자산 (기존 동작 보호)
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, id) = unhashed(&mut w, &vol, "a.jpg", &body(4, 12_000));
    {
        let rec = reconciler(&w.clock, &w.cancel);
        assert_eq!(
            rec.hash_pending_asset(&mut w.db.conn, id, &p).unwrap(),
            PendingHash::Done
        );
    }
    set_mtime(&p, 1_500_000_000_000);

    // Act: 수정 시각만 바뀜
    let mut s = w.begin(&vol);
    let touched = w.scan(&vol, &mut s, &ID, &p).unwrap();

    // Assert: 같은 자산 유지, 새 자산·관계 없음
    assert_eq!(touched, Outcome::Touched { asset_id: id });
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM asset_relations"), 0);

    // Act 2: 내용 변경 (크기 같음, 가운데만 다름) -> 전체 해시 비교로 신규 자산
    let mut middle = body(4, 12_000);
    middle[6_000] ^= 0xFF;
    fs::write(&p, &middle).unwrap();
    set_mtime(&p, 1_600_000_000_000);
    let mut s2 = w.begin(&vol);
    let changed = w.scan(&vol, &mut s2, &ID, &p).unwrap();
    assert!(matches!(changed, Outcome::ContentChanged { old_asset_id, .. } if old_asset_id == id));
}

#[test]
fn t_hash_edge_12_r1_r2_r3_reconnect_without_any_full_hash() {
    // Arrange: 전체 해시 없는 자산
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, id) = unhashed(&mut w, &vol, "dir/a.jpg", &body(5, 9_000));
    let stash = vol.path("stash.jpg");

    // Act / Assert R1: 같은 경로 missing -> present (크기·수정 시각 동일)
    fs::rename(&p, &stash).unwrap();
    let s_gone = w.begin(&vol);
    assert_eq!(w.finish(&s_gone), 1);
    fs::rename(&stash, &p).unwrap();
    let mut s = w.begin(&vol);
    assert_eq!(
        w.scan(&vol, &mut s, &ID, &p).unwrap(),
        Outcome::Reappeared { asset_id: id }
    );

    // R2: 같은 볼륨 파일 ID 이동 (이름 변경)
    let moved = vol.path("dir/renamed.jpg");
    fs::rename(&p, &moved).unwrap();
    let mut s2 = w.begin(&vol);
    assert_eq!(
        w.scan(&vol, &mut s2, &ID, &moved).unwrap(),
        Outcome::Moved {
            asset_id: id,
            by: mam_core::MoveBy::FileId
        }
    );

    // R3: 폴더 제거 후 재등록 (위치 detach -> 같은 경로·크기·수정 시각)
    let rec = reconciler(&w.clock, &w.cancel);
    assert_eq!(
        rec.detach_under(&mut w.db.conn, vol.id, "", 100).unwrap(),
        1
    );
    let mut s3 = w.begin(&vol);
    assert_eq!(
        w.scan(&vol, &mut s3, &ID, &moved).unwrap(),
        Outcome::Reattached { asset_id: id }
    );

    // Assert: 내내 같은 자산 하나, 전체 해시는 계산하지 않았다
    let st = asset_state(&w.db.conn, id);
    assert_eq!(
        (st.status.as_str(), st.hash_state.as_str()),
        ("active", "pending")
    );
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
    assert_eq!(
        count(
            &w.db.conn,
            "SELECT count(*) FROM media_assets WHERE content_hash IS NOT NULL"
        ),
        0
    );
}

#[test]
fn t_hash_edge_15_asset_without_edge_sig_is_replaced_conservatively_and_new_asset_gets_one() {
    // Arrange: V11 이전 자산 = edge_sig NULL (reconcile 이 새로 만든 자산도 NULL 로 시작한다)
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, old) = unhashed(&mut w, &vol, "a.jpg", &body(6, 15_000));
    assert!(!edge_is_set(&w, old));
    set_mtime(&p, 1_500_000_000_000); // 크기·내용 같고 수정 시각만 다름

    // Act
    let mut s = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    // Assert: 비교할 근거가 없으므로 신규 자산, 새 자산은 edge_sig 를 바로 받는다
    assert_superseded_with_relation(&w, old, outcome);
}

#[test]
fn t_hash_edge_15b_identity_only_change_with_same_size_and_mtime_keeps_the_asset() {
    // Arrange: 크기·수정 시각이 같은데 파일 ID 만 바뀐 경우(같은 내용을 같은 시각으로 덮어쓴 복사 등)는 내용 변경 근거가 아니다
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, id) = unhashed(&mut w, &vol, "a.jpg", &body(7, 8_000));
    let mtime = fs::metadata(&p).unwrap().modified().unwrap();
    let bytes = fs::read(&p).unwrap();
    fs::remove_file(&p).unwrap();
    fs::write(&p, &bytes).unwrap(); // 새 파일 ID
    fs::OpenOptions::new()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(mtime)
        .unwrap();

    // Act
    let mut s = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    // Assert: 기존 동작 유지 (메타 갱신, 같은 자산)
    assert_eq!(outcome, Outcome::PendingUpdated { asset_id: id });
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
}
