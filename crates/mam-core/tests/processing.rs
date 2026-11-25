//! 자산 처리 통합 테스트 (P2-3): 메타 -> 썸네일 -> 해시 실행기, 파생물 상태, 실패 격리, 멱등, 원본 보호, 일시정지.
//!
//! 분류: Happy (01, 02) / External (03, 10) / Concurrency-멱등 (04) / Business rule (05, 06, 11, 12) / Concurrency-정지 (07).
//! 실제 SQLite + 임시 볼륨 + 실제 작업 실행기(스레드). 외부 도구(ffmpeg/ffprobe)는 대역 실행기로 결과를 정한다.

mod common;

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use common::env::{Env, snapshot, wait_until};
use common::proc_env::{
    ProcEnv, failing_tools, fixture, no_tools, png_bytes, real_tools, stub_tools, wav_bytes,
};
use mam_core::jobs::{JobKind, NewJob};
use mam_core::processing::{DerivKind, DerivState, media_info, states_for};
use mam_media::{MediaError, ThumbSize, strip_path, thumbnail_path};

fn add_and_run(p: &ProcEnv, rel: &str) {
    p.env
        .ingest
        .add_folder(p.env.path(rel).to_str().unwrap())
        .unwrap();
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle(), "jobs did not drain:\n{}", p.jobs_dump());
}

fn is_webp(path: &std::path::Path) -> bool {
    let b = fs::read(path).unwrap_or_default();
    b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP"
}

fn mixed_library(p: &ProcEnv) {
    p.put("lib/a.png", &png_bytes(200, 100));
    p.put("lib/v.mp4", b"not a real video; tools are stubbed");
    p.put("lib/s.wav", &wav_bytes());
}

#[test]
fn t_proc_01_meta_is_saved_for_image_video_and_audio_with_revision_and_outbox() {
    // Arrange
    let p = ProcEnv::new(stub_tools(|| {}));
    mixed_library(&p);

    // Act
    add_and_run(&p, "lib");
    p.shutdown();

    // Assert: 이미지 - 헤더 크기, 영상 - ffprobe 값, 음성 - 길이
    let (img, vid, aud) = (
        p.asset_of("a.png"),
        p.asset_of("v.mp4"),
        p.asset_of("s.wav"),
    );
    let ii = p.env.read(|c| media_info(c, img).unwrap().unwrap());
    assert_eq!((ii.width, ii.height), (Some(200), Some(100)));
    assert_eq!(ii.codec.as_deref(), Some("png"));
    assert!(ii.playable);
    let vi = p.env.read(|c| media_info(c, vid).unwrap().unwrap());
    assert_eq!(
        (vi.width, vi.height, vi.duration_ms),
        (Some(640), Some(360), Some(10_000))
    );
    assert_eq!(
        (
            vi.codec.as_deref(),
            vi.audio_codec.as_deref(),
            vi.fps,
            vi.bitrate
        ),
        (Some("h264"), Some("aac"), Some(30.0), Some(800_000))
    );
    assert_eq!(vi.captured_at, Some(1_714_566_896_000));
    assert!(vi.playable);
    assert_eq!(
        vi.extra.get("container").map(String::as_str),
        Some("mov,mp4,m4a,3gp,3g2,mj2")
    );
    let ai = p.env.read(|c| media_info(c, aud).unwrap().unwrap());
    assert_eq!(ai.duration_ms, Some(100));
    assert_eq!(ai.codec.as_deref(), Some("pcm_s16le"));

    // 메타 변경은 revision 증가 + 같은 revision 의 outbox 이벤트
    for id in [img, vid, aud] {
        assert_eq!(p.deriv(id, "meta").unwrap().0, "done");
        let rev = p.revision(id);
        assert!(rev >= 2, "asset {id} revision {rev}");
        assert_eq!(
            p.env.count(&format!(
                "SELECT COUNT(*) FROM search_outbox WHERE asset_id = {id} AND revision = {rev} AND event_type = 'upsert'"
            )),
            1
        );
    }
}

#[test]
fn t_proc_02_thumbnails_two_sizes_strip_and_states_are_recorded() {
    // Arrange
    let p = ProcEnv::new(stub_tools(|| {}));
    mixed_library(&p);

    // Act
    add_and_run(&p, "lib");
    p.shutdown();

    // Assert: 이미지 - 320/1024 WebP + done
    let (img, vid, aud) = (
        p.asset_of("a.png"),
        p.asset_of("v.mp4"),
        p.asset_of("s.wav"),
    );
    for size in [ThumbSize::Small, ThumbSize::Large] {
        assert!(is_webp(&thumbnail_path(&p.cache, img, size)), "{size:?}");
    }
    assert_eq!(p.deriv(img, "thumb").unwrap().0, "done");
    // 영상 - 대표 프레임 썸네일 + 프레임 스트립
    assert_eq!(p.deriv(vid, "thumb").unwrap().0, "done");
    assert_eq!(p.deriv(vid, "strip").unwrap().0, "done");
    assert!(is_webp(&strip_path(&p.cache, vid)));
    // 음성 - 썸네일 없음(unsupported), 파일 없음
    assert_eq!(p.deriv(aud, "thumb").unwrap().0, "unsupported");
    assert!(!thumbnail_path(&p.cache, aud, ThumbSize::Small).exists());
    // 작업은 모두 정상 종료 (실패 행 없음). 크기가 모두 달라 앞뒤 4K 체크섬만 채워지고 전체 해시는 계산하지 않는다
    assert_eq!(p.env.count("SELECT COUNT(*) FROM jobs"), 0);
    assert_eq!(
        p.env.count(
            "SELECT COUNT(*) FROM media_assets WHERE hash_state = 'pending' AND edge_sig IS NOT NULL AND content_hash IS NULL"
        ),
        3
    );
    // 읽기 모델: 페이지 단위 조회
    let states = p
        .env
        .read(|c| states_for(c, DerivKind::Thumb, &[img, vid, aud, 9999]).unwrap());
    assert_eq!(states[&img], DerivState::Done);
    assert_eq!(states[&aud], DerivState::Unsupported);
    assert_eq!(states[&9999], DerivState::Pending);
}

#[test]
fn t_proc_03_missing_or_timed_out_tools_fail_that_asset_and_others_continue() {
    for (label, tools) in [
        ("no tools", no_tools()),
        (
            "timeout",
            failing_tools(|| MediaError::Timeout {
                tool: "ffmpeg",
                timeout: Duration::from_secs(30),
            }),
        ),
    ] {
        // Arrange
        let p = ProcEnv::new(tools);
        mixed_library(&p);

        // Act
        add_and_run(&p, "lib");
        p.shutdown();

        // Assert: 영상 - 메타·썸네일 failed + 오류 기록, 이미지는 정상 (도구 없이 처리 가능)
        let (img, vid) = (p.asset_of("a.png"), p.asset_of("v.mp4"));
        for kind in ["meta", "thumb"] {
            let (state, err) = p
                .deriv(vid, kind)
                .unwrap_or_else(|| panic!("{label}: {kind}"));
            assert_eq!(state, "failed", "{label}: {kind}");
            assert!(!err.unwrap_or_default().is_empty(), "{label}: {kind} error");
        }
        assert_eq!(p.deriv(img, "thumb").unwrap().0, "done", "{label}");
        assert!(is_webp(&thumbnail_path(&p.cache, img, ThumbSize::Small)));
        // 실패는 파생물 상태에 남고 작업 자체는 끝났다 (재시도 폭주 없음), 앞뒤 체크섬은 도구와 무관하게 완료
        assert_eq!(p.env.count("SELECT COUNT(*) FROM jobs"), 0, "{label}");
        assert_eq!(
            p.env
                .count("SELECT COUNT(*) FROM media_assets WHERE edge_sig IS NOT NULL"),
            3,
            "{label}"
        );
    }
}

#[test]
fn t_proc_04_rerunning_the_same_jobs_changes_nothing() {
    // Arrange: 한 번 끝까지 처리
    let p = ProcEnv::new(stub_tools(|| {}));
    mixed_library(&p);
    add_and_run(&p, "lib");
    let snapshot_of = |p: &ProcEnv| {
        (
            p.env.count("SELECT COUNT(*) FROM asset_derivatives"),
            p.env.count("SELECT COUNT(*) FROM media_metadata"),
            p.env.count("SELECT COUNT(*) FROM search_outbox"),
            p.env.count("SELECT SUM(revision) FROM media_assets"),
        )
    };
    let before = snapshot_of(&p);

    // Act: 같은 자산에 세 종류 작업을 다시 등록해 실행
    for id in [
        p.asset_of("a.png"),
        p.asset_of("v.mp4"),
        p.asset_of("s.wav"),
    ] {
        for kind in [JobKind::Meta, JobKind::Thumb, JobKind::Hash] {
            p.env
                .ingest
                .jobs()
                .enqueue(NewJob::for_asset(kind, id))
                .unwrap();
        }
    }
    assert!(p.wait_idle());
    p.shutdown();

    // Assert: 행 수·revision·outbox 모두 그대로 (값이 같으면 revision 을 올리지 않는다)
    assert_eq!(snapshot_of(&p), before);
}

#[test]
fn t_proc_05_originals_are_untouched_and_outputs_go_only_under_mamdata() {
    // Arrange
    let Some(real) = real_tools() else {
        eprintln!("SKIP: ffmpeg/ffprobe not on PATH (real video part)");
        return;
    };
    let p = ProcEnv::new(real);
    p.put("lib/a.png", &png_bytes(120, 80));
    fs::copy(fixture("tiny.mp4"), p.env.path("lib/clip.mp4")).unwrap();
    fs::copy(fixture("tiny.webm"), p.env.path("lib/clip.webm")).unwrap();
    p.put("lib/s.wav", &wav_bytes());
    let before = snapshot(&p.env.vol);

    // Act
    add_and_run(&p, "lib");
    p.shutdown();

    // Assert: 원본 폴더의 파일 목록·내용·수정 시각 불변 (임시 파일 포함 아무것도 추가되지 않음)
    assert_eq!(snapshot(&p.env.vol), before);
    // 산출물은 MAMData/cache 에만, 실제 영상 처리 결과가 상태로 기록됨
    let outputs = snapshot(&p.cache);
    assert!(
        outputs.iter().any(|(rel, ..)| rel.ends_with(".webp")),
        "{outputs:?}"
    );
    for name in ["clip.mp4", "clip.webm"] {
        let id = p.asset_of(name);
        assert_eq!(p.deriv(id, "meta").unwrap().0, "done", "{name}");
        let thumb = p.deriv(id, "thumb");
        assert_eq!(
            thumb.as_ref().map(|t| t.0.as_str()),
            Some("done"),
            "{name}: {thumb:?}"
        );
        assert!(
            thumbnail_path(&p.cache, id, ThumbSize::Large).exists(),
            "{name}"
        );
    }
}

#[test]
fn t_proc_06_result_is_discarded_when_the_asset_is_superseded_mid_processing() {
    // Arrange: 영상 썸네일을 만드는 ffmpeg 호출 도중 자산이 superseded 로 바뀐다
    let env = Env::new();
    let pool = Arc::clone(&env.pool);
    let flipped = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&flipped);
    let tools = stub_tools(move || {
        if !flag.swap(true, Ordering::SeqCst) {
            pool.write(mam_db::Priority::High, |c| {
                c.execute(
                    "UPDATE media_assets SET status = 'superseded' WHERE media_type = 'video'",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        }
    });
    let p = ProcEnv::with_env(env, tools);
    p.put("lib/v.mp4", b"stubbed video");

    // Act
    add_and_run(&p, "lib");
    p.shutdown();

    // Assert: 썸네일 상태가 done 이 아니고, 만들어진 산출물도 지워졌다
    assert!(flipped.load(Ordering::SeqCst), "hook did not run");
    let vid = p.asset_of("v.mp4");
    assert!(p.deriv(vid, "thumb").is_none_or(|(s, _)| s != "done"));
    assert!(!thumbnail_path(&p.cache, vid, ThumbSize::Small).exists());
    assert!(!thumbnail_path(&p.cache, vid, ThumbSize::Large).exists());
    assert_eq!(p.env.count("SELECT COUNT(*) FROM jobs"), 0);
}

#[test]
fn t_proc_06b_file_changed_after_scan_is_not_recorded_with_stale_values() {
    // Arrange: 스캔 뒤(작업 실행 전) 파일 내용·크기가 바뀐다
    let p = ProcEnv::new(no_tools());
    let path = p.put("lib/a.png", &png_bytes(100, 100));
    let folder = p
        .env
        .ingest
        .add_folder(p.env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    mam_core::scan::scan_folder(
        p.env.ingest.env(),
        folder,
        &|| false,
        &AtomicBool::new(false),
    )
    .unwrap();
    let id = p.asset_of("a.png");
    fs::write(&path, png_bytes(300, 150)).unwrap();
    // add_folder 가 예약한 스캔 작업은 지운다 (직접 스캔했고, 시작하면 곧바로 변경을 반영해 버리므로)
    p.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute("DELETE FROM jobs WHERE kind = 'scan'", [])?;
            Ok(())
        })
        .unwrap();

    // Act: 낡은 DB 값 기준 작업은 결과를 쓰지 않는다 (실시간 감시가 먼저 변경을 반영하지 못하게 끈다 - t_proc_09 와 같은 경합)
    let _watch_off = p.env.ingest.suppress_watch(&p.env.path("lib"));
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle());

    // Assert
    assert!(
        p.deriv(id, "meta").is_none(),
        "stale meta must not be stored"
    );
    assert!(
        p.deriv(id, "thumb").is_none(),
        "stale thumb must not be stored"
    );

    // Act 2: 재스캔이 변경을 반영 (전체 해시 없는 자산의 제자리 변경 -> 신규 자산 + 옛 자산 superseded, 새 자산을 처리)
    mam_core::scan::scan_folder(
        p.env.ingest.env(),
        folder,
        &|| false,
        &AtomicBool::new(false),
    )
    .unwrap();
    p.env.ingest.jobs().notify();
    assert!(p.wait_idle());
    p.shutdown();

    // Assert 2: 옛 자산은 superseded, 새 자산이 새 값으로 처리됨
    assert_eq!(
        p.env.count(&format!(
            "SELECT COUNT(*) FROM media_assets WHERE asset_id = {id} AND status = 'superseded'"
        )),
        1
    );
    let new_id = p.asset_of("a.png");
    assert_ne!(new_id, id);
    let info = p.env.read(|c| media_info(c, new_id).unwrap().unwrap());
    assert_eq!((info.width, info.height), (Some(300), Some(150)));
    assert_eq!(p.deriv(new_id, "thumb").unwrap().0, "done");
}

#[test]
fn t_proc_07_pause_lets_the_running_job_finish_then_stops_and_resume_continues() {
    // Arrange: 썸네일 동시성 1, 첫 ffmpeg 호출이 풀어 줄 때까지 멈춰 있는 대역
    let env = Env::with_config(|cfg, _| cfg.concurrency[JobKind::Thumb as usize] = 1);
    let (entered_tx, entered_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let first = Arc::new(AtomicBool::new(true));
    let calls = Arc::new(AtomicUsize::new(0));
    let (first2, calls2) = (Arc::clone(&first), Arc::clone(&calls));
    let entered_tx = std::sync::Mutex::new(entered_tx);
    let tools = stub_tools(move || {
        calls2.fetch_add(1, Ordering::SeqCst);
        if first2.swap(false, Ordering::SeqCst) {
            entered_tx.lock().unwrap().send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
        }
    });
    let p = ProcEnv::with_env(env, tools);
    p.put("lib/v1.mp4", b"video one");
    p.put("lib/v2.mp4", b"video two!");
    p.env
        .ingest
        .add_folder(p.env.path("lib").to_str().unwrap())
        .unwrap();
    p.env.ingest.start().unwrap();
    entered_rx
        .recv_timeout(Duration::from_secs(120))
        .expect("a thumb job should be running");

    // Act: 실행 중에 썸네일 종류를 일시정지하고, 진행 중 작업을 풀어 준다
    p.env.ingest.jobs().pause(Some(JobKind::Thumb)).unwrap();
    release_tx.send(()).unwrap();
    let first_done = wait_until(Duration::from_secs(120), || {
        p.env
            .count("SELECT COUNT(*) FROM asset_derivatives WHERE kind = 'thumb' AND state = 'done'")
            == 1
    });

    // Assert: 진행 중이던 작업은 끝까지 완료, 나머지는 시작하지 않는다 (다른 종류는 계속 진행)
    assert!(first_done);
    // 다른 종류(해시)가 끝날 때까지 기다린다 - 그동안에도 일시정지된 썸네일은 시작하지 않아야 한다 (고정 시간 대기 금지)
    let hashed = wait_until(Duration::from_secs(120), || {
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE edge_sig IS NOT NULL")
            == 2
    });
    assert!(hashed, "hash jobs must continue while thumb is paused");
    assert_eq!(
        p.env.count(
            "SELECT COUNT(*) FROM asset_derivatives WHERE kind = 'thumb' AND state = 'done'"
        ),
        1,
        "paused: the second thumb job must not start"
    );
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM jobs WHERE kind = 'thumb' AND status = 'pending'"),
        1
    );
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE edge_sig IS NOT NULL"),
        2
    );

    // Act 2: 재개
    p.env.ingest.jobs().resume(Some(JobKind::Thumb)).unwrap();
    assert!(p.wait_idle());
    p.shutdown();

    // Assert 2
    assert_eq!(
        p.env.count(
            "SELECT COUNT(*) FROM asset_derivatives WHERE kind = 'thumb' AND state = 'done'"
        ),
        2
    );
}

#[test]
fn t_proc_10_heic_without_any_decoder_records_both_fallback_failures() {
    // Arrange: HEIC 컨테이너 표식만 있는 손상 파일 - ffmpeg 없음, WIC 도 디코딩 불가
    let p = ProcEnv::new(no_tools());
    let mut bogus = vec![0, 0, 0, 24];
    bogus.extend_from_slice(b"ftypheic");
    bogus.extend_from_slice(&[0u8; 64]);
    p.put("lib/bad.heic", &bogus);

    // Act
    add_and_run(&p, "lib");
    p.shutdown();

    // Assert: 실패로 기록되고 두 폴백의 사유가 함께 남는다 (다른 작업은 영향 없음)
    let id = p.asset_of("bad.heic");
    let (state, err) = p.deriv(id, "thumb").unwrap();
    let err = err.unwrap();
    assert_eq!(state, "failed");
    assert!(err.contains("ffmpeg") && err.contains("wic"), "{err}");
    assert!(!thumbnail_path(&p.cache, id, ThumbSize::Small).exists());
    assert_eq!(p.env.count("SELECT COUNT(*) FROM jobs"), 0);
}

#[test]
fn t_proc_10b_heic_decodes_through_wic_when_the_heif_codec_is_installed() {
    // Arrange: 실제 HEIC 픽스처. WIC HEIF 확장이 없는 PC 에서는 건너뛴다 (환경 의존 - 결과는 PC 마다 일정)
    let fixture = fixture("tiny.heic");
    if mam_media::decode_with_wic(&fixture, 64_000_000).is_err() {
        eprintln!("SKIP: no WIC HEIF codec installed");
        return;
    }
    let p = ProcEnv::new(no_tools());
    p.put("lib/x.heic", &fs::read(&fixture).unwrap());

    // Act
    add_and_run(&p, "lib");
    p.shutdown();

    // Assert: ffmpeg 없이도 WIC 로 썸네일 생성
    let id = p.asset_of("x.heic");
    assert_eq!(p.deriv(id, "thumb").unwrap().0, "done");
    assert!(is_webp(&thumbnail_path(&p.cache, id, ThumbSize::Small)));
}

#[test]
fn t_proc_11_offline_assets_are_skipped_not_failed_and_reprocessed_when_back() {
    // Arrange: 스캔만 해 둔 상태에서 볼륨이 분리됨 (DB 도 오프라인으로 맞춰 둔다)
    let p = ProcEnv::new(stub_tools(|| {}));
    p.put("lib/a.png", &png_bytes(50, 50));
    let folder = p
        .env
        .ingest
        .add_folder(p.env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    mam_core::scan::scan_folder(
        p.env.ingest.env(),
        folder,
        &|| false,
        &AtomicBool::new(false),
    )
    .unwrap();
    let id = p.asset_of("a.png");
    p.env.volumes.set_connected("vol-a", false);
    p.env
        .pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE volumes SET availability = 'offline'", [])?;
            Ok(())
        })
        .unwrap();

    // Act: 오프라인 상태에서 작업 실행
    // 첫 점검(보충 등록)을 여기서 미리 치러 두어, 시작 뒤 점검 스레드가 비동기로 작업을 추가하지 않게 한다 (결정적)
    p.env.ingest.supervise_once().unwrap();
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle());

    // Assert: 실패가 아니라 "처리 전" 그대로 (오프라인은 삭제·실패가 아니다)
    assert!(p.deriv(id, "meta").is_none() && p.deriv(id, "thumb").is_none());
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM media_assets WHERE hash_state = 'pending'"),
        1
    );
    assert_eq!(p.env.count("SELECT COUNT(*) FROM jobs"), 0);

    // Act 2: 볼륨이 돌아오면 점검이 온라인 전환과 함께 처리 작업을 다시 등록한다
    p.env.volumes.set_connected("vol-a", true);
    let done = wait_until(Duration::from_secs(120), || {
        p.deriv(id, "thumb").is_some_and(|(s, _)| s == "done")
            && p.env
                .count("SELECT COUNT(*) FROM media_assets WHERE edge_sig IS NOT NULL")
                == 1
    });
    assert!(p.wait_idle());
    p.shutdown();

    // Assert 2
    assert!(done, "reconnected volume must be processed");
    assert_eq!(p.deriv(id, "meta").unwrap().0, "done");
}

#[test]
fn t_proc_12_backfill_covers_assets_scanned_before_processing_and_is_idempotent() {
    // Arrange: 처리를 켜기 전에 스캔한 자산 (기존 DB 와 같은 상태). 처리 환경은 스캔 뒤에 켠다
    let env = Env::new();
    let folder_path = env.path("lib");
    for (i, name) in ["a.png", "b.png", "c.png"].iter().enumerate() {
        let p = env.path(&format!("lib/{name}"));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, png_bytes(40 + i as u32, 40)).unwrap();
    }
    let folder = env
        .ingest
        .add_folder(folder_path.to_str().unwrap())
        .unwrap()
        .folder_id;
    mam_core::scan::scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE kind IN ('meta','thumb','hash')"),
        0
    );
    let p = ProcEnv::with_env(env, no_tools());

    // Act: 첫 점검이 보충을 한다, 두 번째 점검은 하지 않는다
    let first = p.env.ingest.supervise_once().unwrap();
    let second = p.env.ingest.supervise_once().unwrap();

    // Assert
    let r = first.backfill.expect("first tick backfills");
    assert_eq!((r.meta, r.thumb, r.hash), (3, 3, 3));
    assert!(second.backfill.is_none());
    // 직접 다시 불러도 대기 작업이 중복되지 않는다
    let clock = Arc::clone(&p.env.clock);
    let again =
        mam_core::processing::backfill_jobs(&p.env.pool, move || mam_db::Clock::now_ms(&*clock))
            .unwrap();
    assert_eq!((again.meta, again.thumb, again.hash), (0, 0, 0));
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM jobs WHERE kind IN ('meta','thumb','hash')"),
        9
    );
    p.env.ingest.start().unwrap();
    assert!(p.wait_idle());
    p.shutdown();
    assert_eq!(
        p.env
            .count("SELECT COUNT(*) FROM asset_derivatives WHERE state = 'done'"),
        6
    );
}

#[test]
fn t_proc_13_hash_failure_is_retried_then_recorded_as_failed_and_can_be_retried_manually() {
    // Arrange: 해시가 항상 실패하는 환경, 이미 4번 시도한 작업
    struct FailingHasher;
    impl mam_core::ContentHasher for FailingHasher {
        fn hash_file(
            &self,
            path: &std::path::Path,
            _cancel: &AtomicBool,
        ) -> Result<mam_core::HashedFile, mam_core::CoreError> {
            Err(mam_core::CoreError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("sharing violation"),
            })
        }
        // 시험 목적상 앞뒤 체크섬 읽기도 같은 이유로 실패한다 (해시 작업의 첫 단계)
        fn edge_signature(
            &self,
            path: &std::path::Path,
            _cancel: &AtomicBool,
        ) -> Result<mam_core::EdgeFile, mam_core::CoreError> {
            Err(mam_core::CoreError::Io {
                path: path.to_path_buf(),
                source: std::io::Error::other("sharing violation"),
            })
        }
    }
    let env = Env::new();
    let clock: mam_core::SharedClock = env.clock.clone();
    let proc = mam_core::processing::ProcessingEnv::new(
        Arc::clone(&env.pool),
        clock,
        no_tools(),
        Arc::new(FailingHasher),
        env.data.join("cache"),
    );
    env.ingest.enable_processing(proc);
    let path = env.path("lib/a.png");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, png_bytes(30, 30)).unwrap();
    let folder = env
        .ingest
        .add_folder(env.path("lib").to_str().unwrap())
        .unwrap()
        .folder_id;
    mam_core::scan::scan_folder(env.ingest.env(), folder, &|| false, &AtomicBool::new(false))
        .unwrap();
    env.pool
        .write(mam_db::Priority::High, |c| {
            c.execute("UPDATE jobs SET attempts = 4 WHERE kind = 'hash'", [])?;
            Ok(())
        })
        .unwrap();

    // Act
    env.ingest.start().unwrap();
    let settled = wait_until(Duration::from_secs(120), || {
        env.count("SELECT COUNT(*) FROM jobs WHERE status IN ('pending','running')") == 0
    });
    env.ingest.shutdown();

    // Assert: 마지막 시도에서 failed 로 기록 (작업 행은 남기지 않는다)
    assert!(settled);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE hash_state = 'failed'"),
        1
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'hash'"),
        0
    );

    // Act 2: 수동 재시도는 failed 를 pending 으로 되돌리고 작업을 다시 등록한다
    let c2 = Arc::clone(&env.clock);
    let reset =
        mam_core::processing::retry_failed(&env.pool, move || mam_db::Clock::now_ms(&*c2), None)
            .unwrap();

    // Assert 2
    assert_eq!(reset, 1);
    assert_eq!(
        env.count("SELECT COUNT(*) FROM media_assets WHERE hash_state = 'pending'"),
        1
    );
    assert_eq!(
        env.count("SELECT COUNT(*) FROM jobs WHERE kind = 'hash' AND status = 'pending'"),
        1
    );
}
