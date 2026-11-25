//! 원본 삭제 (D3) — Windows 휴지통으로만 보낸다. 영구 삭제 경로는 없다.

use mam_db::Clock;
use rusqlite::{Connection, TransactionBehavior};

use super::locate::locate;
use super::{AssetFailure, OriginalsResult, ProtectError, ProtectionGuard, Trash, store};

/// 삭제·이동 공통 의존성. 호출마다 만든다 (`password_set` 은 settings 의 해시 존재 여부).
pub struct OriginalsEnv<'a> {
    pub guard: &'a ProtectionGuard,
    pub password_set: bool,
    pub clock: &'a dyn Clock,
    pub trash: &'a dyn Trash,
}

/// 중복을 제거한 자산 ID 목록. 비어 있으면 `InvalidInput`.
pub(super) fn unique_ids(asset_ids: &[i64]) -> Result<Vec<i64>, ProtectError> {
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<i64> = asset_ids
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect();
    if ids.is_empty() {
        return Err(ProtectError::InvalidInput("no assets selected".into()));
    }
    Ok(ids)
}

/// 인증 면제 시간 안에서만 실행한다 (`AuthRequired` 이면 아무것도 건드리지 않는다).
/// 자산별 실패(오프라인·파일 없음·잠김 등)는 `failed` 에 모으고 나머지는 계속 처리한다.
pub fn delete_originals(
    conn: &mut Connection,
    env: &OriginalsEnv<'_>,
    asset_ids: &[i64],
) -> Result<OriginalsResult, ProtectError> {
    // 입력 검증을 끝낸 뒤에 인증을 소모한다 (면제 0분의 1회용 인증이 잘못된 입력에 낭비되지 않게)
    env.guard.check_auth(env.password_set)?;
    let ids = unique_ids(asset_ids)?;
    env.guard.require_auth(env.password_set)?;
    let mut result = OriginalsResult::default();
    for id in ids {
        match delete_one(conn, env, id) {
            Ok(()) => result.succeeded += 1,
            Err(e) => {
                tracing::warn!(asset_id = id, error = %e, "original delete failed");
                result.failed.push(AssetFailure {
                    asset_id: id,
                    message: e.asset_message().into(),
                });
            }
        }
    }
    Ok(result)
}

fn delete_one(
    conn: &mut Connection,
    env: &OriginalsEnv<'_>,
    asset_id: i64,
) -> Result<(), ProtectError> {
    let loc = locate(conn, asset_id)?;
    let path = loc.checked_path()?;
    env.trash.trash(&path)?;
    // 파일은 이미 휴지통에 있다 — DB 실패 시 다음 스캔이 "파일 없음" 으로 정리한다
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    store::mark_deleted(&tx, env.clock.now_ms(), asset_id, loc.location_id)
        .and_then(|()| tx.commit())
        .map_err(|e| {
            tracing::error!(asset_id, error = %e, "moved to recycle bin but database update failed");
            ProtectError::Internal("moved_db_pending".into())
        })
}
