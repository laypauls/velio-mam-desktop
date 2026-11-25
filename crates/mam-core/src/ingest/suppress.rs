//! 감시 이벤트 억제 (P2-9) — 원본 이동이 관리 폴더 안으로 파일을 옮기는 동안, 대상 폴더의 감시 이벤트가
//! 이동 작업의 자산 재연결보다 먼저 반영돼 같은 파일이 새 자산으로 잡히는 경합을 줄인다.
//!
//! [`WatchSuppression`] 가드가 살아 있는 동안 그 폴더 아래 경로의 감시 이벤트는 버려진다(수신 시, 반영 직전 모두).
//! 가드가 풀린 뒤 늦게 도착하는 이벤트는 이미 재연결된 경로라 스캔의 멱등 판정이 처리한다.

use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::volume::strip_verbatim;

fn norm(p: &Path) -> String {
    strip_verbatim(&p.to_string_lossy())
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

#[derive(Default)]
pub(super) struct SuppressSet {
    dirs: Mutex<Vec<(u64, String)>>,
    next: Mutex<u64>,
}

impl SuppressSet {
    pub(super) fn add(self: &Arc<Self>, dir: &Path) -> WatchSuppression {
        let id = {
            let mut n = self.next.lock().unwrap_or_else(|p| p.into_inner());
            *n += 1;
            *n
        };
        self.dirs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push((id, norm(dir)));
        WatchSuppression {
            set: Arc::clone(self),
            id,
        }
    }

    pub(super) fn covers(&self, path: &Path) -> bool {
        let p = norm(path);
        self.dirs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|(_, d)| {
                p == *d
                    || p.strip_prefix(d.as_str())
                        .is_some_and(|r| r.starts_with('\\'))
            })
    }
}

/// Drop 시 억제를 푼다.
#[must_use = "suppression ends when the guard is dropped"]
pub struct WatchSuppression {
    set: Arc<SuppressSet>,
    id: u64,
}

impl Drop for WatchSuppression {
    fn drop(&mut self) {
        self.set
            .dirs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(id, _)| *id != self.id);
    }
}

impl std::fmt::Debug for WatchSuppression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchSuppression").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    // Categories: Happy (하위 경로 억제) / Boundary (형제 폴더·접두 이름) / Concurrency (가드 중첩·해제)
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn t_prot_14_covers_paths_under_the_dir_until_the_guard_drops() {
        // Arrange
        let set = Arc::new(SuppressSet::default());
        let dir = PathBuf::from(r"D:\Media\Inbox");

        // Act
        let g1 = set.add(&dir);
        let g2 = set.add(&dir);

        // Assert
        assert!(set.covers(Path::new(r"d:\media\inbox\a.jpg")));
        assert!(set.covers(Path::new(r"D:\Media\Inbox\sub\b.jpg")));
        assert!(set.covers(&dir));
        assert!(
            !set.covers(Path::new(r"D:\Media\Inbox2\a.jpg")),
            "prefix name"
        );
        assert!(!set.covers(Path::new(r"D:\Media\Other\a.jpg")));
        drop(g1);
        assert!(
            set.covers(Path::new(r"D:\Media\Inbox\a.jpg")),
            "nested guard"
        );
        drop(g2);
        assert!(!set.covers(Path::new(r"D:\Media\Inbox\a.jpg")));
    }
}
