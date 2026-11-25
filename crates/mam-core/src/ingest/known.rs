//! 기억하는 디스크 — `volumes` 행과 묻지 않는 디스크(`ignored_volumes`)를 `volume_key` 로 합쳐 보여 주고, 기록을 지운다.
//!
//! 지우는 것은 앱이 따로 저장한 기록뿐이다. 원본 파일은 읽지도 쓰지도 않는다.

use std::collections::HashMap;

use mam_db::Priority;
use mam_db::ignored_volumes::{load_ignored_volumes, remove_ignored_volume};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use super::Ingest;
use super::removal::PURGE_BATCH;
use crate::purge::{RangeSpec, purge_range_batch};
use crate::scan::{Availability, IngestError};
use crate::volume::VolumeKind;

/// 기억하는 디스크 한 줄 (계약 `KnownVolume`). `volumes` 행이 없는 묻지 않음 항목은 `kind`·`availability`·`last_seen_at` 이 `None`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnownVolume {
    pub volume_key: String,
    pub label: Option<String>,
    pub kind: Option<VolumeKind>,
    pub last_mount_point: Option<String>,
    pub availability: Option<Availability>,
    pub last_seen_at: Option<i64>,
    /// 활성 관리 폴더 수 (제거 진행 중인 폴더 제외)
    pub folder_count: i64,
    pub ignored: bool,
}

fn volume_rows(c: &Connection) -> rusqlite::Result<Vec<KnownVolume>> {
    let mut stmt = c.prepare(
        "SELECT v.volume_key, v.label, v.kind, v.last_mount_point, v.availability, v.last_seen_at,
                (SELECT COUNT(*) FROM managed_folders f WHERE f.volume_id = v.volume_id AND f.enabled = 1)
         FROM volumes v ORDER BY v.volume_id",
    )?;
    let rows = stmt.query_map([], |r| {
        let kind: String = r.get(2)?;
        let availability: String = r.get(4)?;
        Ok(KnownVolume {
            volume_key: r.get(0)?,
            label: r.get(1)?,
            kind: Some(VolumeKind::parse(&kind).unwrap_or(VolumeKind::Fixed)),
            last_mount_point: r.get(3)?,
            availability: Some(if availability == "online" {
                Availability::Online
            } else {
                Availability::Offline
            }),
            last_seen_at: Some(r.get(5)?),
            folder_count: r.get(6)?,
            ignored: false,
        })
    })?;
    rows.collect()
}

/// 연결됨 먼저, 마지막 연결이 최근인 순, `volumes` 행이 없는 묻지 않음 항목은 끝 (키 순).
fn sort_key(v: &KnownVolume) -> (u8, std::cmp::Reverse<i64>, String) {
    let rank = match v.availability {
        Some(Availability::Online) => 0,
        Some(Availability::Offline) => 1,
        None => 2,
    };
    (
        rank,
        std::cmp::Reverse(v.last_seen_at.unwrap_or(0)),
        v.volume_key.clone(),
    )
}

impl Ingest {
    /// 앱이 기억하는 디스크 목록 (`volumes` 행 + 묻지 않음 항목, `volume_key` 당 1행).
    pub fn list_known_volumes(&self) -> Result<Vec<KnownVolume>, IngestError> {
        let (mut list, ignored) = self
            .env
            .pool
            .read(|c| Ok((volume_rows(c)?, load_ignored_volumes(c)?)))?;
        let mut index: HashMap<String, usize> = list
            .iter()
            .enumerate()
            .map(|(i, v)| (v.volume_key.clone(), i))
            .collect();
        for entry in ignored {
            match index.get(&entry.volume_key) {
                Some(&i) => list[i].ignored = true,
                None => {
                    index.insert(entry.volume_key.clone(), list.len());
                    list.push(KnownVolume {
                        volume_key: entry.volume_key,
                        label: entry.label,
                        kind: None,
                        last_mount_point: entry.last_mount_point,
                        availability: None,
                        last_seen_at: None,
                        folder_count: 0,
                        ignored: true,
                    });
                }
            }
        }
        list.sort_by_key(sort_key);
        Ok(list)
    }

    /// 디스크 기록을 지운다: `volumes` 행 + 묻지 않음 항목. 관리 폴더(제거 진행 중 포함)가 있으면 `VolumeHasFolders`.
    /// 삭제·대체된 자산의 옛 위치 행(`volumes` FK 가 RESTRICT 라 먼저 지운다)은 함께 지우되 연결 데이터는 건드리지 않는다.
    /// 아직 정리되지 않은 옛 보관 자산(활성인데 위치만 남은)이 이 디스크에 있으면 그 연결 데이터도 지운다 — 그 경우만 사전 백업을 만든다.
    /// 모르는 키는 아무 일도 없이 성공한다. 지운 뒤 연결돼 있으면 새 디스크로 다시 묻는다.
    pub fn forget_volume_record(&self, volume_key: &str) -> Result<(), IngestError> {
        if volume_key.is_empty() || volume_key.len() > 1024 || volume_key.contains('\0') {
            return Err(IngestError::InvalidInput("volumeKey"));
        }
        self.with_prompt_reset(volume_key, || self.forget_locked(volume_key))
    }

    fn forget_locked(&self, volume_key: &str) -> Result<(), IngestError> {
        let key = volume_key.to_owned();
        let volume_id: Option<i64> = self.env.pool.read(move |c| {
            Ok(c.query_row(
                "SELECT volume_id FROM volumes WHERE volume_key = ?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?)
        })?;
        if let Some(volume_id) = volume_id {
            let (folders, active_rows) = self.env.pool.read(move |c| {
                Ok((
                    folder_total(c, volume_id)?,
                    c.query_row(
                        "SELECT EXISTS (SELECT 1 FROM file_locations l
                           JOIN media_assets a ON a.asset_id = l.asset_id
                           WHERE l.volume_id = ?1 AND a.status = 'active')",
                        [volume_id],
                        |r| r.get::<_, bool>(0),
                    )?,
                ))
            })?;
            if folders > 0 {
                return Err(IngestError::VolumeHasFolders);
            }
            // 1) 삭제·대체된 자산의 옛 위치 행만 — 연결 데이터는 건드리지 않는다
            loop {
                let n = self.env.write(Priority::Low, move |c| {
                    guard_no_folders(c, volume_id)?;
                    Ok(c.execute(
                        "DELETE FROM file_locations WHERE location_id IN (
                           SELECT l.location_id FROM file_locations l
                           JOIN media_assets a ON a.asset_id = l.asset_id
                           WHERE l.volume_id = ?1 AND a.status <> 'active' LIMIT ?2)",
                        params![volume_id, PURGE_BATCH as i64],
                    )?)
                })?;
                if n == 0 {
                    break;
                }
            }
            // 2) 옛 보관 자산이 남아 있으면 폴더 제거와 같은 방식으로 정리 (백업 먼저)
            if active_rows {
                self.backup_before_purge()?;
                loop {
                    let clock = std::sync::Arc::clone(&self.env.clock);
                    let batch = self.env.write(Priority::Low, move |c| {
                        guard_no_folders(c, volume_id)?;
                        let spec = RangeSpec {
                            volume_id,
                            root_key: "",
                            media_types: None,
                        };
                        Ok(purge_range_batch(c, clock.now_ms(), &spec, PURGE_BATCH)?)
                    })?;
                    if batch.locations == 0 {
                        break;
                    }
                    self.remove_cache(&batch.purged);
                    self.notify_embeddings(&batch.embedding_keys);
                }
            }
        }
        let key = volume_key.to_owned();
        self.env.write(Priority::High, move |c| {
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if let Some(volume_id) = volume_id {
                guard_no_folders(&tx, volume_id)?;
                tx.execute("DELETE FROM volumes WHERE volume_id = ?1", [volume_id])?;
            }
            remove_ignored_volume(&tx, &key)?;
            tx.commit()?;
            Ok(())
        })?;
        *super::lock(&self.totals) = None;
        Ok(())
    }
}

/// 이 디스크의 관리 폴더 행 수 (제거 진행 중 `enabled = 0` 포함)
fn folder_total(c: &Connection, volume_id: i64) -> rusqlite::Result<i64> {
    c.query_row(
        "SELECT COUNT(*) FROM managed_folders WHERE volume_id = ?1",
        [volume_id],
        |r| r.get(0),
    )
}

/// 쓰기 직전 재확인 — 기록을 지우는 사이 같은 디스크에 폴더가 추가됐으면 멈춘다 (쓰기 스레드는 하나라 이 확인 뒤에 끼어들 수 없다).
fn guard_no_folders(c: &Connection, volume_id: i64) -> Result<(), IngestError> {
    if folder_total(c, volume_id)? > 0 {
        return Err(IngestError::VolumeHasFolders);
    }
    Ok(())
}
