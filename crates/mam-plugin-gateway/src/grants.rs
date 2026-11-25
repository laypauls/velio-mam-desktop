//! 권한 레지스트리 — 호출마다 읽기 락 1회 + 비트 검사(핫 경로에 DB 읽기 없음).
//!
//! `effective = manifest 가 선언한 권한 ∩ 사용자가 허용한 권한(plugin_granted)`, 비활성이면 0. **실패 시 닫힘**: 모르는·비활성 플러그인,
//! 손상된 JSON, 모르는 권한 문자열(교집합에서 탈락), 설정을 읽지 못한 경우 모두 거부한다. 채움·무효화는 `register`·`refresh`·`unregister` 로 한다
//! (P6 이 설치·활성·권한 변경·업그레이드·제거 뒤에 부른다). 레지스트리 락은 말단 락 — DB·호스트 호출 중에는 쥐지 않는다.

use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use mam_db::plugin_settings::{ENABLED, GRANTED, get, parse_enabled, parse_granted};
use mam_db::{DbPool, PoolError};
use mam_plugin_manifest::{Permission, PluginId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub enabled: bool,
    /// 6종 권한 비트마스크 (비활성이면 0)
    pub effective: u8,
}

const CLOSED: Grant = Grant {
    enabled: false,
    effective: 0,
};

fn bit(p: Permission) -> u8 {
    1 << Permission::ALL.iter().position(|q| *q == p).unwrap_or(0)
}

fn mask(perms: &BTreeSet<Permission>) -> u8 {
    perms.iter().fold(0, |m, p| m | bit(*p))
}

struct Entry {
    id: PluginId,
    manifest: u8,
    grant: Arc<Grant>,
}

#[derive(Default)]
pub struct GrantRegistry {
    map: RwLock<HashMap<String, Entry>>,
}

/// 설정 원문 2개 -> 유효 권한. 순수 함수 — 손상·모르는 값은 모두 권한 없음으로 떨어진다
pub fn compute_grant(manifest: u8, enabled_raw: Option<&str>, granted_raw: Option<&str>) -> Grant {
    let enabled = enabled_raw.is_some_and(parse_enabled);
    if !enabled {
        return CLOSED;
    }
    let granted = granted_raw.map(parse_granted).unwrap_or_default();
    let allowed = granted
        .iter()
        .filter_map(|s| Permission::from_str(s).ok())
        .fold(0u8, |m, p| m | bit(p));
    Grant {
        enabled,
        effective: manifest & allowed,
    }
}

impl GrantRegistry {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, Entry>> {
        self.map.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, Entry>> {
        self.map.write().unwrap_or_else(|p| p.into_inner())
    }

    /// 설치·업그레이드 시 manifest 가 선언한 권한을 등록한다. 허용 상태는 `refresh` 전까지 닫힘.
    pub fn register(&self, id: &PluginId, manifest_perms: &BTreeSet<Permission>) {
        self.write().insert(
            id.as_str().to_owned(),
            Entry {
                id: id.clone(),
                manifest: mask(manifest_perms),
                grant: Arc::new(CLOSED),
            },
        );
    }

    pub fn unregister(&self, id: &PluginId) {
        self.write().remove(id.as_str());
    }

    pub fn registered_ids(&self) -> Vec<PluginId> {
        self.read().values().map(|e| e.id.clone()).collect()
    }

    /// 지금 유효한 권한 (모르는 플러그인은 `None`)
    pub fn grant_of(&self, id: &str) -> Option<Grant> {
        self.read().get(id).map(|e| *e.grant)
    }

    /// 호출 1회의 검사. 모르는·비활성·권한 없음은 `false` (읽기 락 1회, `Arc` 를 쥐고 나가지 않는다)
    pub fn allows(&self, id: &str, perm: Permission) -> bool {
        self.read()
            .get(id)
            .is_some_and(|e| e.grant.enabled && e.grant.effective & bit(perm) != 0)
    }

    /// 설정 키 2개를 DB 에서 한 번 읽어 반영한다. 읽기에 실패하면 닫고 오류를 돌려준다(실패 시 닫힘).
    pub fn refresh(
        &self,
        pool: &DbPool,
        id: &PluginId,
        deadline: Instant,
    ) -> Result<(), PoolError> {
        let key = id.as_str().to_owned();
        let read = pool.read_until(deadline, |c| {
            Ok((get(c, ENABLED, &key)?, get(c, GRANTED, &key)?))
        });
        let mut map = self.write();
        let Some(entry) = map.get_mut(id.as_str()) else {
            return read.map(|_| ());
        };
        match read {
            Ok((enabled, granted)) => {
                entry.grant = Arc::new(compute_grant(
                    entry.manifest,
                    enabled.as_deref(),
                    granted.as_deref(),
                ));
                Ok(())
            }
            Err(e) => {
                entry.grant = Arc::new(CLOSED);
                Err(e)
            }
        }
    }

    /// 부팅 시 — 등록된 모든 플러그인을 다시 읽는다. 하나라도 실패하면(그 플러그인은 닫힘) 첫 오류를 돌려준다
    pub fn refresh_all(&self, pool: &DbPool, deadline: Instant) -> Result<(), PoolError> {
        let mut first_err = None;
        for id in self.registered_ids() {
            if let Err(e) = self.refresh(pool, &id, deadline) {
                first_err.get_or_insert(e);
            }
        }
        first_err.map_or(Ok(()), Err)
    }
}
