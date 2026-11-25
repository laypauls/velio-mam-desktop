//! `remote_sources`·`watch_cursors` 저장소 — 접속 CRUD, 비밀 암호화(DPAPI), 커서. SQL 은 호출자가 쥔 연결/트랜잭션에서만 실행한다.
//!
//! 비밀(비밀번호·개인 키 PEM)은 `SecretBox` 로 감싼 blob 으로만 저장하고, 읽기 모델(`SourceView`)에는 `has_secret` 만 싣는다.
//! 에러 값에는 입력 값을 담지 않는다 (필드 이름만).

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::mapper::{Mapping, is_absolute};
use super::ses::valid_db_path;
use super::{Auth, RemoteSource, SourceKind};
use crate::secret::{Secret, SecretBox, SecretError};

pub const MAX_PASSWORD_LEN: usize = 1024;
pub const MAX_KEY_LEN: usize = 16 * 1024;
const MAX_RULES: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// 값이 규칙에 맞지 않는다 (필드 이름만 — 값은 싣지 않는다)
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("remote source not found")]
    NotFound,
    #[error("a remote source with this name already exists")]
    NameTaken,
    /// 감시 폴더가 이 접속을 쓰고 있다
    #[error("remote source is used by a folder")]
    InUse,
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

/// 생성·수정 입력 (비밀은 따로 받는다).
#[derive(Debug, Clone)]
pub struct NewSource {
    pub kind: SourceKind,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub host_key: Option<String>,
    pub log_dir: Option<String>,
    pub db_path: Option<String>,
    pub poll_sec: u32,
    pub path_map: Vec<Mapping>,
}

/// IPC 읽기 모델 — **비밀 없음**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceView {
    pub source_id: i64,
    pub kind: &'static str,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_kind: &'static str,
    pub has_secret: bool,
    pub host_key: Option<String>,
    pub log_dir: Option<String>,
    pub db_path: Option<String>,
    pub poll_sec: u32,
    pub path_map: Vec<Mapping>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn no_control(s: &str) -> bool {
    !s.chars().any(char::is_control)
}

/// `SHA256:` + 43자 base64(무패딩) — OpenSSH 지문 형식
pub fn valid_fingerprint(s: &str) -> bool {
    s.strip_prefix("SHA256:")
        .is_some_and(|b| b.len() == 43 && b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/'))
}

/// 입력 검증 — 저장·접속 시험 공용.
pub fn validate(i: &NewSource) -> Result<(), StoreError> {
    let ok = |c: bool, f: &'static str| if c { Ok(()) } else { Err(StoreError::Invalid(f)) };
    ok((1..=100).contains(&i.name.trim().chars().count()) && no_control(&i.name), "name")?;
    ok(!i.host.is_empty() && i.host.len() <= 253 && !i.host.chars().any(|c| c.is_whitespace() || c.is_control()), "host")?;
    ok(i.port != 0, "port")?;
    ok((1..=128).contains(&i.user.len()) && no_control(&i.user), "user")?;
    ok((1..=3600).contains(&i.poll_sec), "pollSec")?;
    ok(i.host_key.as_deref().is_none_or(valid_fingerprint), "hostKey")?;
    ok(
        i.log_dir.as_deref().is_none_or(|d| d.starts_with('/') && d.len() <= 512 && no_control(d)),
        "logDir",
    )?;
    // 종류마다 쓰는 필드가 다르다 — `ddp_audit` = 로그 폴더(선택), `ses_queue` = 이벤트 DB 경로(필수, 셸에 실리므로 엄격히 검증)
    match i.kind {
        SourceKind::DdpAudit => ok(i.db_path.is_none(), "dbPath")?,
        SourceKind::SesQueue => {
            ok(i.log_dir.is_none(), "logDir")?;
            ok(i.db_path.as_deref().is_some_and(valid_db_path), "dbPath")?;
        }
    }
    ok(
        (1..=MAX_RULES).contains(&i.path_map.len())
            && i.path_map.iter().all(|r| r.from.len() <= 512 && no_control(&r.from) && r.to.len() <= 512 && is_absolute(&r.to)),
        "pathMap",
    )
}

pub fn validate_auth(a: &Auth) -> Result<(), StoreError> {
    let (s, max) = match a {
        Auth::Password(s) => (s, MAX_PASSWORD_LEN),
        Auth::Key(s) => (s, MAX_KEY_LEN),
    };
    if s.is_empty() || s.expose().len() > max {
        return Err(StoreError::Invalid("auth"));
    }
    Ok(())
}

fn map_unique(e: rusqlite::Error) -> StoreError {
    match &e {
        rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::ConstraintViolation => StoreError::NameTaken,
        _ => StoreError::Db(e),
    }
}

fn rules_json(rules: &[Mapping]) -> String {
    serde_json::to_string(rules).unwrap_or_else(|_| "[]".into())
}

pub fn create(conn: &Connection, i: &NewSource, auth: &Auth, secrets: &dyn SecretBox, now: i64) -> Result<i64, StoreError> {
    validate(i)?;
    validate_auth(auth)?;
    let blob = secrets.seal(auth.secret().expose().as_bytes())?;
    conn.execute(
        "INSERT INTO remote_sources (kind, name, host, port, user, auth_kind, auth_blob, host_key, log_dir, db_path, poll_sec, path_map_json, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
        params![i.kind.as_str(), i.name.trim(), i.host, i.port, i.user, auth.kind_str(), blob, i.host_key, i.log_dir, i.db_path, i.poll_sec, rules_json(&i.path_map), now],
    )
    .map_err(map_unique)?;
    Ok(conn.last_insert_rowid())
}

/// 수정. `auth` 가 `None` 이면 저장된 비밀을 그대로 둔다.
pub fn update(conn: &Connection, id: i64, i: &NewSource, auth: Option<&Auth>, secrets: &dyn SecretBox, now: i64) -> Result<(), StoreError> {
    validate(i)?;
    // 종류는 바꿀 수 없다 (쓰는 필드·수집기가 다르다)
    match kind_of(conn, id) {
        None => return Err(StoreError::NotFound),
        Some(k) if k != i.kind => return Err(StoreError::Invalid("kind")),
        Some(_) => {}
    }
    let n = conn
        .execute(
            "UPDATE remote_sources SET name = ?2, host = ?3, port = ?4, user = ?5, host_key = ?6, log_dir = ?7, db_path = ?8, poll_sec = ?9,
                    path_map_json = ?10, updated_at = ?11 WHERE source_id = ?1",
            params![id, i.name.trim(), i.host, i.port, i.user, i.host_key, i.log_dir, i.db_path, i.poll_sec, rules_json(&i.path_map), now],
        )
        .map_err(map_unique)?;
    if n == 0 {
        return Err(StoreError::NotFound);
    }
    if let Some(a) = auth {
        validate_auth(a)?;
        let blob = secrets.seal(a.secret().expose().as_bytes())?;
        conn.execute("UPDATE remote_sources SET auth_kind = ?2, auth_blob = ?3 WHERE source_id = ?1", params![id, a.kind_str(), blob])?;
    }
    Ok(())
}

/// 이 접속을 쓰는 감시 폴더 ID 들 (`watch_options` 의 `source`). 깨진 JSON 은 건너뛴다.
pub fn referencing_folders(conn: &Connection, id: i64) -> Result<Vec<i64>, StoreError> {
    let mut st = conn.prepare(
        "SELECT folder_id FROM managed_folders
          WHERE watch_mode IN ('ddp_audit', 'fanotify_feed')
            AND CASE WHEN json_valid(watch_options) THEN json_extract(watch_options, '$.source') END = ?1",
    )?;
    let ids = st.query_map([id], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
    Ok(ids)
}

/// 삭제 — 쓰는 폴더가 있으면 `InUse` (커서는 CASCADE 로 함께 지워진다).
pub fn delete(conn: &Connection, id: i64) -> Result<(), StoreError> {
    if !referencing_folders(conn, id)?.is_empty() {
        return Err(StoreError::InUse);
    }
    if conn.execute("DELETE FROM remote_sources WHERE source_id = ?1", [id])? == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

struct Row {
    view: SourceView,
    auth_blob: Vec<u8>,
}

const COLS: &str = "source_id, kind, name, host, port, user, auth_kind, auth_blob, host_key, log_dir, poll_sec, path_map_json, created_at, updated_at, db_path";

fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    let kind: String = r.get(1)?;
    let auth_kind: String = r.get(6)?;
    let blob: Vec<u8> = r.get(7)?;
    let rules: String = r.get(11)?;
    Ok(Row {
        view: SourceView {
            source_id: r.get(0)?,
            kind: SourceKind::parse(&kind).map_or("ddp_audit", SourceKind::as_str),
            name: r.get(2)?,
            host: r.get(3)?,
            port: r.get(4)?,
            user: r.get(5)?,
            auth_kind: if auth_kind == "key" { "key" } else { "password" },
            has_secret: !blob.is_empty(),
            host_key: r.get(8)?,
            log_dir: r.get(9)?,
            poll_sec: r.get(10)?,
            path_map: serde_json::from_str(&rules).unwrap_or_default(),
            db_path: r.get(14)?,
            created_at: r.get(12)?,
            updated_at: r.get(13)?,
        },
        auth_blob: blob,
    })
}

pub fn list(conn: &Connection) -> Result<Vec<SourceView>, StoreError> {
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM remote_sources ORDER BY name COLLATE NOCASE"))?;
    let rows = st.query_map([], read_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().map(|r| r.view).collect())
}

fn row(conn: &Connection, id: i64) -> Result<Row, StoreError> {
    conn.query_row(&format!("SELECT {COLS} FROM remote_sources WHERE source_id = ?1"), [id], read_row)
        .optional()?
        .ok_or(StoreError::NotFound)
}

pub fn view(conn: &Connection, id: i64) -> Result<SourceView, StoreError> {
    row(conn, id).map(|r| r.view)
}

/// 종류만 (감지기 `probe` 용 — 복호하지 않는다)
pub fn kind_of(conn: &Connection, id: i64) -> Option<SourceKind> {
    let k: Option<String> = conn.query_row("SELECT kind FROM remote_sources WHERE source_id = ?1", [id], |r| r.get(0)).optional().ok()?;
    SourceKind::parse(&k?)
}

/// 복호한 접속 설정 (접속할 때만 만든다).
pub fn load(conn: &Connection, id: i64, secrets: &dyn SecretBox) -> Result<RemoteSource, StoreError> {
    let Row { view: v, auth_blob } = row(conn, id)?;
    let plain = secrets.open(&auth_blob)?;
    let secret = Secret::new(String::from_utf8(plain.to_vec()).map_err(|_| StoreError::Invalid("auth"))?);
    Ok(RemoteSource {
        source_id: v.source_id,
        kind: SourceKind::parse(v.kind).unwrap_or(SourceKind::DdpAudit),
        name: v.name,
        host: v.host,
        port: v.port,
        user: v.user,
        auth: if v.auth_kind == "key" { Auth::Key(secret) } else { Auth::Password(secret) },
        host_key: v.host_key,
        log_dir: v.log_dir,
        db_path: v.db_path,
        poll_sec: v.poll_sec,
        path_map: v.path_map,
    })
}

pub fn load_cursor(conn: &Connection, id: i64) -> Option<String> {
    conn.query_row("SELECT cursor FROM watch_cursors WHERE source_id = ?1", [id], |r| r.get(0)).optional().ok().flatten()
}

/// 커서 저장 — **호출자의 트랜잭션 안에서** 대기열 UPSERT 와 함께 부른다.
pub fn save_cursor(conn: &Connection, id: i64, cursor: &str, now: i64) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO watch_cursors (source_id, cursor, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (source_id) DO UPDATE SET cursor = excluded.cursor, updated_at = excluded.updated_at",
        params![id, cursor, now],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
