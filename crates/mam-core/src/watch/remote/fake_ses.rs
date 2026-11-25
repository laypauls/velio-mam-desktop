//! 시험용 가짜 SES 호스트 — 진짜 SQLite 파일(SES `events` 스키마)에 대고 수집기가 보낸 **같은 SQL** 을 읽기 전용 연결로 실행한다.
//! `sqlite3 -readonly -separator '|'` 의 출력(NULL = 빈 칸, 열은 `|` 로 이음)을 흉내 낸다. SSH·네트워크 없음 (실호출 금지 정책).
//! 통합 시험(`tests/`)과 `src-tauri` 시험도 쓴다.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};

use super::runner::{RemoteRunner, RunError};

/// storage-event-service `internal/queue/sqlite.go`(커밋 c1d4e76) 의 `events` 스키마 그대로
pub const SES_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    captured_at     TEXT    NOT NULL,
    event_mask      INTEGER NOT NULL,
    event_types     TEXT    NOT NULL,
    path            TEXT,
    name            TEXT,
    dir             TEXT,
    is_dir          INTEGER NOT NULL DEFAULT 0,
    pid             INTEGER,
    status          TEXT    NOT NULL DEFAULT 'pending',
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    updated_at      TEXT,
    session_id      TEXT,
    ingest_job_id   TEXT,
    delivered_at    TEXT,
    next_attempt_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_events_status ON events(status, id);
";

/// SES 가 하는 것처럼 이벤트 1건을 쓴다 (`path`·`name`·`dir` 는 컨테이너 경로에서 나눈다). 만든 행의 id 를 돌려준다.
pub fn insert_event(conn: &Connection, types: &str, path: &str, is_dir: bool) -> rusqlite::Result<i64> {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    conn.execute(
        "INSERT INTO events (captured_at, event_mask, event_types, path, name, dir, is_dir, status) VALUES ('2026-11-22T00:00:00Z', 0, ?1, ?2, ?3, ?4, ?5, 'pending')",
        rusqlite::params![types, path, name, dir, i64::from(is_dir)],
    )?;
    Ok(conn.last_insert_rowid())
}

#[derive(Default)]
struct State {
    file: PathBuf,
    logical: String,
    fail: Option<RunError>,
    commands: Vec<String>,
    max_output: usize,
}

/// 가짜 호스트 하나. 복제해도 같은 상태를 가리킨다.
#[derive(Clone)]
pub struct FakeSes(Arc<Mutex<State>>);

impl FakeSes {
    /// `file` = 실제 SQLite 파일(스키마는 호출자가 `SES_SCHEMA` 로 만든다), `logical` = 사용자가 설정한 원격 DB 경로
    pub fn new(file: &Path, logical: &str) -> Self {
        Self(Arc::new(Mutex::new(State { file: file.to_path_buf(), logical: logical.into(), max_output: 4 * 1024 * 1024, ..State::default() })))
    }

    fn st(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 설정하면 모든 명령이 이 오류로 실패한다.
    pub fn set_fail(&self, e: Option<RunError>) {
        self.st().fail = e;
    }

    /// 명령 출력 상한 (실제 SSH 는 4 MiB)
    pub fn set_max_output(&self, n: usize) {
        self.st().max_output = n;
    }

    /// 지금까지 받은 명령 전문
    pub fn commands(&self) -> Vec<String> {
        self.st().commands.clone()
    }

    pub fn runner(&self) -> FakeSesRunner {
        FakeSesRunner(self.clone())
    }
}

pub struct FakeSesRunner(FakeSes);

/// 명령에서 홑따옴표로 감싼 DB 경로(`[ -r '…' ]`)
fn quoted_db(cmd: &str) -> Option<&str> {
    let rest = cmd.split_once("[ -r '")?.1;
    Some(rest.split_once("' ]")?.0)
}

fn sql_of(cmd: &str) -> Option<&str> {
    let rest = cmd.split_once('"')?.1;
    Some(rest.split_once('"')?.0)
}

fn render(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Blob(_) => String::new(),
    }
}

fn run_sql(file: &Path, sql: &str) -> Result<String, RunError> {
    let conn = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| RunError::DbUnreadable(e.to_string()))?;
    let mut out = String::new();
    for stmt in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        let mut st = conn.prepare(stmt).map_err(|e| RunError::DbUnreadable(e.to_string()))?;
        let n = st.column_count();
        let mut rows = st.query([]).map_err(|e| RunError::DbUnreadable(e.to_string()))?;
        while let Some(r) = rows.next().map_err(|e| RunError::DbUnreadable(e.to_string()))? {
            let cols: Vec<String> = (0..n).map(|i| r.get_ref(i).map(render).unwrap_or_default()).collect();
            out.push_str(&cols.join("|"));
            out.push('\n');
        }
    }
    Ok(out)
}

impl RemoteRunner for FakeSesRunner {
    fn run(&mut self, cmd: &str) -> Result<Vec<u8>, RunError> {
        let mut st = self.0.st();
        st.commands.push(cmd.to_string());
        if let Some(e) = st.fail.clone() {
            return Err(e);
        }
        if quoted_db(cmd) != Some(st.logical.as_str()) {
            return Err(RunError::DbUnreadable(String::new())); // 설정한 경로가 아닌 곳 = 파일 없음 (종료 코드 11)
        }
        let sql = sql_of(cmd).ok_or_else(|| RunError::Exec(format!("unknown command: {cmd:?}")))?;
        let out = run_sql(&st.file, sql)?;
        if out.len() > st.max_output {
            return Err(RunError::Exec("output too large".into()));
        }
        Ok(out.into_bytes())
    }
}
