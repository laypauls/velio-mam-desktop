//! `volumes` 테이블 관리 — 등록과 온라인/오프라인 전환.
//!
//! 오프라인 전환은 `volumes.availability` 한 행만 바꾼다. 위치·자산 행은 건드리지 않고
//! 검색 정보·썸네일·AI 결과를 그대로 유지한다 (연결 끊김은 삭제가 아니다).

use std::collections::HashSet;

use mam_db::Clock;
use rusqlite::{Connection, params};

use crate::error::CoreError;
use crate::volume::{VolumeInfo, VolumeProvider};

/// 볼륨을 등록하거나(`volume_key` 기준) 이미 있으면 마운트 지점·라벨을 갱신하고 online 으로 만든다.
/// 드라이브 문자가 바뀌어 다시 연결돼도 같은 `volume_id` 를 돌려준다.
pub fn register_volume(
    conn: &Connection,
    clock: &dyn Clock,
    info: &VolumeInfo,
) -> Result<i64, CoreError> {
    Ok(conn.query_row(
        "INSERT INTO volumes (kind, volume_key, label, last_mount_point, availability, last_seen_at)
         VALUES (?1, ?2, ?3, ?4, 'online', ?5)
         ON CONFLICT (volume_key) DO UPDATE SET
           kind = excluded.kind, label = excluded.label,
           last_mount_point = excluded.last_mount_point,
           availability = 'online', last_seen_at = excluded.last_seen_at
         RETURNING volume_id",
        params![
            info.kind.as_str(),
            info.key,
            info.label,
            info.mount_point,
            clock.now_ms()
        ],
        |r| r.get(0),
    )?)
}

/// 네트워크 볼륨처럼 호출자가 도달 가능 여부를 직접 판단할 때 쓴다. `volumes` 한 행만 갱신한다.
pub fn set_volume_online(
    conn: &Connection,
    clock: &dyn Clock,
    volume_id: i64,
    online: bool,
) -> Result<(), CoreError> {
    let changed = if online {
        conn.execute(
            "UPDATE volumes SET availability = 'online', last_seen_at = ?2 WHERE volume_id = ?1",
            params![volume_id, clock.now_ms()],
        )?
    } else {
        conn.execute(
            "UPDATE volumes SET availability = 'offline' WHERE volume_id = ?1",
            [volume_id],
        )?
    };
    if changed == 0 {
        return Err(CoreError::VolumeNotFound(volume_id));
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub went_online: Vec<i64>,
    pub went_offline: Vec<i64>,
    /// 이번 점검에서 열거된 연결 중인 로컬 볼륨 (새 디스크 판정이 같은 목록을 쓴다)
    pub present: Vec<VolumeInfo>,
    /// "꺼내기 준비" 로 붙들려 있던 볼륨 중 이번 점검에서 열거되지 않은(=장치가 실제로 빠진) 것 — 호출자가 붙들기를 푼다
    pub released: Vec<i64>,
}

/// 등록된 로컬(고정·이동식) 볼륨의 연결 상태를 현재 OS 상태와 맞춘다.
/// 열거에 나타난 볼륨은 online + 현재 마운트 지점으로, 사라진 볼륨은 offline 으로 바꾼다.
/// 열거되지 않는 네트워크 볼륨은 건드리지 않는다 ([`set_volume_online`] 사용). 새 볼륨은 등록하지 않는다.
pub fn sync_volumes(
    conn: &Connection,
    clock: &dyn Clock,
    provider: &dyn VolumeProvider,
) -> Result<SyncReport, CoreError> {
    sync_volumes_holding(conn, clock, provider, &HashSet::new())
}

/// [`sync_volumes`] 이되 `held` 볼륨(USB "꺼내기 준비")은 **연결돼 있어도 오프라인으로 둔다**: 가용성·마운트 지점을 바꾸지 않고
/// `went_online` 에도 넣지 않는다. 열거에서 사라진 held 볼륨은 `released` 에 담는다 (오프라인으로 남는다).
pub fn sync_volumes_holding(
    conn: &Connection,
    clock: &dyn Clock,
    provider: &dyn VolumeProvider,
    held: &HashSet<i64>,
) -> Result<SyncReport, CoreError> {
    let present = provider.list_volumes()?;
    let known: Vec<(i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT volume_id, volume_key, availability FROM volumes WHERE kind <> 'network'",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut report = SyncReport {
        present: present.clone(),
        ..SyncReport::default()
    };
    for (volume_id, key, availability) in known {
        let found = present.iter().find(|v| v.key == key);
        if held.contains(&volume_id) {
            if found.is_none() {
                report.released.push(volume_id);
            }
            if availability == "online" {
                conn.execute(
                    "UPDATE volumes SET availability = 'offline' WHERE volume_id = ?1",
                    [volume_id],
                )?;
            }
            continue;
        }
        match found {
            Some(info) => {
                conn.execute(
                    "UPDATE volumes SET availability = 'online', last_mount_point = ?2,
                       label = ?3, last_seen_at = ?4 WHERE volume_id = ?1",
                    params![volume_id, info.mount_point, info.label, clock.now_ms()],
                )?;
                if availability != "online" {
                    report.went_online.push(volume_id);
                }
            }
            None if availability == "online" => {
                conn.execute(
                    "UPDATE volumes SET availability = 'offline' WHERE volume_id = ?1",
                    [volume_id],
                )?;
                report.went_offline.push(volume_id);
            }
            None => {}
        }
    }
    Ok(report)
}
