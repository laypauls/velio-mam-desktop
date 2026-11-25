//! P4 복원력 — 볼륨 탈착·재연결의 검색 측 효과(T-res-08s, 09s)와 태그 수정 반영 순서(T-res-12).
//! 분류: Business rule / Happy. 실제 SQLite(마이그레이션) + 임시 디렉터리 Tantivy + 주입 시계.
//! 볼륨 provider·reconcile 쪽은 `crates/mam-core/tests/resilience_volume.rs` (mam-search 는 mam-core 에 의존하지 않으므로
//! 여기서는 reconcile 이 남기는 SQLite 상태 변화(revision 증가 + outbox)를 같은 규칙으로 재현한다).
//! 원본 보호: 시나리오 끝에 픽스처 원본 불변 단언.

mod common;
#[path = "common/resilience.rs"]
mod res;

use mam_search::sync::SyncConfig;
use mam_search::{Filters, Presence, SearchRequest};
use res::*;
use rusqlite::params;

/// PL5 기본 필터: 정상 + 오프라인 표시, 파일 없음 숨김 (`includeOffline=true`, `includeMissing=false`).
fn pl5(text: &str) -> SearchRequest {
    SearchRequest {
        filters: Filters {
            presence: Some(Presence::Present),
            ..Default::default()
        },
        ..req(text)
    }
}

fn online_only(text: &str, online: &[u64]) -> SearchRequest {
    SearchRequest {
        filters: Filters {
            online_volume_ids: Some(online.to_vec()),
            presence: Some(Presence::Present),
            ..Default::default()
        },
        ..req(text)
    }
}

fn set_volume(env: &Env, volume_id: i64, availability: &str) {
    env.conn
        .execute(
            "UPDATE volumes SET availability = ?2 WHERE volume_id = ?1",
            params![volume_id, availability],
        )
        .unwrap();
}

#[test]
fn t_res_08s_offline_volume_keeps_documents_searchable_under_pl5_and_missing_is_hidden() {
    // Arrange: 볼륨 1(고정)·2(이동식) 에 자산, 이동식 쪽에 태그와 'missing' 1건
    let env = Env::new();
    let originals = Originals::new(&["x.jpg"]);
    let on = add_asset(&env.conn, 1, "trip_home.jpg", 0);
    let usb = add_asset(&env.conn, 2, "trip_usb.jpg", 0);
    let lost = add_asset(&env.conn, 2, "trip_lost.jpg", 0);
    set_tags(&env.conn, usb, &["휴가"], 0);
    env.conn
        .execute(
            "UPDATE file_locations SET presence = 'missing' WHERE asset_id = ?1",
            [lost as i64],
        )
        .unwrap();
    bump(&env.conn, lost, 0);
    let w = env.worker();
    w.drain(&env.conn, &LATE).unwrap();
    let idx = w.index().unwrap();
    assert_eq!(ids_of(idx, &pl5("trip")), [on, usb], "missing is hidden");
    let outbox_before = count(&env.conn, "SELECT count(*) FROM search_outbox");
    let rev_before = count(&env.conn, "SELECT sum(revision) FROM media_assets");

    // Act: USB 탈착 (volumes 한 행만 offline) 후 워커 구동
    set_volume(&env, 2, "offline");
    let r = w.drain(&env.conn, &LATE).unwrap();

    // Assert: 삭제·missing 이 아니다 — 문서·태그 유지, 이벤트·revision 변화 없음
    assert_eq!(r.events_selected, 0);
    assert_eq!(
        count(&env.conn, "SELECT count(*) FROM search_outbox"),
        outbox_before
    );
    assert_eq!(
        count(&env.conn, "SELECT sum(revision) FROM media_assets"),
        rev_before
    );
    assert_eq!(idx.num_docs(), 3, "offline docs stay in the index");
    assert_eq!(
        ids_of(idx, &pl5("trip")),
        [on, usb],
        "PL5: offline is shown"
    );
    assert_eq!(find(idx, "휴가"), [usb], "tag survives offline");
    assert_eq!(
        ids_of(idx, &req("trip")),
        [on, usb, lost],
        "missing only hidden by the filter"
    );
    // '온라인만' 옵션을 켜면 오프라인 자산이 빠지고, 재연결하면 바로 돌아온다 (재색인 없이)
    assert_eq!(ids_of(idx, &online_only("trip", &[1])), [on]);
    set_volume(&env, 2, "online");
    assert_eq!(ids_of(idx, &online_only("trip", &[1, 2])), [on, usb]);
    originals.assert_untouched();
}

#[test]
fn t_res_09s_relinked_asset_after_reconnect_keeps_id_and_search_follows_new_path() {
    // Arrange: 이동식 볼륨(2)의 자산 + 태그. 탈착 후 다른 드라이브 문자로 재연결되어 다른 볼륨(1) 경로로 재연결(Moved)된다.
    let env = Env::new();
    let originals = Originals::new(&["x.jpg"]);
    let id = add_asset(&env.conn, 2, "old_dir/clip_old.jpg", 0);
    let other = add_asset(&env.conn, 1, "bystander.jpg", 0);
    set_tags(&env.conn, id, &["보존태그"], 0);
    let w = env.worker();
    w.drain(&env.conn, &LATE).unwrap();
    assert_eq!(find(w.index().unwrap(), "clip_old"), [id]);
    set_volume(&env, 2, "offline");

    // Act: reconcile 의 Moved 와 같은 SQLite 변화 — 위치 갱신 + revision 증가 + outbox 를 한 트랜잭션으로
    let tx = env.conn.unchecked_transaction().unwrap();
    tx.execute(
        "UPDATE file_locations SET volume_id = 1, rel_path = 'new_dir/clip_new.jpg',
             path_key = 'new_dir/clip_new.jpg', presence = 'present' WHERE asset_id = ?1",
        [id as i64],
    )
    .unwrap();
    let rev = bump_in(&tx, id);
    event(&tx, id, rev, "upsert", 1);
    tx.commit().unwrap();
    let r = w.drain(&env.conn, &LATE).unwrap();

    // Assert: 같은 asset_id, 새 경로로 검색, 옛 이름 없음, 문서 중복 없음, 태그 유지
    let idx = w.index().unwrap();
    assert_eq!(r.error, None);
    assert_eq!(find(idx, "clip_new"), [id]);
    assert!(find(idx, "clip_old").is_empty());
    assert_eq!(find(idx, "보존태그"), [id]);
    assert_eq!(idx.num_docs(), 2);
    assert_eq!(all_indexed_ids(idx), [id, other]);
    assert_eq!(progress(&env.conn, id), Some(rev));
    assert_eq!(ids_of(idx, &online_only("clip_new", &[1])), [id]);
    originals.assert_untouched();
}

#[test]
fn t_res_12_tag_edit_is_applied_before_older_bulk_events_and_becomes_searchable() {
    // Arrange: 대량 분석(priority 1) 이벤트가 먼저 쌓이고(낮은 event_id), 사용자가 나중에 태그를 수정
    let env = Env::new();
    let originals = Originals::new(&["x.jpg"]);
    let w = env.worker();
    let target = add_asset(&env.conn, 1, "target.jpg", 0);
    w.drain(&env.conn, &LATE).unwrap();
    let bulk: Vec<u64> = (0..5)
        .map(|i| add_asset(&env.conn, 1, &format!("bulk_{i}.jpg"), 1))
        .collect();
    let rev = set_tags(&env.conn, target, &["즉시반영"], 0);
    let early = FixedClock(T0 + 10); // 대량 이벤트 모으는 중(2s 미만)

    // Act 1: 모으는 중에도 사용자 편집은 즉시 처리
    let r1 = w.process_batch(&env.conn, &early).unwrap();

    // Assert 1: 태그 검색 적중, 대량 이벤트는 아직 대기
    let idx = w.index().unwrap();
    assert_eq!((r1.upserted, r1.events_done), (1, 1));
    assert_eq!(find(idx, "즉시반영"), [target]);
    assert_eq!(progress(&env.conn, target), Some(rev));
    assert!(find(idx, "bulk_0").is_empty());
    assert_eq!(count_status(&env.conn, "pending"), 5);
    let r2 = w.process_batch(&env.conn, &early).unwrap();
    assert_eq!(
        (r2.events_selected, r2.deferred_until_ms),
        (0, Some(T0 + 2_000))
    );

    // Act 2: 시간이 지나면 대량 이벤트도 처리, 이어서 태그 제거도 즉시 반영
    w.drain(&env.conn, &LATE).unwrap();
    assert_eq!(ids_of(w.index().unwrap(), &req("bulk")), bulk);
    set_tags(&env.conn, target, &[], 0);
    bump(&env.conn, bulk[0], 1); // 저우선 대기 중에도
    w.process_batch(&env.conn, &early).unwrap();

    // Assert 2
    assert!(find(w.index().unwrap(), "즉시반영").is_empty());
    assert_eq!(
        count_status(&env.conn, "pending"),
        1,
        "bulk edit still waiting"
    );
    originals.assert_untouched();
}

#[test]
fn t_res_12b_tag_edit_survives_restart_before_it_was_indexed() {
    // Arrange: 태그 수정이 SQLite 에 커밋됐지만 색인 전에 앱이 종료 -> 재시작
    let env = Env::new();
    let id = add_asset(&env.conn, 1, "photo.jpg", 0);
    env.worker().drain(&env.conn, &LATE).unwrap();
    let rev = set_tags(&env.conn, id, &["재시작후"], 0);

    // Act
    let (w, out) = env.start(&env.conn, SyncConfig::default());
    w.drain(&env.conn, &LATE).unwrap();

    // Assert
    assert!(out.rebuilt.is_none());
    assert_eq!(find(w.index().unwrap(), "재시작후"), [id]);
    assert_eq!(progress(&env.conn, id), Some(rev));
}
