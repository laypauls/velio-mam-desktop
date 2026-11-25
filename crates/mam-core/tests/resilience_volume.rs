//! P4 복원력 — 볼륨 탈착·재연결·드라이브 문자 변경 (T-res-08 ~ 10) + 사용자 태그 편집의 outbox 우선순위(T-res-12c).
//! 분류: Business rule / External(가짜 `VolumeProvider`). 실제 SQLite(마이그레이션) + 임시 디렉터리 볼륨.
//! 실기 USB·NAS 의 지연·부분 연결은 재현하지 못한다. 원본 보호: 시나리오 끝에 픽스처 원본의 내용·수정 시각 불변 단언.
#![cfg(windows)]

mod common;

use std::fs;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::env::{Env, snapshot};
use common::*;
use mam_core::scan::{Availability, IngestError, ScanOutcome, scan_folder};
use mam_core::userdata::{UpdateTagsInput, UserData};
use mam_core::{
    MoveBy, Outcome, VolumeInfo, VolumeKind, WindowsFileIdentity, set_volume_online, sync_volumes,
};
use rusqlite::Connection;

const ID: WindowsFileIdentity = WindowsFileIdentity;

// ---------------------------------------------------------------------------------------------
// 공용 도우미
// ---------------------------------------------------------------------------------------------

/// 질의 결과를 행 단위 문자열로 (값 비교용 스냅샷).
fn dump(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    let n = stmt.column_count();
    stmt.query_map([], |r| {
        let cols: Vec<String> = (0..n)
            .map(|i| format!("{:?}", r.get::<_, rusqlite::types::Value>(i).unwrap()))
            .collect();
        Ok(cols.join("|"))
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

/// 자산·위치·태그·파생물·outbox 전체 상태 (오프라인이 이 중 아무것도 바꾸지 않아야 한다).
fn db_state(conn: &Connection) -> Vec<Vec<String>> {
    [
        "SELECT asset_id, status, revision, hash_state, file_size FROM media_assets ORDER BY asset_id",
        "SELECT location_id, asset_id, volume_id, rel_path, presence, detached_at, last_seen_at
           FROM file_locations ORDER BY location_id",
        "SELECT asset_id, tag_id FROM asset_tags ORDER BY asset_id, tag_id",
        "SELECT asset_id, kind, state FROM asset_derivatives ORDER BY asset_id, kind",
        "SELECT event_id, asset_id, revision, event_type, priority, status FROM search_outbox ORDER BY event_id",
    ]
    .iter()
    .map(|sql| dump(conn, sql))
    .collect()
}

fn scan(env: &Env, folder: i64) -> ScanOutcome {
    scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false)).unwrap()
}

fn clear_jobs(env: &Env) {
    env.pool
        .write(mam_db::Priority::Low, |c| {
            Ok(c.execute("DELETE FROM jobs", [])?)
        })
        .unwrap();
}

/// 파일 3개를 스캔하고, `a.jpg` 에 태그를, 모든 자산에 썸네일 완료 상태를 만든 환경.
struct Scanned {
    env: Env,
    folder: i64,
    ids: Vec<i64>,
}

fn scanned() -> Scanned {
    let env = Env::new();
    for (i, n) in ["a.jpg", "b.jpg", "c.jpg"].iter().enumerate() {
        env.write(&format!("lib/{n}"), i as u8 + 1);
    }
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    assert!(matches!(scan(&env, folder), ScanOutcome::Completed { .. }));
    clear_jobs(&env);
    let ids: Vec<i64> = env.read(|c| {
        let mut s = c
            .prepare("SELECT asset_id FROM media_assets ORDER BY asset_id")
            .unwrap();
        s.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());
    ud.update_asset_tags(UpdateTagsInput {
        asset_ids: vec![ids[0]],
        add: vec!["휴가".into()],
        remove: vec![],
    })
    .unwrap();
    env.pool
        .write(mam_db::Priority::Low, {
            let ids = ids.clone();
            move |c| {
                for id in &ids {
                    c.execute(
                        "INSERT INTO asset_derivatives (asset_id, kind, state, updated_at)
                         VALUES (?1, 'thumb', 'done', 0)",
                        [id],
                    )?;
                }
                Ok(())
            }
        })
        .unwrap();
    Scanned { env, folder, ids }
}

fn presence(env: &Env, asset_id: i64) -> String {
    env.read(|c| {
        c.query_row(
            "SELECT presence FROM file_locations WHERE asset_id = ?1 AND detached_at IS NULL",
            [asset_id],
            |r| r.get(0),
        )
        .unwrap()
    })
}

fn availability(env: &Env) -> String {
    env.read(|c| {
        c.query_row("SELECT availability FROM volumes LIMIT 1", [], |r| r.get(0))
            .unwrap()
    })
}

// ---------------------------------------------------------------------------------------------
// T-res-08 / T-res-10 : 오프라인 전환과 오프라인 중 재스캔 (ingest 계층, 가짜 VolumeProvider)
// ---------------------------------------------------------------------------------------------

#[test]
fn t_res_08_volume_going_offline_changes_only_availability_and_keeps_tags_and_derivatives() {
    // Arrange
    let s = scanned();
    let guard = snapshot(&s.env.vol);
    let before = s.env.read(db_state);

    // Act: USB 탈착 (가짜 provider 열거에서 사라짐) -> 점검
    s.env.volumes.set_connected("vol-a", false);
    let report = s.env.ingest.supervise_once().unwrap();

    // Assert: 볼륨 한 행만 offline, 자산·위치·태그·썸네일 상태·outbox 는 그대로 (삭제도 missing 도 아님)
    assert_eq!(report.went_offline.len(), 1);
    assert_eq!(availability(&s.env), "offline");
    let after = s.env.read(db_state);
    assert_eq!(after, before);
    let view = s.env.ingest.list_folders().unwrap().remove(0);
    assert_eq!(view.availability, Availability::Offline);
    assert_eq!(view.asset_count, 3, "offline assets keep counting");
    for id in &s.ids {
        assert_eq!(presence(&s.env, *id), "present");
    }
    assert_eq!(
        s.env
            .count("SELECT count(*) FROM asset_derivatives WHERE state = 'done'"),
        3
    );
    assert_eq!(s.env.count("SELECT count(*) FROM asset_tags"), 1);
    // 다시 점검해도 변화가 없다 (멱등)
    let again = s.env.ingest.supervise_once().unwrap();
    assert!(again.went_offline.is_empty() && again.scheduled.is_empty());
    assert_eq!(s.env.read(db_state), before);
    assert_eq!(guard, snapshot(&s.env.vol));
    s.env.ingest.shutdown();
}

#[test]
fn t_res_10_rescan_while_offline_never_marks_files_missing_and_reconnect_catches_up() {
    // Arrange: 오프라인 동안 디스크 쪽에서는 b.jpg 가 사라졌다
    let s = scanned();
    let gone_asset = s.ids[1];
    s.env.volumes.set_connected("vol-a", false);
    s.env.ingest.supervise_once().unwrap();
    fs::remove_file(s.env.path("lib/b.jpg")).unwrap();
    let guard = snapshot(&s.env.vol);
    let frozen = s.env.read(db_state);
    s.env.clock.advance(10_000);

    // Act 1: 오프라인 상태 재스캔 (직접 실행 + 수동 재스캔 요청)
    let outcome = scan(&s.env, s.folder);
    let manual = s.env.ingest.rescan_folder(s.folder);

    // Assert 1
    assert_eq!(outcome, ScanOutcome::Offline);
    assert!(matches!(manual, Err(IngestError::Offline)));
    assert_eq!(
        s.env.read(db_state),
        frozen,
        "offline rescan changes nothing"
    );
    assert_eq!(presence(&s.env, gone_asset), "present");

    // Act 2: 재연결 -> 점검이 재스캔을 예약 -> 스캔이 놓친 변경을 따라잡는다
    s.env.volumes.set_connected("vol-a", true);
    let up = s.env.ingest.supervise_once().unwrap();
    s.env.clock.advance(10_000);
    let done = scan(&s.env, s.folder);

    // Assert 2: 이제서야 b.jpg 만 missing (자산은 active 유지), 나머지 정상
    assert_eq!(
        (up.went_online.len(), up.scheduled.clone()),
        (1, vec![s.folder])
    );
    let ScanOutcome::Completed { counts } = done else {
        panic!("expected Completed, got {done:?}");
    };
    assert_eq!(counts.missing, 1);
    assert_eq!(presence(&s.env, gone_asset), "missing");
    assert_eq!(presence(&s.env, s.ids[0]), "present");
    assert_eq!(presence(&s.env, s.ids[2]), "present");
    assert_eq!(
        s.env.count(&format!(
            "SELECT count(*) FROM media_assets WHERE asset_id = {gone_asset} AND status = 'active'"
        )),
        1
    );
    assert_eq!(
        s.env.count("SELECT count(*) FROM asset_tags"),
        1,
        "tag kept"
    );
    assert_eq!(guard, snapshot(&s.env.vol));
    s.env.ingest.shutdown();
}

#[test]
fn t_res_10b_volume_flipping_offline_mid_scan_blocks_missing_marking_at_every_entry_point() {
    // Arrange: 파일 3개를 스캔한 뒤, 새 스캔 세션은 online 으로 시작했지만 끝나기 전에 볼륨이 offline 이 된다
    let mut w = World::new();
    let vol = w.volume("usb", VolumeKind::Removable);
    let mut ids = Vec::new();
    let mut s0 = w.begin(&vol);
    for n in ["a.jpg", "b.jpg", "c.jpg"] {
        let p = vol.write(n, n.as_bytes());
        ids.push(created_id(w.scan(&vol, &mut s0, &ID, &p).unwrap()));
    }
    assert_eq!(w.finish(&s0), 0);
    let guard = Snapshot::take(vol.dir.path());
    let frozen = (
        dump(
            &w.db.conn,
            "SELECT asset_id, revision, status FROM media_assets ORDER BY asset_id",
        ),
        dump(
            &w.db.conn,
            "SELECT location_id, presence FROM file_locations ORDER BY location_id",
        ),
        outbox(&w.db.conn),
    );
    let online_session = w.begin(&vol); // 아무 파일도 보지 못한 스캔 (읽기 도중 연결 끊김)
    set_volume_online(&w.db.conn, &w.clock, vol.id, false).unwrap();
    let offline_session = w.begin(&vol); // 처음부터 offline 이던 스캔

    // Act
    let rec = reconciler(&w.clock, &w.cancel);
    let full_online_session = rec.finish_scan(&mut w.db.conn, &online_session).unwrap();
    let full_offline_session = rec.finish_scan(&mut w.db.conn, &offline_session).unwrap();
    let folder_scoped = rec
        .finish_scan_in(&mut w.db.conn, &online_session, "")
        .unwrap();
    let watch_removal = rec
        .mark_missing_under(&mut w.db.conn, vol.id, "a.jpg")
        .unwrap();

    // Assert: 어느 경로로도 missing 이 만들어지지 않고 revision·outbox 도 그대로
    assert_eq!(
        (
            full_online_session,
            full_offline_session,
            folder_scoped,
            watch_removal
        ),
        (0, 0, 0, 0)
    );
    let after = (
        dump(
            &w.db.conn,
            "SELECT asset_id, revision, status FROM media_assets ORDER BY asset_id",
        ),
        dump(
            &w.db.conn,
            "SELECT location_id, presence FROM file_locations ORDER BY location_id",
        ),
        outbox(&w.db.conn),
    );
    assert_eq!(after, frozen);
    assert!(
        ids.iter()
            .all(|i| asset_state(&w.db.conn, *i).status == "active")
    );
    assert_eq!(guard, Snapshot::take(vol.dir.path()));

    // 재연결 후 같은 세션 규칙으로 실제 부재가 확인되면 그때서야 missing
    set_volume_online(&w.db.conn, &w.clock, vol.id, true).unwrap();
    fs::remove_file(vol.path("c.jpg")).unwrap();
    let mut s1 = w.begin(&vol);
    for n in ["a.jpg", "b.jpg"] {
        w.scan(&vol, &mut s1, &ID, &vol.path(n)).unwrap();
    }
    assert_eq!(w.finish(&s1), 1);
    assert_eq!(
        active_locations(&w.db.conn)
            .iter()
            .filter(|l| l.presence == "missing")
            .map(|l| l.asset_id)
            .collect::<Vec<_>>(),
        vec![ids[2]]
    );
}

// ---------------------------------------------------------------------------------------------
// T-res-09 : 다른 드라이브 문자(마운트 지점)로 재연결
// ---------------------------------------------------------------------------------------------

#[test]
fn t_res_09_reconnect_under_a_new_mount_relinks_same_assets_and_records_one_event_per_change() {
    // Arrange: 이동식 볼륨의 파일 2개를 스캔·해시. 탈착.
    let mut w = World::new();
    let usb = w.volume("vol-usb", VolumeKind::Removable);
    let mut ids = Vec::new();
    let mut s = w.begin(&usb);
    for (n, bytes) in [("photos/a.jpg", b"alpha"), ("photos/b.jpg", b"bravo")] {
        let p = usb.write(n, bytes);
        let id = created_id(w.scan(&usb, &mut s, &ID, &p).unwrap());
        reconciler(&w.clock, &w.cancel)
            .hash_pending_asset(&mut w.db.conn, id, &p)
            .unwrap();
        ids.push(id);
    }
    assert_eq!(w.finish(&s), 0);
    w.db.conn
        .execute(
            "INSERT INTO tags (name, name_key) VALUES ('휴가', '휴가')",
            [],
        )
        .unwrap();
    w.db.conn
        .execute(
            "INSERT INTO asset_tags (asset_id, tag_id, created_at) VALUES (?1, 1, 0)",
            [ids[0]],
        )
        .unwrap();
    let rev_b = asset_state(&w.db.conn, ids[1]).revision;
    let rev_a = asset_state(&w.db.conn, ids[0]).revision;
    let events_before = outbox(&w.db.conn).len();
    w.clock.advance(5_000);
    let went = sync_volumes(&w.db.conn, &w.clock, &FakeVolumes(vec![])).unwrap();
    assert_eq!(went.went_offline, vec![usb.id]);

    // 오프라인 동안: 같은 디스크가 다른 드라이브 문자로 마운트되고(디렉터리째 이동), a.jpg 는 다른 폴더로 옮겨졌다
    let holder = tempfile::TempDir::new().unwrap();
    let new_root = holder.path().join("drive_e");
    fs::rename(usb.dir.path(), &new_root).unwrap();
    fs::create_dir_all(new_root.join("archive")).unwrap();
    fs::rename(
        new_root.join("photos").join("a.jpg"),
        new_root.join("archive").join("a_renamed.jpg"),
    )
    .unwrap();
    let guard = Snapshot::take(&new_root);
    let reattached = VolumeInfo {
        mount_point: new_root.to_string_lossy().into_owned(),
        label: Some("USB (E:)".into()),
        ..usb.info.clone()
    };

    // Act 1: 재연결 동기화 -> 새 마운트로 스캔
    w.clock.advance(5_000);
    let up = sync_volumes(&w.db.conn, &w.clock, &FakeVolumes(vec![reattached.clone()])).unwrap();
    let moved_vol = Vol {
        dir: tempfile::TempDir::new().unwrap(),
        info: reattached.clone(),
        id: usb.id,
    };
    let mut s2 = w.begin(&moved_vol);
    let a = w
        .scan(
            &moved_vol,
            &mut s2,
            &ID,
            &new_root.join("archive").join("a_renamed.jpg"),
        )
        .unwrap();
    let b = w
        .scan(
            &moved_vol,
            &mut s2,
            &ID,
            &new_root.join("photos").join("b.jpg"),
        )
        .unwrap();
    let missing = w.finish(&s2);

    // Assert 1: 같은 asset_id, 새 경로·새 마운트, 새 자산 없음
    assert_eq!(up.went_online, vec![usb.id]);
    assert_eq!(
        a,
        Outcome::Moved {
            asset_id: ids[0],
            by: MoveBy::FileId
        }
    );
    assert_eq!(b, Outcome::Unchanged { asset_id: ids[1] });
    assert_eq!(missing, 0);
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM media_assets"), 2);
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM volumes"), 1);
    let mount: String =
        w.db.conn
            .query_row(
                "SELECT last_mount_point FROM volumes WHERE volume_id = ?1",
                [usb.id],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(mount, reattached.mount_point);
    let locs = active_locations(&w.db.conn);
    let loc = |id: i64| locs.iter().find(|l| l.asset_id == id).unwrap();
    assert_eq!(
        (
            loc(ids[0]).volume_id,
            loc(ids[0]).rel_path.as_str(),
            loc(ids[0]).presence.as_str()
        ),
        (usb.id, r"archive\a_renamed.jpg", "present")
    );
    assert_eq!(loc(ids[1]).rel_path, r"photos\b.jpg");
    // revision·outbox: 바뀐 자산(a)만 +1 과 이벤트 1건, 그대로인 자산(b)은 변화 없음
    assert_eq!(asset_state(&w.db.conn, ids[0]).revision, rev_a + 1);
    assert_eq!(asset_state(&w.db.conn, ids[1]).revision, rev_b);
    assert_eq!(
        outbox(&w.db.conn)[events_before..],
        [(ids[0], rev_a + 1, "upsert".to_string())]
    );
    assert_eq!(
        count(&w.db.conn, "SELECT count(*) FROM asset_tags"),
        1,
        "tag follows the asset"
    );
    assert_eq!(guard, Snapshot::take(&new_root));
}

#[test]
fn t_res_09b_unplugged_volume_with_no_known_mount_stays_offline_and_unregistered_volumes_are_ignored()
 {
    // Arrange: 등록된 볼륨 1개(offline) 와, 등록된 적 없는 다른 디스크가 연결됨
    let w = World::new();
    let usb = w.volume("vol-usb", VolumeKind::Removable);
    sync_volumes(&w.db.conn, &w.clock, &FakeVolumes(vec![])).unwrap();
    let stranger = VolumeInfo {
        key: "vol-stranger".into(),
        kind: VolumeKind::Removable,
        label: None,
        mount_point: "Z:\\".into(),
    };

    // Act
    let report = sync_volumes(&w.db.conn, &w.clock, &FakeVolumes(vec![stranger])).unwrap();

    // Assert: 다른 디스크를 같은 볼륨으로 오인해 재연결하지 않는다
    // `present`(연결된 볼륨 목록, 새 디스크 감지용)는 상태 변화가 아니다 - 전환이 없다는 것만 본다
    assert!(report.went_online.is_empty() && report.went_offline.is_empty(), "{report:?}");
    assert_eq!(count(&w.db.conn, "SELECT count(*) FROM volumes"), 1);
    let avail: String =
        w.db.conn
            .query_row(
                "SELECT availability FROM volumes WHERE volume_id = ?1",
                [usb.id],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(avail, "offline");
}

// ---------------------------------------------------------------------------------------------
// T-res-12c : 실제 태그 편집 경로의 outbox 우선순위
// ---------------------------------------------------------------------------------------------

#[test]
fn t_res_12c_real_tag_edit_outranks_older_scan_events_in_the_pending_queue() {
    // Arrange: 스캔이 만든 저우선 이벤트가 이미 pending (낮은 event_id)
    let env = Env::new();
    for i in 0..4u8 {
        env.write(&format!("lib/f{i}.jpg"), i + 1);
    }
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    assert!(matches!(scan(&env, folder), ScanOutcome::Completed { .. }));
    let target: i64 = env.count("SELECT asset_id FROM media_assets ORDER BY asset_id DESC LIMIT 1");
    let rev_before = env.count(&format!(
        "SELECT revision FROM media_assets WHERE asset_id = {target}"
    ));
    let guard = snapshot(&env.vol);
    let ud = UserData::new(Arc::clone(&env.pool), env.clock.clone());

    // Act: 사용자가 마지막 자산에 태그를 단다
    ud.update_asset_tags(UpdateTagsInput {
        asset_ids: vec![target],
        add: vec!["즉시".into()],
        remove: vec![],
    })
    .unwrap();

    // Assert: 워커의 선택 순서(`priority, event_id`)에서 태그 이벤트가 오래된 스캔 이벤트보다 먼저다
    let order: Vec<(i64, i64, i64)> = env.read(|c| {
        let mut s = c.prepare(mam_search_pending_sql()).unwrap();
        s.query_map([10], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    let (first_asset, first_rev, first_priority) = order[0];
    assert_eq!(
        (first_asset, first_rev, first_priority),
        (target, rev_before + 1, 0)
    );
    assert!(order[1..].iter().all(|(_, _, p)| *p > 0));
    assert_eq!(order.len(), 5, "4 scan events + 1 tag edit");
    assert_eq!(guard, snapshot(&env.vol));
}

/// `mam_search::sync::SELECT_PENDING_SQL` 과 같은 정렬 규칙 (mam-core 는 mam-search 에 의존하지 않는다).
fn mam_search_pending_sql() -> &'static str {
    "SELECT asset_id, revision, priority FROM search_outbox
     WHERE status = 'pending' ORDER BY priority, event_id LIMIT ?1"
}
