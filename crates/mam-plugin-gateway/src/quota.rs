//! 플러그인별 KV 사용량 계량기(메모리). 세션 첫 `kv.set` 때 DB 합계로 초기화하고, 쓰기 클로저(단일 writer 스레드) 안에서만 갱신한다.
//! 행 비용 = `len(key) + len(value) + 64` 바이트 (`mam_db::plugin_kv::ROW_OVERHEAD`). CASCADE 삭제는 카운터를 모르게 줄이므로
//! 상한에 닿으면 호출자가 합계를 다시 계산해 `reset` 한다.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct Meter {
    used: AtomicU64,
    ready: AtomicBool,
}

impl Meter {
    pub(crate) fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub(crate) fn used(&self) -> u64 {
        self.used.load(Ordering::Acquire)
    }

    /// DB 합계로 (재)설정한다
    pub(crate) fn reset(&self, sum: i64) {
        self.used
            .store(u64::try_from(sum).unwrap_or(0), Ordering::Release);
        self.ready.store(true, Ordering::Release);
    }

    /// 줄어들거나 같은 변화는 항상 허용(상한을 넘은 상태에서도 삭제·축소는 된다). 늘어나면 `used + delta <= limit` 일 때만
    pub(crate) fn fits(&self, delta: i64, limit: u64) -> bool {
        match u64::try_from(delta) {
            Err(_) | Ok(0) => true,
            Ok(d) => self.used().checked_add(d).is_some_and(|n| n <= limit),
        }
    }

    /// 성공한 쓰기·삭제 뒤에만 부른다
    pub(crate) fn apply(&self, delta: i64) {
        let _ = self
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |u| {
                Some(match u64::try_from(delta) {
                    Ok(d) => u.saturating_add(d),
                    Err(_) => u.saturating_sub(delta.unsigned_abs()),
                })
            });
    }
}

#[derive(Default)]
pub(crate) struct Meters {
    map: Mutex<HashMap<String, Arc<Meter>>>,
}

impl Meters {
    pub(crate) fn of(&self, plugin: &str) -> Arc<Meter> {
        let mut m = self.map.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(m.entry(plugin.to_owned()).or_default())
    }

    /// 제거 뒤 계량기를 버린다 — 다음 `kv.set` 이 DB 합계로 다시 초기화한다 (재설치 시 낡은 사용량이 남지 않게)
    pub(crate) fn forget(&self, plugin: &str) {
        self.map
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(plugin);
    }
}

#[cfg(test)]
mod tests {
    use super::Meter;

    // 분류: Boundary

    #[test]
    fn t_pgw_quota_01_fits_allows_shrinking_even_when_over_the_limit() {
        let m = Meter::default();
        m.reset(100);
        assert!(
            m.fits(0, 50) && m.fits(-10, 50),
            "초과 상태에서도 축소·삭제는 허용"
        );
        assert!(m.fits(10, 110) && !m.fits(11, 110));
        assert!(!m.fits(i64::MAX, 50), "큰 증가는 거부");
        m.apply(-30);
        assert_eq!(m.used(), 70);
        m.apply(i64::MIN);
        assert_eq!(m.used(), 0, "saturating");
        assert!(m.ready());
    }
}
