//! T-id-12, 14, 16: 내용 변경(새 자산), 파일 없음/오프라인, 대소문자 충돌.
//!
//! 분류: Business rule / Validation
#![cfg(windows)]

mod common;

use std::fs;

use common::*;
use mam_core::{
    CoreError, FileMeta, MediaType, Outcome, Reconciler, ScanEntry, VolumeKind,
    WindowsFileIdentity, set_volume_online,
};

const ID: WindowsFileIdentity = WindowsFileIdentity;

fn hashed_asset(w: &mut World, vol: &Vol, rel: &str, bytes: &[u8]) -> (std::path::PathBuf, i64) {
    let p = vol.write(rel, bytes);
    let mut s = w.begin(vol);
    let id = created_id(w.scan(vol, &mut s, &ID, &p).unwrap());
    let rec = reconciler(&w.clock, &w.cancel);
    rec.hash_pending_asset(&mut w.db.conn, id, &p).unwrap();
    (p, id)
}

#[test]
fn t_id_12_content_change_creates_new_asset_and_supersedes_old() {
    // Arrange
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, old) = hashed_asset(&mut w, &vol, "a.jpg", b"first version");
    let old_rev = asset_state(&w.db.conn, old).revision;
    let events_before = outbox(&w.db.conn).len();
    fs::write(&p, b"second version!").unwrap();
    let guard = Snapshot::take(vol.dir.path());

    // Act
    let mut s = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    // Assert
    let Outcome::ContentChanged {
        old_asset_id,
        new_asset_id,
    } = outcome
    else {
        panic!("expected ContentChanged, got {outcome:?}");
    };
    assert_eq!(old_asset_id, old);
    let o = asset_state(&w.db.conn, old);
    let n = asset_state(&w.db.conn, new_asset_id);
    assert_eq!(
        (o.status.as_str(), o.hash_state.as_str(), o.revision),
        ("superseded", "done", old_rev + 1)
    );
    assert_eq!(
        (n.status.as_str(), n.hash_state.as_str(), n.revision, n.size),
        ("active", "done", 1, 15)
    );
    // 관계 + 위치: 구 위치 detach, 같은 경로에 신 위치
    let rel: (i64, i64, String) =
        w.db.conn
            .query_row(
                "SELECT from_asset_id, to_asset_id, relation FROM asset_relations",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
    assert_eq!(rel, (old, new_asset_id, "content_changed".to_string()));
    let locs = active_locations(&w.db.conn);
    assert_eq!(locs.len(), 1);
    assert_eq!(
        (locs[0].asset_id, locs[0].rel_path.as_str()),
        (new_asset_id, "a.jpg")
    );
    // outbox 2건: 구자산(superseded) 갱신 + 신자산 생성, 같은 트랜잭션에서
    let events = outbox(&w.db.conn);
    assert_eq!(
        events[events_before..],
        [
            (old, old_rev + 1, "upsert".to_string()),
            (new_asset_id, 1, "upsert".to_string())
        ]
    );
    assert_eq!(guard, Snapshot::take(vol.dir.path()));
}

#[test]
fn t_id_12_same_size_different_content_with_new_mtime_is_new_asset() {
    // 크기가 같아도 수정 시각이 바뀌면 해시를 비교하고, 내용이 다르면 새 자산
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, old) = hashed_asset(&mut w, &vol, "a.jpg", b"AAAA");
    fs::write(&p, b"BBBB").unwrap();
    set_mtime(&p, 1_400_000_000_000);

    let mut s = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    assert!(
        matches!(outcome, Outcome::ContentChanged { old_asset_id, .. } if old_asset_id == old),
        "{outcome:?}"
    );
}

#[test]
fn t_id_12_atomic_replace_with_different_file_id_same_content_keeps_asset() {
    // 저장 시 임시 파일로 바꿔치기(파일 ID 변경)했지만 내용이 같으면 같은 자산
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let (p, id) = hashed_asset(&mut w, &vol, "a.jpg", b"identical bytes");
    let tmp = vol.write("a.jpg.tmp", b"identical bytes");
    fs::rename(&tmp, &p).unwrap(); // 덮어쓰기 이동

    let mut s = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s, &ID, &p).unwrap();

    assert_eq!(outcome, Outcome::Touched { asset_id: id });
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
}

#[test]
fn t_id_14_unseen_files_become_missing_only_when_volume_online() {
    // Arrange: 파일 2개
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Removable);
    let (a, id_a) = hashed_asset(&mut w, &vol, "a.jpg", b"aaa");
    let (b, id_b) = hashed_asset(&mut w, &vol, "b.jpg", b"bbb");
    let rev_b = asset_state(&w.db.conn, id_b).revision;

    // Act 1: 볼륨 online, 두 번째 스캔에서 a 만 본다 (b 는 외부에서 삭제됨)
    fs::remove_file(&b).unwrap();
    let mut s = w.begin(&vol);
    w.scan(&vol, &mut s, &ID, &a).unwrap();
    let marked = w.finish(&s);

    // Assert 1: b 만 missing — 자산은 active 로 유지(검색 정보 보존), revision/outbox 로 알린다
    assert_eq!(marked, 1);
    let locs = active_locations(&w.db.conn);
    assert_eq!(
        locs.iter().find(|l| l.asset_id == id_b).unwrap().presence,
        "missing"
    );
    assert_eq!(
        locs.iter().find(|l| l.asset_id == id_a).unwrap().presence,
        "present"
    );
    assert_eq!(asset_state(&w.db.conn, id_b).status, "active");
    assert_eq!(asset_state(&w.db.conn, id_b).revision, rev_b + 1);
    // 멱등: 다시 끝내도 추가 표시 없음
    let s_again = w.begin(&vol);
    w.finish(&s_again); // a 는 이번에 못 봤으므로 missing 이 된다 — 아래에서 별도 검증
    assert_eq!(
        count(
            &w.db.conn,
            "SELECT count(*) FROM file_locations WHERE presence = 'missing'"
        ),
        2
    );

    // Act 2: 볼륨 offline 이면 못 봐도 missing 으로 만들지 않는다
    let (c, id_c) = hashed_asset(&mut w, &vol, "c.jpg", b"ccc");
    set_volume_online(&w.db.conn, &w.clock, vol.id, false).unwrap();
    let s_off = w.begin(&vol);
    let marked_off = w.finish(&s_off);

    // Assert 2
    assert_eq!(marked_off, 0);
    let loc_c = active_locations(&w.db.conn)
        .into_iter()
        .find(|l| l.asset_id == id_c)
        .unwrap();
    assert_eq!(loc_c.presence, "present");
    assert!(c.exists());

    // Act 3: 다시 online 이 되고 b 가 돌아오면 present 로 복귀
    set_volume_online(&w.db.conn, &w.clock, vol.id, true).unwrap();
    let b2 = vol.write("b.jpg", b"bbb");
    let mut s_on = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s_on, &ID, &b2).unwrap();
    assert!(
        matches!(outcome, Outcome::Reappeared { asset_id } | Outcome::Touched { asset_id } if asset_id == id_b),
        "{outcome:?}"
    );
    let loc_b = active_locations(&w.db.conn)
        .into_iter()
        .find(|l| l.asset_id == id_b)
        .unwrap();
    assert_eq!(loc_b.presence, "present");
}

#[test]
fn t_id_14_set_online_unknown_volume_is_error() {
    let w = World::new();
    assert!(matches!(
        set_volume_online(&w.db.conn, &w.clock, 999, false),
        Err(CoreError::VolumeNotFound(999))
    ));
    assert!(matches!(
        mam_core::ScanSession::begin(&w.db.conn, &w.clock, 999),
        Err(CoreError::VolumeNotFound(999))
    ));
}

#[test]
fn t_id_16_case_only_collision_is_scan_error_and_changes_nothing() {
    // Arrange: 대소문자만 다른 두 경로 (NTFS 기본 설정에서는 공존할 수 없으므로 스캔 입력을 직접 구성)
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let mut s = w.begin(&vol);
    let meta = |n: u64| FileMeta {
        size: n,
        modified_ms: 1_000,
        file_identity: Some(format!("id-{n}")),
    };
    let rec = Reconciler {
        clock: &w.clock,
        hasher: &mam_core::Blake3Hasher,
        cancel: &w.cancel,
    };
    let first = rec
        .reconcile(
            &mut w.db.conn,
            &mut s,
            &ScanEntry {
                rel_path: r"Photos\IMG.jpg",
                abs_path: std::path::Path::new("unused"),
                media_type: MediaType::Image,
                meta: meta(5),
            },
        )
        .unwrap();
    let id = created_id(first);
    let before = (
        asset_state(&w.db.conn, id).revision,
        outbox(&w.db.conn).len(),
        count(&w.db.conn, "SELECT count(*) FROM file_locations"),
    );

    // Act
    let second = rec.reconcile(
        &mut w.db.conn,
        &mut s,
        &ScanEntry {
            rel_path: r"photos\img.JPG",
            abs_path: std::path::Path::new("unused"),
            media_type: MediaType::Image,
            meta: meta(9),
        },
    );

    // Assert: 타입화된 오류, 기존 데이터 무변경
    assert!(
        matches!(&second, Err(CoreError::CaseCollision { rel_path, existing_rel_path, .. })
            if rel_path == r"photos\img.JPG" && existing_rel_path == r"Photos\IMG.jpg"),
        "{second:?}"
    );
    let after = (
        asset_state(&w.db.conn, id).revision,
        outbox(&w.db.conn).len(),
        count(&w.db.conn, "SELECT count(*) FROM file_locations"),
    );
    assert_eq!(before, after);
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
    assert_eq!(active_locations(&w.db.conn)[0].rel_path, r"Photos\IMG.jpg");
}

#[test]
fn t_id_16_case_only_rename_between_scans_is_same_asset() {
    // 충돌이 아닌 경우: 같은 파일의 대소문자만 바꾼 이름 변경 -> 같은 자산, 표기만 갱신
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let p = vol.write("photo.jpg", b"case rename");
    let mut s1 = w.begin(&vol);
    let id = created_id(w.scan(&vol, &mut s1, &ID, &p).unwrap());
    let renamed = vol.path("PHOTO.JPG");
    fs::rename(&p, &renamed).unwrap();

    let mut s2 = w.begin(&vol);
    let outcome = w.scan(&vol, &mut s2, &ID, &renamed).unwrap();

    assert_eq!(outcome, Outcome::Unchanged { asset_id: id });
    assert_eq!(active_locations(&w.db.conn)[0].rel_path, "PHOTO.JPG");
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
}

#[test]
fn t_id_16_invalid_relative_path_is_validation_error() {
    let mut w = World::new();
    let vol = w.volume("vol-a", VolumeKind::Fixed);
    let mut s = w.begin(&vol);
    let rec = reconciler(&w.clock, &w.cancel);
    let r = rec.reconcile(
        &mut w.db.conn,
        &mut s,
        &ScanEntry {
            rel_path: "",
            abs_path: std::path::Path::new("unused"),
            media_type: MediaType::Image,
            meta: FileMeta {
                size: 1,
                modified_ms: 1,
                file_identity: None,
            },
        },
    );
    assert!(matches!(r, Err(CoreError::PathKey(_))), "{r:?}");
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 0);
}
