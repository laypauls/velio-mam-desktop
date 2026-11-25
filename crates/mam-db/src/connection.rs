use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::DbError;

/// 다른 연결(백그라운드 작업·인덱스 워커)이 쓰기 잠금을 잡고 있을 때 기다리는 시간.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// DB 파일을 열고 연결 PRAGMA 를 설정한다 (`foreign_keys = ON`, `journal_mode = WAL`, `busy_timeout`).
///
/// `foreign_keys` 는 연결 단위 설정이라 연결을 새로 열 때마다 이 함수를 써야 한다.
pub fn open(path: &Path) -> Result<Connection, DbError> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    // journal_mode 는 결과 행을 돌려주므로 query 로 읽어 실제 적용 여부를 확인한다
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(DbError::Pragma(format!(
            "failed to enable WAL (journal_mode = {mode})"
        )));
    }
    Ok(conn)
}
