//! `plugin_tasks` 규칙 시험 (SQL 수준) — T-pjob-02(재공급 멱등) / 03(revision 역전) / 05(비활성·삭제 자산 이벤트) / 06(유형 필터) /
//! 09(커서) / 13~14(재시도·실패) / 16(비활성 닫기) / 17(행 삭제) / 19(전체 실행) / 20(실패 재시도).
//!
//! 분류: Happy / Boundary / Business rule / Concurrency(멱등). 실제 SQLite 에 마이그레이션을 적용해 쓴다 — 쿼리 mock 없음.

mod common;

use common::{count, insert_asset, migrated_db};
use mam_db::plugin_tasks::*;
use rusqlite::{Connection, Transaction, params};

const NOW: i64 = 1_000;
const P: &str = "com.example.p";
const IMG: ProviderTypes<'static> = ("meta", &["image"]);

fn event(c: &Connection, asset: i64, rev: i64, kind: &str) -> i64 {
    c.execute("INSERT INTO search_outbox (asset_id, revision, event_type, created_at) VALUES (?1, ?2, ?3, 0)", params![asset, rev, kind]).unwrap();
    c.last_insert_rowid()
}

/// (status, target, done, attempts) — 행이 없으면 `None`
fn row(
    c: &Connection,
    plugin: &str,
    provider: &str,
    asset: i64,
) -> Option<(String, i64, i64, i64)> {
    c.query_row(
        "SELECT status, target_revision, done_revision, attempts FROM plugin_tasks t JOIN plugins p USING (plugin_ref) JOIN plugin_providers v USING (provider_ref) WHERE p.plugin_id=?1 AND v.name=?2 AND t.asset_id=?3",
        (plugin, provider, asset),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .ok()
}

fn task_id(c: &Connection, asset: i64) -> i64 {
    c.query_row(
        "SELECT task_id FROM plugin_tasks WHERE asset_id = ?1",
        [asset],
        |r| r.get(0),
    )
    .unwrap()
}

fn with_tx<T>(c: &mut Connection, f: impl FnOnce(&Transaction<'_>) -> T) -> T {
    let tx = c.transaction().unwrap();
    let out = f(&tx);
    tx.commit().unwrap();
    out
}

fn feed(c: &mut Connection, after: i64, upto: i64) -> Vec<i64> {
    with_tx(c, |tx| feed_batch(tx, P, &[IMG], after, upto, NOW).unwrap())
}

#[test]
fn t_pjob_03_revision_reversal_keeps_the_highest_target_and_late_completion_never_lowers_done() {
    // Arrange: 이벤트 rev 5 다음 rev 3 (서로 다른 배치)
    let mut db = migrated_db();
    let a = insert_asset(&db.conn);
    let e5 = event(&db.conn, a, 5, "upsert");
    let e3 = event(&db.conn, a, 3, "upsert");

    // Act
    assert_eq!(feed(&mut db.conn, 0, e5), vec![a]);
    assert_eq!(
        feed(&mut db.conn, e5, e3),
        vec![a],
        "이미 pending 인 자산도 보고된다(job 등록은 멱등)"
    );

    // Assert
    assert_eq!(
        row(&db.conn, P, "meta", a),
        Some(("pending".into(), 5, 0, 0))
    );
    let t = task_id(&db.conn, a);
    with_tx(&mut db.conn, |tx| complete(tx, t, 5, 5, NOW).unwrap());
    assert_eq!(row(&db.conn, P, "meta", a), Some(("done".into(), 5, 5, 0)));
    with_tx(&mut db.conn, |tx| complete(tx, t, 3, 3, NOW).unwrap());
    assert_eq!(
        row(&db.conn, P, "meta", a),
        Some(("done".into(), 5, 5, 0)),
        "늦은 완료(rev 3)는 done 을 낮추지 못한다"
    );
}

#[test]
fn t_pjob_03_same_batch_folds_to_the_max_revision_and_completing_below_target_stays_pending() {
    // Arrange: 한 배치에 rev 5 -> 3 -> 7 순서의 이벤트
    let mut db = migrated_db();
    let a = insert_asset(&db.conn);
    let (e1, _, e3) = (
        event(&db.conn, a, 5, "upsert"),
        event(&db.conn, a, 3, "upsert"),
        event(&db.conn, a, 7, "upsert"),
    );
    assert!(e1 < e3);

    // Act
    feed(&mut db.conn, 0, e3);
    let t = task_id(&db.conn, a);
    with_tx(&mut db.conn, |tx| complete(tx, t, 5, 5, NOW).unwrap());

    // Assert: target 7, 5 까지만 끝남 -> 계속 pending
    assert_eq!(
        row(&db.conn, P, "meta", a),
        Some(("pending".into(), 7, 5, 0))
    );
    with_tx(&mut db.conn, |tx| complete(tx, t, 7, 7, NOW).unwrap());
    assert_eq!(row(&db.conn, P, "meta", a).unwrap().0, "done");
    // 완료 뒤 더 높은 revision 이 오면 다시 pending (Boundary: done == target 이면 done 유지)
    let e9 = event(&db.conn, a, 9, "upsert");
    assert_eq!(feed(&mut db.conn, e3, e9), vec![a]);
    assert_eq!(
        row(&db.conn, P, "meta", a),
        Some(("pending".into(), 9, 7, 0))
    );
}

#[test]
fn t_pjob_03_loop_guard_done_rev_above_processed_keeps_the_own_event_from_repending() {
    // Arrange: rev 5 처리 중 자기 태그 쓰기가 rev 6 이벤트를 만들었다 -> done_rev 6 으로 완료
    let mut db = migrated_db();
    let a = insert_asset(&db.conn);
    let e5 = event(&db.conn, a, 5, "upsert");
    feed(&mut db.conn, 0, e5);
    let t = task_id(&db.conn, a);
    with_tx(&mut db.conn, |tx| complete(tx, t, 5, 6, NOW).unwrap());

    // Act: 자기 이벤트(rev 6)가 피드에 도착
    let e6 = event(&db.conn, a, 6, "upsert");
    let repended = feed(&mut db.conn, e5, e6);

    // Assert
    assert!(repended.is_empty());
    assert_eq!(row(&db.conn, P, "meta", a), Some(("done".into(), 6, 6, 0)));
}

#[test]
fn t_pjob_02_refeeding_a_rewound_range_creates_no_duplicates_and_revives_nothing() {
    // T-pref-07 (feed_batch idempotent re-run with integer refs)
    // Arrange: done 1건·failed 1건·pending 1건
    let mut db = migrated_db();
    let (done, failed, pend) = (
        insert_asset(&db.conn),
        insert_asset(&db.conn),
        insert_asset(&db.conn),
    );
    let last = [done, failed, pend].map(|a| event(&db.conn, a, 2, "upsert"))[2];
    assert_eq!(feed(&mut db.conn, 0, last), vec![done, failed, pend]);
    let (t_done, t_failed) = (task_id(&db.conn, done), task_id(&db.conn, failed));
    with_tx(&mut db.conn, |tx| {
        complete(tx, t_done, 2, 2, NOW).unwrap();
        fail(tx, t_failed, "script_error", NOW).unwrap();
    });
    let before = count(&db.conn, "SELECT COUNT(*) FROM plugin_tasks");

    // Act: 커서를 되감아 같은 구간을 다시 공급
    let again = feed(&mut db.conn, 0, last);

    // Assert
    assert_eq!(
        again,
        vec![pend],
        "이미 pending 인 것만 다시 보고되고(작업 등록은 멱등), done·failed 는 되살아나지 않는다"
    );
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM plugin_tasks"), before);
    assert_eq!(row(&db.conn, P, "meta", done).unwrap().0, "done");
    assert_eq!(row(&db.conn, P, "meta", failed).unwrap().0, "failed");
    // 새 revision 은 실패 행도 되살린다 (시도 횟수 초기화)
    let e = event(&db.conn, failed, 3, "upsert");
    assert_eq!(feed(&mut db.conn, last, e), vec![failed]);
    assert_eq!(
        row(&db.conn, P, "meta", failed),
        Some(("pending".into(), 3, 0, 0))
    );
}

#[test]
fn t_pjob_05_events_for_inactive_or_vanished_assets_create_no_row_and_do_not_fail_the_tx() {
    // Arrange: 활성·삭제·병합(superseded)·hard delete 된 자산과 delete 이벤트
    let mut db = migrated_db();
    let (active, deleted, superseded, gone, delete_ev) = (
        insert_asset(&db.conn),
        insert_asset(&db.conn),
        insert_asset(&db.conn),
        insert_asset(&db.conn),
        insert_asset(&db.conn),
    );
    db.conn
        .execute(
            "UPDATE media_assets SET status='deleted' WHERE asset_id=?1",
            [deleted],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE media_assets SET status='superseded' WHERE asset_id=?1",
            [superseded],
        )
        .unwrap();
    let mut last = 0;
    for a in [active, deleted, superseded, gone] {
        last = event(&db.conn, a, 2, "upsert");
    }
    db.conn
        .execute("DELETE FROM media_assets WHERE asset_id=?1", [gone])
        .unwrap(); // outbox 에는 FK 가 없다
    last = last.max(event(&db.conn, delete_ev, 3, "delete"));

    // Act
    let pending = feed(&mut db.conn, 0, last);

    // Assert
    assert_eq!(pending, vec![active]);
    assert_eq!(count(&db.conn, "SELECT COUNT(*) FROM plugin_tasks"), 1);
}

#[test]
fn t_pjob_06_provider_media_types_filter_the_feed_and_empty_means_all() {
    // Arrange: 이미지 1·음성 1
    let mut db = migrated_db();
    let img = insert_asset(&db.conn);
    let aud = insert_asset(&db.conn);
    db.conn
        .execute(
            "UPDATE media_assets SET media_type='audio' WHERE asset_id=?1",
            [aud],
        )
        .unwrap();
    let last = [img, aud].map(|a| event(&db.conn, a, 1, "upsert"))[1];

    // Act
    let got = with_tx(&mut db.conn, |tx| {
        feed_batch(tx, P, &[("img", &["image"]), ("any", &[])], 0, last, NOW).unwrap()
    });

    // Assert
    assert_eq!(got, vec![img, aud]);
    assert!(row(&db.conn, P, "img", img).is_some() && row(&db.conn, P, "img", aud).is_none());
    assert!(row(&db.conn, P, "any", img).is_some() && row(&db.conn, P, "any", aud).is_some());
    assert!(
        with_tx(&mut db.conn, |tx| feed_batch(tx, P, &[], 0, last, NOW)
            .unwrap())
        .is_empty(),
        "provider 가 없으면 아무것도 하지 않는다"
    );
    assert!(feed(&mut db.conn, last, last).is_empty(), "빈 구간");
}

#[test]
fn t_pjob_09_cursor_initializes_to_max_event_once_and_keeps_an_existing_value() {
    // Arrange
    let mut db = migrated_db();
    let a = insert_asset(&db.conn);
    let e = event(&db.conn, a, 1, "upsert");
    assert_eq!(
        (
            feed_cursor(&db.conn, P).unwrap(),
            max_event_id(&db.conn).unwrap()
        ),
        (None, e)
    );

    // Act / Assert
    assert_eq!(
        with_tx(&mut db.conn, |tx| init_feed_cursor(tx, P).unwrap()),
        e
    );
    event(&db.conn, a, 2, "upsert");
    assert_eq!(
        with_tx(&mut db.conn, |tx| init_feed_cursor(tx, P).unwrap()),
        e,
        "이미 있으면 그대로"
    );
    with_tx(&mut db.conn, |tx| set_feed_cursor(tx, P, 0).unwrap());
    assert_eq!(
        feed_cursor(&db.conn, P).unwrap(),
        Some(0),
        "0 도 유효한 커서"
    );
    db.conn
        .execute(
            "UPDATE settings SET value='garbage' WHERE key = 'plugin_feed_cursor:com.example.p'",
            [],
        )
        .unwrap();
    assert_eq!(
        feed_cursor(&db.conn, P).unwrap(),
        None,
        "손상은 없음으로 본다"
    );
    assert_eq!(
        with_tx(&mut db.conn, |tx| init_feed_cursor(tx, P).unwrap()),
        e + 1
    );
}

#[test]
fn t_pjob_13_retry_later_counts_attempts_and_stays_pending_and_fail_records_the_code() {
    // Arrange
    let mut db = migrated_db();
    let a = insert_asset(&db.conn);
    let e = event(&db.conn, a, 1, "upsert");
    feed(&mut db.conn, 0, e);
    let t = task_id(&db.conn, a);

    // Act / Assert
    assert_eq!(
        with_tx(&mut db.conn, |tx| retry_later(tx, t, NOW).unwrap()),
        Some(1)
    );
    assert_eq!(
        with_tx(&mut db.conn, |tx| retry_later(tx, t, NOW).unwrap()),
        Some(2)
    );
    assert_eq!(
        row(&db.conn, P, "meta", a),
        Some(("pending".into(), 1, 0, 2))
    );
    assert!(with_tx(&mut db.conn, |tx| fail(tx, t, "timeout", NOW).unwrap()));
    assert_eq!(pending_count(&db.conn, P).unwrap(), 0);
    assert_eq!(
        db.conn
            .query_row("SELECT last_error FROM plugin_tasks", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "timeout"
    );
    assert_eq!(
        with_tx(&mut db.conn, |tx| retry_later(tx, 999, NOW).unwrap()),
        None,
        "없는 행"
    );
    // 성공 완료는 시도 횟수·오류를 지운다
    with_tx(&mut db.conn, |tx| complete(tx, t, 1, 1, NOW).unwrap());
    assert_eq!(row(&db.conn, P, "meta", a), Some(("done".into(), 1, 1, 0)));
}

#[test]
fn t_pjob_16_mark_inactive_done_closes_pending_tasks_of_the_asset_only() {
    // Arrange: 같은 자산의 두 provider·다른 자산 1건
    let mut db = migrated_db();
    let (a, b) = (insert_asset(&db.conn), insert_asset(&db.conn));
    let last = [a, b].map(|x| event(&db.conn, x, 4, "upsert"))[1];
    with_tx(&mut db.conn, |tx| {
        feed_batch(
            tx,
            P,
            &[("m1", &["image"]), ("m2", &["image"])],
            0,
            last,
            NOW,
        )
        .unwrap()
    });

    // Act
    let closed = with_tx(&mut db.conn, |tx| mark_inactive_done(tx, a, NOW).unwrap());

    // Assert
    assert_eq!(closed, 2);
    assert_eq!(row(&db.conn, P, "m1", a), Some(("done".into(), 4, 4, 0)));
    assert_eq!(
        db.conn
            .query_row(
                "SELECT last_error FROM plugin_tasks WHERE asset_id=?1 LIMIT 1",
                [a],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "inactive"
    );
    assert_eq!(row(&db.conn, P, "m1", b).unwrap().0, "pending");
    assert!(pending_for_asset(&db.conn, a).unwrap().is_empty());
}

#[test]
fn t_pjob_17_delete_task_removes_only_that_row_and_is_idempotent() {
    // Arrange
    let mut db = migrated_db();
    let a = insert_asset(&db.conn);
    let e = event(&db.conn, a, 1, "upsert");
    with_tx(&mut db.conn, |tx| {
        feed_batch(tx, P, &[("m1", &["image"]), ("m2", &["image"])], 0, e, NOW).unwrap()
    });
    let tasks = pending_for_asset(&db.conn, a).unwrap();
    assert_eq!(
        tasks
            .iter()
            .map(|t| t.provider.as_str())
            .collect::<Vec<_>>(),
        ["m1", "m2"],
        "task_id 순"
    );

    // Act / Assert
    assert!(with_tx(&mut db.conn, |tx| delete_task(
        tx,
        tasks[0].task_id
    )
    .unwrap()));
    assert!(!with_tx(&mut db.conn, |tx| delete_task(
        tx,
        tasks[0].task_id
    )
    .unwrap()));
    assert!(row(&db.conn, P, "m1", a).is_none() && row(&db.conn, P, "m2", a).is_some());
    assert!(
        !with_tx(&mut db.conn, |tx| complete(tx, tasks[0].task_id, 1, 1, NOW)
            .unwrap()),
        "사라진 행의 완료는 false"
    );
}

#[test]
fn t_pjob_19_run_all_walks_active_assets_by_keyset_skips_up_to_date_rows_and_retasks_on_new_revision()
 {
    // T-pref-07 (run_all_batch idempotent re-run with integer refs)
    // Arrange: 활성 2,500 + 삭제 1 (limit 1,000 으로 3배치)
    let mut db = migrated_db();
    let ids: Vec<i64> = {
        let tx = db.conn.transaction().unwrap();
        let ids = (0..2_500).map(|_| insert_asset(&tx)).collect();
        tx.commit().unwrap();
        ids
    };
    let dead = insert_asset(&db.conn);
    db.conn
        .execute(
            "UPDATE media_assets SET status='deleted' WHERE asset_id=?1",
            [dead],
        )
        .unwrap();

    // Act: 전체 순회
    let (mut after, mut tasked, mut batches) = (0, Vec::new(), 0);
    loop {
        let page = with_tx(&mut db.conn, |tx| {
            run_all_batch(tx, P, &[IMG], after, 1_000, NOW).unwrap()
        });
        batches += 1;
        tasked.extend(page.pending);
        match page.last {
            Some(l) if page.scanned == 1_000 => after = l,
            _ => break,
        }
    }

    // Assert
    assert_eq!((batches, tasked.len()), (3, 2_500));
    assert_eq!(tasked, ids);
    assert_eq!(
        count(&db.conn, "SELECT COUNT(*) FROM plugin_tasks"),
        2_500,
        "비활성 자산은 제외"
    );
    // 처리 완료 후 다시 실행: 건너뜀. 한 자산만 revision 이 오르면 그것만 다시 대기
    db.conn
        .execute(
            "UPDATE plugin_tasks SET status='done', done_revision=target_revision",
            [],
        )
        .unwrap();
    db.conn
        .execute(
            "UPDATE media_assets SET revision = revision + 1 WHERE asset_id = ?1",
            [ids[7]],
        )
        .unwrap();
    let page = with_tx(&mut db.conn, |tx| {
        run_all_batch(tx, P, &[IMG], 0, RUN_ALL_BATCH, NOW).unwrap()
    });
    assert_eq!((page.pending, page.scanned), (vec![ids[7]], 2_500));
    assert_eq!(
        row(&db.conn, P, "meta", ids[7]),
        Some(("pending".into(), 2, 1, 0))
    );
    let end = with_tx(&mut db.conn, |tx| {
        run_all_batch(tx, P, &[IMG], page.last.unwrap(), 10, NOW).unwrap()
    });
    assert_eq!(
        (end.scanned, end.last, end.pending.len()),
        (0, None, 0),
        "끝 -> 빈 페이지"
    );
}

#[test]
fn t_pjob_20_retry_failed_requeues_only_that_plugins_failed_rows_and_jobs_are_deduplicated() {
    // Arrange: p 에 failed 2(같은 자산 두 provider 포함)·done 1, 다른 플러그인 failed 1
    let mut db = migrated_db();
    let (a, b, c) = (
        insert_asset(&db.conn),
        insert_asset(&db.conn),
        insert_asset(&db.conn),
    );
    let last = [a, b, c].map(|x| event(&db.conn, x, 2, "upsert"))[2];
    with_tx(&mut db.conn, |tx| {
        feed_batch(
            tx,
            P,
            &[("m1", &["image"]), ("m2", &["image"])],
            0,
            last,
            NOW,
        )
        .unwrap();
        feed_batch(tx, "other", &[("m1", &["image"])], 0, last, NOW).unwrap();
    });
    db.conn.execute("UPDATE plugin_tasks SET status='failed', attempts=3, last_error='x' WHERE plugin_ref=(SELECT plugin_ref FROM plugins WHERE plugin_id=?1) AND asset_id IN (?2, ?3) AND NOT (asset_id=?3 AND provider_ref=(SELECT provider_ref FROM plugin_providers WHERE name='m2' AND plugin_ref=plugin_tasks.plugin_ref))", (P, a, b)).unwrap();
    db.conn.execute("UPDATE plugin_tasks SET status='done', done_revision=2 WHERE plugin_ref=(SELECT plugin_ref FROM plugins WHERE plugin_id=?1) AND asset_id=?2", (P, c)).unwrap();
    db.conn.execute("UPDATE plugin_tasks SET status='failed' WHERE plugin_ref=(SELECT plugin_ref FROM plugins WHERE plugin_id='other')", []).unwrap();
    let failed_before = failed_count(&db.conn, P).unwrap();
    assert_eq!(failed_before, 3);

    // Act
    let assets = with_tx(&mut db.conn, |tx| retry_failed(tx, P, NOW + 5).unwrap());
    let queued = with_tx(&mut db.conn, |tx| {
        enqueue_jobs_tx(tx, &assets, 70, NOW).unwrap()
    });
    let again = with_tx(&mut db.conn, |tx| {
        enqueue_jobs_tx(tx, &assets, 70, NOW).unwrap()
    });

    // Assert
    assert_eq!(assets, vec![a, b]);
    assert_eq!(
        (
            failed_count(&db.conn, P).unwrap(),
            pending_count(&db.conn, P).unwrap()
        ),
        (0, 4)
    );
    assert_eq!(
        row(&db.conn, P, "m1", a),
        Some(("pending".into(), 2, 0, 0)),
        "시도 횟수 초기화"
    );
    assert_eq!(
        failed_count(&db.conn, "other").unwrap(),
        3,
        "다른 플러그인은 그대로"
    );
    assert_eq!(
        (queued, again),
        (2, 0),
        "대기 중 job 은 중복 등록되지 않는다"
    );
    assert_eq!(
        count(
            &db.conn,
            "SELECT COUNT(*) FROM jobs WHERE kind='plugin' AND status='pending' AND priority=70"
        ),
        2
    );
    assert!(with_tx(&mut db.conn, |tx| retry_failed(tx, P, NOW).unwrap()).is_empty());
}
