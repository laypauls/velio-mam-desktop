//! SES 큐 읽기 시험 (T-ses-01 ~ 16). 진짜 SQLite 파일(SES `events` 스키마) + 같은 SQL 을 읽기 전용으로 실행하는 가짜 호스트 — 네트워크 없음.
//!
//! 분류: Happy (03 기준선·04 이어 읽기·05 hex 왕복) / Boundary (06 빈 경로·09 밀림 경계·11 출력 상한·13 커서 JSON) / Validation (01 DB 경로·02 명령 모양) /
//! Business rule (07 분류·08 큐 초기화·10 커서 행 사라짐) / External (12 실패 전달·14 접속 시험) / 원본 보호 (15 DB 불변·16 읽기 전용).

use rusqlite::Connection;
use tempfile::TempDir;

use super::*;
use crate::watch::remote::fake_ses::{FakeSes, SES_SCHEMA, insert_event};

const LOGICAL: &str = "/srv/ses/events.db";

struct Fx {
    _dir: TempDir,
    file: std::path::PathBuf,
    conn: Connection,
    fake: FakeSes,
}

fn fx() -> Fx {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("events.db");
    let conn = Connection::open(&file).unwrap();
    conn.execute_batch(SES_SCHEMA).unwrap();
    let fake = FakeSes::new(&file, LOGICAL);
    Fx { _dir: dir, file, conn, fake }
}

impl Fx {
    fn add(&self, types: &str, path: &str) -> i64 {
        insert_event(&self.conn, types, path, false).unwrap()
    }

    fn feed(&self, cursor: Option<i64>) -> SesFeed {
        SesFeed::new(LOGICAL, cursor.map(|id| Cursor { id }))
    }
}

#[test]
fn t_ses_01_db_paths_with_quotes_or_shell_metacharacters_or_traversal_are_rejected() {
    // Arrange / Act / Assert
    for ok in ["/srv/ses/events.db", "/var/lib/ses/큐 폴더/events.db", "/data/a_b-c.d/events.sqlite", "/events.db"] {
        assert!(valid_db_path(ok), "{ok}");
    }
    for bad in [
        "", "events.db", "relative/events.db", "/a/'b", "/a/\"b", "/a/`id`", "/a/$(id)", "/a/$HOME", "/a;rm -rf /", "/a&b", "/a|b", "/a>b", "/a<b",
        "/a\nb", "/a\rb", "/a\0b", "/a\\b", "/a/../b", "/a/b/", "/a/(b)", "/a/{b}", "/a/[b]", "/a/*", "/a/?", "/a/!b", "/a/#b", "/a/~b",
    ] {
        assert!(!valid_db_path(bad), "{bad:?}");
    }
    assert!(!valid_db_path(&format!("/{}", "a".repeat(512))), "length cap");
}

#[test]
fn t_ses_02_the_command_is_one_readonly_select_with_an_integer_cursor_and_a_quoted_path() {
    // Arrange / Act
    let c = cmd::read(LOGICAL, 42, 1000);
    // Assert: 읽기 전용 플래그, 정수 커서·행 수, SELECT 만 (쓰기 키워드 없음)
    assert!(c.contains("sqlite3 -readonly "), "{c}");
    assert!(c.contains("WHERE id > 42 ORDER BY id LIMIT 1000"), "{c}");
    assert!(c.contains("[ -r '/srv/ses/events.db' ]"), "{c}");
    let up = c.to_uppercase();
    for verb in ["INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "ATTACH", "PRAGMA", "VACUUM", "REPLACE"] {
        assert!(!up.contains(verb), "{verb} in {c}");
    }
    // 가짜 경로가 따옴표를 품어도 한 단어로 남는다 (저장 검증이 먼저 막지만 명령 생성도 스스로 지킨다)
    assert!(cmd::read("/a'b", 1, 10).contains(r"'/a'\''b'"));
    // 음수·큰 커서도 숫자 그대로
    assert!(cmd::read(LOGICAL, i64::MAX, 10).contains(&format!("id > {}", i64::MAX)));
}

#[test]
fn t_ses_03_the_first_poll_takes_the_current_max_id_as_a_baseline_reads_no_history_and_reports_a_gap() {
    // Arrange
    let f = fx();
    for p in ["/data/incoming/a.mov", "/data/incoming/b.mov", "/data/incoming/c.mov"] {
        f.add("create", p);
    }
    let mut feed = f.feed(None);
    // Act
    let p = feed.poll(&mut f.fake.runner()).unwrap();
    // Assert
    assert_eq!((p.rows.len(), p.next, p.gap, p.reset), (0, Some(Cursor { id: 3 }), true, false));
    assert_eq!(feed.cursor(), None, "poll does not change state — only commit does");
    feed.commit(p.next.unwrap());
    f.add("create", "/data/incoming/d.mov");
    let p = feed.poll(&mut f.fake.runner()).unwrap();
    assert_eq!((p.rows.len(), p.next, p.gap), (1, Some(Cursor { id: 4 }), false));
}

#[test]
fn t_ses_04_a_resumed_cursor_reads_only_newer_rows_in_id_order_and_an_empty_queue_keeps_the_cursor() {
    // Arrange
    let f = fx();
    let ids: Vec<i64> = ["a", "b", "c", "d"].iter().map(|n| f.add("close_write", &format!("/data/incoming/{n}.mov"))).collect();
    // Act
    let p = f.feed(Some(ids[1])).poll(&mut f.fake.runner()).unwrap();
    // Assert
    assert_eq!(p.rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![ids[2], ids[3]]);
    assert_eq!((p.next, p.gap), (Some(Cursor { id: ids[3] }), false));
    let idle = f.feed(Some(ids[3])).poll(&mut f.fake.runner()).unwrap();
    assert_eq!((idle.rows.len(), idle.next, idle.gap), (0, Some(Cursor { id: ids[3] }), false));
}

#[test]
fn t_ses_05_hex_transport_round_trips_paths_with_separators_newlines_quotes_and_korean() {
    // Arrange: 구분자(`|`)·줄바꿈·따옴표·한글 — 구분자 방식이면 깨질 이름
    let f = fx();
    let weird = ["/data/incoming/a|b.mov", "/data/incoming/줄\n바꿈.mov", "/data/incoming/it's \"q\".mov", "/data/incoming/한글 폴더/영상.mp4"];
    for w in weird {
        f.add("create,close_write", w);
    }
    // Act
    let p = f.feed(Some(0)).poll(&mut f.fake.runner()).unwrap();
    // Assert
    assert_eq!(p.rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), weird);
    assert_eq!(p.bad, 0);
    assert_eq!(p.rows[0].types, vec!["create", "close_write"]);
}

#[test]
fn t_ses_06_a_null_path_falls_back_to_dir_and_name_and_an_unusable_row_is_counted_but_the_cursor_passes_it() {
    // Arrange
    let f = fx();
    f.conn.execute("INSERT INTO events (captured_at, event_mask, event_types, path, name, dir, is_dir) VALUES ('t', 0, 'create', NULL, 'x.mov', '/data/incoming/', 0)", []).unwrap();
    f.conn.execute("INSERT INTO events (captured_at, event_mask, event_types, path, name, dir, is_dir) VALUES ('t', 0, 'create', NULL, NULL, NULL, 0)", []).unwrap();
    // Act
    let p = f.feed(Some(0)).poll(&mut f.fake.runner()).unwrap();
    // Assert
    assert_eq!(p.rows.len(), 1);
    assert_eq!(p.rows[0].path, "/data/incoming/x.mov");
    assert_eq!(p.bad, 1, "the row without any path (lost CAP_DAC_READ_SEARCH) is counted");
    assert_eq!(p.next, Some(Cursor { id: 2 }), "and never re-read");
}

#[test]
fn t_ses_07_events_become_path_or_subtree_reports_and_unrelated_kinds_are_ignored() {
    // Arrange
    let row = |types: &str, is_dir: bool| Row { id: 1, types: types.split(',').map(str::to_string).collect(), path: "/data/incoming/x".into(), is_dir };
    // Act / Assert: (types, is_dir) -> Some(subtree?)
    for (types, dir, want) in [
        ("create", true, Some(true)),
        ("moved_to", true, Some(true)),
        ("create,close_write", true, Some(true)),
        ("create", false, Some(false)),
        ("close_write", false, Some(false)),
        ("moved_to", false, Some(false)),
        ("delete", false, Some(false)),
        ("moved_from", false, Some(false)),
        ("delete", true, Some(false)),
        ("moved_from", true, Some(false)),
        ("close_write", true, Some(false)),
        ("modify", false, None),
        ("open", false, None),
        ("", false, None),
    ] {
        assert_eq!(classify(&row(types, dir)), want, "{types} dir={dir}");
    }
}

#[test]
fn t_ses_08_a_max_id_below_the_cursor_means_the_queue_was_reset_so_the_cursor_goes_back_and_a_gap_is_reported() {
    // Arrange: 커서 10 인데 큐에는 3건뿐 (DB 재생성)
    let f = fx();
    for _ in 0..3 {
        f.add("create", "/data/incoming/n.mov");
    }
    // Act
    let p = f.feed(Some(10)).poll(&mut f.fake.runner()).unwrap();
    // Assert
    assert_eq!((p.reset, p.gap, p.next, p.rows.len()), (true, true, Some(Cursor { id: 3 }), 0));
}

#[test]
fn t_ses_09_a_backlog_over_the_limit_skips_to_the_end_with_a_gap_but_exactly_the_limit_is_still_read() {
    // Arrange: id 1..=50_001 (한 문장으로 대량 삽입)
    let f = fx();
    f.conn
        .execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 50001)
             INSERT INTO events (captured_at, event_mask, event_types, path, name, dir, is_dir) SELECT 't', 0, 'create', '/data/incoming/f' || i, 'f', '/data/incoming', 0 FROM n;",
        )
        .unwrap();
    // Act: 커서 1 -> 밀림 50_000 = 한계 정확히 (읽는다)
    let at_limit = f.feed(Some(1)).poll(&mut f.fake.runner()).unwrap();
    // 커서 0 -> 밀림 50_001 (건너뛴다)
    let over = f.feed(Some(0)).poll(&mut f.fake.runner()).unwrap();
    // Assert
    assert_eq!((at_limit.gap, at_limit.rows.len(), at_limit.skipped_rows), (false, MAX_ROWS as usize, 0));
    assert_eq!(at_limit.next, Some(Cursor { id: 1 + i64::from(MAX_ROWS) }));
    assert_eq!((over.gap, over.rows.len(), over.skipped_rows, over.next), (true, 0, 50_001, Some(Cursor { id: 50_001 })));
}

#[test]
fn t_ses_10_a_missing_cursor_row_reports_a_gap_but_keeps_reading_the_newer_rows() {
    // Arrange: 2번 행이 사라졌다 (SES 쪽에서 지워졌거나 큐가 바뀜)
    let f = fx();
    for _ in 0..4 {
        f.add("create", "/data/incoming/n.mov");
    }
    f.conn.execute("DELETE FROM events WHERE id = 2", []).unwrap();
    // Act
    let p = f.feed(Some(2)).poll(&mut f.fake.runner()).unwrap();
    // Assert
    assert_eq!((p.gap, p.reset, p.rows.len(), p.next), (true, false, 2, Some(Cursor { id: 4 })));
    // 커서 0(아직 아무것도 못 본 상태)은 "행이 없다" 를 공백으로 보지 않는다
    assert!(!f.feed(Some(0)).poll(&mut f.fake.runner()).unwrap().gap);
}

#[test]
fn t_ses_11_a_too_large_output_halves_the_row_limit_for_the_next_poll_and_success_grows_it_back() {
    // Arrange
    let f = fx();
    for i in 0..20 {
        f.add("create", &format!("/data/incoming/{}{i}.mov", "x".repeat(200)));
    }
    f.fake.set_max_output(1000);
    let mut feed = f.feed(Some(0));
    // Act
    let first = feed.poll(&mut f.fake.runner());
    let second = feed.poll(&mut f.fake.runner());
    // Assert
    assert!(matches!(first, Err(RunError::Exec(m)) if m.contains("output too large")));
    assert!(second.is_err(), "still too large at 500 rows");
    let cmds = f.fake.commands();
    assert!(cmds[0].contains("LIMIT 1000") && cmds[1].contains("LIMIT 500"), "{cmds:?}");
    f.fake.set_max_output(4 * 1024 * 1024);
    assert_eq!(feed.poll(&mut f.fake.runner()).unwrap().rows.len(), 20, "250 rows still covers the 20");
    feed.poll(&mut f.fake.runner()).unwrap();
    feed.poll(&mut f.fake.runner()).unwrap();
    assert!(f.fake.commands().last().unwrap().contains("LIMIT 1000"), "each success doubles the limit back up to the cap");
}

#[test]
fn t_ses_12_remote_failures_pass_through_and_leave_the_state_untouched() {
    // Arrange
    let f = fx();
    f.add("create", "/data/incoming/a.mov");
    let mut feed = f.feed(Some(0));
    for e in [RunError::NoSqlite, RunError::DbUnreadable("database is locked".into()), RunError::Timeout, RunError::Closed] {
        // Act
        f.fake.set_fail(Some(e.clone()));
        // Assert
        assert_eq!(feed.poll(&mut f.fake.runner()).unwrap_err(), e);
        assert_eq!(feed.cursor(), Some(Cursor { id: 0 }));
    }
    f.fake.set_fail(None);
    assert_eq!(feed.poll(&mut f.fake.runner()).unwrap().rows.len(), 1, "and it reads normally again");
}

#[test]
fn t_ses_13_the_cursor_json_round_trips_and_garbage_or_negative_values_mean_no_cursor() {
    // Arrange / Act / Assert
    assert_eq!(Cursor::from_json(&Cursor { id: 7 }.to_json()), Some(Cursor { id: 7 }));
    assert_eq!(Cursor::to_json(Cursor { id: 7 }), r#"{"id":7}"#);
    assert_eq!(Cursor::from_json(r#"{"id":3,"extra":1}"#), Some(Cursor { id: 3 }), "unknown fields are ignored");
    for bad in ["", "{}", "null", r#"{"id":-1}"#, r#"{"id":"x"}"#, "7", r#"{"offsets":{}}"#] {
        assert_eq!(Cursor::from_json(bad), None, "{bad}");
    }
}

#[test]
fn t_ses_14_probe_returns_the_row_count_and_max_id_and_maps_unreadable_dbs_to_reasons() {
    // Arrange
    let f = fx();
    for _ in 0..3 {
        f.add("create", "/data/incoming/n.mov");
    }
    f.conn.execute("DELETE FROM events WHERE id = 1", []).unwrap();
    // Act / Assert
    assert_eq!(probe(&mut f.fake.runner(), LOGICAL).unwrap(), Probe { rows: 2, max_id: 3 });
    // 설정한 경로에 파일이 없다 (종료 코드 11 에 해당)
    assert!(matches!(probe(&mut f.fake.runner(), "/srv/nowhere.db"), Err(RunError::DbUnreadable(_))));
    f.fake.set_fail(Some(RunError::NoSqlite));
    assert_eq!(probe(&mut f.fake.runner(), LOGICAL).unwrap_err(), RunError::NoSqlite);
    // sqlite3 가 아닌 것이 답했다
    struct Junk;
    impl RemoteRunner for Junk {
        fn run(&mut self, _: &str) -> Result<Vec<u8>, RunError> {
            Ok(b"hello\n".to_vec())
        }
    }
    assert!(matches!(probe(&mut Junk, LOGICAL), Err(RunError::Exec(_))));
    assert!(matches!(f.feed(Some(0)).poll(&mut Junk), Err(RunError::Exec(_))), "a poll also refuses output without the M line");
}

#[test]
fn t_ses_15_polling_never_changes_the_event_database() {
    // Arrange
    let f = fx();
    for _ in 0..5 {
        f.add("create", "/data/incoming/n.mov");
    }
    let before = (std::fs::read(&f.file).unwrap(), std::fs::metadata(&f.file).unwrap().modified().unwrap());
    // Act
    let mut feed = f.feed(None);
    let p = feed.poll(&mut f.fake.runner()).unwrap();
    feed.commit(p.next.unwrap());
    feed.poll(&mut f.fake.runner()).unwrap();
    let _ = probe(&mut f.fake.runner(), LOGICAL);
    // Assert
    let after = (std::fs::read(&f.file).unwrap(), std::fs::metadata(&f.file).unwrap().modified().unwrap());
    assert!(before == after, "events.db bytes and mtime unchanged");
}

#[test]
fn t_ses_16_the_fake_host_executes_the_production_sql_on_a_readonly_connection_so_a_write_would_fail() {
    // Arrange: 가짜가 읽기 전용 연결로 여는지 — 쓰기 SQL 은 실패해야 시험이 읽기 전용을 보증한다
    let f = fx();
    let mut run = f.fake.runner();
    // Act
    let r = run.run(&format!("[ -r '{LOGICAL}' ]\nsqlite3 -readonly -separator '|' '{LOGICAL}' \"DELETE FROM events;\""));
    // Assert
    assert!(matches!(r, Err(RunError::DbUnreadable(_))), "{r:?}");
}
