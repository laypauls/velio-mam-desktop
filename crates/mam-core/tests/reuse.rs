//! P2-8 재활용 (T-reuse-*) — 실제 SQLite + 스캔한 자산 + 임시 디렉터리. OS 부수효과(창·클립보드)는 가짜로 대체한다.
//!
//! 분류: Happy (01) / Business rule (02, 03) / External (04, 05) / Validation (06, 07, 16) /
//! Boundary (14) / Performance (15)

mod common;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::env::{AtomicClock, Env, snapshot};
use mam_core::reuse::{
    AbortReason, Clipboard, ConflictPolicy, ExportFs, ExportInput, ExportProgress, Launcher,
    ProgressSink, Reuse, StdFs,
};
use mam_core::scan::{IngestError, scan_folder};
use mam_core::userdata::{UpdateTagsInput, UserData};

#[derive(Default)]
struct FakeLauncher {
    calls: Mutex<Vec<(String, PathBuf)>>,
}

impl Launcher for FakeLauncher {
    fn open(&self, path: &Path) -> io::Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(("open".into(), path.into()));
        Ok(())
    }
    fn reveal(&self, path: &Path) -> io::Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(("reveal".into(), path.into()));
        Ok(())
    }
}

#[derive(Default)]
struct FakeClipboard {
    set: Mutex<Vec<Vec<PathBuf>>>,
}

impl Clipboard for FakeClipboard {
    fn set_files(&self, paths: &[PathBuf]) -> io::Result<()> {
        self.set.lock().unwrap().push(paths.to_vec());
        Ok(())
    }
}

/// n 번째 복사부터 디스크 가득 참으로 실패한다.
struct FullDiskFs {
    fail_from: usize,
    seen: Mutex<usize>,
}

impl ExportFs for FullDiskFs {
    fn copy(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
        let mut n = self.seen.lock().unwrap();
        *n += 1;
        if *n >= self.fail_from {
            return Err(io::Error::from(io::ErrorKind::StorageFull));
        }
        StdFs.copy(src, dst, replace)
    }
    fn write(&self, dst: &Path, data: &[u8], replace: bool) -> io::Result<()> {
        StdFs.write(dst, data, replace)
    }
}

/// 복사할 때마다 시계를 앞당긴다 (진행 이벤트 제한 시험용).
struct TickingFs(Arc<AtomicClock>);

impl ExportFs for TickingFs {
    fn copy(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
        self.0.advance(100);
        StdFs.copy(src, dst, replace)
    }
    fn write(&self, dst: &Path, data: &[u8], replace: bool) -> io::Result<()> {
        StdFs.write(dst, data, replace)
    }
}

struct Fixture {
    env: Env,
    ids: Vec<i64>,
    launcher: Arc<FakeLauncher>,
    dest: PathBuf,
}

impl Fixture {
    fn reuse_with(&self, fs: Arc<dyn ExportFs>) -> Reuse {
        Reuse::new(
            Arc::clone(&self.env.pool),
            self.env.clock.clone(),
            fs,
            self.launcher.clone(),
            vec![self.env.data.clone()],
        )
    }
    fn reuse(&self) -> Reuse {
        self.reuse_with(Arc::new(StdFs))
    }
    fn input(&self, ids: &[i64], conflict: ConflictPolicy, manifest: bool) -> ExportInput {
        ExportInput {
            asset_ids: ids.to_vec(),
            dest_dir: self.dest.to_string_lossy().into_owned(),
            conflict,
            include_manifest: manifest,
        }
    }
    fn lib_snapshot(&self) -> Vec<(String, u64, u128, String)> {
        snapshot(&self.env.path("lib"))
    }
}

/// `files` = lib 아래 상대 경로. 파일 내용은 경로마다 다르다. 자산은 경로 오름차순이 아니라 스캔 순서(= asset_id 순)다.
fn fixture(files: &[&str]) -> Fixture {
    let env = Env::new();
    for (i, f) in files.iter().enumerate() {
        env.write(&format!("lib/{f}"), u8::try_from(i).unwrap() + 1);
    }
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false)).unwrap();
    let ids = env.read(|c| {
        let mut s = c
            .prepare(
                "SELECT a.asset_id FROM media_assets a JOIN file_locations l ON l.asset_id = a.asset_id
                 ORDER BY l.rel_path",
            )
            .unwrap();
        s.query_map([], |r| r.get::<_, i64>(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    let dest = env.vol.parent().unwrap().join("out");
    fs::create_dir_all(&dest).unwrap();
    Fixture {
        env,
        ids,
        launcher: Arc::new(FakeLauncher::default()),
        dest,
    }
}

/// 동기 실행: 준비 -> 실행, 모든 진행 이벤트를 모아 돌려준다.
fn run(reuse: &Reuse, input: ExportInput) -> (mam_core::reuse::ExportReport, Vec<ExportProgress>) {
    let events = Mutex::new(Vec::new());
    let job = reuse.prepare_export(input).unwrap();
    let report = job.run(&|p| events.lock().unwrap().push(p));
    (report, events.into_inner().unwrap())
}

/// 확장 접두어(`\\?\`)를 떼고 소문자로 — 임시 디렉터리는 canonicalize 되어 확장 경로일 수 있다
fn plain(p: &Path) -> String {
    mam_core::volume::strip_verbatim(&p.to_string_lossy()).to_lowercase()
}

fn dest_names(f: &Fixture) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(&f.dest)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn t_reuse_01_exports_copies_manifest_and_progress_events_with_task_id() {
    // Arrange
    let f = fixture(&["a.jpg", "b.png", "c.mp4"]);
    let ud = UserData::new(Arc::clone(&f.env.pool), f.env.clock.clone());
    ud.update_asset_tags(UpdateTagsInput {
        asset_ids: vec![f.ids[0]],
        add: vec!["여행".into(), "=SUM".into()],
        remove: vec![],
    })
    .unwrap();
    ud.set_memo(f.ids[0], "메모, 쉼표").unwrap();
    let before = f.lib_snapshot();
    let reuse = f.reuse();
    let (tx, rx) = channel();
    let sink: ProgressSink = Arc::new(move |p| {
        tx.send(p).unwrap();
    });

    // Act
    let started = reuse
        .start_export(f.input(&f.ids, ConflictPolicy::Rename, true), sink)
        .unwrap();
    let mut events = Vec::new();
    loop {
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(p) => {
                let last = p.finished;
                events.push(p);
                if last {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                panic!("no finished event: {events:?}")
            }
        }
    }

    // Assert: 모든 이벤트가 같은 taskId, 마지막만 finished, 집계 일치
    assert!(events.iter().all(|e| e.task_id == started.task_id));
    assert_eq!(events.iter().filter(|e| e.finished).count(), 1);
    let last = events.last().unwrap();
    assert_eq!(
        (
            last.done,
            last.total,
            last.failed,
            last.skipped,
            last.finished
        ),
        (3, 3, 0, 0, true)
    );
    // 복사본 내용이 같고 매니페스트가 있다
    assert_eq!(
        dest_names(&f),
        vec!["a.jpg", "b.png", "c.mp4", "mam-export.csv"]
    );
    assert_eq!(
        fs::read(f.dest.join("a.jpg")).unwrap(),
        fs::read(f.env.path("lib/a.jpg")).unwrap()
    );
    let csv = fs::read_to_string(f.dest.join("mam-export.csv")).unwrap();
    assert!(csv.starts_with('\u{feff}'));
    assert_eq!(csv.lines().count(), 4, "{csv}");
    let a_row = csv.lines().find(|l| l.contains("a.jpg")).unwrap();
    assert!(a_row.contains("여행") && a_row.contains("=SUM"), "{a_row}");
    assert!(a_row.contains("\"메모, 쉼표\""), "{a_row}");
    // 원본 불변
    assert_eq!(f.lib_snapshot(), before);
}

#[test]
fn t_reuse_02_conflict_policies_skip_rename_overwrite() {
    // Arrange: 대상에 같은 이름이 이미 있다
    let f = fixture(&["a.jpg", "b.jpg"]);
    let reuse = f.reuse();
    let source_a = fs::read(f.env.path("lib/a.jpg")).unwrap();
    fs::write(f.dest.join("a.jpg"), b"OLD").unwrap();

    // Act + Assert: skip -> 건너뜀, 기존 파일 유지
    let (r, ev) = run(&reuse, f.input(&f.ids, ConflictPolicy::Skip, false));
    assert_eq!((r.copied, r.skipped, r.failed), (1, 1, 0));
    assert_eq!(fs::read(f.dest.join("a.jpg")).unwrap(), b"OLD");
    let last = ev.last().unwrap();
    assert_eq!((last.done, last.skipped, last.finished), (2, 1, true));
    assert!(f.dest.join("b.jpg").exists());

    // rename -> "a (1).jpg" 로 저장, 기존 파일 유지
    let (r, _) = run(&reuse, f.input(&f.ids[..1], ConflictPolicy::Rename, false));
    assert_eq!((r.copied, r.skipped), (1, 0));
    assert_eq!(fs::read(f.dest.join("a.jpg")).unwrap(), b"OLD");
    assert_eq!(fs::read(f.dest.join("a (1).jpg")).unwrap(), source_a);
    let (r, _) = run(&reuse, f.input(&f.ids[..1], ConflictPolicy::Rename, false));
    assert_eq!(r.copied, 1);
    assert!(f.dest.join("a (2).jpg").exists());

    // overwrite -> 같은 이름만 덮어쓴다 (다른 파일은 그대로)
    fs::write(f.dest.join("unrelated.txt"), b"keep").unwrap();
    let (r, _) = run(
        &reuse,
        f.input(&f.ids[..1], ConflictPolicy::Overwrite, false),
    );
    assert_eq!((r.copied, r.failed), (1, 0));
    assert_eq!(fs::read(f.dest.join("a.jpg")).unwrap(), source_a);
    assert_eq!(fs::read(f.dest.join("unrelated.txt")).unwrap(), b"keep");
    assert!(
        fs::read_dir(&f.dest).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")),
        "임시 파일이 남지 않는다"
    );
}

#[test]
fn t_reuse_02b_same_name_from_different_folders_never_overwrites_this_exports_files() {
    // Arrange: 폴더가 달라 이름이 같은 두 자산
    let f = fixture(&["x/same.jpg", "y/same.jpg"]);
    let reuse = f.reuse();

    // Act: overwrite 정책이어도 이번 내보내기에서 쓴 파일을 서로 덮어쓰지 않는다
    let (r, _) = run(&reuse, f.input(&f.ids, ConflictPolicy::Overwrite, false));

    // Assert
    assert_eq!((r.copied, r.failed), (2, 0));
    assert_eq!(dest_names(&f), vec!["same (1).jpg", "same.jpg"]);
    let a = fs::read(f.dest.join("same.jpg")).unwrap();
    let b = fs::read(f.dest.join("same (1).jpg")).unwrap();
    assert_ne!(a, b);
}

#[test]
fn t_reuse_03_originals_untouched_and_destinations_inside_managed_data_rejected() {
    // Arrange
    let f = fixture(&["a.jpg", "sub/b.jpg"]);
    let reuse = f.reuse();
    let before = f.lib_snapshot();

    // Act: 모든 정책으로 내보내기
    for policy in [
        ConflictPolicy::Skip,
        ConflictPolicy::Rename,
        ConflictPolicy::Overwrite,
        ConflictPolicy::Overwrite,
    ] {
        run(&reuse, f.input(&f.ids, policy, true));
    }

    // Assert: 원본 내용·수정 시각·파일 수 불변
    assert_eq!(f.lib_snapshot(), before);

    // 관리 폴더 안 / 그 자신 / 하위 / 대소문자만 다른 경로 / MAM 데이터 안 -> invalid_input, 아무것도 안 쓴다
    let lib = f.env.path("lib");
    let bad: Vec<PathBuf> = vec![
        lib.clone(),
        lib.join("sub"),
        PathBuf::from(lib.to_string_lossy().to_uppercase()),
        f.env.data.clone(),
    ];
    for dest in bad {
        let mut input = f.input(&f.ids, ConflictPolicy::Overwrite, true);
        input.dest_dir = dest.to_string_lossy().into_owned();
        assert!(
            matches!(
                reuse.prepare_export(input),
                Err(IngestError::InvalidInput(_))
            ),
            "{}",
            dest.display()
        );
    }
    assert_eq!(f.lib_snapshot(), before);

    // 이름이 접두어만 같은 형제 폴더(lib2)는 허용
    let sibling = f.env.vol.join("lib2");
    fs::create_dir_all(&sibling).unwrap();
    let mut input = f.input(&f.ids, ConflictPolicy::Skip, false);
    input.dest_dir = sibling.to_string_lossy().into_owned();
    assert!(reuse.prepare_export(input).is_ok());
}

#[test]
fn t_reuse_04_offline_missing_and_unknown_assets_count_as_failed_and_the_rest_continue() {
    // Arrange
    let f = fixture(&["a.jpg", "b.jpg", "c.jpg", "d.jpg"]);
    let reuse = f.reuse();
    fs::remove_file(f.env.path("lib/b.jpg")).unwrap(); // 디스크에서 사라짐 (DB 는 아직 present)
    let c = f.ids[2];
    f.env
        .pool
        .write(mam_db::Priority::High, move |conn| {
            conn.execute(
                "UPDATE file_locations SET presence = 'missing' WHERE asset_id = ?1",
                [c],
            )?;
            Ok(())
        })
        .unwrap();
    let ids = [f.ids[0], f.ids[1], c, f.ids[3], 777_777];

    // Act
    let (r, ev) = run(&reuse, f.input(&ids, ConflictPolicy::Rename, false));

    // Assert: a,d 만 복사. b(없음)·c(missing)·777777(모르는 id) 실패. 마지막 이벤트 finished
    assert_eq!((r.total, r.copied, r.failed, r.skipped), (5, 2, 3, 0));
    assert_eq!(dest_names(&f), vec!["a.jpg", "d.jpg"]);
    let last = ev.last().unwrap();
    assert_eq!((last.done, last.failed, last.finished), (5, 3, true));

    // 볼륨이 오프라인이면 전부 실패 (삭제가 아니다 — 위치는 그대로)
    f.env
        .pool
        .write(mam_db::Priority::High, |conn| {
            conn.execute("UPDATE volumes SET availability = 'offline'", [])?;
            Ok(())
        })
        .unwrap();
    let before = dest_names(&f);
    let (r, ev) = run(&reuse, f.input(&f.ids, ConflictPolicy::Rename, true));
    assert_eq!((r.copied, r.failed), (0, 4));
    assert!(!r.manifest_written);
    assert_eq!(dest_names(&f), before);
    assert!(ev.last().unwrap().finished);
}

#[test]
fn t_reuse_05_destination_write_failures() {
    // Arrange: 두 번째 복사부터 디스크 가득 참
    let f = fixture(&["a.jpg", "b.jpg", "c.jpg"]);
    let reuse = f.reuse_with(Arc::new(FullDiskFs {
        fail_from: 2,
        seen: Mutex::new(0),
    }));

    // Act
    let (r, ev) = run(&reuse, f.input(&f.ids, ConflictPolicy::Rename, true));

    // Assert: 중단 사유 기록, 남은 항목은 실패로 집계, finished 이벤트, 매니페스트는 쓰지 않는다
    assert_eq!(r.abort, Some(AbortReason::DiskFull));
    assert_eq!((r.copied, r.failed), (1, 2));
    assert!(!r.manifest_written);
    let last = ev.last().unwrap();
    assert_eq!(
        (last.done, last.total, last.failed, last.finished),
        (3, 3, 2, true)
    );

    // 대상이 없는 폴더 / 파일이면 시작 전에 invalid_input
    let reuse = f.reuse();
    for dest in [f.dest.join("nope"), f.env.path("lib/a.jpg")] {
        let mut input = f.input(&f.ids, ConflictPolicy::Rename, false);
        input.dest_dir = dest.to_string_lossy().into_owned();
        assert!(matches!(
            reuse.prepare_export(input),
            Err(IngestError::InvalidInput(_))
        ));
    }
    for dest in ["", "relative\\dir"] {
        let mut input = f.input(&f.ids, ConflictPolicy::Rename, false);
        input.dest_dir = dest.into();
        assert!(matches!(
            reuse.prepare_export(input),
            Err(IngestError::InvalidInput(_))
        ));
    }

    // 실제 파일 연산 실패 (대상 디렉터리가 복사 도중 사라짐) -> 항목 실패로 집계, 원본은 그대로
    let before = f.lib_snapshot();
    let job = reuse
        .prepare_export(f.input(&f.ids, ConflictPolicy::Rename, false))
        .unwrap();
    fs::remove_dir_all(&f.dest).unwrap();
    let report = job.run(&|_| {});
    assert_eq!((report.copied, report.failed), (0, 3));
    assert_eq!(f.lib_snapshot(), before);
}

#[test]
fn t_reuse_06_open_and_reveal_distinguish_offline_and_missing() {
    // Arrange
    let f = fixture(&["a.jpg", "b.jpg"]);
    let reuse = f.reuse();
    let (a, b) = (f.ids[0], f.ids[1]);

    // Act + Assert: 정상 -> 실행기 호출 (실제 창은 열지 않는다)
    reuse.open_original(a).unwrap();
    reuse.reveal_in_explorer(a).unwrap();
    let expected = plain(&f.env.path("lib/a.jpg"));
    {
        let calls = f.launcher.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "open");
        assert_eq!(calls[1].0, "reveal");
        assert!(calls.iter().all(|(_, p)| plain(p) == expected), "{calls:?}");
    }

    // 파일 없음 / 모르는 자산 -> not_found, 실행기 호출 없음
    fs::remove_file(f.env.path("lib/b.jpg")).unwrap();
    for id in [b, 888_888] {
        assert!(matches!(
            reuse.open_original(id),
            Err(IngestError::NotFound)
        ));
        assert!(matches!(
            reuse.reveal_in_explorer(id),
            Err(IngestError::NotFound)
        ));
    }

    // 오프라인 -> offline
    f.env
        .pool
        .write(mam_db::Priority::High, |conn| {
            conn.execute("UPDATE volumes SET availability = 'offline'", [])?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(reuse.open_original(a), Err(IngestError::Offline)));
    assert!(matches!(
        reuse.reveal_in_explorer(a),
        Err(IngestError::Offline)
    ));
    assert_eq!(f.launcher.calls.lock().unwrap().len(), 2);
}

#[test]
fn t_reuse_07_clipboard_gets_only_available_originals() {
    // Arrange
    let f = fixture(&["a.jpg", "b.jpg", "c.jpg"]);
    let reuse = f.reuse();
    let clip = FakeClipboard::default();
    fs::remove_file(f.env.path("lib/b.jpg")).unwrap();

    // Act: 없는 파일은 제외
    let n = reuse
        .copy_to_clipboard(&[f.ids[0], f.ids[1], f.ids[2], f.ids[0]], &clip)
        .unwrap();

    // Assert: 중복 제거 + 존재하는 2건, 원본 경로
    assert_eq!(n, 2);
    let set = clip.set.lock().unwrap();
    assert_eq!(set.len(), 1);
    let lib = plain(&f.env.path("lib"));
    assert!(set[0].iter().all(|p| plain(p).starts_with(&lib)), "{set:?}");
    drop(set);

    // 하나도 없으면 not_found, 빈 목록은 invalid_input, 오프라인이면 offline (클립보드는 건드리지 않는다)
    assert!(matches!(
        reuse.copy_to_clipboard(&[f.ids[1], 999_999], &clip),
        Err(IngestError::NotFound)
    ));
    assert!(matches!(
        reuse.copy_to_clipboard(&[], &clip),
        Err(IngestError::InvalidInput(_))
    ));
    f.env
        .pool
        .write(mam_db::Priority::High, |conn| {
            conn.execute("UPDATE volumes SET availability = 'offline'", [])?;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        reuse.copy_to_clipboard(&f.ids, &clip),
        Err(IngestError::Offline)
    ));
    assert_eq!(clip.set.lock().unwrap().len(), 1);
}

#[test]
fn t_reuse_14_progress_events_are_rate_limited_and_the_last_one_always_sent() {
    // Arrange: 복사마다 시계 100ms 전진 -> 250ms 간격 제한이 걸린다
    let names: Vec<String> = (0..10).map(|i| format!("f{i}.jpg")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let f = fixture(&refs);
    let reuse = f.reuse_with(Arc::new(TickingFs(f.env.clock.clone())));

    // Act
    let (r, ev) = run(&reuse, f.input(&f.ids, ConflictPolicy::Rename, false));

    // Assert: 10건 처리에 이벤트는 5개(1,4,7,10번째 + 마지막), finished 는 마지막 하나
    assert_eq!(r.copied, 10);
    assert!(ev.len() < 10, "{} events", ev.len());
    assert_eq!(ev.iter().filter(|e| e.finished).count(), 1);
    assert!(ev.last().unwrap().finished);
    assert!(ev.windows(2).all(|w| w[0].done <= w[1].done));
}

#[test]
fn t_reuse_15_resolve_query_is_driven_by_the_requested_ids() {
    // Arrange
    let f = fixture(&["a.jpg"]);

    // Act
    let plan: String = f.env.read(|c| {
        let mut s = c
            .prepare(&format!(
                "EXPLAIN QUERY PLAN {}",
                mam_core::reuse::RESOLVE_SQL.replace("?1", "'[1,2,3]'")
            ))
            .unwrap();
        s.query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join(" | ")
    });

    // Assert: 자산 테이블 전체 스캔 없음
    assert!(!plan.contains("SCAN a"), "{plan}");
    assert!(!plan.contains("SCAN media_assets"), "{plan}");
}

#[test]
fn t_reuse_16_contract_shapes() {
    // Arrange + Act: 프론트가 보내는 JSON / 프론트가 받는 JSON
    let input: ExportInput = serde_json::from_value(serde_json::json!({
        "assetIds": [1], "destDir": "D:\\out", "conflict": "overwrite", "includeManifest": true
    }))
    .unwrap();
    let bad = serde_json::from_value::<ExportInput>(serde_json::json!({
        "assetIds": [1], "destDir": "D:\\out", "conflict": "merge", "includeManifest": true
    }));
    let progress = serde_json::to_value(ExportProgress {
        task_id: "export-1".into(),
        done: 1,
        total: 2,
        failed: 0,
        skipped: 1,
        finished: false,
    })
    .unwrap();
    let started = serde_json::to_value(mam_core::reuse::TaskStarted {
        task_id: "export-1".into(),
    })
    .unwrap();

    // Assert
    assert_eq!(input.conflict, ConflictPolicy::Overwrite);
    assert!(input.include_manifest);
    assert!(bad.is_err(), "알 수 없는 충돌 정책은 거부");
    assert_eq!(
        progress,
        serde_json::json!({"taskId": "export-1", "done": 1, "total": 2, "failed": 0, "skipped": 1, "finished": false})
    );
    assert_eq!(started, serde_json::json!({"taskId": "export-1"}));
}

#[test]
fn t_reuse_17_empty_and_oversized_requests_are_invalid_input() {
    // Arrange
    let f = fixture(&["a.jpg"]);
    let reuse = f.reuse();

    // Act + Assert
    assert!(matches!(
        reuse.prepare_export(f.input(&[], ConflictPolicy::Skip, false)),
        Err(IngestError::InvalidInput(_))
    ));
    let many: Vec<i64> = (1..=10_001).collect();
    assert!(matches!(
        reuse.prepare_export(f.input(&many, ConflictPolicy::Skip, false)),
        Err(IngestError::InvalidInput(_))
    ));
}
