//! 검색용 공유 핸들 — 색인 워커(쓰기·재구축)와 독립적으로 여러 스레드에서 검색한다.
//!
//! 재구축은 `&mut SyncWorker` 를 오래(100만 건 약 34초) 쥔다. 검색은 워커가 아니라 이 핸들로 하며,
//! 핸들은 인덱스 `IndexReader` 복제본과 **세대**(generation)를 들고 있다. 재구축이 디렉터리를 교체하는
//! 짧은 구간(Windows 는 mmap 을 쥔 채 rename 할 수 없다)에만 쓰기 잠금으로 검색을 잠시 막고,
//! 교체가 끝나면 새 리더로 바꾸며 세대를 올린다. 검색은 읽기 잠금을 쥔 채 끝까지 실행하므로
//! 교체 시점에는 진행 중인 검색이 모두 끝나 있다.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use tantivy::IndexReader;

use crate::index::{IndexOptions, SearchIndex};
use crate::schema::Fields;

pub(crate) struct Live {
    pub reader: IndexReader,
    pub fields: Fields,
    pub options: IndexOptions,
}

pub(crate) struct Slot {
    pub live: Option<Live>,
    pub generation: u64,
}

impl Slot {
    pub(crate) fn publish(&mut self, index: Option<&SearchIndex>) {
        self.live = index.map(|i| Live {
            reader: i.reader.clone(),
            fields: i.fields,
            options: i.options.clone(),
        });
        self.generation += 1;
    }
}

/// 재구축이 디렉터리를 교체하는 동안 쥐는 쓰기 잠금. 만들 때 리더를 놓는다.
pub(crate) struct SwapGuard<'a>(RwLockWriteGuard<'a, Slot>);

impl SwapGuard<'_> {
    /// 새 인덱스(없으면 사용 불가 상태)를 게시하고 세대를 올린다.
    pub(crate) fn publish(&mut self, index: Option<&SearchIndex>) {
        self.0.publish(index);
    }
}

/// 복제 가능한 검색 핸들. `SyncWorker::search_handle` 로 얻는다.
#[derive(Clone)]
pub struct SearchHandle {
    inner: Arc<RwLock<Slot>>,
}

impl Default for SearchHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchHandle {
    pub fn new() -> Self {
        // 세대는 프로세스 사이에서도 겹치지 않게 시작 시각(ms)에서 시작한다 (옛 커서가 새 인덱스에 맞는 것을 막는다)
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_millis() as u64);
        let seed = (now_ms << 20) | (COUNTER.fetch_add(1, Ordering::Relaxed) & 0xf_ffff);
        Self {
            inner: Arc::new(RwLock::new(Slot {
                live: None,
                generation: seed,
            })),
        }
    }

    /// 이미 연 인덱스를 직접 검색하는 핸들 (`SyncWorker` 없이 쓰는 시험·도구용).
    pub fn for_index(index: &SearchIndex) -> Self {
        let h = Self::new();
        h.publish(Some(index));
        h
    }

    pub(crate) fn read(&self) -> RwLockReadGuard<'_, Slot> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn publish(&self, index: Option<&SearchIndex>) {
        self.inner
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .publish(index);
    }

    /// 검색을 막고 리더를 놓는다 (진행 중인 검색이 끝나기를 기다린다).
    pub(crate) fn begin_swap(&self) -> SwapGuard<'_> {
        let mut g = self.inner.write().unwrap_or_else(|e| e.into_inner());
        g.live = None;
        SwapGuard(g)
    }

    /// 현재 세대. 재구축·교체마다 올라간다.
    pub fn generation(&self) -> u64 {
        self.read().generation
    }

    /// 검색할 인덱스가 있는가 (재구축 실패·최초 구축 전에는 false).
    pub fn is_ready(&self) -> bool {
        self.read().live.is_some()
    }
}
