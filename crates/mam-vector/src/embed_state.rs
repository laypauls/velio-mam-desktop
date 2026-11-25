//! `EmbedStore` 쓰기 쪽 프로세스 상태.
//!
//! - **raw 참조 하한(floor) 캐시**: `put` 이 매번 `MAX(raw_offset)` 로 전체 행을 훑지 않도록 키별로 기억한다.
//!   단일 쓰기 프로세스 가정 — 다른 프로세스가 같은 DB 에 쓰면 캐시가 낮아질 수 있으나, 쓰기 직전
//!   "파일이 캐시된 floor 보다 짧으면 DB 로 다시 확인"하므로(`EmbedStore::put`) 손상 복구 보장은 유지된다.
//! - **변경 로그**: `put` 이 커밋한 뒤 (키별) 건드린 자산 id 를 순번과 함께 남긴다. 활성 맵 캐시
//!   ([`crate::ActiveMapCache`])가 "바뀌었는가"를 SQL 없이 판정하고(순번 비교), 바뀐 자산만 DB 에서 다시 읽는다.
//!   프로세스 전역이라 `EmbedStore` 인스턴스가 여럿이어도 같은 키는 같은 로그를 본다.
//!   로그가 상한을 넘으면 오래된 쪽을 버리고, 그 구간에 걸린 소비자는 전체 재로드한다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// 한 키의 로그 상한 (자산 id 8B x 100만 = 8MB).
const MAX_LOG: usize = 1 << 20;

/// 변경 표식: `seq` 는 `put` 커밋마다, `resets` 는 `put` 을 거치지 않은 변경 통지마다 오른다.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChangeEpoch {
    pub(crate) seq: u64,
    pub(crate) resets: u64,
}

#[derive(Default)]
struct KeyLog {
    seq: u64,
    resets: u64,
    /// `touched[i]` 는 순번 `base + 1 + i` 의 변경
    base: u64,
    touched: Vec<u64>,
}

static LOGS: LazyLock<Mutex<HashMap<PathBuf, KeyLog>>> = LazyLock::new(Mutex::default);

fn with_log<R>(dir: &Path, f: impl FnOnce(&mut KeyLog) -> R) -> R {
    let mut g = LOGS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.entry(dir.to_path_buf()).or_default())
}

pub(crate) fn epoch(dir: &Path) -> ChangeEpoch {
    with_log(dir, |l| ChangeEpoch {
        seq: l.seq,
        resets: l.resets,
    })
}

/// `put` 이 커밋한 뒤 자산을 기록한다.
pub(crate) fn record_touch(dir: &Path, asset: u64) {
    with_log(dir, |l| {
        l.seq += 1;
        l.touched.push(asset);
        if l.touched.len() > MAX_LOG {
            l.touched.drain(..MAX_LOG / 2);
            l.base += (MAX_LOG / 2) as u64;
        }
    });
}

/// `put` 을 거치지 않은 변경 — 소비자는 전체 재로드해야 한다.
pub(crate) fn bump_reset(dir: &Path) {
    with_log(dir, |l| l.resets += 1);
}

/// 순번 `(from, to]` 에 건드린 자산들. 그 구간이 이미 버려졌으면 `None`.
pub(crate) fn touched_between(dir: &Path, from: u64, to: u64) -> Option<Vec<u64>> {
    with_log(dir, |l| {
        if from < l.base || to > l.seq {
            return None;
        }
        let (lo, hi) = ((from - l.base) as usize, (to - l.base) as usize);
        l.touched.get(lo..hi).map(<[u64]>::to_vec)
    })
}

/// 키별 "SQLite 가 참조하는 raw 끝 오프셋" 하한.
#[derive(Default)]
pub(crate) struct Floors(Mutex<HashMap<PathBuf, u64>>);

impl Floors {
    pub(crate) fn get(&self, key_dir: &Path) -> Option<u64> {
        let g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.get(key_dir).copied()
    }

    pub(crate) fn set(&self, key_dir: &Path, end: u64) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        g.insert(key_dir.to_path_buf(), end);
    }
}
