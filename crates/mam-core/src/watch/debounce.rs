//! 이벤트 디바운서 — 같은 키의 이벤트가 `quiet_ms` 동안 더 오지 않으면 한 번만 내보낸다 (D5: 2초).
//! 시각은 호출자가 넘긴다(시계 주입) — 시험에서 실제 시간을 기다리지 않는다.

use std::collections::HashMap;
use std::hash::Hash;

pub struct Debouncer<K> {
    quiet_ms: i64,
    last: HashMap<K, i64>,
}

impl<K: Hash + Eq + Clone> Debouncer<K> {
    pub fn new(quiet_ms: i64) -> Self {
        Self {
            quiet_ms,
            last: HashMap::new(),
        }
    }

    /// 이벤트를 기록한다. 같은 키가 다시 오면 대기 시간이 처음부터 다시 시작된다.
    pub fn push(&mut self, key: K, now_ms: i64) {
        self.last.insert(key, now_ms);
    }

    /// 조용해진 키들을 마지막 이벤트 시각 순으로 꺼낸다.
    pub fn ready(&mut self, now_ms: i64) -> Vec<K> {
        let mut due: Vec<(i64, K)> = self
            .last
            .iter()
            .filter(|(_, t)| now_ms - **t >= self.quiet_ms)
            .map(|(k, t)| (*t, k.clone()))
            .collect();
        due.sort_by_key(|(t, _)| *t);
        for (_, k) in &due {
            self.last.remove(k);
        }
        due.into_iter().map(|(_, k)| k).collect()
    }

    pub fn len(&self) -> usize {
        self.last.len()
    }

    pub fn is_empty(&self) -> bool {
        self.last.is_empty()
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Time / Boundary
    use super::*;

    #[test]
    fn t_watch_deb_01_emits_once_after_quiet_period() {
        let mut d = Debouncer::new(2_000);
        d.push("a", 0);
        d.push("a", 500); // 같은 경로의 연속 이벤트는 하나로
        d.push("a", 900);
        assert!(
            d.ready(2_899).is_empty(),
            "quiet period restarts at the last event"
        );
        assert_eq!(d.ready(2_900), vec!["a"]);
        assert!(d.ready(10_000).is_empty(), "emitted exactly once");
        assert!(d.is_empty());
    }

    #[test]
    fn t_watch_deb_02_keys_are_independent_and_ordered_by_last_event() {
        let mut d = Debouncer::new(1_000);
        d.push("b", 100);
        d.push("a", 50);
        d.push("c", 1_500);
        assert_eq!(d.ready(1_200), vec!["a", "b"]);
        assert_eq!(d.len(), 1);
    }
}
