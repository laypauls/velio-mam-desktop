//! 원천 DB 손상 자동 복구 (PR3·PR4·H5).
//!
//! 기동 시 DB 열기·마이그레이션이 `SQLITE_CORRUPT`/`SQLITE_NOTADB` 로 실패했을 때만 손상으로 본다 — `BUSY`·I/O·권한 오류는
//! 잠금·디스크 문제일 수 있어 DB 를 옮기지 않고 기존대로 기동 실패로 처리한다 (H5). 손상 판정 비용은 평소 0 이다.
//!
//! 순서: 손상 DB(+`-wal`/`-shm`)를 `<db>.corrupt-<ms>` 로 **옮겨 보관** -> 기본 `backups/` 와 백업 위치 힌트(H2)의 백업을 최신 순으로
//! 검증하며 첫 번째 정상 백업 복원 -> 후처리 -> 알림 기록. 쓸 수 있는 백업이 없으면 빈 DB 로 시작한다(손상 파일은 보관).
//!
//! 현재 DB 를 읽을 수 없으므로 보호 설정과 API 키 암호문 행(`ai_key_*`)은 백업 값을 **그대로** 쓴다(그 뒤 지운 키가 되살아날 수 있음 —
//! 동의를 철회 상태로 시작하므로 재동의 전에는 아무것도 전송되지 않는다. 사용자 복원은 현재 키 행을 유지한다).
//! **H1 선택(단순한 쪽)**: 복원 DB 가 모르는 asset_id 를 가진 썸네일·스트립이 캐시에 남아 있을 수 있다 — 캐시를 훑어
//! 가장 큰 asset_id 를 구해 `media_assets` 시퀀스를 그 이상으로 올린다(파일을 지우지 않는다. 정상 자산의 썸네일은 보존).
//! 임베딩은 파일 이름에 ID 가 없고(오프셋으로 참조) 벡터 세대는 재구축하므로 `embedding_id` 재사용은 파일 충돌을 만들지 않는다.
//! 백업이 없어 빈 DB 로 시작하면 모든 자산 캐시가 고아이므로 지운다.

use std::path::{Path, PathBuf};

use mam_core::storage::DataPaths;
use mam_db::{BackupInfo, Clock, DbError};
use rusqlite::ErrorCode;

use super::notice::{self, StartupNotice};
use super::post;

/// 복구가 필요한 손상인가: SQLite 가 `CORRUPT`(깨진 페이지) 또는 `NOTADB`(DB 파일이 아님)를 돌려준 경우만.
pub fn is_corruption(e: &DbError) -> bool {
    matches!(
        e,
        DbError::Sqlite(rusqlite::Error::SqliteFailure(f, _))
            if matches!(f.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
    )
}

#[derive(Debug, PartialEq, Eq)]
pub struct Recovered {
    pub corrupt_file: PathBuf,
    /// 복원한 백업의 (이름, 생성 시각) — 없으면 빈 DB 로 시작했다
    pub backup: Option<(String, i64)>,
    pub consent_reset: bool,
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// 손상 DB 를 보관 위치로 옮긴다 (이미 있는 파일을 덮어쓰지 않도록 `-<n>` 을 붙인다). 보관한 DB 파일 경로를 돌려준다.
fn quarantine(db: &Path, now_ms: i64) -> std::io::Result<PathBuf> {
    let mut dest = with_suffix(db, &format!(".corrupt-{now_ms}"));
    let mut n = 0;
    while dest.exists()
        || with_suffix(&dest, "-wal").exists()
        || with_suffix(&dest, "-shm").exists()
    {
        n += 1;
        dest = with_suffix(db, &format!(".corrupt-{now_ms}-{n}"));
    }
    std::fs::rename(db, &dest)?;
    // WAL 을 남겨 두면 복원된 DB 에 손상된 프레임이 적용될 수 있다 — 함께 옮긴다
    for suffix in ["-wal", "-shm"] {
        let side = with_suffix(db, suffix);
        if side.exists() {
            std::fs::rename(&side, with_suffix(&dest, suffix))?;
        }
    }
    Ok(dest)
}

/// 후보 백업(최신 순): 기본 `backups/` 와 위치 힌트 폴더. 같은 폴더는 한 번만.
fn candidates(paths: &DataPaths) -> Vec<(PathBuf, BackupInfo)> {
    let mut dirs = vec![paths.backups()];
    if let Some(hint) = super::hint::read(paths)
        && !dirs.contains(&hint)
    {
        dirs.push(hint);
    }
    let mut all: Vec<(PathBuf, BackupInfo)> = dirs
        .into_iter()
        .flat_map(|dir| {
            mam_db::list_backups(&dir)
                .unwrap_or_default()
                .into_iter()
                .map(move |info| (dir.join(&info.file_name), info))
        })
        .collect();
    all.sort_by(|a, b| {
        b.1.created_at
            .cmp(&a.1.created_at)
            .then_with(|| b.1.file_name.cmp(&a.1.file_name))
    });
    all
}

/// 검증을 통과하고 스키마가 앱보다 새롭지 않은 첫 백업을 복원한다.
fn restore_newest(paths: &DataPaths) -> Option<(String, i64)> {
    for (path, info) in candidates(paths) {
        let usable = mam_db::inspect_backup(&path)
            .is_ok_and(|m| m.schema_version <= mam_db::latest_version());
        if !usable {
            tracing::warn!(backup = %info.file_name, "skipping an unusable backup during recovery");
            continue;
        }
        match mam_db::restore_backup(&path, &paths.db()) {
            Ok(()) => return Some((info.file_name, info.created_at)),
            Err(e) => {
                tracing::warn!(%e, backup = %info.file_name, "recovery restore failed; trying the next backup")
            }
        }
    }
    None
}

/// 손상 복구를 수행한다. DB 를 옮긴 뒤에는 어떤 경우에도 (빈 DB 로라도) 다시 열 수 있는 상태로 끝낸다.
/// `Err` 는 손상 DB 를 옮기지 못한 경우뿐이다(아무것도 바뀌지 않음).
pub fn recover(paths: &DataPaths, clock: &dyn Clock) -> Result<Recovered, String> {
    let corrupt_file = quarantine(&paths.db(), clock.now_ms()).map_err(|e| e.to_string())?;
    let backup = restore_newest(paths);
    let mut consent_reset = false;
    match &backup {
        Some(_) => {
            // 후처리 실패는 복구 자체를 되돌리지 않는다 — 로그로 남기고 계속한다 (재동의·인덱스는 안전한 쪽으로 기운다)
            match mam_db::open(&paths.db()) {
                Ok(conn) => {
                    consent_reset = post::withdraw_consent(&conn).unwrap_or_else(|e| {
                        tracing::warn!(%e, "cannot reset consent after recovery");
                        false
                    });
                    // 플러그인도 같은 이유로 철회한다 — 사용자가 다시 켜고 권한을 다시 허용해야 한다 (알림은 P6/P7)
                    match super::plugins::withdraw_plugins(&conn) {
                        Ok(0) => {}
                        Ok(n) => tracing::warn!(plugins = n, "plugins disabled after recovery"),
                        Err(e) => tracing::warn!(%e, "cannot withdraw plugins after recovery"),
                    }
                    let floor = post::max_cache_asset_id(&paths.cache());
                    if floor > 0
                        && let Err(e) = post::raise_sequences(
                            &conn,
                            &[("media_assets".to_owned(), floor)].into(),
                        )
                    {
                        tracing::warn!(%e, "cannot raise asset id sequence after recovery");
                    }
                }
                Err(e) => {
                    tracing::warn!(%e, "cannot open the recovered database for post-processing")
                }
            }
        }
        None => {
            if let Err(e) = post::clear_asset_caches(paths) {
                tracing::warn!(%e, "cannot clear orphaned asset caches");
            }
        }
    }
    if let Err(e) = post::request_derived_rebuild(paths) {
        tracing::warn!(%e, "cannot request a derived index rebuild after recovery");
    }
    let recovered = Recovered {
        corrupt_file,
        backup,
        consent_reset,
    };
    let startup_notice = StartupNotice::Recovered {
        backup_file: recovered.backup.as_ref().map(|b| b.0.clone()),
        created_at: recovered.backup.as_ref().map(|b| b.1),
        corrupt_file: recovered.corrupt_file.to_string_lossy().into_owned(),
        consent_reset,
    };
    if let Err(e) = notice::write(paths, &startup_notice) {
        tracing::warn!(%e, "cannot record the recovery notice");
    }
    Ok(recovered)
}
