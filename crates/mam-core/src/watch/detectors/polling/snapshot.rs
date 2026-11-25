//! 디렉터리 단위 스냅숏과 가지치기 비교.
//!
//! 디렉터리마다 (수정 시각, 파일 이름 집합, 하위 폴더 이름 집합) 만 쥔다 — 파일 하나하나의 크기·시각은 쥐지 않는다.
//! 이름 집합은 정렬해 `\0` 로 이은 문자열 하나라서(파일 이름에는 제어 문자가 없다) 항목당 이름 길이 + 1 바이트가 든다.
//! 한 주기: 알고 있는 디렉터리의 수정 시각만 조회하고, **바뀐 디렉터리만** 목록을 읽어 이름 집합을 비교한다.
//! 디렉터리 수정 시각은 자식의 추가·삭제·이름 변경에는 바뀌지만 **자식 파일의 내용만 바뀐 경우에는 바뀌지 않는다**(NTFS 확인) —
//! 그 변경은 주기 재스캔이 잡는다.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::fs::{Listing, PollError, PollFs};

const RECENT_MS: i64 = 3_000;
const FUTURE_SKEW_MS: i64 = 60_000;

/// 정렬된 이름 집합 (`\0` 구분)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Names(Box<str>);

impl Names {
    fn of(mut v: Vec<String>) -> Self {
        v.sort_unstable();
        Self(v.join("\0").into_boxed_str())
    }

    fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.split('\0').filter(|s| !s.is_empty())
    }

    fn heap_bytes(&self) -> usize {
        self.0.len()
    }
}

#[derive(Debug, Clone)]
struct DirSnap {
    mtime: i64,
    /// 직전에 목록을 읽어 바뀐 것을 확인했다 — 수정 시각이 같아도 한 번 더 읽는다.
    /// 시각 해상도(SMB·FAT 는 최대 2초)가 거칠어 목록을 읽은 뒤 같은 칸 안에서 생긴 변경을 시각만으로는 알 수 없기 때문이다 (시계 비교 없이 해결).
    verify: bool,
    files: Names,
    dirs: Names,
}

#[derive(Debug, Default)]
pub struct Snapshot {
    dirs: HashMap<PathBuf, DirSnap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Path(PathBuf),
    Subtree(PathBuf),
}

/// 한 주기에 건 호출 수 (측정·진단용)
#[derive(Debug, Default, Clone, Copy)]
pub struct TickStats {
    pub stats: usize,
    pub lists: usize,
}

#[derive(Debug, Default)]
pub struct TickOutcome {
    pub changes: Vec<Change>,
    pub failure: Option<PollError>,
    pub stats: TickStats,
}

impl Snapshot {
    pub fn dir_count(&self) -> usize {
        self.dirs.len()
    }

    pub fn entry_count(&self) -> usize {
        self.dirs
            .values()
            .map(|d| d.files.iter().count() + d.dirs.iter().count())
            .sum()
    }

    /// 스냅숏이 쥔 대략의 힙 바이트 — 이름 문자열 + 디렉터리 항목(키 경로 + 구조체). 해시 테이블 여유분은 제외한다.
    pub fn approx_heap_bytes(&self) -> usize {
        self.dirs
            .iter()
            .map(|(k, v)| {
                k.as_os_str().len()
                    + std::mem::size_of::<PathBuf>()
                    + std::mem::size_of::<DirSnap>()
                    + v.files.heap_bytes()
                    + v.dirs.heap_bytes()
            })
            .sum()
    }
}

/// 읽을 수 없어 추적하지 않는 디렉터리인가 (사라졌거나 권한 없음) — 전체를 실패시키지 않고 건너뛴다.
fn skippable(e: PollError) -> bool {
    matches!(
        e,
        PollError::NotFound | PollError::Io(std::io::ErrorKind::PermissionDenied)
    )
}

/// 수정 시각이 "방금"인가 — 목록을 읽은 직후 같은 시각 칸 안에서 생긴 변경은 시각이 같아 보이므로 다음 주기에 한 번 더 읽는다.
/// PC 시계와 비교하므로 NAS 시계가 어긋나도 **누락은 생기지 않고**(어긋난 방향이 미래면 한 번 더 읽을 뿐, 과거면 아래 `changed` 검증이 받친다) 비용만 는다.
fn recent(mtime_ns: i64, now_ms: i64) -> bool {
    let age = now_ms.saturating_sub(mtime_ns / 1_000_000);
    age > -FUTURE_SKEW_MS && age < RECENT_MS
}

fn snap_of(mtime: i64, verify: bool, l: Listing) -> DirSnap {
    DirSnap {
        mtime,
        verify,
        files: Names::of(l.files),
        dirs: Names::of(l.dirs),
    }
}

/// `root` 아래 모든 디렉터리를 읽어 `out` 에 담는다 (조용히 — 보고 없음).
/// `top` 이면 `root` 자신을 못 읽을 때 오류, 그 밖의 디렉터리는 `skippable` 이면 건너뛴다.
fn walk_into(
    fs: &dyn PollFs,
    root: &Path,
    top: bool,
    now_ms: i64,
    out: &mut HashMap<PathBuf, DirSnap>,
    stop: &dyn Fn() -> bool,
    stats: &mut TickStats,
) -> Result<(), PollError> {
    let mut stack = vec![root.to_path_buf()];
    let mut first = top;
    while let Some(dir) = stack.pop() {
        if stop() {
            return Err(PollError::Stopped);
        }
        stats.stats += 1;
        stats.lists += 1;
        // 수정 시각을 목록보다 **먼저** 읽는다 — 그 사이 생긴 변경은 다음 주기에 시각이 달라 다시 읽힌다
        let read = fs.dir_mtime(&dir).and_then(|m| Ok((m, fs.list_dir(&dir)?)));
        let (mtime, listing) = match read {
            Ok(r) => r,
            Err(e) if !first && skippable(e) => continue,
            Err(e) => return Err(e),
        };
        first = false;
        stack.extend(listing.dirs.iter().map(|d| dir.join(d)));
        out.insert(dir, snap_of(mtime, recent(mtime, now_ms), listing));
    }
    Ok(())
}

/// `a` 에만 있는 이름, `b` 에만 있는 이름 (둘 다 정렬되어 있다).
fn diff<'a>(a: &'a Names, b: &'a Names) -> (Vec<&'a str>, Vec<&'a str>) {
    let (mut only_a, mut only_b) = (Vec::new(), Vec::new());
    let (mut ia, mut ib) = (a.iter().peekable(), b.iter().peekable());
    loop {
        match (ia.peek(), ib.peek()) {
            (None, None) => break,
            (Some(_), None) => only_a.extend(ia.by_ref()),
            (None, Some(_)) => only_b.extend(ib.by_ref()),
            (Some(x), Some(y)) => match x.cmp(y) {
                std::cmp::Ordering::Less => only_a.extend(ia.next()),
                std::cmp::Ordering::Greater => only_b.extend(ib.next()),
                std::cmp::Ordering::Equal => {
                    ia.next();
                    ib.next();
                }
            },
        }
    }
    (only_a, only_b)
}

/// 사라진 디렉터리와 그 하위를 스냅숏에서 뺀다.
fn forget(map: &mut HashMap<PathBuf, DirSnap>, dir: &Path) {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Some(s) = map.remove(&d) {
            stack.extend(s.dirs.iter().map(|n| d.join(n)));
        }
    }
}

impl Snapshot {
    /// 기준선 — 보고 없이 전체를 한 번 읽는다. 하나라도 실패하면 스냅숏을 만들지 않는다 (부분 스냅숏은 누락을 숨긴다).
    pub fn baseline(
        fs: &dyn PollFs,
        root: &Path,
        now_ms: i64,
        stop: &dyn Fn() -> bool,
    ) -> (Result<Self, PollError>, TickStats) {
        let mut stats = TickStats::default();
        let mut dirs = HashMap::new();
        let r = walk_into(fs, root, true, now_ms, &mut dirs, stop, &mut stats);
        (r.map(|()| Self { dirs }), stats)
    }

    /// 한 주기 — 수정 시각이 바뀐 디렉터리만 목록을 읽어 비교하고 스냅숏을 갱신한다.
    /// 디렉터리 단위로 원자적이다: 한 디렉터리의 읽기가 실패하면 그 디렉터리는 스냅숏을 바꾸지 않고(다음 주기에 다시 본다) 주기를 멈춘다.
    pub fn tick(
        &mut self,
        fs: &dyn PollFs,
        root: &Path,
        now_ms: i64,
        stop: &dyn Fn() -> bool,
    ) -> TickOutcome {
        let mut out = TickOutcome::default();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            if stop() {
                out.failure = Some(PollError::Stopped);
                break;
            }
            let Some(old) = self.dirs.get(&dir) else {
                continue;
            };
            out.stats.stats += 1;
            let is_root = dir == root;
            let mtime = match fs.dir_mtime(&dir) {
                Ok(m) => m,
                Err(e) if !is_root && skippable(e) => continue, // 부모의 다음 읽기가 알려 준다
                Err(e) => {
                    out.failure = Some(e);
                    break;
                }
            };
            if mtime == old.mtime && !old.verify {
                stack.extend(old.dirs.iter().map(|n| dir.join(n)));
                continue;
            }
            out.stats.lists += 1;
            let listing = match fs.list_dir(&dir) {
                Ok(l) => l,
                Err(e) if !is_root && skippable(e) => continue,
                Err(e) => {
                    out.failure = Some(e);
                    break;
                }
            };
            let new = snap_of(mtime, false, listing);
            let (gone_files, added_files) = diff(&old.files, &new.files);
            let (gone_dirs, added_dirs) = diff(&old.dirs, &new.dirs);
            let changed = !(gone_files.is_empty()
                && added_files.is_empty()
                && gone_dirs.is_empty()
                && added_dirs.is_empty());

            // 새 폴더는 안쪽까지 조용히 기준선에 담고 `Subtree` 한 건으로 보고한다 (그 안의 파일은 반영 쪽이 게이트를 거쳐 처리한다)
            let mut fresh = HashMap::new();
            let mut failure = None;
            for n in &added_dirs {
                if let Err(e) = walk_into(
                    fs,
                    &dir.join(n),
                    false,
                    now_ms,
                    &mut fresh,
                    stop,
                    &mut out.stats,
                ) {
                    failure = Some(e);
                    break;
                }
            }
            if failure.is_some() {
                out.failure = failure;
                break;
            }

            let mut changes: Vec<Change> = gone_files
                .iter()
                .chain(gone_dirs.iter())
                .map(|n| Change::Path(dir.join(n)))
                .collect();
            changes.extend(added_files.iter().map(|n| Change::Path(dir.join(n))));
            changes.extend(added_dirs.iter().map(|n| Change::Subtree(dir.join(n))));
            let gone: Vec<PathBuf> = gone_dirs.iter().map(|n| dir.join(n)).collect();
            let kept: Vec<PathBuf> = new.dirs.iter().map(|n| dir.join(n)).collect();

            out.changes.extend(changes);
            for g in &gone {
                forget(&mut self.dirs, g);
            }
            self.dirs.extend(fresh);
            self.dirs.insert(
                dir,
                DirSnap {
                    verify: changed || recent(mtime, now_ms),
                    ..new
                },
            );
            stack.extend(kept);
        }
        out
    }
}
