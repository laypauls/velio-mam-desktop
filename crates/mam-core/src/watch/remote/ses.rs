//! storage-event-service(SES) 큐 읽기 — 리눅스 스토리지 호스트의 `events` 테이블(SQLite)을 **읽기 전용**으로 `id` 커서로 이어 읽는다 (`fanotify_feed`).
//!
//! 원격에 보내는 명령은 `sqlite3 -readonly <db> "SELECT …"` 하나뿐이다 (SES 저장소는 수정하지 않는다).
//!
//! **출력 형식**: `sqlite3 -separator '|'` + **텍스트 열을 SQL 의 `hex()` 로 내보낸다**. 경로·이름은 어떤 문자(`|`·줄바꿈·따옴표)도 들어갈 수 있어
//! 구분자 방식은 모호하고, `-json` 은 sqlite3 3.33 이상이 필요하다. hex 는 ASCII 만 나와 구분자·줄바꿈과 겹칠 수 없고 어떤 sqlite3 버전에서도 같다
//! (그래서 버전 감지·대체 경로가 없다 — 경로가 하나뿐이라 시험이 실제로 도는 경로를 전부 덮는다). 대가는 출력 2 배 → 한 번에 가져올 행 수를
//! `MAX_ROWS` 에서 시작해 출력이 너무 크면 절반으로 줄인다.
//!
//! **커서 규칙**: 커서 = 마지막으로 반영한 `id` (`{"id": N}`, `watch_cursors`). 처음(커서 없음)은 과거를 읽지 않고 현재 `max(id)` 를 기준선으로 삼는다.
//! `max(id) < 커서`(큐 초기화) · 커서 행이 사라짐 · 밀린 행이 `BACKLOG_MAX` 초과는 놓쳤을 수 있다는 뜻이라 호출자가 재스캔한다.

use serde::{Deserialize, Serialize};

use super::runner::{RemoteRunner, RunError};
use super::tail::cmd::sh_quote;

/// 한 번에 가져올 최대 행 수
pub const MAX_ROWS: u32 = 1000;
const MIN_ROWS: u32 = 10;
/// 커서보다 이만큼 넘게 밀렸으면 읽지 않고 끝으로 건너뛴다 (재스캔이 만회)
pub const BACKLOG_MAX: i64 = 50_000;
const MAX_PATH_BYTES: usize = 4096;

/// 원격 호스트의 DB 경로로 쓸 수 있는가 — 절대 경로, 제어 문자·셸 메타 문자·따옴표·`..` 거부. 홑따옴표로 한 번 더 감싸지만 그것에만 기대지 않는다.
pub fn valid_db_path(p: &str) -> bool {
    p.starts_with('/')
        && !p.ends_with('/')
        && p.len() <= 512
        && !p
            .chars()
            .any(|c| c.is_control() || "'\"`$\\;&|<>(){}[]*?!#~^".contains(c))
        && !p.split('/').any(|s| s == "..")
}

/// 원격에 보내는 명령 — 읽기 전용. 종료 코드: 10 = sqlite3 없음, 11 = DB 파일을 읽을 수 없음, 12 = sqlite3 실패(잠김·테이블 없음 …).
pub(super) mod cmd {
    use super::sh_quote;

    fn wrap(db: &str, sql: &str) -> String {
        let db = sh_quote(db);
        format!(
            "command -v sqlite3 >/dev/null 2>&1 || exit 10\n[ -r {db} ] || exit 11\nsqlite3 -readonly -separator '|' {db} \"{sql}\" || exit 12"
        )
    }

    /// 커서 뒤의 행. `after`·`limit` 는 정수라 셸에 실려도 안전하다. 첫 줄 `M` = max(id), `C` = 커서 행 존재, `R` = 행(텍스트는 hex)
    pub fn read(db: &str, after: i64, limit: u32) -> String {
        wrap(
            db,
            &format!(
                "SELECT 'M', COALESCE(MAX(id), 0) FROM events; \
                 SELECT 'C', EXISTS(SELECT 1 FROM events WHERE id = {after}); \
                 SELECT 'R', id, hex(event_types), hex(path), hex(name), hex(dir), is_dir FROM events WHERE id > {after} ORDER BY id LIMIT {limit};"
            ),
        )
    }

    /// 접속 시험 — 행 수와 최대 id
    pub fn probe(db: &str) -> String {
        wrap(
            db,
            "SELECT 'P', COUNT(*), COALESCE(MAX(id), 0) FROM events;",
        )
    }
}

/// 영속 커서 (`watch_cursors.cursor`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub id: i64,
}

impl Cursor {
    pub fn to_json(self) -> String {
        serde_json::to_string(&self).unwrap_or_else(|_| r#"{"id":0}"#.into())
    }

    /// 해석할 수 없거나 음수면 `None` — 처음처럼 현재 끝에서 다시 기준선을 잡는다 (재스캔이 만회)
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str::<Self>(s).ok().filter(|c| c.id >= 0)
    }
}

/// `events` 한 행 (필요한 열만)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: i64,
    pub types: Vec<String>,
    /// 컨테이너 기준 전체 경로 (`path`, 비어 있으면 `dir/name`)
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Page {
    max_id: i64,
    cursor_row: bool,
    rows: Vec<Row>,
    /// 읽은 마지막 행의 id (해석하지 못한 행 포함 — 그 행을 다시 읽지 않게 커서는 여기까지 올린다)
    last_id: i64,
    /// 해석하지 못한 행
    bad: u64,
}

fn unhex(s: &str) -> Option<String> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

fn parse_row(f: &[&str]) -> Option<Row> {
    let [id, types, path, name, dir, is_dir] = f else {
        return None;
    };
    let (path, name, dir) = (unhex(path)?, unhex(name)?, unhex(dir)?);
    let path = if !path.is_empty() {
        path
    } else if !name.is_empty() {
        format!("{}/{name}", dir.trim_end_matches('/'))
    } else {
        return None; // 경로를 알 수 없다 (CAP_DAC_READ_SEARCH 없이 쌓인 행)
    };
    if path.len() > MAX_PATH_BYTES {
        return None;
    }
    Some(Row {
        id: id.parse().ok()?,
        types: unhex(types)?
            .split(',')
            .map(|t| t.trim().to_string())
            .collect(),
        path,
        is_dir: *is_dir != "0",
    })
}

fn parse_page(out: &[u8]) -> Result<Page, RunError> {
    let text = String::from_utf8_lossy(out);
    let (mut p, mut got_max) = (Page::default(), false);
    for line in text.lines() {
        let f: Vec<&str> = line.trim_end_matches('\r').split('|').collect();
        match f.as_slice() {
            ["M", m] => {
                (p.max_id, got_max) = (
                    m.parse()
                        .map_err(|_| RunError::Exec("unexpected sqlite3 output".into()))?,
                    true,
                )
            }
            ["C", c] => p.cursor_row = *c == "1",
            ["R", rest @ ..] => {
                p.last_id = p
                    .last_id
                    .max(rest.first().and_then(|i| i.parse().ok()).unwrap_or(0));
                match parse_row(rest) {
                    Some(r) => p.rows.push(r),
                    None => p.bad += 1,
                }
            }
            _ => {}
        }
    }
    // `M` 은 행이 없어도 항상 한 줄 나온다 — 없으면 sqlite3 가 아닌 것이 답한 것이다
    if got_max {
        Ok(p)
    } else {
        Err(RunError::Exec("unexpected sqlite3 output".into()))
    }
}

/// 접속 시험 결과 — 이벤트 수와 최대 id
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Probe {
    pub rows: u64,
    pub max_id: i64,
}

/// `sqlite3` 존재·DB 읽기 가능 여부를 확인하고 행 수·최대 id 를 돌려준다 (접속 시험).
pub fn probe(run: &mut dyn RemoteRunner, db: &str) -> Result<Probe, RunError> {
    let out = run.run(&cmd::probe(db))?;
    let text = String::from_utf8_lossy(&out);
    text.lines()
        .find_map(|l| {
            match l
                .trim_end_matches('\r')
                .split('|')
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["P", n, m] => Some(Probe {
                    rows: n.parse().ok()?,
                    max_id: m.parse().ok()?,
                }),
                _ => None,
            }
        })
        .ok_or_else(|| RunError::Exec("unexpected sqlite3 output".into()))
}

/// 한 번 읽은 결과 — 상태를 바꾸지 않았다. 호출자가 기록(대기열 + 커서)에 성공한 뒤에만 [`SesFeed::commit`] 한다.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Polled {
    pub rows: Vec<Row>,
    pub next: Option<Cursor>,
    /// 놓쳤을 수 있다 — 첫 기준선·큐 초기화·커서 행 사라짐·건너뜀. 호출자가 재스캔한다
    pub gap: bool,
    pub skipped_rows: u64,
    pub bad: u64,
    pub reset: bool,
}

pub struct SesFeed {
    db: String,
    cursor: Option<Cursor>,
    limit: u32,
}

impl SesFeed {
    pub fn new(db: &str, resumed: Option<Cursor>) -> Self {
        Self {
            db: db.to_string(),
            cursor: resumed,
            limit: MAX_ROWS,
        }
    }

    pub fn cursor(&self) -> Option<Cursor> {
        self.cursor
    }

    pub fn db(&self) -> &str {
        &self.db
    }

    pub fn commit(&mut self, next: Cursor) {
        self.cursor = Some(next);
    }

    /// 읽기 한 번. 출력이 너무 커서 실패하면 다음 주기부터 행 수를 줄인다 (성공하면 다시 늘린다).
    pub fn poll(&mut self, run: &mut dyn RemoteRunner) -> Result<Polled, RunError> {
        let after = self.cursor.map_or(0, |c| c.id);
        let out = match run.run(&cmd::read(&self.db, after, self.limit)) {
            Ok(o) => o,
            Err(RunError::Exec(m)) if m.contains("output too large") && self.limit > MIN_ROWS => {
                self.limit = (self.limit / 2).max(MIN_ROWS);
                return Err(RunError::Exec(m));
            }
            Err(e) => return Err(e),
        };
        self.limit = (self.limit * 2).min(MAX_ROWS);
        let page = parse_page(&out)?;
        let mut p = Polled {
            bad: page.bad,
            ..Polled::default()
        };
        let Some(cur) = self.cursor else {
            // 첫 폴링: 지금까지의 행은 읽지 않는다 (앱을 처음 켤 때 과거 전체가 "방금 바뀜" 으로 쏟아진다) — 공백은 재스캔이 맞춘다
            p.next = Some(Cursor { id: page.max_id });
            p.gap = true;
            return Ok(p);
        };
        if page.max_id < cur.id {
            // 큐 초기화(DB 재생성) — 이전 id 는 의미가 없다. 현재 끝으로 되돌리고 재스캔
            p.next = Some(Cursor { id: page.max_id });
            (p.gap, p.reset) = (true, true);
        } else if page.max_id - cur.id > BACKLOG_MAX {
            p.next = Some(Cursor { id: page.max_id });
            p.gap = true;
            p.skipped_rows = (page.max_id - cur.id) as u64;
        } else {
            p.next = Some(Cursor {
                id: page.last_id.max(cur.id),
            });
            p.gap = cur.id > 0 && !page.cursor_row;
            p.rows = page.rows;
        }
        Ok(p)
    }
}

/// 이벤트 종류 -> 보고 종류. `None` = 무시 (`modify`·`open` 등 — SES 기본 수집 대상이 아니다).
/// 새 하위 폴더(`create`·`moved_to` + `is_dir`)는 하위 순회(`Subtree`), 나머지 5종은 경로 1건(실제 상태는 `apply_path` 가 판단).
pub fn classify(row: &Row) -> Option<bool> {
    let has = |t: &str| row.types.iter().any(|x| x == t);
    if row.is_dir && (has("create") || has("moved_to")) {
        return Some(true);
    }
    ["create", "close_write", "moved_to", "moved_from", "delete"]
        .iter()
        .any(|t| has(t))
        .then_some(false)
}

#[cfg(test)]
#[path = "ses_tests.rs"]
mod tests;
