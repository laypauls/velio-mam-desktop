//! USB 디스크 "꺼내기 준비" — 디스크 단위로 앱이 파일에서 손을 뗀다 (Windows 안전 제거가 "사용 중" 으로 막히지 않게).
//!
//! **상태는 메모리뿐이다** (볼륨 ID -> 마운트 지점). 붙들린 볼륨은 연결돼 있어도 오프라인으로 취급한다: `volumes.availability = 'offline'`
//! 로 바꿔 기존 오프라인 처리(처리 작업 건너뜀, 감시·주기 재스캔 대상 제외)를 그대로 쓰고, 점검(`sync_volumes_holding`)이
//! 다시 온라인으로 되돌리지 못하게 한다. 해제는 ① 장치가 실제로 빠진 것을 점검이 확인했을 때(다시 꽂으면 일반 재연결 처리)
//! ② [`Ingest::resume_volume_watch`] ③ 앱 재시작(메모리 상태).
//!
//! 준비 순서 (모두 끝낸 뒤 반환): 붙들기·사이드카 차단 -> DB 오프라인 -> 그 디스크 전 폴더 감시 중지 -> 그 디스크를 쓰는 실행 중 작업 취소 +
//! FFmpeg 사이드카 트리 종료(PID) + 재생 정지 훅 -> 작업·사이드카가 파일을 놓을 때까지 제한 시간 대기.
//! 취소한 작업은 `Yielded` 로 대기열에 남는다 (오프라인 건너뛰기·재연결 보충은 기존 흐름).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_db::{DbPool, Priority};
use mam_media::SidecarGate;
use serde::Serialize;

use super::{Ingest, lock};
use crate::jobs::Job;
use crate::registry::set_volume_online;
use crate::scan::{IngestError, load_all, load_folder};
use crate::volume::VolumeKind;

/// 작업·사이드카가 파일을 놓기를 기다리는 기본 최대 시간
const DEFAULT_WAIT: Duration = Duration::from_secs(10);

/// `prepare_volume_eject` 결과 (프론트 계약 `EjectPrepared`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EjectPrepared {
    /// 사용자에게 보일 마운트 지점 (`E:\`)
    pub mount_point: String,
    /// 같은 디스크의 관리 폴더 전부 (제거 중인 폴더 제외)
    pub folder_ids: Vec<i64>,
}

/// 훅이 받는 대상 디스크 — 재생 중인 자산이 이 디스크에 있는지 묻는 데 쓴다.
pub struct EjectScope {
    pub volume_id: i64,
    pub mount_point: String,
    pool: Arc<DbPool>,
}

impl EjectScope {
    /// 자산의 (활성) 파일 위치가 이 디스크에 있는가. 조회 실패는 `false`.
    pub fn contains_asset(&self, asset_id: i64) -> bool {
        self.pool
            .read(|c| {
                Ok(c.query_row(
                    "SELECT EXISTS (SELECT 1 FROM file_locations
                     WHERE asset_id = ?1 AND volume_id = ?2 AND detached_at IS NULL)",
                    rusqlite::params![asset_id, self.volume_id],
                    |r| r.get::<_, bool>(0),
                )?)
            })
            .unwrap_or(false)
    }
}

/// 준비 도중 불리는 콜백 (앱: libVLC 가 이 디스크 파일을 재생 중이면 정지). 반환 시점에 파일이 닫혀 있어야 한다.
pub type EjectHook = Arc<dyn Fn(&EjectScope) + Send + Sync>;

#[derive(Default)]
pub(super) struct Held {
    /// 볼륨 ID -> 마운트 지점
    pub(super) volumes: HashMap<i64, String>,
    /// 붙들린 볼륨의 폴더 (감시 이벤트 필터용)
    folders: HashSet<i64>,
}

#[derive(Default)]
pub(super) struct Ejects {
    held: Arc<Mutex<Held>>,
    /// 준비·해제를 직렬화한다
    ctl: Mutex<()>,
    gate: Mutex<Option<Arc<SidecarGate>>>,
    hook: Mutex<Option<EjectHook>>,
    wait: Mutex<Option<Duration>>,
    /// 볼륨 ID -> (조회 때의 마운트 지점, 꺼낼 수 있는가). 마운트 지점이 바뀌면 다시 계산한다
    ejectable: Mutex<HashMap<i64, (String, bool)>>,
    /// 장치 조회가 실패했다고 이미 경고한 볼륨 (볼륨당 한 번)
    warned: Mutex<HashSet<i64>>,
    system_root: Mutex<Option<std::path::PathBuf>>,
}

impl Ejects {
    pub(super) fn held_volumes(&self) -> HashSet<i64> {
        lock(&self.held).volumes.keys().copied().collect()
    }

    /// 점검의 쓰기 클로저가 쓰는 공유 핸들 (DB 쓰기 스레드 안에서 읽어 준비와 순서가 어긋나지 않게 한다)
    pub(super) fn shared(&self) -> Arc<Mutex<Held>> {
        Arc::clone(&self.held)
    }
}

impl Ingest {
    /// 사이드카(FFmpeg 등) 실행기와 같은 게이트를 꽂는다 — 준비 중 그 디스크 아래 파일을 쓰는 사이드카를 막고 죽인다.
    pub fn set_sidecar_gate(&self, gate: Arc<SidecarGate>) {
        *lock(&self.eject.gate) = Some(gate);
    }

    /// 준비 도중 불릴 콜백 (재생 정지). 없으면 아무것도 안 한다.
    pub fn set_eject_hook(&self, hook: EjectHook) {
        *lock(&self.eject.hook) = Some(hook);
    }

    /// 작업·사이드카가 파일을 놓기를 기다리는 최대 시간 (기본 10초).
    pub fn set_eject_wait(&self, wait: Duration) {
        *lock(&self.eject.wait) = Some(wait);
    }

    /// 시스템 볼륨 판정에 쓸 `%SystemRoot%` (기본: 환경 변수). 시험용 재정의.
    pub fn set_system_root(&self, root: std::path::PathBuf) {
        *lock(&self.eject.system_root) = Some(root);
        lock(&self.eject.ejectable).clear();
    }

    /// 꺼낼 수 있는 볼륨 (`ManagedFolderView.ejectable`). 네트워크·시스템·MAMData(백업 포함) 볼륨은 제외하고, 장치가
    /// USB·1394·SD·MMC 버스이거나 이동식이면 포함한다. 장치 조회가 실패하면 꺼낼 수 있는 것으로 취급한다(볼륨당 경고 1회).
    /// "꺼내기 준비" 중인 볼륨은 항상 포함.
    pub fn ejectable_volumes(&self) -> HashSet<i64> {
        type Row = (i64, String, String, Option<String>);
        let rows: Vec<Row> = self
            .env
            .pool
            .read(|c| {
                let mut stmt = c.prepare(
                    "SELECT volume_id, volume_key, kind, last_mount_point FROM volumes
                     WHERE volume_id IN (SELECT volume_id FROM managed_folders)",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .unwrap_or_default();
        let held = self.ejected_volumes();
        rows.into_iter()
            .filter(|(id, key, kind, mount)| {
                held.contains(id) || self.is_ejectable(*id, key, kind, mount.as_deref())
            })
            .map(|(id, ..)| id)
            .collect()
    }

    fn is_ejectable(&self, volume_id: i64, key: &str, kind: &str, mount: Option<&str>) -> bool {
        if kind == VolumeKind::Network.as_str() {
            return false;
        }
        let Some(mount) = mount else {
            return true;
        };
        if let Some((m, v)) = lock(&self.eject.ejectable).get(&volume_id)
            && m == mount
        {
            return *v;
        }
        let same_volume = |p: &std::path::Path| self.volumes.volume_of(p).is_ok_and(|v| v.key == key);
        let system = lock(&self.eject.system_root)
            .clone()
            .or_else(|| std::env::var_os("SystemRoot").map(std::path::PathBuf::from));
        let internal = system.as_deref().is_some_and(same_volume)
            || self.config.auto_excludes.iter().any(|p| same_volume(p));
        let result = !internal
            && self.volumes.is_ejectable_device(mount).unwrap_or_else(|| {
                if lock(&self.eject.warned).insert(volume_id) {
                    tracing::warn!(volume_id, "cannot query the storage device; treating it as ejectable");
                }
                true
            });
        lock(&self.eject.ejectable).insert(volume_id, (mount.to_string(), result));
        result
    }

    /// 폴더 감시가 실행 중인가 (진단·시험용)
    pub fn is_watching(&self, folder_id: i64) -> bool {
        self.watchers.is_watching(folder_id)
    }

    /// 지금 "꺼내기 준비" 로 붙들린 볼륨 (`volumes.volume_id`).
    pub fn ejected_volumes(&self) -> HashSet<i64> {
        self.eject.held_volumes()
    }

    /// 폴더가 있는 디스크를 꺼낼 수 있게 앱이 손을 뗀다. 네트워크 폴더는 `InvalidInput`, 오프라인이면 `Offline`,
    /// 없는 폴더·제거 중인 폴더는 `NotFound`. 이미 준비된 디스크는 정리를 한 번 더 확인하고 같은 결과로 성공한다 (멱등).
    /// 작업·사이드카가 제한 시간 안에 파일을 놓지 않으면 `Busy` (붙들린 상태는 유지 — 다시 호출하면 이어서 기다린다).
    pub fn prepare_volume_eject(&self, folder_id: i64) -> Result<EjectPrepared, IngestError> {
        let _ctl = lock(&self.eject.ctl);
        let folder = self.env.pool.read(|c| Ok(load_folder(c, folder_id)))??;
        if !folder.enabled {
            return Err(IngestError::NotFound);
        }
        if folder.volume_kind == VolumeKind::Network {
            return Err(IngestError::InvalidInput("network volume"));
        }
        let volume_id = folder.volume_id;
        let already = lock(&self.eject.held).volumes.contains_key(&volume_id);
        if !already && !self.ejectable_volumes().contains(&volume_id) {
            return Err(IngestError::InvalidInput("volume is not ejectable"));
        }
        if !already && !folder.online {
            return Err(IngestError::Offline);
        }
        let Some(mount) = folder.mount_point.clone() else {
            return Err(IngestError::Offline);
        };
        let gate = lock(&self.eject.gate).clone();

        // 1) 붙들기 + 사이드카 차단 - DB 를 오프라인으로 바꾸기 전에 (점검이 그 사이 온라인으로 되돌리지 못하게)
        let siblings: Vec<i64> = self
            .env
            .pool
            .read(|c| Ok(load_all(c)))??
            .into_iter()
            .filter(|f| f.volume_id == volume_id)
            .map(|f| f.folder_id)
            .collect();
        {
            let mut h = lock(&self.eject.held);
            h.volumes.insert(volume_id, mount.clone());
            h.folders.extend(siblings.iter().copied());
        }
        if let Some(g) = &gate {
            g.block(&mount);
        }
        // 2) 오프라인 전환 (기존 오프라인 처리 재사용)
        let clock = Arc::clone(&self.env.clock);
        if let Err(e) = self
            .env
            .write(Priority::High, move |c| Ok(set_volume_online(c, &*clock, volume_id, false)?))
        {
            if !already {
                self.release_held(volume_id);
            }
            return Err(e);
        }
        // 3) 이 디스크 전 폴더 감시 중지 (감시 재시작은 `start_watch_locked` 가 붙들린 볼륨이면 거부한다)
        {
            let _w = lock(&self.watch_ctl);
            for id in &siblings {
                self.watchers.stop(*id);
            }
        }
        // 4) 실행 중 작업 취소 + 사이드카 종료 + 재생 정지, 5) 놓을 때까지 대기
        let deadline = Instant::now() + lock(&self.eject.wait).unwrap_or(DEFAULT_WAIT);
        self.jobs.notify();
        let ids = self.running_jobs_on(volume_id)?;
        self.jobs.cancel_running(&ids);
        if let Some(g) = &gate {
            let pids = g.cancel_under(&mount);
            tracing::info!(volume_id, ?pids, jobs = ids.len(), "eject: cancelled running work on the volume");
        }
        let hook = lock(&self.eject.hook).clone();
        if let Some(h) = hook {
            h(&EjectScope {
                volume_id,
                mount_point: mount.clone(),
                pool: Arc::clone(&self.env.pool),
            });
        }
        let left = |d: Instant| d.saturating_duration_since(Instant::now());
        let jobs_done = self.jobs.wait_jobs_finished(&ids, left(deadline));
        let sidecars_done = gate
            .as_ref()
            .is_none_or(|g| g.wait_none_under(&mount, left(deadline)));
        // 감시 핸들: `stop` 은 notify 스레드에 종료를 알릴 뿐이라 실제로 닫힐 때까지 기다린다
        let watches_done = self.watchers.wait_released(left(deadline));
        if !(jobs_done && sidecars_done && watches_done) {
            tracing::warn!(volume_id, jobs_done, sidecars_done, watches_done, "eject: work did not release the volume in time");
            return Err(IngestError::Busy);
        }
        let folder_ids = self
            .env
            .pool
            .read(|c| Ok(load_all(c)))??
            .into_iter()
            .filter(|f| f.volume_id == volume_id && f.enabled)
            .map(|f| f.folder_id)
            .collect();
        Ok(EjectPrepared {
            mount_point: crate::volume::strip_verbatim(&mount),
            folder_ids,
        })
    }

    /// 꺼내기 준비를 풀고 기존 재연결 처리(온라인 전환·감시·따라잡기 재스캔·처리 작업 보충)를 한다.
    /// 준비 상태가 아니면 아무 일 없이 성공. 없는 폴더·제거 중인 폴더는 `NotFound`.
    pub fn resume_volume_watch(&self, folder_id: i64) -> Result<(), IngestError> {
        let _ctl = lock(&self.eject.ctl);
        let folder = self.env.pool.read(|c| Ok(load_folder(c, folder_id)))??;
        if !folder.enabled {
            return Err(IngestError::NotFound);
        }
        if !self.release_held(folder.volume_id) {
            return Ok(());
        }
        // 점검이 `went_online` 으로 감시 재시작 + 재스캔 + 보충을 한다 (장치가 이미 빠졌다면 오프라인 그대로)
        self.supervise_once()?;
        Ok(())
    }

    /// 붙들기를 풀고 사이드카 차단을 푼다. 붙들려 있었으면 true.
    pub(super) fn release_held(&self, volume_id: i64) -> bool {
        let mount = {
            let mut h = lock(&self.eject.held);
            let Some(m) = h.volumes.remove(&volume_id) else {
                return false;
            };
            if h.volumes.is_empty() {
                h.folders.clear();
            } else {
                // 남은 붙들린 볼륨의 폴더만 유지
                let keep = h.volumes.keys().copied().collect::<HashSet<_>>();
                let rows = self.env.pool.read(|c| Ok(load_all(c))).ok().and_then(Result::ok);
                if let Some(rows) = rows {
                    h.folders = rows
                        .into_iter()
                        .filter(|f| keep.contains(&f.volume_id))
                        .map(|f| f.folder_id)
                        .collect();
                }
            }
            m
        };
        if let Some(g) = lock(&self.eject.gate).clone() {
            g.unblock(&mount);
        }
        true
    }

    /// 이 폴더의 감시 이벤트를 버려야 하는가 (붙들린 디스크)
    pub(super) fn folder_held(&self, folder_id: i64) -> bool {
        lock(&self.eject.held).folders.contains(&folder_id)
    }

    /// 지금 붙들려 있는 폴더 전체 (변경 대기열 처리기가 이 폴더의 행을 건너뛴다)
    pub(super) fn held_folder_ids(&self) -> Vec<i64> {
        lock(&self.eject.held).folders.iter().copied().collect()
    }

    /// 볼륨 키(`volumes.volume_key`)의 볼륨이 붙들려 있는가 (조회 실패는 `false`)
    pub(super) fn key_held(&self, key: &str) -> bool {
        let key = key.to_string();
        let id: Option<i64> = self
            .env
            .pool
            .read(move |c| {
                Ok(c.query_row(
                    "SELECT volume_id FROM volumes WHERE volume_key = ?1",
                    [key],
                    |r| r.get(0),
                )
                .ok())
            })
            .ok()
            .flatten();
        id.is_some_and(|id| self.volume_held(id))
    }

    /// 이 볼륨이 붙들려 있는가
    pub(super) fn volume_held(&self, volume_id: i64) -> bool {
        lock(&self.eject.held).volumes.contains_key(&volume_id)
    }

    /// 실행 중인 작업 중 이 볼륨의 파일을 다루는 것 (스캔: 폴더의 볼륨, 그 밖: 자산의 활성 위치 볼륨)
    fn running_jobs_on(&self, volume_id: i64) -> Result<Vec<i64>, IngestError> {
        let running: Vec<Job> = self.jobs.running_jobs();
        if running.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.env.pool.read(|c| {
            let mut out = Vec::new();
            for j in &running {
                let on: bool = c.query_row(
                    "SELECT EXISTS (
                       SELECT 1 FROM managed_folders WHERE folder_id = ?1 AND volume_id = ?3
                       UNION ALL
                       SELECT 1 FROM file_locations
                       WHERE asset_id = ?2 AND volume_id = ?3 AND detached_at IS NULL)",
                    rusqlite::params![j.folder_id, j.asset_id, volume_id],
                    |r| r.get(0),
                )?;
                if on {
                    out.push(j.job_id);
                }
            }
            Ok(out)
        })?)
    }
}
