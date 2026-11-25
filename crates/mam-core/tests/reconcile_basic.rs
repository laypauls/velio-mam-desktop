//! T-id-08, 09, 10, 13 (+ 보류·취소·pending 해시): 신규 / 멱등 / 같은 볼륨 이동 / 터치만.
//!
//! 분류: Happy / Business rule / Concurrency(멱등) / External
//! 실제 SQLite(마이그레이션 적용) + 임시 디렉터리 + 주입 시계. 파일 ID 는 실제 NTFS 값을 쓴다.
#![cfg(windows)]

mod common;

use std::fs;
use std::sync::atomic::Ordering;

use common::*;
use mam_core::{
    Blake3Hasher, CoreError, MoveBy, Outcome, PendingHash, Reconciler, VolumeKind,
    WindowsFileIdentity,
};

const ID: WindowsFileIdentity = WindowsFileIdentity;

#[test]
fn t_id_08_new_file_creates_asset_location_and_one_outbox_event() {
    // Arrange
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let p = vol.write("Photos/a.jpg", b"hello image");
    let guard = Snapshot::take(vol.dir.path());
    let mut s = w.begin(&vol);

    // Act
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    // Assert
    let id = created_id(outcome);
    let st = asset_state(&w.db.conn, id);
    assert_eq!(
        (st.status.as_str(), st.revision, st.hash_state.as_str()),
        ("active", 1, "pending")
    );
    assert_eq!(st.size, 11);
    let locs = active_locations(&w.db.conn);
    assert_eq!(locs.len(), 1);
    assert_eq!((locs[0].asset_id, locs[0].volume_id), (id, vol.id));
    assert_eq!(locs[0].rel_path, r"Photos\a.jpg");
    assert_eq!(locs[0].presence, "present");
    assert_eq!(
        count(
            &w.db.conn,
            "SELECT count(*) FROM file_locations WHERE file_identity IS NOT NULL"
        ),
        1
    );
    assert_eq!(outbox(&w.db.conn), vec![(id, 1, "upsert".to_string())]);
    assert_eq!(
        guard,
        Snapshot::take(vol.dir.path()),
        "original must be untouched"
    );
}

#[test]
fn t_id_09_same_scan_twice_is_idempotent() {
    // Arrange
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let p = vol.write("a.jpg", b"abc");
    let mut s1 = w.begin(&vol);
    let id = created_id(w.scan(&vol, &mut s1, &ID, &p).unwrap());

    // Act: 같은 세션 안에서 다시, 그리고 새 세션에서 다시
    let again_same_session = w.scan(&vol, &mut s1, &ID, &p).unwrap();
    let mut s2 = w.begin(&vol);
    let again_new_session = w.scan(&vol, &mut s2, &ID, &p).unwrap();
    let missing = w.finish(&s2);

    // Assert: 자산·revision·outbox 변화 없음
    assert_eq!(again_same_session, Outcome::Unchanged { asset_id: id });
    assert_eq!(again_new_session, Outcome::Unchanged { asset_id: id });
    assert_eq!(missing, 0);
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
    assert_eq!(asset_state(&w.db.conn, id).revision, 1);
    assert_eq!(outbox(&w.db.conn).len(), 1);
}

#[test]
fn t_id_10_same_volume_move_relinks_same_asset() {
    // Arrange: 파일을 스캔한 뒤 탐색기에서 다른 폴더로 이동·이름 변경한 상황
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let old = vol.write("a.jpg", b"payload");
    let mut s1 = w.begin(&vol);
    let id = created_id(w.scan(&vol, &mut s1, &ID, &old).unwrap());
    fs::create_dir_all(vol.path("moved")).unwrap();
    let new = vol.path("moved/renamed.jpg");
    fs::rename(&old, &new).unwrap();

    // Act
    let mut s2 = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s2, &ID, &new).unwrap();
    let missing = w.finish(&s2);

    // Assert: 같은 asset, 이전 위치 detach, 새 위치 1개, revision 증가, 파일 없음 처리 없음
    assert_eq!(
        outcome,
        Outcome::Moved {
            asset_id: id,
            by: MoveBy::FileId
        }
    );
    assert_eq!(missing, 0);
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
    assert_eq!(
        count(
            &w.db.conn,
            "SELECT count(*) FROM file_locations WHERE detached_at IS NOT NULL"
        ),
        1
    );
    let locs = active_locations(&w.db.conn);
    assert_eq!(locs.len(), 1);
    assert_eq!(
        (locs[0].asset_id, locs[0].rel_path.as_str()),
        (id, r"moved\renamed.jpg")
    );
    assert_eq!(asset_state(&w.db.conn, id).revision, 2);
    assert_eq!(
        outbox(&w.db.conn).last().unwrap(),
        &(id, 2, "upsert".to_string())
    );
}

#[test]
fn t_id_10_hard_link_is_not_a_move() {
    // 같은 파일 ID 를 가진 두 경로(하드 링크)를 한 스캔에서 모두 보면 둘 다 존재한다 -> 이동이 아니다
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let a = vol.write("a.jpg", b"linked");
    let b = vol.path("b.jpg");
    fs::hard_link(&a, &b).unwrap();
    let mut s = w.begin(&vol);

    let oa = w.scan(&vol, &mut s, &ID, &a).unwrap();
    let ob = w.scan(&vol, &mut s, &ID, &b).unwrap();

    assert!(matches!(oa, Outcome::Created { .. }));
    assert!(matches!(ob, Outcome::Created { .. }), "got {ob:?}");
    assert_eq!(active_locations(&w.db.conn).len(), 2);
}

#[test]
fn t_id_13_touch_only_keeps_same_asset() {
    // Arrange: 해시까지 끝난 자산
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let p = vol.write("a.jpg", b"stable content");
    let mut s1 = w.begin(&vol);
    let id = created_id(w.scan(&vol, &mut s1, &ID, &p).unwrap());
    {
        let rec = reconciler(&w.clock, &w.cancel);
        assert_eq!(
            rec.hash_pending_asset(&mut w.db.conn, id, &p).unwrap(),
            PendingHash::Done
        );
    }
    let rev_before = asset_state(&w.db.conn, id).revision;
    // 내용은 그대로, 수정 시각만 변경
    set_mtime(&p, 1_500_000_000_000);

    // Act
    let mut s2 = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s2, &ID, &p).unwrap();

    // Assert
    assert_eq!(outcome, Outcome::Touched { asset_id: id });
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM asset_relations"), 0);
    assert_eq!(asset_state(&w.db.conn, id).revision, rev_before + 1);
    let mtime: i64 =
        w.db.conn
            .query_row(
                "SELECT modified_at FROM media_assets WHERE asset_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(mtime, 1_500_000_000_000);
}

#[test]
fn t_id_13_pending_asset_changed_becomes_a_new_asset() {
    // 전체 해시를 아직 모르는 자산은 비교 대상이 없다 -> 보수적으로 내용 변경으로 보고 새 자산을 만든다 (T-hash-edge-13)
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let p = vol.write("a.jpg", b"v1");
    let mut s1 = w.begin(&vol);
    let id = created_id(w.scan(&vol, &mut s1, &ID, &p).unwrap());
    fs::write(&p, b"version two").unwrap();

    let mut s2 = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s2, &ID, &p).unwrap();

    let Outcome::ContentChanged {
        old_asset_id,
        new_asset_id,
    } = outcome
    else {
        panic!("expected ContentChanged, got {outcome:?}");
    };
    assert_eq!(old_asset_id, id);
    let old = asset_state(&w.db.conn, id);
    assert_eq!((old.status.as_str(), old.size), ("superseded", 2));
    let new = asset_state(&w.db.conn, new_asset_id);
    assert_eq!(
        (new.status.as_str(), new.size, new.hash_state.as_str()),
        ("active", 11, "pending")
    );
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 2);
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM asset_relations"), 1);
    // 새 자산의 앞뒤 체크섬은 스캔 중에 채워진다
    assert_eq!(
        count(
            &w.db.conn,
            &format!(
                "SELECT count(*) FROM media_assets WHERE asset_id = {new_asset_id} AND edge_sig IS NOT NULL"
            )
        ),
        1
    );
}

#[test]
fn t_id_06_reconcile_discards_hash_changed_during_hashing() {
    // Arrange: 해시 완료 자산 + 내용이 바뀐 파일, 해시 계산기는 "도중 변경" 을 보고
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let p = vol.write("a.jpg", b"original");
    let mut s1 = w.begin(&vol);
    let id = created_id(w.scan(&vol, &mut s1, &ID, &p).unwrap());
    {
        let rec = reconciler(&w.clock, &w.cancel);
        rec.hash_pending_asset(&mut w.db.conn, id, &p).unwrap();
    }
    fs::write(&p, b"edited content").unwrap();
    let before = (
        asset_state(&w.db.conn, id).revision,
        outbox(&w.db.conn).len(),
    );

    // Act
    let mut s2 = w.begin(&vol);
    let rec = Reconciler {
        clock: &w.clock,
        hasher: &AlwaysChanged,
        cancel: &w.cancel,
    };
    let outcome = scan(&mut w.db.conn, &rec, &mut s2, &ID, &vol, &p).unwrap();
    let missing = rec.finish_scan(&mut w.db.conn, &s2).unwrap();

    // Assert: 결과 폐기 — 자산·outbox 무변화, 파일은 present 로 유지 (재시도는 다음 스캔)
    assert_eq!(outcome, Outcome::HashDeferred);
    assert_eq!(missing, 0);
    assert_eq!(
        before,
        (
            asset_state(&w.db.conn, id).revision,
            outbox(&w.db.conn).len()
        )
    );
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
    assert_eq!(active_locations(&w.db.conn)[0].presence, "present");
}

#[test]
fn t_id_07_reconcile_cancel_leaves_no_partial_result() {
    // Arrange: 해시가 필요한 변경 + 취소 플래그 ON
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let p = vol.write("a.jpg", b"original");
    let mut s1 = w.begin(&vol);
    let id = created_id(w.scan(&vol, &mut s1, &ID, &p).unwrap());
    {
        let rec = reconciler(&w.clock, &w.cancel);
        rec.hash_pending_asset(&mut w.db.conn, id, &p).unwrap();
    }
    fs::write(&p, b"changed!").unwrap();
    let before = (
        asset_state(&w.db.conn, id).revision,
        outbox(&w.db.conn).len(),
    );
    w.cancel.store(true, Ordering::Relaxed);

    // Act
    let mut s2 = w.begin(&vol);
    let r = w.scan(&vol, &mut s2, &ID, &p);

    // Assert
    assert!(matches!(r, Err(CoreError::Cancelled)), "{r:?}");
    assert_eq!(
        before,
        (
            asset_state(&w.db.conn, id).revision,
            outbox(&w.db.conn).len()
        )
    );
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
}

#[test]
fn t_id_05_hash_pending_asset_is_idempotent_and_guards_stale_meta() {
    // Arrange
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let p = vol.write("a.jpg", b"to be hashed");
    let mut s = w.begin(&vol);
    let id = created_id(w.scan(&vol, &mut s, &ID, &p).unwrap());
    let rec = Reconciler {
        clock: &w.clock,
        hasher: &Blake3Hasher,
        cancel: &w.cancel,
    };

    // Act / Assert: 첫 호출은 저장 (revision 2 + outbox), 두 번째는 변화 없음
    assert_eq!(
        rec.hash_pending_asset(&mut w.db.conn, id, &p).unwrap(),
        PendingHash::Done
    );
    assert_eq!(
        rec.hash_pending_asset(&mut w.db.conn, id, &p).unwrap(),
        PendingHash::Done
    );
    let st = asset_state(&w.db.conn, id);
    assert_eq!((st.hash_state.as_str(), st.revision), ("done", 2));
    assert_eq!(outbox(&w.db.conn).len(), 2);
    let stored: Vec<u8> =
        w.db.conn
            .query_row(
                "SELECT content_hash FROM media_assets WHERE asset_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(stored, blake3::hash(b"to be hashed").as_bytes().to_vec());

    // DB 메타와 다른 파일(해시 사이에 바뀜) -> 폐기, pending 유지
    let q = vol.write("b.jpg", b"second");
    let id2 = created_id(scan(&mut w.db.conn, &rec, &mut s, &ID, &vol, &q).unwrap());
    fs::write(&q, b"second file changed").unwrap();
    assert_eq!(
        rec.hash_pending_asset(&mut w.db.conn, id2, &q).unwrap(),
        PendingHash::Deferred
    );
    assert_eq!(asset_state(&w.db.conn, id2).hash_state, "pending");
}
