//! DDP(AVFS) 감사 로그 한 줄 해석. watch_folder `ddpaudit/parse.go` 의 규칙을 옮겼다 (코드를 그대로 옮기지 않았다).
//!
//! ```text
//! 2021-05-25 15:13:06.139037187  CREATE  iqn.mini-ibc-2  |/cp1.png
//! 2026-10-12 11:23:02.913798994  RENAME  iqn.win10test   |/New folder|/New folder/test
//! 2021-05-25 15:02:32.799029007  CONNECT iqn.mini-ibc-2  MOUNT volume 'foldervol2' to ...
//! ```
//!
//! 경로는 `|` 로 시작하고 RENAME 은 `|옛|새` 로 둘이 붙는다. 파일 이름에 공백이 들어갈 수 있어 앞 네 조각(날짜·시각·종류·주체)만
//! 공백으로 가르고 나머지는 통째로 둔다. 여기서 얻은 경로는 "그 경로를 다시 확인하라" 는 힌트일 뿐이다 — 실제 판정은 `apply_path` 가 한다.

use chrono::NaiveDateTime;

/// 감사 로그 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// DDP 가 찍은 시각 (시간대 없는 DDP 현지 시각 — 판정에 쓰지 않는다)
    pub time: NaiveDateTime,
    /// CREATE / WRITE / READ / RENAME / DELETE / CONNECT / DISCONNECT / INFO — 문서에 없는 값이 나올 수 있어 열거형으로 가두지 않는다
    pub kind: String,
    /// 작업한 데스크톱의 iqn 이름 (DISCONNECT 는 `------`)
    pub who: String,
    /// 볼륨 기준 상대 경로 (경로 없는 줄은 빈 문자열)
    pub path: String,
    /// RENAME 의 도착지 (그 외 빈 문자열)
    pub new_path: String,
}

const STAMP: &str = "%Y-%m-%d %H:%M:%S%.f";

/// 한 줄을 읽는다. 시각으로 시작하지 않으면(이어지는 줄·잡음) `None`.
pub fn parse_line(line: &str) -> Option<Event> {
    let s = line.trim_end_matches(['\r', '\n']);
    if s.trim().is_empty() {
        return None;
    }
    let (date, rest) = cut(s)?;
    let (clock, rest) = cut(rest)?;
    let time = NaiveDateTime::parse_from_str(&format!("{date} {clock}"), STAMP).ok()?;
    let (kind, rest) = cut(rest)?;
    let (who, rest) = cut(rest).unwrap_or(("", ""));
    let detail = rest.trim();
    let mut ev = Event {
        time,
        kind: kind.to_string(),
        who: who.to_string(),
        path: String::new(),
        new_path: String::new(),
    };
    if let Some(body) = detail.strip_prefix('|') {
        // 경로 안에 '|' 가 있으면 잘못 갈리지만 앞쪽은 멀쩡하다 — 힌트일 뿐이라 치명적이지 않다
        match body.split_once('|') {
            Some((a, b)) => {
                ev.path = a.trim().to_string();
                ev.new_path = b.strip_prefix('|').unwrap_or(b).trim().to_string();
            }
            None => ev.path = body.trim().to_string(),
        }
    }
    Some(ev)
}

/// 앞쪽 공백을 버리고 다음 공백까지를 떼어 낸다 (빈 입력은 `None`).
fn cut(s: &str) -> Option<(&str, &str)> {
    let s = s.trim_start_matches([' ', '\t']);
    if s.is_empty() {
        return None;
    }
    match s.find([' ', '\t']) {
        Some(i) => Some((&s[..i], &s[i + 1..])),
        None => Some((s, "")),
    }
}

impl Event {
    /// "무언가 생기거나 바뀌었다" — CREATE·WRITE·RENAME 만. READ 는 남이 보기만 해도 남고, DELETE 는 주기 재스캔의 장부가 맞춘다
    /// (watch_folder 와 같은 규칙. 대량 삭제 때 보고를 몰아 쓰지 않는다).
    pub fn is_change(&self) -> bool {
        ["CREATE", "WRITE", "RENAME"]
            .iter()
            .any(|k| self.kind.eq_ignore_ascii_case(k))
    }

    pub fn is_rename(&self) -> bool {
        self.kind.eq_ignore_ascii_case("RENAME")
    }

    /// 이 줄이 가리키는 볼륨 상대 경로들. RENAME 은 도착지가 중요하지만 출발지 폴더도 내용이 바뀌었으므로 둘 다 준다.
    pub fn targets(&self) -> Vec<&str> {
        [self.path.as_str(), self.new_path.as_str()]
            .into_iter()
            .filter(|p| !p.is_empty())
            .collect()
    }
}

/// 사람이 넣은 소재가 아닌 것이 뻔한 경로 — 숨김(`.`)·AVFS 내부(`^`)·휴지통·`System Volume Information` 이 한 칸이라도 있거나
/// 비었거나 `/` 로 끝나면(폴더) 잡음이다. 걸러도 감지에 손해가 없다.
pub fn is_noise(path: &str) -> bool {
    if path.trim().is_empty() {
        return true;
    }
    let p = path.replace('\\', "/");
    for seg in p.split('/').filter(|s| !s.is_empty()) {
        if seg.starts_with(['.', '^']) || seg == "$RECYCLE.BIN" || seg == "System Volume Information" {
            return true;
        }
    }
    path.ends_with('/')
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (01·03·05) / Boundary (02 공백이 든 이름, 06 빈 입력) / Validation (04 잡음·깨진 줄) / Business rule (07 깨우는 종류)
    //! watch_folder `parse_test.go` 의 입력·기대값 이식.
    use super::*;

    struct Case(&'static str, &'static str, &'static str, &'static str, &'static str, bool);

    #[test]
    fn t_ddp_01_documented_and_real_sample_lines_parse_into_kind_who_path_newpath() {
        // 벤더 문서 그림 1 + 실기 DDP 가 남긴 줄 (DDP_원격접속_스토리지모니터링.pdf 그림 2·4·6)
        let cases = [
            Case("2021-05-25 15:03:22.210453132  READ         iqn.mini-ibc-2  |/test1.mov", "READ", "iqn.mini-ibc-2", "/test1.mov", "", false),
            Case("2021-05-25 15:13:06.139037187  CREATE       iqn.mini-ibc-2  |/cp1.png", "CREATE", "iqn.mini-ibc-2", "/cp1.png", "", true),
            Case("2021-05-25 15:13:06.145854866  WRITE        iqn.mini-ibc-2  |/cp1.png", "WRITE", "iqn.mini-ibc-2", "/cp1.png", "", true),
            Case("2021-05-25 15:03:44.727854721  READ         iqn.mini-ibc-2  |/Protools/mov3.mov", "READ", "iqn.mini-ibc-2", "/Protools/mov3.mov", "", false),
            Case("2021-05-25 15:13:06.139037187  RENAME       iqn.mini-ibc-2  |/tmp/a.mxf|/1TV/a.mxf", "RENAME", "iqn.mini-ibc-2", "/tmp/a.mxf", "/1TV/a.mxf", true),
            Case("2021-05-25 15:02:32.799029007  CONNECT      iqn.mini-ibc-2  MOUNT volume 'foldervol2' to iqn.mini-ibc-2 (172.16.4.16) user 'test' conn_id 11", "CONNECT", "iqn.mini-ibc-2", "", "", false),
            Case("2021-05-25 15:01:50.779328107  DISCONNECT   ------          UNMOUNT volume 'foldervol2' from iqn.mini-ibc-2 (172.16.4.16) user 'test' conn_id 9", "DISCONNECT", "------", "", "", false),
            Case("2026-10-12 11:23:02.913798994  RENAME  iqn.win10test  |/New folder|/New folder/test", "RENAME", "iqn.win10test", "/New folder", "/New folder/test", true),
            Case("2026-10-12 13:01:50.916844350  DELETE  iqn.win10test  |/blue.png", "DELETE", "iqn.win10test", "/blue.png", "", false),
        ];
        for Case(line, kind, who, path, new_path, change) in cases {
            let ev = parse_line(line).unwrap_or_else(|| panic!("not parsed: {line}"));
            assert_eq!((ev.kind.as_str(), ev.who.as_str(), ev.path.as_str(), ev.new_path.as_str()), (kind, who, path, new_path), "{line}");
            assert_eq!(ev.is_change(), change, "{line}");
        }
    }

    #[test]
    fn t_ddp_02_names_with_spaces_are_kept_whole() {
        let ev = parse_line("2026-10-23 09:30:00.123456789  CREATE       iqn.kbs-01     |/1TV/뉴스 특보 001.mxf").unwrap();
        assert_eq!(ev.path, "/1TV/뉴스 특보 001.mxf");
        let ev = parse_line("2026-10-12 11:25:33.603700762  CREATE  iqn.win10test  |/New folder (2)").unwrap();
        assert_eq!(ev.path, "/New folder (2)");
    }

    #[test]
    fn t_ddp_03_time_is_read_with_nanosecond_precision() {
        let ev = parse_line("2021-05-25 15:13:06.139037187  CREATE  iqn.x  |/a.png").unwrap();
        assert_eq!(ev.time.format("%Y-%m-%d %H:%M:%S%.9f").to_string(), "2021-05-25 15:13:06.139037187");
    }

    #[test]
    fn t_ddp_04_empty_blank_noise_and_broken_lines_are_not_events() {
        for bad in [
            "",
            "   ",
            "waiting for data...(interrupt to abort)",
            "2021-05-25",
            "2021-05-25 15:03:44.43.727854721 READ iqn.x |/a.mov",
            "2021-13-45 15:03:44.1 READ iqn.x |/a.mov",
        ] {
            assert!(parse_line(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn t_ddp_05_targets_give_both_ends_of_a_rename_and_none_for_pathless_lines() {
        let ev = parse_line("2021-05-25 15:13:06.139037187  RENAME  iqn.x  |/a.mxf|/b.mxf").unwrap();
        assert_eq!(ev.targets(), vec!["/a.mxf", "/b.mxf"]);
        let ev = parse_line("2021-05-25 15:02:32.799029007  CONNECT  iqn.x  MOUNT volume 'v'").unwrap();
        assert!(ev.targets().is_empty());
    }

    #[test]
    fn t_ddp_06_noise_paths_are_hidden_internal_trash_folder_or_empty() {
        for p in ["/.DS_Store", "/1TV/._a.mxf", "/^51647.fseventsd/fseventsd-uuid", "/.fseventsd/0001", "/1TV/.Trashes/x.mxf", "", "  ", "/1TV/", "/$RECYCLE.BIN/x", "/System Volume Information/y"] {
            assert!(is_noise(p), "{p:?}");
        }
        for p in ["/a.mxf", "/1TV/2026/뉴스 001.mxf", "/Protools/mov3.mov", "/New folder"] {
            assert!(!is_noise(p), "{p:?}");
        }
    }

    #[test]
    fn t_ddp_07_only_create_write_rename_wake_and_kind_case_is_ignored() {
        let mk = |k: &str| parse_line(&format!("2021-05-25 15:13:06.1  {k}  iqn.x  |/a")).unwrap();
        for k in ["CREATE", "write", "Rename"] {
            assert!(mk(k).is_change(), "{k}");
        }
        for k in ["READ", "DELETE", "CONNECT", "INFO", "UNKNOWN_FUTURE_KIND"] {
            assert!(!mk(k).is_change(), "{k}");
        }
    }
}
