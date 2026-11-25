//! `polling` 시험 (T-poll-01 ~ 20).
//!
//! 분류: Happy (01 기준선 무보고, 02 추가·삭제·이름 변경, 03 새 폴더 -> Subtree) / Boundary (05 같은 시각 칸, 12 상한, 15 값 범위) /
//! Business rule (04 내용만 바뀜 미감지, 11 응답 없음 != 삭제, 14 자동 선택 제외) / External (07~10 응답 없음·실패·회복, 16~17 제한 시간) /
//! Concurrency (18 중지가 즉시) / 원본 보호 (19 실제 파일시스템).
//! 가짜 파일시스템(트리 + 수정 시각)과 주입 시계로 결정적으로 돌린다 — 스레드가 필요한 시험만 짧은 실시간 대기를 쓴다.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use super::*;
use crate::scan::io_guard::IoGuardConfig;
use crate::watch::contract::{manager_for, target};
use crate::watch::detector::{DetectError, ProbeResult};
use crate::watch::health::SourceHealth;
use crate::watch::manager::WatchManager;
use crate::watch::registry::DetectorRegistry;
use crate::watch::sink::WatchMsg;
use crate::watch::{ChangeDetector, WatchHandle};

const NOW: i64 = 1_700_000_000_000;
const MS: i64 = 1_000_000;
/// 모든 디렉터리의 처음 수정 시각 — "방금"이 아니도록 한 시간 전
const OLD: i64 = (NOW - 3_600_000) * MS;

#[derive(Default)]
struct FDir {
    mtime: i64,
    files: BTreeSet<String>,
    dirs: BTreeSet<String>,
}

#[derive(Default)]
struct Inner {
    dirs: BTreeMap<PathBuf, FDir>,
    clock: i64,
    fail: Option<PollError>,
    fail_list_at: Option<PathBuf>,
    denied: BTreeSet<PathBuf>,
    lists: usize,
}

struct FakeFs {
    root: PathBuf,
    inner: Mutex<Inner>,
}

impl FakeFs {
    fn new() -> Arc<Self> {
        let root = PathBuf::from("/lib");
        let mut inner = Inner { clock: OLD, ..Inner::default() };
        inner.dirs.insert(root.clone(), FDir { mtime: OLD, ..FDir::default() });
        Arc::new(Self { root, inner: Mutex::new(inner) })
    }

    fn p(&self, rel: &str) -> PathBuf {
        rel.split('/').filter(|s| !s.is_empty()).fold(self.root.clone(), |acc, s| acc.join(s))
    }

    fn with<T>(&self, f: impl FnOnce(&mut Inner) -> T) -> T {
        f(&mut self.inner.lock().unwrap())
    }

    /// 자식 변경 — 부모 디렉터리의 수정 시각이 오른다 (`bump = false` 면 시각 해상도 때문에 같은 값인 경우)
    fn touch(i: &mut Inner, dir: &Path, bump: bool) {
        i.clock += MS;
        let t = i.clock;
        if bump {
            i.dirs.get_mut(dir).unwrap().mtime = t;
        }
    }

    fn mkdir(&self, rel: &str) {
        let path = self.p(rel);
        self.with(|i| {
            let parent = path.parent().unwrap().to_path_buf();
            i.dirs.get_mut(&parent).unwrap().dirs.insert(path.file_name().unwrap().to_string_lossy().into_owned());
            i.clock += MS;
            let t = i.clock;
            i.dirs.insert(path, FDir { mtime: t, ..FDir::default() });
            Self::touch(i, &parent, true);
        });
    }

    fn add_file(&self, rel: &str) {
        self.add_file_with(rel, true);
    }

    fn add_file_with(&self, rel: &str, bump: bool) {
        let path = self.p(rel);
        self.with(|i| {
            let dir = path.parent().unwrap().to_path_buf();
            i.dirs.get_mut(&dir).unwrap().files.insert(path.file_name().unwrap().to_string_lossy().into_owned());
            Self::touch(i, &dir, bump);
        });
    }

    fn rm_file(&self, rel: &str) {
        let path = self.p(rel);
        self.with(|i| {
            let dir = path.parent().unwrap().to_path_buf();
            i.dirs.get_mut(&dir).unwrap().files.remove(path.file_name().unwrap().to_string_lossy().as_ref());
            Self::touch(i, &dir, true);
        });
    }

    fn rename_file(&self, dir: &str, from: &str, to: &str) {
        self.rm_file(&format!("{dir}/{from}"));
        self.add_file(&format!("{dir}/{to}"));
    }

    fn rmdir(&self, rel: &str) {
        let path = self.p(rel);
        self.with(|i| {
            let parent = path.parent().unwrap().to_path_buf();
            i.dirs.get_mut(&parent).unwrap().dirs.remove(path.file_name().unwrap().to_string_lossy().as_ref());
            i.dirs.retain(|k, _| !k.starts_with(&path));
            Self::touch(i, &parent, true);
        });
    }

    fn set_fail(&self, e: Option<PollError>) {
        self.with(|i| i.fail = e);
    }

    fn lists(&self) -> usize {
        self.with(|i| i.lists)
    }
}

impl PollFs for FakeFs {
    fn dir_mtime(&self, dir: &Path) -> Result<i64, PollError> {
        self.with(|i| {
            if let Some(e) = i.fail {
                return Err(e);
            }
            if i.denied.contains(dir) {
                return Err(PollError::Io(std::io::ErrorKind::PermissionDenied));
            }
            i.dirs.get(dir).map(|d| d.mtime).ok_or(PollError::NotFound)
        })
    }

    fn list_dir(&self, dir: &Path) -> Result<Listing, PollError> {
        self.with(|i| {
            i.lists += 1;
            if let Some(e) = i.fail {
                return Err(e);
            }
            if i.fail_list_at.as_deref() == Some(dir) {
                return Err(PollError::TimedOut);
            }
            let d = i.dirs.get(dir).ok_or(PollError::NotFound)?;
            Ok(Listing {
                files: d.files.iter().cloned().collect(),
                dirs: d.dirs.iter().cloned().collect(),
            })
        })
    }
}

fn rel(fs: &FakeFs, p: &Path) -> String {
    p.strip_prefix(&fs.root).unwrap().components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// 변경을 "P:상대경로" / "S:상대경로" 집합으로 본다
fn set(fs: &FakeFs, changes: &[Change]) -> BTreeSet<String> {
    changes
        .iter()
        .map(|c| match c {
            Change::Path(p) => format!("P:{}", rel(fs, p)),
            Change::Subtree(p) => format!("S:{}", rel(fs, p)),
        })
        .collect()
}

fn strs(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

/// 트리: lib/{top.txt, a/{x.txt, y.txt, deep/{z.txt}}, b/{w.txt}}
fn tree() -> Arc<FakeFs> {
    let fs = FakeFs::new();
    fs.mkdir("a");
    fs.mkdir("a/deep");
    fs.mkdir("b");
    for f in ["top.txt", "a/x.txt", "a/y.txt", "a/deep/z.txt", "b/w.txt"] {
        fs.add_file(f);
    }
    fs
}

fn baseline(fs: &FakeFs) -> Snapshot {
    Snapshot::baseline(fs, &fs.root, NOW, &|| false).0.unwrap()
}

fn tick(fs: &FakeFs, snap: &mut Snapshot) -> TickOutcome {
    snap.tick(fs, &fs.root, NOW, &|| false)
}

#[test]
fn t_poll_01_baseline_reports_nothing_and_an_unchanged_tick_only_stats_directories() {
    // Arrange / Act
    let fs = tree();
    let mut snap = baseline(&fs);
    let lists_after_baseline = fs.lists();
    let out = tick(&fs, &mut snap);

    // Assert: 4 디렉터리·5 파일 + 하위 폴더 3 = 8 항목, 변경 없는 주기는 목록을 한 번도 읽지 않는다 (가지치기)
    assert_eq!((snap.dir_count(), snap.entry_count()), (4, 8));
    assert!(out.changes.is_empty() && out.failure.is_none());
    assert_eq!((out.stats.stats, out.stats.lists), (4, 0));
    assert_eq!(fs.lists(), lists_after_baseline);
}

#[test]
fn t_poll_02_add_delete_and_rename_in_a_nested_directory_are_reported_as_paths_and_only_that_directory_is_listed() {
    // Arrange
    let fs = tree();
    let mut snap = baseline(&fs);
    let before = fs.lists();

    // Act: 중첩 폴더에서 추가·삭제·이름 변경
    fs.add_file("a/deep/new.txt");
    fs.rm_file("a/deep/z.txt");
    fs.rename_file("a/deep", "new.txt", "renamed.txt");
    let out = tick(&fs, &mut snap);

    // Assert
    assert_eq!(set(&fs, &out.changes), strs(&["P:a/deep/z.txt", "P:a/deep/renamed.txt"]));
    assert_eq!(fs.lists() - before, 1, "only the changed directory is listed");
    // 한 번 더 — 바뀐 폴더는 확인용으로 다시 읽지만 새 변경은 없다, 그다음엔 읽지 않는다
    let again = tick(&fs, &mut snap);
    assert!(again.changes.is_empty());
    assert_eq!(again.stats.lists, 1);
    assert_eq!(tick(&fs, &mut snap).stats.lists, 0);
}

#[test]
fn t_poll_03_new_directory_is_one_subtree_and_its_contents_join_the_snapshot_silently() {
    // Arrange
    let fs = tree();
    let mut snap = baseline(&fs);

    // Act: 새 폴더(안에 하위 폴더·파일)
    fs.mkdir("b/new");
    fs.mkdir("b/new/inner");
    fs.add_file("b/new/inner/f.txt");
    let out = tick(&fs, &mut snap);

    // Assert: Subtree 한 건, 안쪽 파일은 Path 로 따로 보고하지 않는다
    assert_eq!(set(&fs, &out.changes), strs(&["S:b/new"]));
    assert_eq!(snap.dir_count(), 6);
    // 새 폴더 안의 이후 변경은 스냅숏에 있어 Path 로 잡힌다
    fs.add_file("b/new/inner/g.txt");
    assert_eq!(set(&fs, &tick(&fs, &mut snap).changes), strs(&["P:b/new/inner/g.txt"]));
}

#[test]
fn t_poll_04_content_only_change_is_not_detected_documented_limitation() {
    // Arrange: 파일 내용만 바뀌면 디렉터리 수정 시각도 이름 집합도 바뀌지 않는다
    let fs = tree();
    let mut snap = baseline(&fs);

    // Act: (가짜에서는 아무 일도 일어나지 않는 것이 곧 내용만 바뀐 상태다)
    let out = tick(&fs, &mut snap);

    // Assert
    assert!(out.changes.is_empty());
    assert_eq!(out.stats.lists, 0);
}

#[test]
fn t_poll_05_a_change_in_the_same_timestamp_slot_after_a_listing_is_found_by_the_verification_read() {
    // Arrange
    let fs = tree();
    let mut snap = baseline(&fs);
    fs.add_file("b/first.txt");
    assert_eq!(set(&fs, &tick(&fs, &mut snap).changes), strs(&["P:b/first.txt"]));

    // Act: 목록을 읽은 뒤 같은 시각 칸에서 또 생겼다 — 수정 시각이 그대로다
    fs.add_file_with("b/second.txt", false);
    let verify = tick(&fs, &mut snap);

    // Assert: 확인용 재읽기가 잡고, 그다음에는 다시 읽지 않는다
    assert_eq!(set(&fs, &verify.changes), strs(&["P:b/second.txt"]));
    assert_eq!(tick(&fs, &mut snap).stats.lists, 1);
    assert_eq!(tick(&fs, &mut snap).stats.lists, 0);
}

#[test]
fn t_poll_05b_directories_modified_within_the_last_seconds_are_read_again_old_ones_are_not() {
    // Arrange: 루트만 방금(1초 전) 바뀐 것으로 한다
    let fs = tree();
    fs.with(|i| i.dirs.get_mut(&fs.root).unwrap().mtime = (NOW - 1_000) * MS);
    let mut snap = baseline(&fs);

    // Act: 시각이 같은 채로 파일이 생겼다
    fs.add_file_with("late.txt", false);
    let out = tick(&fs, &mut snap);

    // Assert: 방금 바뀐 루트만 다시 읽었고(목록 1회) 오래된 하위 폴더는 읽지 않았다
    assert_eq!(set(&fs, &out.changes), strs(&["P:late.txt"]));
    assert_eq!(out.stats.lists, 1);
}

#[test]
fn t_poll_06_removed_directory_is_a_path_and_its_subtree_leaves_the_snapshot() {
    // Arrange
    let fs = tree();
    let mut snap = baseline(&fs);

    // Act
    fs.rmdir("a");
    let out = tick(&fs, &mut snap);

    // Assert: 폴더 자신의 Path 한 건 (반영 쪽이 하위를 missing 으로 만든다), 스냅숏에서는 하위까지 빠진다
    assert_eq!(set(&fs, &out.changes), strs(&["P:a"]));
    assert_eq!(snap.dir_count(), 2);
}

#[test]
fn t_poll_07_renamed_directory_is_old_path_plus_new_subtree() {
    // Arrange
    let fs = tree();
    let mut snap = baseline(&fs);

    // Act: a -> a2 (안쪽 포함)
    fs.rmdir("a");
    fs.mkdir("a2");
    fs.add_file("a2/x.txt");
    let out = tick(&fs, &mut snap);

    // Assert
    assert_eq!(set(&fs, &out.changes), strs(&["P:a", "S:a2"]));
}

#[test]
fn t_poll_08_unreadable_subdirectory_does_not_fail_the_baseline_or_the_tick() {
    // Arrange: 시스템 폴더처럼 권한이 없는 하위 폴더
    let fs = tree();
    fs.mkdir("locked");
    let locked = fs.p("locked");
    fs.with(|i| i.denied.insert(locked));

    // Act
    let (snap, _) = Snapshot::baseline(&*fs, &fs.root, NOW, &|| false);
    let mut snap = snap.unwrap();
    let out = tick(&fs, &mut snap);

    // Assert
    assert!(out.failure.is_none() && out.changes.is_empty());
    assert_eq!(snap.dir_count(), 4, "the locked directory is not tracked");
}

// ---- Poller.step: 보고·상태 ----

struct Parking(Arc<Mutex<Option<ChangeSink>>>);

struct Handle;
impl WatchHandle for Handle {}

impl ChangeDetector for Parking {
    fn id(&self) -> &'static str {
        "polling"
    }
    fn probe(&self, _t: &WatchTarget) -> ProbeResult {
        ProbeResult::Available
    }
    fn start(&self, _t: &WatchTarget, _o: &DetectorOptions, sink: ChangeSink) -> Result<Box<dyn WatchHandle>, DetectError> {
        *self.0.lock().unwrap() = Some(sink);
        Ok(Box::new(Handle))
    }
}

struct Rig {
    m: WatchManager,
    rx: Receiver<WatchMsg>,
    sink: ChangeSink,
    fs: Arc<FakeFs>,
    poller: Poller,
}

impl Rig {
    fn new(fs: Arc<FakeFs>) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let (tx, rx) = std::sync::mpsc::channel();
        let registry = Arc::new(DetectorRegistry::new(vec![Arc::new(Parking(Arc::clone(&slot)))], &["polling"]));
        let m = WatchManager::new(tx, registry);
        m.start(&target(1, fs.root.clone(), crate::volume::VolumeKind::Network), None, None);
        let sink = slot.lock().unwrap().take().unwrap();
        Self { m, rx, sink, fs, poller: Poller::default() }
    }

    fn step(&mut self) -> Vec<WatchMsg> {
        let fs = Arc::clone(&self.fs);
        self.poller.step(&*fs, &fs.root, &self.sink, NOW, &|| false);
        self.rx.try_iter().collect()
    }

    fn health(&self) -> SourceHealth {
        self.m.health(1).unwrap()
    }
}

fn is_rescan(m: &WatchMsg) -> bool {
    matches!(m, WatchMsg::Rescan { .. })
}

#[test]
fn t_poll_09_first_step_builds_the_baseline_silently_and_health_stays_ok() {
    // Arrange
    let mut rig = Rig::new(tree());

    // Act
    let msgs = rig.step();

    // Assert
    assert!(msgs.is_empty());
    assert!(rig.poller.snapshot().is_some());
    assert!(matches!(rig.health(), SourceHealth::Ok { last_signal_at: None }));
}

#[test]
fn t_poll_10_changes_become_messages_for_the_folder_and_update_the_last_signal() {
    // Arrange
    let mut rig = Rig::new(tree());
    rig.step();

    // Act
    rig.fs.add_file("a/new.txt");
    rig.fs.mkdir("fresh");
    let msgs = rig.step();

    // Assert
    assert_eq!(msgs.len(), 2);
    assert!(msgs.iter().all(|m| m.folder_id() == 1));
    assert!(msgs.iter().any(|m| matches!(m, WatchMsg::Path { path, .. } if path.ends_with("new.txt"))));
    assert!(msgs.iter().any(|m| matches!(m, WatchMsg::Subtree { dir, .. } if dir.ends_with("fresh"))));
    assert!(matches!(rig.health(), SourceHealth::Ok { last_signal_at: Some(_) }));
}

#[test]
fn t_poll_11_unresponsive_tick_degrades_keeps_the_snapshot_and_recovery_sends_one_rescan_plus_the_gap_changes() {
    // Arrange
    let mut rig = Rig::new(tree());
    rig.step();

    // Act 1: 응답 없음 — 그 사이 파일이 생겼다
    rig.fs.add_file("b/during.txt");
    rig.fs.set_fail(Some(PollError::TimedOut));
    let during = rig.step();

    // Assert 1: 보고 없음(삭제로 오해해 아무것도 지우지 않는다), Degraded + 사유 키
    assert!(during.is_empty());
    assert!(matches!(rig.health(), SourceHealth::Degraded { reason_key: HEALTH_TIMEOUT, .. }));
    assert!(rig.poller.failing);
    assert_eq!(rig.poller.snapshot().unwrap().dir_count(), 4);
    // 계속 실패해도 Rescan 은 없다
    assert!(rig.step().iter().all(|m| !is_rescan(m)));

    // Act 2: 회복
    rig.fs.set_fail(None);
    let after = rig.step();

    // Assert 2: 밀린 변경은 Path 로, 간격을 덮는 Rescan 은 정확히 한 번, 상태는 Ok
    assert_eq!(after.iter().filter(|m| is_rescan(m)).count(), 1);
    assert!(after.iter().any(|m| matches!(m, WatchMsg::Path { path, .. } if path.ends_with("during.txt"))));
    assert!(matches!(rig.health(), SourceHealth::Ok { .. }));
    assert!(rig.step().is_empty(), "no second rescan");
}

#[test]
fn t_poll_12_baseline_failure_degrades_then_recovery_builds_it_with_one_rescan_and_no_paths() {
    // Arrange: 첫 기준선이 읽기 오류로 실패한다
    let fs = tree();
    fs.set_fail(Some(PollError::Io(std::io::ErrorKind::ConnectionReset)));
    let mut rig = Rig::new(fs);

    // Act 1
    let first = rig.step();

    // Assert 1
    assert!(first.is_empty());
    assert!(rig.poller.snapshot().is_none());
    assert!(matches!(rig.health(), SourceHealth::Degraded { reason_key: HEALTH_ERROR, .. }));

    // Act 2
    rig.fs.set_fail(None);
    let second = rig.step();

    // Assert 2: 기존 파일은 보고하지 않고(기준선), Rescan 한 번만
    assert_eq!(second.len(), 1);
    assert!(is_rescan(&second[0]));
    assert!(rig.poller.snapshot().is_some());
    assert!(matches!(rig.health(), SourceHealth::Ok { .. }));
}

#[test]
fn t_poll_13_a_failure_in_one_directory_keeps_earlier_directories_reported_and_leaves_that_one_for_the_next_tick() {
    // Arrange
    let fs = tree();
    let mut snap = baseline(&fs);
    fs.add_file("a/early.txt");
    fs.add_file("b/late.txt");
    let a = fs.p("a");
    let b = fs.p("b");
    fs.with(|i| i.fail_list_at = Some(b.clone()));

    // Act 1: b 는 읽다가 실패한다 (스택 순서상 a 가 먼저일 수도, 뒤일 수도 있다)
    let out = tick(&fs, &mut snap);

    // Assert 1: 실패는 알리고, b 의 변경은 아직 보고하지 않는다
    assert_eq!(out.failure, Some(PollError::TimedOut));
    assert!(!set(&fs, &out.changes).contains("P:b/late.txt"));

    // Act 2: 회복 — 남은 변경이 모두 한 번씩만 나온다
    fs.with(|i| i.fail_list_at = None);
    let rest = tick(&fs, &mut snap);
    let all: Vec<_> = out.changes.iter().chain(rest.changes.iter()).cloned().collect();

    // Assert 2
    assert_eq!(all.len(), 2, "each change is reported exactly once: {all:?}");
    assert_eq!(set(&fs, &all), strs(&["P:a/early.txt", "P:b/late.txt"]));
    let _ = (a, b);
}

#[test]
fn t_poll_14_more_changes_than_the_cap_are_replaced_by_one_rescan() {
    // Arrange
    let fs = tree();
    let mut rig = Rig::new(Arc::clone(&fs));
    rig.step();
    fs.with(|i| {
        let d = i.dirs.get_mut(&fs.root).unwrap();
        for n in 0..(MAX_CHANGES_PER_TICK + 10) {
            d.files.insert(format!("f{n:06}.jpg"));
        }
        d.mtime += MS;
    });

    // Act
    let msgs = rig.step();

    // Assert
    assert_eq!(msgs.iter().filter(|m| matches!(m, WatchMsg::Path { .. })).count(), MAX_CHANGES_PER_TICK);
    assert_eq!(msgs.iter().filter(|m| is_rescan(m)).count(), 1);
}

// ---- 감지기 설명·옵션·등록 ----

#[test]
fn t_poll_15_interval_option_range_type_and_default_are_validated_and_declared() {
    // Arrange
    let d = Polling::new();

    // Assert: 선언 (UI 가 그대로 그린다)
    assert_eq!(d.id(), "polling");
    assert!(d.experimental());
    assert_eq!(d.option_fields(), FIELDS);
    assert_eq!((FIELDS[0].min, FIELDS[0].max, FIELDS[0].default), (60, 86_400, 300));
    // 경계는 포함, 기본값은 저장하지 않는다
    assert_eq!(d.parse_options(Some(&json!({"intervalSec": 60}))).unwrap().get_i64("intervalSec"), Some(60));
    assert_eq!(d.parse_options(Some(&json!({"intervalSec": 86_400}))).unwrap().get_i64("intervalSec"), Some(86_400));
    assert_eq!(d.parse_options(Some(&json!({"intervalSec": 300}))).unwrap().to_stored(), None);
    // 거부: 하한 미만·상한 초과·잘못된 형·알 수 없는 키
    for bad in [
        json!({"intervalSec": 59}),
        json!({"intervalSec": 86_401}),
        json!({"intervalSec": 0}),
        json!({"intervalSec": -1}),
        json!({"intervalSec": "60"}),
        json!({"intervalSec": 60.5}),
        json!({"intervalSec": null}),
        json!({"interval": 60}),
    ] {
        assert!(matches!(d.parse_options(Some(&bad)), Err(DetectError::InvalidOptions { .. })), "{bad}");
    }
}

#[test]
fn t_poll_16_polling_is_registered_but_never_picked_by_automatic_selection() {
    // Arrange
    let registry = DetectorRegistry::platform_default();

    for kind in [crate::volume::VolumeKind::Fixed, crate::volume::VolumeKind::Removable, crate::volume::VolumeKind::Network] {
        // Act: 자동 (저장된 방식 없음)
        let plan = registry.plan(&target(1, PathBuf::from("."), kind), None, None);

        // Assert
        assert!(plan.candidates.iter().all(|c| c.detector.id() != "polling"), "{kind:?}");
    }
    // 사용자가 고르면 쓴다 (옵션 포함)
    let explicit = registry.plan(&target(1, PathBuf::from("."), crate::volume::VolumeKind::Network), Some("polling"), Some(r#"{"intervalSec":120}"#));
    assert_eq!(explicit.candidates[0].detector.id(), "polling");
    assert_eq!(explicit.candidates[0].options.get_i64("intervalSec"), Some(120));
    assert!(explicit.reason.is_none());
    // 저장된 옵션이 범위 밖이면 자동으로 폴백한다
    let bad = registry.plan(&target(1, PathBuf::from("."), crate::volume::VolumeKind::Network), Some("polling"), Some(r#"{"intervalSec":5}"#));
    assert!(bad.candidates.iter().all(|c| c.detector.id() != "polling") && bad.reason.is_some());
}

// ---- io_guard 와 중지 ----

/// 열 때까지 막히는 가짜 — 응답 없는 공유
struct Gate {
    open: Mutex<bool>,
    cv: Condvar,
    entered: std::sync::atomic::AtomicUsize,
}

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self { open: Mutex::new(false), cv: Condvar::new(), entered: std::sync::atomic::AtomicUsize::new(0) })
    }
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.cv.notify_all();
    }
    fn wait(&self) {
        self.entered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let g = self.open.lock().unwrap();
        let _g = self.cv.wait_while(g, |o| !*o).unwrap();
    }
}

struct Hung(Arc<Gate>);

impl PollFs for Hung {
    fn dir_mtime(&self, _dir: &Path) -> Result<i64, PollError> {
        self.0.wait();
        Ok(OLD)
    }
    fn list_dir(&self, _dir: &Path) -> Result<Listing, PollError> {
        self.0.wait();
        Ok(Listing::default())
    }
}

fn guarded(gate: &Arc<Gate>, cfg: IoGuardConfig, stop: &Arc<StopSignal>) -> GuardedFs {
    GuardedFs {
        inner: Arc::new(Hung(Arc::clone(gate))),
        guard: IoGuard::new(cfg),
        lane: 1,
        timeout: cfg.timeout,
        slice: Duration::from_millis(20),
        stop: Arc::clone(stop),
    }
}

#[test]
fn t_poll_17_hung_call_times_out_without_piling_up_threads_and_recovers_when_released() {
    // Arrange
    let gate = Gate::new();
    let cfg = IoGuardConfig { timeout: Duration::from_millis(150), max_per_volume: 1 };
    let g = guarded(&gate, cfg, &Arc::new(StopSignal::default()));

    // Act 1: 막힌 호출 -> 제한 시간 후 TimedOut (스레드 1개가 뒤에서 막혀 있다)
    let started = Instant::now();
    let first = g.dir_mtime(Path::new("/lib"));
    // 다른 경로 — 볼륨(차선) 상한에 닿아 스레드를 만들지 않고 곧바로 돌아온다
    let second = g.dir_mtime(Path::new("/lib/other"));

    // Assert 1
    assert_eq!(first, Err(PollError::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(second, Err(PollError::TimedOut));
    // 시계가 아니라 호출 수로 확인한다: 상한에 닿은 두 번째 검사는 파일 호출을 시작하지 않았다
    assert!(gate.entered.load(std::sync::atomic::Ordering::SeqCst) <= 1, "cap hit never starts a call");
    assert_eq!(g.guard.live_checks(1), 1, "no extra threads for blocked calls");

    // Act 2: 풀린다 -> 다음 호출은 성공
    gate.open();
    let third = g.dir_mtime(Path::new("/lib"));

    // Assert 2
    assert_eq!(third, Ok(OLD));
}

#[test]
fn t_poll_18_stop_ends_the_thread_promptly_even_while_the_volume_is_hung() {
    // Arrange: 막힌 네트워크 볼륨 + 아주 긴 간격
    let gate = Gate::new();
    let cfg = IoGuardConfig { timeout: Duration::from_secs(30), max_per_volume: 4 };
    let d = Arc::new(Polling::for_tests(Some(Arc::new(Hung(Arc::clone(&gate)))), Duration::from_secs(3600), cfg));
    let (m, _rx) = manager_for(d);
    let dir = tempfile::tempdir().unwrap();
    let started = Instant::now();
    m.start(&target(1, dir.path().to_path_buf(), crate::volume::VolumeKind::Network), None, None);

    // Act
    m.stop(1);
    let released = m.wait_released(Duration::from_secs(5));

    // Assert: 막힌 호출이 30초 제한을 채우도록 기다리지 않는다
    assert!(released);
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    gate.open(); // 뒤에서 막혀 있던 검사 스레드를 정리한다
}

#[test]
fn t_poll_19_stop_during_the_idle_wait_releases_immediately_for_a_local_folder() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let d = Arc::new(Polling::for_tests(None, Duration::from_secs(3600), IoGuardConfig::default()));
    let (m, _rx) = manager_for(d);
    m.start(&target(1, dir.path().to_path_buf(), crate::volume::VolumeKind::Fixed), None, None);
    assert!(m.live_tokens() >= 1);

    // Act
    let started = Instant::now();
    m.stop(1);

    // Assert
    assert!(m.wait_released(Duration::from_secs(2)));
    assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
}

// ---- 실제 파일시스템 ----

fn snapshot_of(p: &Path) -> (Vec<u8>, std::time::SystemTime) {
    (std::fs::read(p).unwrap(), std::fs::metadata(p).unwrap().modified().unwrap())
}

#[test]
fn t_poll_20_real_directory_add_rename_delete_and_content_change_with_originals_untouched() {
    // Arrange: 임시 폴더의 픽스처 (실제 사용자 폴더 아님)
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub").join("orig.bin"), b"original-bytes").unwrap();
    std::fs::write(root.join("sub").join("grow.bin"), b"a").unwrap();
    let before = snapshot_of(&root.join("sub").join("orig.bin"));
    let now = mam_db::Clock::now_ms(&mam_db::SystemClock);
    let mut snap = Snapshot::baseline(&StdFs, &root, now, &|| false).0.unwrap();
    let rel = |c: &Change| match c {
        Change::Path(p) => format!("P:{}", p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")),
        Change::Subtree(p) => format!("S:{}", p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")),
    };
    let run = |snap: &mut Snapshot| -> BTreeSet<String> {
        snap.tick(&StdFs, &root, mam_db::Clock::now_ms(&mam_db::SystemClock), &|| false).changes.iter().map(rel).collect()
    };

    // Act / Assert: 내용만 바뀜 -> 미감지
    let mut f = std::fs::OpenOptions::new().append(true).open(root.join("sub").join("grow.bin")).unwrap();
    f.write_all(b"more-bytes").unwrap();
    drop(f);
    assert!(run(&mut snap).is_empty());
    // 추가 -> Path
    std::fs::write(root.join("sub").join("new.txt"), b"x").unwrap();
    assert_eq!(run(&mut snap), strs(&["P:sub/new.txt"]));
    // 이름 변경 -> 옛 이름 + 새 이름
    std::fs::rename(root.join("sub").join("new.txt"), root.join("sub").join("moved.txt")).unwrap();
    assert_eq!(run(&mut snap), strs(&["P:sub/new.txt", "P:sub/moved.txt"]));
    // 삭제 -> Path, 새 폴더 -> Subtree
    std::fs::remove_file(root.join("sub").join("moved.txt")).unwrap();
    assert_eq!(run(&mut snap), strs(&["P:sub/moved.txt"]));
    std::fs::create_dir(root.join("album")).unwrap();
    assert_eq!(run(&mut snap), strs(&["S:album"]));
    // 원본 보호: 감지기가 건드린 것은 없다
    assert_eq!(snapshot_of(&root.join("sub").join("orig.bin")), before);
}
