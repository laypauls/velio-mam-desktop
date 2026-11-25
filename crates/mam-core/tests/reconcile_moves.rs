//! T-id-11, 15: 볼륨 간 이동(해시 재연결), 오프라인 -> 온라인(드라이브 문자 변경), 파일 ID 가 없는 볼륨.
//!
//! 분류: Business rule / External(가짜 provider)
//! USB 탈착·NAS·드라이브 문자 변경은 실기 장비가 없어 가짜 `VolumeProvider` 와 임시 디렉터리로 흉내낸다.
#![cfg(windows)]

mod common;

use std::fs;

use common::*;
use mam_core::{
    MetadataOnly, MoveBy, Outcome, VolumeInfo, VolumeKind, WindowsFileIdentity, register_volume,
    sync_volumes,
};

const ID: WindowsFileIdentity = WindowsFileIdentity;

/// A 볼륨에서 파일을 스캔·해시한 뒤 지워 `missing` 으로 만든다. 자산 id 반환.
fn scanned_then_missing(w: &mut World, a: &Vol, name: &str, bytes: &[u8]) -> i64 {
    let p = a.write(name, bytes);
    let mut s = w.begin(a);
    let id = created_id(w.scan(a, &mut s, &ID, &p).unwrap());
    let rec = reconciler(&w.clock, &w.cancel);
    rec.hash_pending_asset(&mut w.db.conn, id, &p).unwrap();
    fs::remove_file(&p).unwrap(); // 외장 디스크에서 파일을 옮겨 간 상황
    let s2 = w.begin(a);
    assert_eq!(w.finish(&s2), 1);
    id
}

#[test]
fn t_id_11_cross_volume_move_relinks_when_hash_matches() {
    // Arrange
    let mut w = World::new();
    let a = w.volume("vol-a", VolumeKind::Removable);
    let b = w.volume("vol-b", VolumeKind::Fixed);
    let id = scanned_then_missing(&mut w, &a, "clip.mp4", b"same bytes on both volumes");
    let copied = b.write("archive/clip.mp4", b"same bytes on both volumes");
    let guard = Snapshot::take(b.dir.path());

    // Act
    let mut s = w.begin(&b);
    let outcome = w.scan(&b, &mut s, &ID, &copied).unwrap();

    // Assert: 같은 자산이 B 볼륨의 새 경로에 연결되고 A 의 옛 위치는 detach
    assert_eq!(
        outcome,
        Outcome::Moved {
            asset_id: id,
            by: MoveBy::Hash
        }
    );
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 1);
    let locs = active_locations(&w.db.conn);
    assert_eq!(locs.len(), 1);
    assert_eq!(
        (
            locs[0].asset_id,
            locs[0].volume_id,
            locs[0].presence.as_str()
        ),
        (id, b.id, "present")
    );
    assert_eq!(locs[0].rel_path, r"archive\clip.mp4");
    assert_eq!(
        count(
            &w.db.conn,
            "SELECT count(*) FROM file_locations WHERE detached_at IS NOT NULL"
        ),
        1
    );
    assert_eq!(guard, Snapshot::take(b.dir.path()));
    // 후보는 한 번만 쓰인다: 같은 내용의 두 번째 복사본은 새 자산
    let second = b.write("archive/clip-copy.mp4", b"same bytes on both volumes");
    assert!(matches!(
        w.scan(&b, &mut s, &ID, &second).unwrap(),
        Outcome::Created { .. }
    ));
}

#[test]
fn t_id_11_cross_volume_same_size_different_content_is_new_asset() {
    // Arrange: 크기는 같지만 내용이 다른 파일 (크기는 후보 축소용일 뿐이다)
    let mut w = World::new();
    let a = w.volume("vol-a", VolumeKind::Removable);
    let b = w.volume("vol-b", VolumeKind::Fixed);
    let old = scanned_then_missing(&mut w, &a, "x.jpg", b"AAAAAAAAAA");
    let other = b.write("x.jpg", b"BBBBBBBBBB");

    // Act
    let mut s = w.begin(&b);
    let outcome = w.scan(&b, &mut s, &ID, &other).unwrap();

    // Assert: 신규 자산, 기존 자산은 missing 유지
    let new_id = created_id(outcome);
    assert_ne!(new_id, old);
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 2);
    let missing: i64 = count(
        &w.db.conn,
        "SELECT count(*) FROM file_locations WHERE presence = 'missing' AND detached_at IS NULL",
    );
    assert_eq!(missing, 1);
    assert_eq!(asset_state(&w.db.conn, old).status, "active");
    // 해시는 pending 이 아니라 비교 때문에 이미 계산돼 done 으로 저장된다
    assert_eq!(asset_state(&w.db.conn, new_id).hash_state, "done");
}

#[test]
fn t_id_11_volume_without_file_id_relinks_by_hash_only() {
    // 파일 ID 를 지원하지 않는 볼륨(FAT 등)과 같은 동작: MetadataOnly
    let mut w = World::new();
    let v = w.volume("vol-fat", VolumeKind::Removable);
    let old_path = v.write("a/one.jpg", b"fat content");
    let mut s1 = w.begin(&v);
    let id = created_id(w.scan(&v, &mut s1, &MetadataOnly, &old_path).unwrap());
    {
        let rec = reconciler(&w.clock, &w.cancel);
        rec.hash_pending_asset(&mut w.db.conn, id, &old_path)
            .unwrap();
    }
    assert_eq!(
        count(
            &w.db.conn,
            "SELECT count(*) FROM file_locations WHERE file_identity IS NULL"
        ),
        1
    );
    // 이름 변경 -> 이번 스캔이 끝나기 전에는 옛 위치가 아직 present 라 재연결 후보가 없다
    let new_path = v.path("b/two.jpg");
    fs::create_dir_all(v.path("b")).unwrap();
    fs::rename(&old_path, &new_path).unwrap();
    let mut s2 = w.begin(&v);
    let first = w.scan(&v, &mut s2, &MetadataOnly, &new_path).unwrap();
    assert!(matches!(first, Outcome::Created { .. }), "{first:?}");
    assert_eq!(w.finish(&s2), 1); // 옛 위치는 missing
    // 이미 missing 이 된 뒤에 같은 내용이 새 이름으로 나타나면 해시로 재연결된다
    let third = v.write("c/three.jpg", b"fat content");
    let mut s3 = w.begin(&v);
    assert_eq!(
        w.scan(&v, &mut s3, &MetadataOnly, &third).unwrap(),
        Outcome::Moved {
            asset_id: id,
            by: MoveBy::Hash
        }
    );
}

#[test]
fn t_id_11_network_volume_ignores_file_id() {
    // NAS(SMB) 파일 ID 는 신뢰하지 않는다 -> 저장하지도, 이동 판정에 쓰지도 않는다
    let mut w = World::new();
    let nas = w.volume("//nas/share", VolumeKind::Network);
    let p = nas.write("a.jpg", b"nas content");
    let mut s = w.begin(&nas);
    let id = created_id(w.scan(&nas, &mut s, &ID, &p).unwrap());
    assert_eq!(
        count(
            &w.db.conn,
            "SELECT count(*) FROM file_locations WHERE file_identity IS NOT NULL"
        ),
        0
    );

    let q = nas.path("renamed.jpg");
    fs::rename(&p, &q).unwrap();
    let mut s2 = w.begin(&nas);
    let o = w.scan(&nas, &mut s2, &ID, &q).unwrap();
    assert!(
        matches!(o, Outcome::Created { asset_id } if asset_id != id),
        "{o:?}"
    );
}

#[test]
fn t_id_15_offline_then_online_with_new_mount_keeps_everything() {
    // Arrange: 이동식 볼륨의 파일 2개를 스캔·해시
    let mut w = World::new();
    let usb = w.volume("vol-usb", VolumeKind::Removable);
    let mut ids = Vec::new();
    let mut s = w.begin(&usb);
    for n in ["a.jpg", "b.jpg"] {
        let p = usb.write(n, n.as_bytes());
        let id = created_id(w.scan(&usb, &mut s, &ID, &p).unwrap());
        let rec = reconciler(&w.clock, &w.cancel);
        rec.hash_pending_asset(&mut w.db.conn, id, &p).unwrap();
        ids.push(id);
    }
    assert_eq!(w.finish(&s), 0);
    let revs = |w: &World| -> Vec<i64> {
        ids.iter()
            .map(|i| asset_state(&w.db.conn, *i).revision)
            .collect()
    };
    let rev_before = revs(&w);
    let outbox_before = outbox(&w.db.conn).len();

    // Act 1: 탈착 -> 열거에 없음
    w.clock.advance(5_000);
    let report = sync_volumes(&w.db.conn, &w.clock, &FakeVolumes(vec![])).unwrap();
    // Assert 1: volumes 한 행만 offline, 위치·자산·outbox 는 그대로
    assert_eq!(report.went_offline, vec![usb.id]);
    let avail: String =
        w.db.conn
            .query_row(
                "SELECT availability FROM volumes WHERE volume_id = ?1",
                [usb.id],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(avail, "offline");
    assert_eq!(
        active_locations(&w.db.conn)
            .iter()
            .filter(|l| l.presence == "present")
            .count(),
        2
    );
    assert_eq!(revs(&w), rev_before);
    assert_eq!(outbox(&w.db.conn).len(), outbox_before);
    // 다시 호출해도 전환 없음 (멱등)
    assert_eq!(
        sync_volumes(&w.db.conn, &w.clock, &FakeVolumes(vec![])).unwrap(),
        Default::default()
    );

    // Act 2: 다른 드라이브 문자(마운트 지점)로 재연결
    w.clock.advance(5_000);
    let new_mount = format!(r"\\?\{}", usb.info.mount_point); // 같은 폴더를 다른 표기(=문자)로 마운트
    let reattached = VolumeInfo {
        mount_point: new_mount.clone(),
        label: Some("USB".into()),
        ..usb.info.clone()
    };
    let report =
        sync_volumes(&w.db.conn, &w.clock, &FakeVolumes(vec![reattached.clone()])).unwrap();
    assert_eq!(report.went_online, vec![usb.id]);
    let (mount, avail): (String, String) =
        w.db.conn
            .query_row(
                "SELECT last_mount_point, availability FROM volumes WHERE volume_id = ?1",
                [usb.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
    assert_eq!((mount, avail.as_str()), (new_mount, "online"));
    // 같은 키로 다시 등록해도 같은 volume_id
    assert_eq!(
        register_volume(&w.db.conn, &w.clock, &reattached).unwrap(),
        usb.id
    );
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM volumes"), 1);

    // Assert 2: 새 마운트 지점 기준으로 다시 스캔해도 같은 자산·위치 (신규 자산 없음)
    let moved = Vol {
        dir: tempfile::TempDir::new().unwrap(),
        info: reattached,
        id: usb.id,
    };
    let mut s2 = w.begin(&moved);
    for (n, id) in ["a.jpg", "b.jpg"].iter().zip(&ids) {
        let o = w.scan(&moved, &mut s2, &ID, &usb.path(n)).unwrap();
        assert_eq!(o, Outcome::Unchanged { asset_id: *id });
    }
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 2);
    assert_eq!(revs(&w), rev_before);
}

#[test]
fn t_id_15_sync_does_not_touch_network_volumes_or_register_new_ones() {
    let w = World::new();
    let nas = w.volume("//nas/share", VolumeKind::Network);
    let stranger = VolumeInfo {
        key: "vol-unknown".into(),
        kind: VolumeKind::Fixed,
        label: None,
        mount_point: "Z:\\".into(),
    };

    let report = sync_volumes(&w.db.conn, &w.clock, &FakeVolumes(vec![stranger])).unwrap();

    // `present`(연결된 볼륨 목록, 새 디스크 감지용)는 상태 변화가 아니다 - 전환이 없다는 것만 본다
    assert!(
        report.went_online.is_empty() && report.went_offline.is_empty(),
        "{report:?}"
    );
    let avail: String =
        w.db.conn
            .query_row(
                "SELECT availability FROM volumes WHERE volume_id = ?1",
                [nas.id],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(avail, "online");
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM volumes"), 1);
}
