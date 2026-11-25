//! 스캔 결과 1건을 DB 에 반영하는 판정·적용. 호출자가 연 트랜잭션 안에서 실행한다.
//!
//! 해시가 필요한데 아직 없으면 **쓰기 없이** `Step::NeedHash` 를 돌려준다 — 호출자가 트랜잭션 밖에서
//! (쓰기 잠금을 잡지 않고) 해시를 계산한 뒤 같은 판정을 다시 실행한다.

use rusqlite::Connection;

use super::store::{
    self, AssetChange, AssetRow, LocRow, NewLocation, active_location_at,
    active_locations_by_identity,
};
use super::{MoveBy, Outcome, ScanEntry, ScanSession};
use crate::error::CoreError;
use crate::hash::{EdgeFile, HashedFile};
use mam_db::path_key;

pub(super) enum Step {
    NeedHash,
    /// 전체 해시 없는 자산의 내용 변경 — 새 파일의 앞뒤 4K 체크섬이 필요하다 (쓰기 없음)
    NeedEdge,
    Done {
        outcome: Outcome,
        /// 재연결에 쓰인 `missing` 후보 위치 (커밋 후 세션에서 제거)
        consumed_candidate: Option<i64>,
    },
}

fn done(outcome: Outcome) -> Result<Step, CoreError> {
    Ok(Step::Done {
        outcome,
        consumed_candidate: None,
    })
}

pub(super) fn apply(
    conn: &Connection,
    now: i64,
    session: &ScanSession,
    entry: &ScanEntry<'_>,
    hash: Option<&HashedFile>,
    edge: Option<&EdgeFile>,
) -> Result<Step, CoreError> {
    let key = path_key(entry.rel_path)?;
    // NAS(SMB) 의 파일 ID 는 서버마다 안정성이 달라 해시 기반 재연결만 신뢰한다
    let identity = if session.is_network {
        None
    } else {
        entry.meta.file_identity.as_deref()
    };
    match active_location_at(conn, session.volume_id, &key)? {
        Some(loc) => at_known_path(conn, now, session, entry, identity, &loc, hash, edge),
        None => at_new_path(conn, now, session, entry, &key, identity, hash),
    }
}

#[allow(clippy::too_many_arguments)]
fn at_known_path(
    conn: &Connection,
    now: i64,
    session: &ScanSession,
    entry: &ScanEntry<'_>,
    identity: Option<&str>,
    loc: &LocRow,
    hash: Option<&HashedFile>,
    edge: Option<&EdgeFile>,
) -> Result<Step, CoreError> {
    // D6: 이번 스캔에서 이미 본 위치와 키는 같고 표기만 다르면 대소문자 충돌 — 데이터를 바꾸지 않는다
    if loc.rel_path != entry.rel_path && loc.last_seen_at >= session.scan_at {
        return Err(CoreError::CaseCollision {
            volume_id: session.volume_id,
            rel_path: entry.rel_path.to_string(),
            existing_rel_path: loc.rel_path.clone(),
        });
    }
    let asset = store::asset(conn, loc.asset_id)?;
    let meta = &entry.meta;
    let identity_changed =
        matches!((&loc.file_identity, identity), (Some(old), Some(new)) if old != new);
    let changed =
        asset.size != meta.size as i64 || asset.modified_at != meta.modified_ms || identity_changed;
    let seen = |id: Option<&str>| {
        store::mark_location_seen(conn, loc.location_id, entry.rel_path, id, session.scan_at)
    };

    if !changed {
        let reappeared = loc.presence == "missing";
        seen(identity)?;
        if reappeared || loc.rel_path != entry.rel_path {
            store::update_asset(conn, now, loc.asset_id, AssetChange::default())?;
        }
        return done(if reappeared {
            Outcome::Reappeared {
                asset_id: loc.asset_id,
            }
        } else {
            Outcome::Unchanged {
                asset_id: loc.asset_id,
            }
        });
    }

    let new_meta = AssetChange {
        size: Some(meta.size as i64),
        modified_at: Some(meta.modified_ms),
        ..Default::default()
    };
    // 전체 해시를 모르는 자산은 비교할 대상이 없다. 크기·수정 시각이 바뀌었으면 보수적으로 내용 변경으로 본다
    // (옛 내용의 태그·AI 결과가 새 내용에 붙지 않게 — "내용 변경 시 신규 자산"). 가운데만 편집됐을 수 있고
    // 옛 내용이 사라져 확인할 방법이 없다. 파일 ID 만 바뀐 경우(크기·수정 시각 동일)는 메타만 갱신한다.
    let Some(old_hash) = asset.hash else {
        if asset.size == meta.size as i64 && asset.modified_at == meta.modified_ms {
            seen(identity)?;
            store::update_asset(conn, now, loc.asset_id, new_meta)?;
            return done(Outcome::PendingUpdated {
                asset_id: loc.asset_id,
            });
        }
        let Some(edge) = edge else {
            return Ok(Step::NeedEdge);
        };
        seen(identity)?;
        return supersede_with_new(conn, now, session, entry, identity, loc, None, Some(edge));
    };
    let Some(hashed) = hash else {
        return Ok(Step::NeedHash);
    };
    seen(identity)?;
    if *hashed.hash.as_bytes() == old_hash {
        // 터치만: 같은 자산 유지 (D4)
        store::update_asset(conn, now, loc.asset_id, new_meta)?;
        return done(Outcome::Touched {
            asset_id: loc.asset_id,
        });
    }
    supersede_with_new(conn, now, session, entry, identity, loc, Some(hashed), None)
}

/// 내용 변경: 새 자산 + 구자산 superseded + 관계 (D4)
#[allow(clippy::too_many_arguments)]
fn supersede_with_new(
    conn: &Connection,
    now: i64,
    session: &ScanSession,
    entry: &ScanEntry<'_>,
    identity: Option<&str>,
    loc: &LocRow,
    hashed: Option<&HashedFile>,
    edge: Option<&EdgeFile>,
) -> Result<Step, CoreError> {
    store::detach_location(conn, now, loc.location_id)?;
    store::update_asset(
        conn,
        now,
        loc.asset_id,
        AssetChange {
            status: Some("superseded"),
            ..Default::default()
        },
    )?;
    let new_id = create_asset(conn, now, session, entry, identity, hashed, edge)?;
    store::insert_relation(conn, now, loc.asset_id, new_id)?;
    done(Outcome::ContentChanged {
        old_asset_id: loc.asset_id,
        new_asset_id: new_id,
    })
}

fn at_new_path(
    conn: &Connection,
    now: i64,
    session: &ScanSession,
    entry: &ScanEntry<'_>,
    key: &str,
    identity: Option<&str>,
    hash: Option<&HashedFile>,
) -> Result<Step, CoreError> {
    let meta = &entry.meta;
    // 같은 볼륨에서 파일 ID 가 같고 크기·수정 시각도 같으면 외부 이동·이름 변경 (D1)
    if let Some(id) = identity {
        for old in active_locations_by_identity(conn, session.volume_id, id)? {
            // 이번 스캔에서 이미 본 위치는 하드 링크 등 다른 경로다 — 이동으로 보지 않는다
            if old.last_seen_at >= session.scan_at {
                continue;
            }
            let asset = store::asset(conn, old.asset_id)?;
            if asset.size == meta.size as i64
                && asset.modified_at == meta.modified_ms
                && store::is_active_asset(conn, old.asset_id)?
            {
                relink(conn, now, session, entry, key, identity, &old, &asset)?;
                return done(Outcome::Moved {
                    asset_id: old.asset_id,
                    by: MoveBy::FileId,
                });
            }
        }
    }
    // D9: 폴더를 제거했다가 다시 등록 — 같은 경로에 detach 된 위치가 있고 크기·수정 시각이 같으면 그 자산에 재연결
    if let Some(asset_id) = store::detached_asset_at(
        conn,
        session.volume_id,
        key,
        meta.size as i64,
        meta.modified_ms,
    )? {
        store::insert_location(
            conn,
            &NewLocation {
                asset_id,
                volume_id: session.volume_id,
                rel_path: entry.rel_path,
                path_key: key,
                file_identity: identity,
                seen_at: session.scan_at,
            },
        )?;
        store::update_asset(conn, now, asset_id, AssetChange::default())?;
        return done(Outcome::Reattached { asset_id });
    }
    // 볼륨 간 이동·파일 ID 없음: 크기가 같은 missing 후보 -> 전체 해시가 같을 때만 재연결 (D1)
    let candidates = session.candidates_for(meta.size);
    if !candidates.is_empty() {
        let Some(hashed) = hash else {
            return Ok(Step::NeedHash);
        };
        for c in candidates {
            if c.hash != *hashed.hash.as_bytes()
                || !store::is_active_missing_location(conn, c.location_id)?
                || !store::is_active_asset(conn, c.asset_id)?
            {
                continue;
            }
            let asset = store::asset(conn, c.asset_id)?;
            let old = LocRow {
                location_id: c.location_id,
                asset_id: c.asset_id,
                rel_path: String::new(),
                file_identity: None,
                presence: "missing".into(),
                last_seen_at: 0,
            };
            relink(conn, now, session, entry, key, identity, &old, &asset)?;
            return Ok(Step::Done {
                outcome: Outcome::Moved {
                    asset_id: c.asset_id,
                    by: MoveBy::Hash,
                },
                consumed_candidate: Some(c.location_id),
            });
        }
    }
    let asset_id = create_asset(conn, now, session, entry, identity, hash, None)?;
    done(Outcome::Created { asset_id })
}

/// 기존 자산을 새 경로에 다시 연결: 이전 위치 detach + 새 위치 + 메타 갱신 (revision 증가, outbox)
#[allow(clippy::too_many_arguments)]
fn relink(
    conn: &Connection,
    now: i64,
    session: &ScanSession,
    entry: &ScanEntry<'_>,
    key: &str,
    identity: Option<&str>,
    old: &LocRow,
    asset: &AssetRow,
) -> Result<(), CoreError> {
    store::detach_location(conn, now, old.location_id)?;
    store::insert_location(
        conn,
        &NewLocation {
            asset_id: old.asset_id,
            volume_id: session.volume_id,
            rel_path: entry.rel_path,
            path_key: key,
            file_identity: identity,
            seen_at: session.scan_at,
        },
    )?;
    let meta_changed =
        asset.size != entry.meta.size as i64 || asset.modified_at != entry.meta.modified_ms;
    store::update_asset(
        conn,
        now,
        old.asset_id,
        AssetChange {
            size: meta_changed.then_some(entry.meta.size as i64),
            modified_at: meta_changed.then_some(entry.meta.modified_ms),
            ..Default::default()
        },
    )?;
    Ok(())
}

fn create_asset(
    conn: &Connection,
    now: i64,
    session: &ScanSession,
    entry: &ScanEntry<'_>,
    identity: Option<&str>,
    hash: Option<&HashedFile>,
    edge: Option<&EdgeFile>,
) -> Result<i64, CoreError> {
    let key = path_key(entry.rel_path)?;
    let asset_id = store::insert_asset(
        conn,
        now,
        entry.media_type.as_str(),
        entry.meta.size as i64,
        entry.meta.modified_ms,
        hash.map(|h| h.hash.as_bytes()),
        edge.map(|e| e.sig.as_bytes()),
    )?;
    store::insert_location(
        conn,
        &NewLocation {
            asset_id,
            volume_id: session.volume_id,
            rel_path: entry.rel_path,
            path_key: &key,
            file_identity: identity,
            seen_at: session.scan_at,
        },
    )?;
    Ok(asset_id)
}
