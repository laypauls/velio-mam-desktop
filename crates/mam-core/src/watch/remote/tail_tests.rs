//! 분류: Happy (01·02) / Boundary (03 로테이션·05 부분 줄·06 줄어듦·07 1 MiB·09 예산) / Validation (08 이름·10 빈 폴더) /
//! Business rule (04 첫 기준선) / Concurrency (02 중복 없음). watch_folder `source_test.go` 이식.
use super::*;
use crate::watch::remote::fake::{FakeDdp, line};

fn tail(_f: &FakeDdp, cur: Option<Cursor>) -> LogTail {
    LogTail::new("/tmp/audit", cur)
}

fn read(t: &mut LogTail, f: &FakeDdp) -> Polled {
    let mut r = f.runner();
    let p = t.poll(&mut r).unwrap();
    t.commit(p.next.clone());
    p
}

fn primed(f: &FakeDdp) -> LogTail {
    let mut t = tail(f, None);
    read(&mut t, f);
    t
}

#[test]
fn t_remote_tail_01_first_poll_sets_the_baseline_without_replaying_history_and_flags_a_gap() {
    let f = FakeDdp::new("/tmp/audit");
    f.write(
        "audit-log.0",
        &(line("CREATE", "/old1.mxf") + &line("CREATE", "/old2.mxf")),
    );
    let mut t = tail(&f, None);
    let p = read(&mut t, &f);
    assert!(p.lines.is_empty() && p.gap);
    f.write("audit-log.0", &line("CREATE", "/new.mxf"));
    let p = read(&mut t, &f);
    assert_eq!(p.lines.len(), 1);
    assert!(p.lines[0].contains("/new.mxf") && !p.gap);
}

#[test]
fn t_remote_tail_02_the_same_line_is_never_read_twice() {
    let f = FakeDdp::new("/tmp/audit");
    f.write("audit-log.0", "");
    let mut t = primed(&f);
    f.write("audit-log.0", &line("CREATE", "/a.mxf"));
    assert_eq!(read(&mut t, &f).lines.len(), 1);
    assert!(read(&mut t, &f).lines.is_empty());
    assert!(read(&mut t, &f).lines.is_empty());
}

#[test]
fn t_remote_tail_03_a_rotated_in_file_is_read_from_the_start_and_removed_files_are_forgotten() {
    let f = FakeDdp::new("/tmp/audit");
    f.write("audit-log.0", &line("CREATE", "/a.mxf"));
    f.write("audit-log.9", "");
    let mut t = primed(&f);
    f.write(
        "audit-log.10",
        &(line("CREATE", "/b.mxf") + &line("WRITE", "/c.mxf")),
    );
    let p = read(&mut t, &f);
    assert_eq!(p.lines.len(), 2, "new file read from byte 0");
    f.remove("audit-log.0");
    read(&mut t, &f);
    assert!(!t.cursor().offsets.contains_key("audit-log.0"));
    assert_eq!(t.cursor().offsets.len(), 2);
}

#[test]
fn t_remote_tail_04_resuming_from_a_saved_cursor_reads_what_accumulated_while_the_app_was_off() {
    let f = FakeDdp::new("/tmp/audit");
    f.write("audit-log.0", &line("CREATE", "/seen.mxf"));
    let t = primed(&f);
    let saved = t.cursor().to_json();
    drop(t);
    f.write("audit-log.0", &line("CREATE", "/while-off.mxf"));
    let mut t2 = tail(&f, Cursor::from_json(&saved));
    let p = read(&mut t2, &f);
    assert_eq!(p.lines.len(), 1);
    assert!(p.lines[0].contains("/while-off.mxf") && !p.gap, "{p:?}");
    // 깨진 저장값은 None → 처음처럼 기준선
    assert_eq!(Cursor::from_json("not json"), None);
}

#[test]
fn t_remote_tail_05_an_unfinished_line_waits_for_its_newline() {
    let f = FakeDdp::new("/tmp/audit");
    f.write("audit-log.0", "");
    let mut t = primed(&f);
    let full = line("CREATE", "/긴 이름 파일.mxf");
    let cut = full.char_indices().nth(20).unwrap().0;
    f.write("audit-log.0", &full[..cut]);
    assert!(read(&mut t, &f).lines.is_empty());
    f.write("audit-log.0", &full[cut..]);
    let p = read(&mut t, &f);
    assert_eq!(p.lines.len(), 1);
    assert!(p.lines[0].contains("/긴 이름 파일.mxf"));
}

#[test]
fn t_remote_tail_06_a_shrunk_file_is_read_again_from_the_start_and_flags_a_gap() {
    let f = FakeDdp::new("/tmp/audit");
    f.write(
        "audit-log.0",
        &(line("CREATE", "/a.mxf") + &line("CREATE", "/b.mxf")),
    );
    let mut t = primed(&f);
    f.set("audit-log.0", &line("CREATE", "/c.mxf"));
    let p = read(&mut t, &f);
    assert!(p.gap);
    assert_eq!(p.lines.len(), 1);
    assert!(p.lines[0].contains("/c.mxf"));
}

#[test]
fn t_remote_tail_07_more_than_1mib_in_one_read_skips_to_the_end_and_flags_a_gap() {
    let f = FakeDdp::new("/tmp/audit");
    f.write("audit-log.0", "");
    let mut t = primed(&f);
    f.write(
        "audit-log.0",
        &("x".repeat(MAX_FETCH as usize + 1) + "\n" + &line("CREATE", "/a.mxf")),
    );
    let p = read(&mut t, &f);
    assert!(
        p.lines.is_empty() && p.gap && p.skipped_bytes > MAX_FETCH,
        "{p:?}"
    );
    // 건너뛴 뒤에는 다시 이어 읽는다
    f.write("audit-log.0", &line("CREATE", "/b.mxf"));
    let p = read(&mut t, &f);
    assert!(!p.gap && p.lines.len() == 1 && p.lines[0].contains("/b.mxf"));
    // 경계: 정확히 1 MiB 는 읽는다
    f.write("audit-log.0", &"y".repeat(MAX_FETCH as usize - 1));
    f.write("audit-log.0", "\n");
    assert!(!read(&mut t, &f).gap);
}

#[test]
fn t_remote_tail_08_unsafe_file_names_from_the_remote_are_ignored() {
    let f = FakeDdp::new("/tmp/audit");
    f.write("audit-log.0", "");
    f.raw_listing_extra("5 audit-log.0; rm -rf ~\n5 audit-log.1/../../etc\n5 other.txt\n");
    let mut r = f.runner();
    let sizes = list_sizes(&mut r, "/tmp/audit").unwrap();
    assert_eq!(sizes.keys().collect::<Vec<_>>(), vec!["audit-log.0"]);
}

#[test]
fn t_remote_tail_09_a_huge_backlog_is_spread_over_polls_by_the_per_poll_budget() {
    let f = FakeDdp::new("/tmp/audit");
    f.write("audit-log.0", "");
    let mut t = primed(&f);
    let big = |tag: &str| {
        format!(
            "{}{}",
            line("CREATE", tag),
            "z".repeat(MAX_FETCH as usize - 200)
        ) + "\n"
    };
    for i in 1..=12 {
        f.write(&format!("audit-log.{i}"), &big(&format!("/f{i}.mxf")));
    }
    let first = read(&mut t, &f);
    assert!(
        !first.lines.is_empty() && first.lines.len() < 24,
        "{}",
        first.lines.len()
    );
    let mut total = first.lines.len();
    for _ in 0..4 {
        total += read(&mut t, &f).lines.len();
    }
    assert_eq!(total, 24, "every line is eventually read exactly once");
}

#[test]
fn t_remote_tail_10_empty_or_missing_log_dir_are_distinct_errors() {
    let f = FakeDdp::new("/tmp/audit");
    let t = tail(&f, None);
    assert_eq!(t.poll(&mut f.runner()).unwrap_err(), RunError::NoAuditFiles);
    f.set_dir_missing(true);
    assert_eq!(t.poll(&mut f.runner()).unwrap_err(), RunError::NoLogDir);
}

#[test]
fn t_remote_tail_11_log_order_is_numeric_and_quoting_survives_spaces_and_quotes() {
    let mut v = vec!["audit-log.10", "audit-log.2", "audit-log.0", "audit-log.x"];
    v.sort_by(|a, b| log_cmp(a, b));
    assert_eq!(
        v,
        vec!["audit-log.0", "audit-log.2", "audit-log.10", "audit-log.x"]
    );
    assert_eq!(cmd::sh_quote("/tmp/감사 로그"), "'/tmp/감사 로그'");
    assert_eq!(cmd::sh_quote("/tmp/a'b"), r"'/tmp/a'\''b'");
}

#[test]
fn t_remote_tail_12_only_read_commands_are_ever_built() {
    for c in [
        cmd::list_sizes("/tmp/a b"),
        cmd::tail_from("/tmp/a", "audit-log.0", 5),
    ] {
        for banned in [">", "rm ", "mv ", "cp ", "chmod", "tee", "dd ", "sudo"] {
            assert!(
                !c.replace("2>/dev/null", "").contains(banned),
                "{banned} in {c}"
            );
        }
    }
    assert!(cmd::tail_from("/tmp/a", "audit-log.0", 5).starts_with("tail -c +6 "));
}
