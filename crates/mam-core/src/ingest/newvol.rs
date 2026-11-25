//! 새 디스크 감지 — 앱 실행 중에 **새로 나타난** 로컬 볼륨 중 관리 폴더가 없는 것을 "대기 목록"에 올린다.
//!
//! 판정 (플랫폼 무관, `VolumeProvider::list_volumes` 한 경로):
//! - `baseline`: 첫 점검 때 이미 연결돼 있던 볼륨 + 관리 폴더가 있는 볼륨 (묻지 않는다)
//! - 대기 = 연결됨 - baseline - 관리 폴더 있음 - `ignored_volumes` - "이번 연결만 나중에"(`dismissed`)
//! - 사라진 볼륨은 baseline·dismissed·대기에서 빠진다 -> 다시 연결되면 다시 묻는다
//! - 묻지 않는 디스크·대기 디스크는 `volumes` 테이블에 넣지 않는다 (30초 점검의 `went_online` -> 처리 작업 보충 방지)
//!
//! 상태를 바꾸는 모든 경로는 `serialize` 락을 쥐고 실행해(점검·등록 후 갱신·나중에·다시 묻기) 이벤트 순서가 상태 순서와 같다.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use mam_db::Priority;
use mam_db::ignored_volumes::{
    IgnoredVolume, add_ignored_volume, load_ignored_volumes, remove_ignored_volume,
    update_ignored_volume,
};
use serde::Serialize;

use super::{Ingest, lock};
use crate::scan::IngestError;
use crate::volume::{VolumeInfo, VolumeKind, VolumeProvider};

/// 이벤트 `volumes://pending-changed` 이름
pub const PENDING_CHANGED_EVENT: &str = "volumes://pending-changed";

/// 스캔 대상 등록을 물어볼 새 디스크 (계약 `PendingVolume`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingVolume {
    pub volume_key: String,
    pub label: Option<String>,
    /// `"fixed"` | `"removable"`
    pub kind: VolumeKind,
    pub mount_point: String,
    /// 전체 용량(바이트). 조회 실패 시 `None`
    pub total_bytes: Option<u64>,
}

/// 이벤트 페이로드 `{ volumes }` (대기 목록 전체)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingChangedPayload {
    pub volumes: Vec<PendingVolume>,
}

/// 대기 목록이 바뀔 때마다 (전체 목록과 함께) 불린다.
pub type PendingListener = Arc<dyn Fn(Vec<PendingVolume>) + Send + Sync>;

#[derive(Default)]
struct State {
    /// `None` = 아직 첫 점검 전
    baseline: Option<HashSet<String>>,
    /// 이번 연결 동안만 "나중에" 한 디스크
    dismissed: HashSet<String>,
    pending: Vec<PendingVolume>,
}

#[derive(Default)]
pub(super) struct NewVolumes {
    state: Mutex<State>,
    listener: Mutex<Option<PendingListener>>,
    serialize: Mutex<()>,
}

impl State {
    /// 현재 연결 목록으로 상태를 갱신한다. 대기 목록이 바뀌었으면 true.
    fn update(
        &mut self,
        present: &[VolumeInfo],
        managed: &HashSet<String>,
        ignored: &HashSet<String>,
        total_of: impl Fn(&VolumeInfo) -> Option<u64>,
    ) -> bool {
        let keys: HashSet<&str> = present.iter().map(|v| v.key.as_str()).collect();
        let baseline = self
            .baseline
            .get_or_insert_with(|| keys.iter().map(|k| (*k).to_owned()).collect());
        baseline.retain(|k| keys.contains(k.as_str()));
        baseline.extend(managed.iter().filter(|k| keys.contains(k.as_str())).cloned());
        self.dismissed.retain(|k| keys.contains(k.as_str()));

        let known: HashMap<&str, Option<u64>> = self
            .pending
            .iter()
            .map(|p| (p.volume_key.as_str(), p.total_bytes))
            .collect();
        let next: Vec<PendingVolume> = present
            .iter()
            .filter(|v| {
                !baseline.contains(&v.key)
                    && !managed.contains(&v.key)
                    && !ignored.contains(&v.key)
                    && !self.dismissed.contains(&v.key)
                    && v.kind != VolumeKind::Network
            })
            .map(|v| PendingVolume {
                volume_key: v.key.clone(),
                label: v.label.clone(),
                kind: v.kind,
                mount_point: v.mount_point.clone(),
                total_bytes: known
                    .get(v.key.as_str())
                    .copied()
                    .unwrap_or_else(|| total_of(v)),
            })
            .collect();
        let changed = next != self.pending;
        self.pending = next;
        changed
    }
}

impl Ingest {
    /// 대기 목록이 바뀔 때 부를 콜백을 정한다 (Tauri 이벤트 발행용). [`Ingest::start`] 전에 호출한다.
    pub fn set_pending_listener(&self, listener: PendingListener) {
        *lock(&self.newvol.listener) = Some(listener);
    }

    /// 지금 스캔 대상 등록을 물어볼 새 디스크 목록.
    pub fn pending_volumes(&self) -> Vec<PendingVolume> {
        lock(&self.newvol.state).pending.clone()
    }

    /// 대기 목록 전체를 리스너에 알린다 (`serialize` 를 쥔 채 호출).
    fn notify_pending(&self) {
        let listener = lock(&self.newvol.listener).clone();
        if let Some(f) = listener {
            f(self.pending_volumes());
        }
    }

    /// 점검 때 부른다: 연결된 볼륨 목록으로 대기 목록을 다시 계산한다.
    pub(super) fn detect_new_volumes(&self, present: &[VolumeInfo]) -> Result<(), IngestError> {
        let _g = lock(&self.newvol.serialize);
        self.detect_locked(present)
    }

    /// 볼륨 목록을 직접 조회해 대기 목록을 갱신한다 (등록·다시 묻기 직후). 실패는 로그만.
    pub(super) fn refresh_pending(&self) {
        let _g = lock(&self.newvol.serialize);
        self.refresh_locked();
    }

    fn refresh_locked(&self) {
        let result = self
            .volumes
            .list_volumes()
            .map_err(IngestError::from)
            .and_then(|present| self.detect_locked(&present));
        if let Err(e) = result {
            tracing::warn!(%e, "cannot refresh pending volumes");
        }
    }

    fn detect_locked(&self, present: &[VolumeInfo]) -> Result<(), IngestError> {
        let (managed, ignored) = self.env.pool.read(|c| {
            let mut stmt = c.prepare(
                "SELECT DISTINCT v.volume_key FROM managed_folders f
                 JOIN volumes v ON v.volume_id = f.volume_id",
            )?;
            let managed = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<HashSet<_>, _>>()?;
            Ok((managed, load_ignored_volumes(c)?))
        })?;

        // 묻지 않는 디스크가 연결돼 있으면 설정 목록 표시용 라벨·마운트 지점만 갱신한다 (바뀐 때만 쓴다)
        for v in present {
            let Some(entry) = ignored.iter().find(|i| i.volume_key == v.key) else {
                continue;
            };
            if entry.label == v.label && entry.last_mount_point.as_deref() == Some(&v.mount_point) {
                continue;
            }
            let (key, label, mount) = (v.key.clone(), v.label.clone(), v.mount_point.clone());
            self.env.write(Priority::Low, move |c| {
                update_ignored_volume(c, &key, label.as_deref(), Some(&mount))?;
                Ok(())
            })?;
        }

        let ignored_keys: HashSet<String> = ignored.into_iter().map(|i| i.volume_key).collect();
        let provider: &(dyn VolumeProvider + Send + Sync) = &*self.volumes;
        let changed = lock(&self.newvol.state).update(present, &managed, &ignored_keys, |v| {
            provider.total_bytes(&v.mount_point)
        });
        if changed {
            self.notify_pending();
        }
        Ok(())
    }

    /// "나중에". `remember` 면 `ignored_volumes` 에 저장(다시 묻지 않음), 아니면 이번 연결 동안만 묻지 않는다.
    /// 대기 목록에 없는 키는 아무것도 하지 않는다 (이미 분리됐거나 등록됨).
    pub fn dismiss_new_volume(&self, volume_key: &str, remember: bool) -> Result<(), IngestError> {
        let _g = lock(&self.newvol.serialize);
        let Some(vol) = lock(&self.newvol.state)
            .pending
            .iter()
            .find(|p| p.volume_key == volume_key)
            .cloned()
        else {
            return Ok(());
        };
        if remember {
            let entry = IgnoredVolume {
                volume_key: vol.volume_key.clone(),
                label: vol.label.clone(),
                last_mount_point: Some(vol.mount_point.clone()),
                ignored_at: self.env.clock.now_ms(),
            };
            self.env.write(Priority::High, move |c| {
                add_ignored_volume(c, &entry)?;
                Ok(())
            })?;
        }
        {
            let mut st = lock(&self.newvol.state);
            st.pending.retain(|p| p.volume_key != volume_key);
            if !remember {
                st.dismissed.insert(volume_key.to_owned());
            }
        }
        self.notify_pending();
        Ok(())
    }

    /// 설정 "묻지 않는 디스크" 목록 (기억한 순서).
    pub fn list_ignored_volumes(&self) -> Result<Vec<IgnoredVolume>, IngestError> {
        Ok(self.env.pool.read(load_ignored_volumes)?)
    }

    /// 디스크 기록 삭제용: `serialize` 를 쥔 채 `change`(DB 변경)를 실행하고, 성공하면 이 디스크를 처음 보는 것처럼 다시 판정한다
    /// (baseline·"이번 연결만 나중에" 에서 빼고 지금 연결돼 있으면 대기 목록에 오른다). 실패하면 상태는 그대로.
    pub(super) fn with_prompt_reset<T>(
        &self,
        volume_key: &str,
        change: impl FnOnce() -> Result<T, IngestError>,
    ) -> Result<T, IngestError> {
        let _g = lock(&self.newvol.serialize);
        let out = change()?;
        {
            let mut st = lock(&self.newvol.state);
            if let Some(b) = st.baseline.as_mut() {
                b.remove(volume_key);
            }
            st.dismissed.remove(volume_key);
        }
        self.refresh_locked();
        Ok(out)
    }

    /// [다시 묻기]. 목록에 없으면 아무것도 하지 않는다. 지금 연결돼 있고 관리 폴더가 없으면 바로 대기 목록에 오른다.
    pub fn unignore_volume(&self, volume_key: &str) -> Result<(), IngestError> {
        let _g = lock(&self.newvol.serialize);
        let key = volume_key.to_owned();
        let removed = self.env.write(Priority::High, move |c| {
            Ok(remove_ignored_volume(c, &key)?)
        })?;
        if !removed {
            return Ok(());
        }
        {
            let mut st = lock(&self.newvol.state);
            if let Some(b) = st.baseline.as_mut() {
                b.remove(volume_key); // 시작 때부터 연결돼 있던 디스크도 다시 묻는다
            }
            st.dismissed.remove(volume_key);
        }
        self.refresh_locked();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary (첫 점검, 같은 상태 반복) / Business rule (baseline·관리 폴더·묻지 않음·이번 연결만)
    use super::*;

    fn vol(key: &str, kind: VolumeKind) -> VolumeInfo {
        VolumeInfo {
            key: key.into(),
            kind,
            label: Some(key.into()),
            mount_point: format!(r"{key}:\"),
        }
    }

    fn set(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| (*k).to_owned()).collect()
    }

    #[test]
    fn t_newvol_unit_01_first_update_seeds_baseline_and_reports_nothing() {
        let mut st = State::default();

        let changed = st.update(&[vol("c", VolumeKind::Fixed)], &set(&[]), &set(&[]), |_| None);

        assert!(!changed);
        assert!(st.pending.is_empty());
        assert!(st.baseline.as_ref().unwrap().contains("c"));
    }

    #[test]
    fn t_newvol_unit_02_total_bytes_is_queried_once_per_connection() {
        // Arrange
        let mut st = State::default();
        let c = vol("c", VolumeKind::Fixed);
        st.update(std::slice::from_ref(&c), &set(&[]), &set(&[]), |_| None);
        let both = [c, vol("u", VolumeKind::Removable)];
        let calls = std::cell::Cell::new(0);

        // Act: 같은 상태로 두 번
        let first = st.update(&both, &set(&[]), &set(&[]), |_| {
            calls.set(calls.get() + 1);
            Some(64)
        });
        let second = st.update(&both, &set(&[]), &set(&[]), |_| {
            calls.set(calls.get() + 1);
            Some(64)
        });

        // Assert
        assert!(first && !second, "only the first reports a change");
        assert_eq!(calls.get(), 1);
        assert_eq!(st.pending[0].total_bytes, Some(64));
        assert_eq!(st.pending[0].kind, VolumeKind::Removable);
    }

    #[test]
    fn t_newvol_unit_03_network_kind_is_never_pending() {
        let mut st = State::default();
        st.update(&[], &set(&[]), &set(&[]), |_| None);

        st.update(&[vol("n", VolumeKind::Network)], &set(&[]), &set(&[]), |_| None);

        assert!(st.pending.is_empty());
    }

    #[test]
    fn t_newvol_unit_04_pending_serializes_with_the_contract_names() {
        let p = PendingVolume {
            volume_key: "k".into(),
            label: None,
            kind: VolumeKind::Removable,
            mount_point: r"E:\".into(),
            total_bytes: None,
        };

        let v = serde_json::to_value(PendingChangedPayload { volumes: vec![p] }).unwrap();

        assert_eq!(
            v,
            serde_json::json!({"volumes": [{"volumeKey": "k", "label": null, "kind": "removable",
                "mountPoint": "E:\\", "totalBytes": null}]})
        );
    }
}
