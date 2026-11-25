//! 원본 이동 (D4) — 같은 볼륨은 rename, 다른 볼륨은 복사 -> 검증(크기·BLAKE3) -> 원본 휴지통.
//!
//! 자산 연결은 위치 행을 새 경로로 갱신해 유지한다 (`asset_id`·태그 불변). 대상이 관리 폴더 밖이면 위치를
//! detach 하고 [`MoveReport::detached_asset_ids`] 로 알린다. 대상 파일을 덮어쓰지 않는다.
//! 실패 시 원본은 그대로 남는다 (복사본이 만들어졌으면 지운다).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use rusqlite::{Connection, TransactionBehavior};
use serde::Serialize;

use super::delete::{OriginalsEnv, unique_ids};
use super::fsmove::move_no_replace;
use super::locate::{Located, locate};
use super::paths::{is_within, same_path};
use super::store::{self, NewPlace};
use super::{AssetFailure, OriginalsResult, ProtectError};
use crate::hash::ContentHasher;
use crate::identity::FileIdentityProvider;
use crate::volume::{VolumeInfo, VolumeProvider, relative_path};

pub struct MoveEnv<'a> {
    pub base: OriginalsEnv<'a>,
    pub volumes: &'a dyn VolumeProvider,
    pub identity: &'a dyn FileIdentityProvider,
    pub hasher: &'a dyn ContentHasher,
    /// 이동 대상이 될 수 없는 폴더 — MAMData 루트와 사용자 지정 백업 폴더. 그 안(같은 경로 포함)으로의
    /// 이동은 `InvalidInput` 으로 거부한다 (원본이 관리 데이터·백업에 섞이는 것을 막는다).
    pub excluded_dirs: &'a [PathBuf],
}

/// 이동 결과. `result` 가 UI 계약이고, `detached_asset_ids` 는 "관리 밖으로 나갔다" 안내용.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveReport {
    pub result: OriginalsResult,
    pub detached_asset_ids: Vec<i64>,
}

struct Dest<'a> {
    dir: &'a Path,
    volume: VolumeInfo,
    /// 등록된 볼륨이고 대상이 관리 폴더 안이면 `volume_id`
    managed_volume_id: Option<i64>,
}

pub fn move_originals(
    conn: &mut Connection,
    env: &MoveEnv<'_>,
    asset_ids: &[i64],
    dest_dir: &Path,
) -> Result<MoveReport, ProtectError> {
    // 입력 검증을 모두 끝낸 뒤에 인증을 소모한다 (면제 0분의 1회용 인증이 잘못된 입력에 낭비되지 않게)
    env.base.guard.check_auth(env.base.password_set)?;
    let ids = unique_ids(asset_ids)?;
    if !dest_dir.is_absolute() || !dest_dir.is_dir() {
        return Err(ProtectError::InvalidInput(
            "destination must be an existing absolute directory".into(),
        ));
    }
    if env.excluded_dirs.iter().any(|x| is_within(dest_dir, x)) {
        return Err(ProtectError::InvalidInput(
            "destination is inside MAM data or the backup folder".into(),
        ));
    }
    let volume = env.volumes.volume_of(dest_dir)?;
    let (_, dir_key) = crate::scan::root_keys(&volume, dest_dir)
        .map_err(|e| ProtectError::InvalidInput(e.to_string()))?;
    let managed_volume_id = match store::volume_id_by_key(conn, &volume.key)? {
        Some(id) if store::is_managed(conn, id, &dir_key)? => Some(id),
        _ => None,
    };
    let dest = Dest {
        dir: dest_dir,
        volume,
        managed_volume_id,
    };

    env.base.guard.require_auth(env.base.password_set)?;
    let mut report = MoveReport::default();
    for id in ids {
        match move_one(conn, env, &dest, id) {
            Ok(linked) => {
                report.result.succeeded += 1;
                if !linked {
                    report.detached_asset_ids.push(id);
                }
            }
            Err(e) => {
                tracing::warn!(asset_id = id, error = %e, "original move failed");
                report.result.failed.push(AssetFailure {
                    asset_id: id,
                    message: e.asset_message().into(),
                });
            }
        }
    }
    Ok(report)
}

/// `AlreadyExists` 는 고정 코드 `destination_exists`, 그 외는 일반 I/O 오류 (경로는 로그에만)
fn move_err(path: &Path, e: &std::io::Error) -> ProtectError {
    if e.kind() == std::io::ErrorKind::AlreadyExists {
        ProtectError::Io("destination_exists".into())
    } else {
        ProtectError::io(path, e)
    }
}

/// 성공 시 `true` = 새 위치로 재연결, `false` = 관리 밖이라 detach
fn move_one(
    conn: &mut Connection,
    env: &MoveEnv<'_>,
    dest: &Dest<'_>,
    asset_id: i64,
) -> Result<bool, ProtectError> {
    let loc = locate(conn, asset_id)?;
    let src = loc.checked_path()?;
    let name = src
        .file_name()
        .ok_or_else(|| ProtectError::Internal("source has no file name".into()))?;
    let target = dest.dir.join(name);
    if src.parent().is_some_and(|p| same_path(p, dest.dir)) {
        return Err(ProtectError::InvalidInput("already_in_destination".into()));
    }
    // 빠른 사전 확인일 뿐이다 - 실제 덮어쓰기 방지는 `move_no_replace` 가 원자적으로 보장한다
    if target.exists() {
        return Err(ProtectError::Io("destination_exists".into()));
    }
    let new_rel = match dest.managed_volume_id {
        Some(volume_id) => {
            let rel = relative_path(&dest.volume.mount_point, &target)?;
            let key = mam_db::path_key(&rel).map_err(crate::CoreError::from)?;
            if store::active_at(conn, volume_id, &key)? {
                return Err(ProtectError::Io("destination_tracked".into()));
            }
            Some((volume_id, rel, key))
        }
        None => None,
    };

    let same_volume = loc.volume_key == dest.volume.key;
    if same_volume {
        move_no_replace(&src, &target, false).map_err(|e| move_err(&target, &e))?;
    } else {
        copy_verified(env, &loc, &src, dest.dir, &target)?;
    }

    let outcome = commit(conn, env, &loc, &target, new_rel.as_ref());
    if let Err(e) = &outcome {
        if same_volume {
            // 같은 볼륨이면 되돌릴 수 있다 (원래 자리에 다른 파일이 생겼다면 덮어쓰지 않는다)
            if let Err(re) = move_no_replace(&target, &src, false) {
                tracing::error!(asset_id, error = %re, "cannot roll back move after database failure");
            }
        } else {
            // 원본은 이미 휴지통에 있고 복사본이 새 위치에 있다 - DB 만 옛 위치를 가리킨다.
            // 다음 스캔이 새 위치의 파일을 같은 내용으로 다시 연결한다 (자산 정체성은 내용 해시 기준).
            tracing::error!(asset_id, error = %e, "moved to new volume but database update failed");
            return Err(ProtectError::Io("moved_db_pending".into()));
        }
    }
    outcome
}

fn commit(
    conn: &mut Connection,
    env: &MoveEnv<'_>,
    loc: &Located,
    target: &Path,
    new_rel: Option<&(i64, String, String)>,
) -> Result<bool, ProtectError> {
    let now = env.base.clock.now_ms();
    let place_meta = match new_rel {
        Some(_) => Some(env.identity.stat(target)?),
        None => None,
    };
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    match (new_rel, &place_meta) {
        (Some((volume_id, rel, key)), Some(meta)) => store::relink(
            &tx,
            now,
            loc.asset_id,
            loc.location_id,
            &NewPlace {
                volume_id: *volume_id,
                rel_path: rel,
                path_key: key,
                file_identity: meta.file_identity.as_deref(),
                size: i64::try_from(meta.size).unwrap_or(i64::MAX),
                modified_at: meta.modified_ms,
            },
        )?,
        _ => store::detach_moved_out(&tx, now, loc.asset_id, loc.location_id)?,
    }
    tx.commit()?;
    Ok(new_rel.is_some())
}

/// 임시 이름으로 복사 -> 크기·해시 검증 -> 최종 이름 -> 원본 휴지통. 어느 단계든 실패하면 복사본을 지운다.
fn copy_verified(
    env: &MoveEnv<'_>,
    loc: &Located,
    src: &Path,
    dest_dir: &Path,
    target: &Path,
) -> Result<(), ProtectError> {
    let tmp: PathBuf = dest_dir.join(format!(".mam-move-{}.tmp", loc.asset_id));
    if tmp.exists() {
        return Err(ProtectError::Io("temporary copy already exists".into()));
    }
    let cancel = AtomicBool::new(false);
    let result = (|| {
        std::fs::copy(src, &tmp).map_err(|e| ProtectError::io(&tmp, &e))?;
        let want = env.hasher.hash_file(src, &cancel)?;
        let got = env.hasher.hash_file(&tmp, &cancel)?;
        if want.size != got.size || want.hash != got.hash {
            return Err(ProtectError::Io("copy_verify_failed".into()));
        }
        // 확인 이후 대상이 생겼어도 덮어쓰지 않는다
        move_no_replace(&tmp, target, true).map_err(|e| move_err(target, &e))?;
        env.base.trash.trash(src).inspect_err(|_| {
            let _ = std::fs::remove_file(target); // 원본이 그대로 있으므로 복사본을 되돌린다
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
