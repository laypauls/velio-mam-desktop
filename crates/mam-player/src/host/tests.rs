//! 감독자 시험 — 가짜 워커(프로세스 없는 스레드) + 파일 매핑 표면. 실제 워커 시험은 `mam-player-worker/tests` 에 있다.
//! 분류: Happy / Boundary / Validation / Business rule(재시작·복원·타임아웃) / External(워커 크래시·침묵) / Concurrency(오래된 세대·UI 비차단)

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::events::{EventState, PlayerErrorCode, PlayerEvent};
use super::link::Shared;
use super::testing::{FakeScript, FakeSpawner, MemSurface};
use super::*;
use crate::PlayerError;
use crate::protocol::{Command, ControlAction, Message};

type Events = Arc<Mutex<Vec<PlayerEvent>>>;

struct H {
    host: Arc<PlayerHost>,
    script: Arc<FakeScript>,
    surface: Arc<MemSurface>,
    events: Events,
}

fn fast() -> Timeouts {
    Timeouts {
        open: Duration::from_secs(4),
        command: Duration::from_millis(500),
        silence: Duration::from_secs(2),
        spawn_ready: Duration::from_secs(2),
        backoff: [Duration::ZERO, Duration::from_millis(10), Duration::from_millis(20)],
        quit: Duration::from_millis(100),
        frame_grace: Duration::from_millis(100),
        restore: Duration::from_secs(6),
        resize_interval: Duration::from_millis(60),
    }
}

fn harness_with(t: Timeouts) -> H {
    let script = FakeScript::new();
    let surface = MemSurface::new();
    let events: Events = Arc::default();
    let ev = Arc::clone(&events);
    let cfg = HostConfig { timeouts: t, ..HostConfig::default() };
    let host = PlayerHost::with_spawner(cfg, surface.clone(), Arc::new(move |e| ev.lock().unwrap().push(e)), Some(FakeSpawner::new(Arc::clone(&script))));
    H { host: Arc::new(host), script, surface, events }
}

fn harness() -> H {
    harness_with(fast())
}

fn req(autoplay: bool) -> OpenRequest {
    OpenRequest { asset_id: 7, path: PathBuf::from(r"C:\media\clip.mp4"), autoplay, canvas: (1280, 720), ranges: None, duration_hint_ms: Some(30_000), start_ms: None }
}

/// 자기 상태에 대한 유한 대기 (상한 있음)
fn wait_for(ms: u64, mut pred: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    pred()
}

impl H {
    fn states(&self) -> Vec<EventState> {
        self.events.lock().unwrap().iter().map(|e| e.state).collect()
    }
    fn has_state(&self, s: EventState) -> bool {
        self.states().contains(&s)
    }
    fn open(&self, autoplay: bool) -> OpenInfo {
        self.host.open(req(autoplay)).expect("open")
    }
}

#[test]
fn t_player_host_01_open_reply_events_and_attach_before_open() {
    // Arrange
    let h = harness();
    // Act
    let info = h.open(false);
    // Assert: 응답 — 정지 상태, 소스 길이·fps, 새 세션 번호
    assert_eq!(info, OpenInfo { session_id: 1, length_ms: Some(30_000), fps: Some(30.0), paused: true, time_ms: 0 });
    assert_eq!(h.events.lock().unwrap()[0].state, EventState::Opening);
    assert_eq!(h.events.lock().unwrap()[0].session_id, 1);
    // Attach(버퍼 연결)가 Open 보다 먼저, 렌더 크기는 캔버스 그대로(1280x720 ≤ 상한)
    let cmds = h.script.commands();
    let attach = cmds.iter().position(|c| matches!(c, Command::Attach { w: 1280, h: 720, stride: 5120, .. })).expect("attach");
    let open = cmds.iter().position(|c| matches!(c, Command::Open { paused: true, .. })).expect("open");
    assert!(attach < open, "{cmds:?}");
    assert!(h.surface.frame_count() >= 1, "열기 응답 전에 첫 프레임이 게시됐다");
    // 열기 응답이 알린 상태는 중복 전이로 다시 나가지 않는다
    assert!(!h.has_state(EventState::Paused));
}

#[test]
fn t_player_host_02_autoplay_opens_unpaused_and_large_canvas_is_capped() {
    let h = harness();
    let mut r = req(true);
    r.canvas = (3840, 2160);
    let info = h.host.open(r).unwrap();
    assert!(!info.paused);
    assert!(h.script.commands().iter().any(|c| matches!(c, Command::Attach { w: 1920, h: 1080, .. })), "렌더 상한 1920x1080");
}

#[test]
fn t_player_host_03_missing_engine_or_surface_means_unavailable_without_spawning() {
    // Arrange: 스포너 없음
    let surface = MemSurface::new();
    let host = PlayerHost::with_spawner(HostConfig::default(), surface.clone(), Arc::new(|_| {}), None);
    // Act / Assert
    assert!(!host.is_available());
    assert!(matches!(host.open(req(true)), Err(e) if e.is_engine_unavailable()));
    host.control(ControlAction::Play).unwrap(); // 세션이 없으면 no-op
    // 표면이 SharedBuffer 를 지원하지 않으면 (WebView2 구버전) 워커를 띄우지 않는다
    let h = harness();
    h.surface.supported.store(false, Ordering::Relaxed);
    assert!(!h.host.is_available());
    assert!(matches!(h.host.open(req(true)), Err(e) if e.is_engine_unavailable()));
    assert_eq!(h.script.spawns.load(Ordering::SeqCst), 0);
}

#[test]
fn t_player_host_04_protocol_mismatch_and_failed_init_are_engine_unavailable() {
    let h = harness();
    h.script.protocol.store(99, Ordering::Relaxed);
    assert!(matches!(h.host.open(req(true)), Err(PlayerError::ProtocolMismatch { worker: 99, .. })));
    h.script.protocol.store(crate::protocol::PROTOCOL_VERSION, Ordering::Relaxed);
    h.script.fail_init.store(true, Ordering::Relaxed);
    let e = h.host.open(req(true)).unwrap_err();
    assert!(e.is_engine_unavailable(), "{e}");
    // 고치면 같은 호스트가 다시 열 수 있다 (실패는 캐시하지 않는다)
    h.script.fail_init.store(false, Ordering::Relaxed);
    assert!(h.host.open(req(true)).is_ok());
}

#[test]
fn t_player_host_05_crash_before_file_loaded_fails_the_open_without_a_restore_loop() {
    let h = harness();
    h.script.die_on_open.store(true, Ordering::Relaxed);
    let e = h.host.open(req(true)).unwrap_err();
    assert!(matches!(e, PlayerError::OpenFailed(_)), "{e}");
    assert!(!h.has_state(EventState::Restarting), "열기 전 크래시는 복원하지 않는다");
    assert_eq!(h.script.spawns.load(Ordering::SeqCst), 1, "워커는 다음 열기에서 다시 뜬다");
    h.script.die_on_open.store(false, Ordering::Relaxed);
    assert!(h.host.open(req(true)).is_ok());
    assert_eq!(h.script.spawns.load(Ordering::SeqCst), 2);
}

#[test]
fn t_player_host_06_open_timeout_kills_the_worker_and_counts_as_failed() {
    let mut t = fast();
    t.open = Duration::from_millis(300);
    let h = harness_with(t);
    h.script.never_load.store(true, Ordering::Relaxed);
    let started = Instant::now();
    let e = h.host.open(req(true)).unwrap_err();
    assert!(matches!(e, PlayerError::Timeout), "{e}");
    assert!(started.elapsed() < Duration::from_secs(3));
    h.script.never_load.store(false, Ordering::Relaxed);
    assert!(h.host.open(req(true)).is_ok(), "죽인 뒤 새 워커로 다시 열린다");
    assert_eq!(h.script.spawns.load(Ordering::SeqCst), 2);
}

#[test]
fn t_player_host_07_commands_without_a_session_are_ok_no_ops() {
    let h = harness();
    h.host.control(ControlAction::StepForward).unwrap();
    h.host.seek(5000).unwrap();
    h.host.set_rate(1.5).unwrap();
    h.host.set_volume(50, false).unwrap();
    h.host.set_ranges(7, Some(vec![(0, 100)])).unwrap();
    h.host.set_subtitle(7, None).unwrap();
    h.host.set_render_size(640, 360).unwrap();
    h.host.close().unwrap();
    assert_eq!(h.script.spawns.load(Ordering::SeqCst), 0, "세션 없는 명령은 워커를 띄우지 않는다");
    assert!(h.events.lock().unwrap().is_empty());
}

#[test]
fn t_player_host_08_requests_for_another_asset_are_ignored() {
    let h = harness();
    h.open(false);
    let before = h.script.commands().len();
    h.host.set_ranges(999, Some(vec![(0, 100)])).unwrap();
    h.host.set_subtitle(999, Some(PathBuf::from("x.srt"))).unwrap();
    assert_eq!(h.script.commands().len(), before, "다른 자산의 요청은 워커에 가지 않는다");
    h.host.set_ranges(7, Some(vec![(0, 100)])).unwrap();
    assert!(h.script.commands().iter().any(|c| matches!(c, Command::SetRanges { ranges: Some(r), duration_ms: Some(30_000) } if r == &vec![(0, 100)])));
}

#[test]
fn t_player_host_09_seek_is_clamped_to_the_source_length() {
    let h = harness();
    h.open(false);
    h.host.seek(-50).unwrap();
    h.host.seek(10_000_000).unwrap();
    let seeks: Vec<i64> = h.script.commands().iter().filter_map(|c| if let Command::Seek { ms } = c { Some(*ms) } else { None }).collect();
    assert_eq!(seeks, vec![0, 30_000]);
}

#[test]
fn t_player_host_10_toggle_follows_the_reported_paused_state_and_state_events_follow_the_worker() {
    let h = harness();
    h.open(false);
    h.host.control(ControlAction::Toggle).unwrap();
    assert!(h.script.commands().iter().any(|c| matches!(c, Command::Play)), "정지 → 토글은 Play");
    assert!(wait_for(1000, || h.has_state(EventState::Playing)), "{:?}", h.states());
    h.host.control(ControlAction::Toggle).unwrap();
    assert!(h.script.commands().iter().any(|c| matches!(c, Command::Pause)), "재생 → 토글은 Pause");
    assert!(wait_for(1000, || h.states().last() == Some(&EventState::Paused)));
}

#[test]
fn t_player_host_11_idempotent_commands_retry_once_then_time_out_and_restart_the_worker() {
    let h = harness();
    h.open(false);
    *h.script.mute_cmd.lock().unwrap() = Some("seek".into());
    // Act
    let e = h.host.seek(1000).unwrap_err();
    // Assert: 같은 명령을 정확히 두 번(원래 + 재시도) 보냈고, 타임아웃 뒤 워커가 재시작된다
    assert!(matches!(e, PlayerError::Timeout), "{e}");
    assert_eq!(h.script.count(|c| matches!(c, Command::Seek { ms: 1000 })), 2);
    assert!(wait_for(2000, || h.script.spawns.load(Ordering::SeqCst) == 2), "워커 재시작");
    assert!(h.has_state(EventState::Restarting));
}

#[test]
fn t_player_host_12_non_idempotent_commands_are_never_retried() {
    let h = harness();
    h.open(false);
    *h.script.mute_cmd.lock().unwrap() = Some("frame_step".into());
    let e = h.host.control(ControlAction::StepForward).unwrap_err();
    assert!(matches!(e, PlayerError::Timeout), "{e}");
    assert_eq!(h.script.count(|c| matches!(c, Command::FrameStep)), 1, "프레임 이동을 두 번 보내면 두 프레임 간다");
}

#[test]
fn t_player_host_13_crash_restores_file_position_and_playing_state() {
    // Arrange: 재생 중, 12.5초, 배속 1.5, 음량 40, 구간 포함
    let h = harness();
    let mut r = req(true);
    r.ranges = Some(vec![(0, 20_000)]);
    h.host.open(r).unwrap();
    h.host.set_rate(1.5).unwrap();
    h.host.set_volume(40, false).unwrap();
    h.host.seek(12_500).unwrap();
    assert!(wait_for(1000, || h.script.count(|c| matches!(c, Command::Seek { ms: 12_500 })) == 1));
    h.events.lock().unwrap().clear();
    // Act: 워커 비정상 종료
    h.host.kill_worker_for_test();
    // Assert: restarting(attempt 1) → 같은 파일·구간·설정·위치로 복원 → 재생 재개
    assert!(wait_for(3000, || h.script.spawns.load(Ordering::SeqCst) == 2 && h.script.count(|c| matches!(c, Command::Play)) >= 1), "{:?}", h.script.commands());
    let ev = h.events.lock().unwrap().clone();
    // 종료 직전에 도착한 재생 이벤트가 앞에 있을 수 있다 — 첫 Restarting 이 시도 1 이면 된다
    let first = ev.iter().find(|e| e.state == EventState::Restarting).expect("restarting event");
    assert_eq!(first.restart.map(|r| (r.attempt, r.max)), Some((1, 3)));
    let cmds = h.script.commands();
    let second_open = cmds.iter().rposition(|c| matches!(c, Command::Open { .. })).unwrap();
    assert!(matches!(&cmds[second_open], Command::Open { paused: true, ranges: Some(r), .. } if r == &vec![(0, 20_000)]), "정지로 열어 구간을 다시 넣는다");
    let tail = &cmds[second_open..];
    assert!(tail.contains(&Command::SetSpeed { speed: 1.5 }) && tail.contains(&Command::SetVolume { volume: 40, mute: false }));
    let seek = tail.iter().position(|c| matches!(c, Command::Seek { ms: 12_500 })).expect("seek");
    assert!(tail.iter().position(|c| matches!(c, Command::Play)).unwrap() > seek, "목표 도달 뒤에 재생");
    assert!(wait_for(1000, || h.states().last() == Some(&EventState::Playing)), "{:?}", h.states());
    // 새 epoch 로 다시 연결했고 epoch 는 재사용되지 않는다
    let created = h.surface.created.lock().unwrap().clone();
    assert!(created.windows(2).all(|w| w[0] < w[1]) && created.len() >= 2, "{created:?}");
}

#[test]
fn t_player_host_14_commands_during_restart_update_the_memory_and_steps_are_rejected() {
    let mut t = fast();
    t.backoff = [Duration::from_millis(300); 3];
    let h = harness_with(t);
    h.open(false);
    h.host.kill_worker_for_test();
    assert!(wait_for(1000, || h.has_state(EventState::Restarting)));
    // Act: 백오프 중
    assert!(matches!(h.host.control(ControlAction::StepForward), Err(PlayerError::Restarting)));
    assert!(matches!(h.host.control(ControlAction::StepBack), Err(PlayerError::Restarting)));
    h.host.seek(4000).unwrap();
    h.host.control(ControlAction::Play).unwrap();
    h.host.set_rate(2.0).unwrap();
    // Assert: 복원이 바뀐 목표(위치·재생·배속)를 쓴다
    assert!(wait_for(3000, || h.script.count(|c| matches!(c, Command::Play)) >= 1), "{:?}", h.script.commands());
    let cmds = h.script.commands();
    assert!(cmds.contains(&Command::Seek { ms: 4000 }) && cmds.contains(&Command::SetSpeed { speed: 2.0 }), "{cmds:?}");
}

#[test]
fn t_player_host_15_third_consecutive_failure_gives_up_and_open_or_restart_recovers() {
    // Arrange
    let h = harness();
    h.open(true);
    h.script.fail_spawns.store(100, Ordering::SeqCst);
    // Act: 죽이면 재시작 시도가 모두 spawn 실패
    h.host.kill_worker_for_test();
    assert!(wait_for(3000, || h.has_state(EventState::RestartFailed)), "{:?}", h.states());
    // Assert: 시도 1·2·3 후 포기 — 이벤트에 worker_crashed 와 3/3
    let ev = h.events.lock().unwrap().clone();
    let attempts: Vec<u8> = ev.iter().filter(|e| e.state == EventState::Restarting).filter_map(|e| e.restart.map(|r| r.attempt)).collect();
    assert_eq!(attempts, vec![1, 2, 3]);
    let failed = ev.iter().find(|e| e.state == EventState::RestartFailed).unwrap();
    assert_eq!(failed.error_code, Some(PlayerErrorCode::PlayerWorkerCrashed));
    assert_eq!(failed.restart.map(|r| (r.attempt, r.max)), Some((3, 3)));
    // 이후 일반 명령은 player_restart_failed, 닫기는 허용
    assert!(matches!(h.host.seek(10), Err(PlayerError::RestartFailed)));
    assert!(matches!(h.host.control(ControlAction::Play), Err(PlayerError::RestartFailed)));
    // 수동 재시작: 실패 상태를 풀고 복원한다
    h.script.fail_spawns.store(0, Ordering::SeqCst);
    let info = h.host.restart().expect("restart");
    assert_eq!(info.session_id, 2, "수동 재시작은 새 세션 번호");
    h.host.seek(10).unwrap();
}

#[test]
fn t_player_host_16_open_after_failure_resets_the_counter() {
    let h = harness();
    h.open(true);
    h.script.fail_spawns.store(100, Ordering::SeqCst);
    h.host.kill_worker_for_test();
    assert!(wait_for(3000, || h.has_state(EventState::RestartFailed)));
    h.script.fail_spawns.store(0, Ordering::SeqCst);
    assert!(h.host.open(req(true)).is_ok());
    h.script.die_after_load.store(0, Ordering::SeqCst);
    h.host.kill_worker_for_test();
    assert!(wait_for(3000, || h.events.lock().unwrap().iter().filter(|e| e.state == EventState::Restarting).count() == 4), "카운터가 0 에서 다시 시작: {:?}", h.states());
}

#[test]
fn t_player_host_17_a_restore_that_keeps_dying_counts_each_failure() {
    let h = harness();
    h.open(true);
    h.script.die_after_load.store(100, Ordering::SeqCst); // 복원 열기마다 로드 직후 죽는다
    h.host.kill_worker_for_test();
    assert!(wait_for(5000, || h.has_state(EventState::RestartFailed)), "{:?}", h.states());
    let n = h.events.lock().unwrap().iter().filter(|e| e.state == EventState::Restarting).count();
    assert_eq!(n, 3);
}

#[test]
fn t_player_host_18_messages_from_an_old_incarnation_are_dropped() {
    let h = harness();
    h.open(false);
    let old = h.host.current_inc();
    h.host.kill_worker_for_test();
    assert!(wait_for(3000, || h.host.current_inc() != old && h.host.current_inc() != 0 && h.states().last() == Some(&EventState::Paused)), "{:?}", h.states());
    let n = h.events.lock().unwrap().len();
    h.host.inject(old, Message::Ended { time_ms: 999 });
    h.host.inject(old, Message::Paused { paused: false, time_ms: 1 });
    h.host.seek(0).unwrap(); // 큐 순서 보장용 동기 호출
    assert_eq!(h.events.lock().unwrap().len(), n, "오래된 세대 메시지는 상태를 바꾸지 않는다");
}

#[test]
fn t_player_host_19_release_goes_to_the_worker_only_for_the_current_epoch() {
    let h = harness();
    h.open(false);
    let epoch = h.host.current_epoch();
    assert!(epoch > 0);
    h.host.release_slots(epoch + 1, &[(0, 1)]);
    h.host.release_slots(epoch, &[]);
    h.host.release_slots(epoch, &[(1, 2), (2, 3)]);
    assert!(wait_for(1000, || h.script.count(|c| matches!(c, Command::Release { .. })) == 1), "{:?}", h.script.commands());
    let rel = h.script.commands().into_iter().find(|c| matches!(c, Command::Release { .. })).unwrap();
    assert!(matches!(rel, Command::Release { epoch: e, slots } if e == epoch && slots.len() == 2));
}

#[test]
fn t_player_host_20_release_never_blocks_the_ui_thread_when_the_worker_is_stuck() {
    // Arrange: 아무도 읽지 않는 크기 1 큐 (멈춘 워커)
    let shared = Shared::new();
    let (tx, _rx) = sync_channel::<String>(1);
    shared.set_writer(Some(tx));
    shared.epoch.store(5, Ordering::Release);
    // Act
    let t = Instant::now();
    for i in 0..2000u64 {
        shared.release(5, &[(0, i)]);
    }
    // Assert: 즉시 끝나고 넘친 것은 버려 센다 (침묵 감시가 멈춘 워커를 처리한다)
    assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
    assert!(shared.release_dropped.load(Ordering::Relaxed) >= 1999);
}

#[test]
fn t_player_host_21_close_waits_for_the_ack_and_later_old_session_events_never_emit() {
    let h = harness();
    h.open(true);
    let inc = h.host.current_inc();
    h.host.close().unwrap();
    assert!(h.script.commands().contains(&Command::Close), "워커가 파일을 놓았다는 응답까지 기다린다");
    let n = h.events.lock().unwrap().len();
    h.host.inject(inc, Message::Paused { paused: false, time_ms: 77 });
    h.host.inject(inc, Message::Ended { time_ms: 88 });
    h.host.close().unwrap();
    assert_eq!(h.events.lock().unwrap().len(), n, "닫은 뒤 이전 세션 이벤트 없음");
    assert!(wait_for(500, || !h.surface.retired.lock().unwrap().is_empty()), "버퍼를 놓았다");
}

#[test]
fn t_player_host_22_close_nowait_cancels_a_pending_open() {
    let h = harness();
    h.script.never_load.store(true, Ordering::Relaxed);
    let host = Arc::clone(&h.host);
    let t = std::thread::spawn(move || host.open(req(true)));
    assert!(wait_for(1000, || h.script.count(|c| matches!(c, Command::Open { .. })) == 1));
    let started = Instant::now();
    h.host.close_nowait(); // 페이지 재로드 훅 — 기다리지 않는다
    assert!(started.elapsed() < Duration::from_millis(50));
    let r = t.join().unwrap();
    assert!(matches!(r, Err(PlayerError::Cancelled)), "{r:?}");
    assert!(started.elapsed() < Duration::from_secs(1), "열기 마감(4초)을 기다리지 않는다");
}

#[test]
fn t_player_host_23_render_inactive_gate_is_sent_and_restored_after_a_restart() {
    // Gate ON: 렌더 꺼짐 상태
    let h = harness();
    h.open(true);
    h.host.set_render_active(false);
    assert!(wait_for(1000, || h.script.count(|c| matches!(c, Command::SetRenderActive { active: false })) == 1));
    // 재시작 복원이 꺼짐 상태를 Open 보다 먼저 다시 건다
    h.host.kill_worker_for_test();
    assert!(wait_for(3000, || h.script.spawns.load(Ordering::SeqCst) == 2 && h.script.count(|c| matches!(c, Command::Open { .. })) == 2));
    let cmds = h.script.commands();
    let open2 = cmds.iter().rposition(|c| matches!(c, Command::Open { .. })).unwrap();
    assert_eq!(cmds[open2 - 1], Command::SetRenderActive { active: false });
    // 켜면 다시 보낸다
    assert!(wait_for(2000, || h.states().last() == Some(&EventState::Playing)));
    h.host.set_render_active(true);
    assert!(wait_for(1000, || h.script.count(|c| matches!(c, Command::SetRenderActive { active: true })) == 1));
}

#[test]
fn t_player_host_24_open_with_render_inactive_completes_without_waiting_for_a_frame() {
    let h = harness();
    h.host.set_render_active(false); // 세션 전에는 기억만
    h.host.seek(0).unwrap();
    h.script.never_load.store(false, Ordering::Relaxed);
    h.host.close().unwrap();
    // 세션 없이 보낸 render_active 는 다음 열기에 반영된다
    h.open(false);
    assert!(h.script.commands().iter().any(|c| matches!(c, Command::SetRenderActive { active: false })), "{:?}", h.script.commands());
}

#[test]
fn t_player_host_25_ranges_on_path_open_update_and_restore() {
    let h = harness();
    let mut r = req(false);
    r.ranges = Some(vec![(1000, 2000), (5000, 6000)]);
    h.host.open(r).unwrap();
    assert!(h.script.commands().iter().any(|c| matches!(c, Command::Open { ranges: Some(v), duration_ms: Some(30_000), .. } if v.len() == 2)));
    // 열린 채로 구간 해제 → 이후 복원은 구간 없이 연다
    h.host.set_ranges(7, None).unwrap();
    assert!(h.script.commands().iter().any(|c| matches!(c, Command::SetRanges { ranges: None, .. })));
    h.host.kill_worker_for_test();
    assert!(wait_for(3000, || h.script.count(|c| matches!(c, Command::Open { .. })) == 2));
    assert!(h.script.commands().iter().rev().find(|c| matches!(c, Command::Open { .. })).is_some_and(|c| matches!(c, Command::Open { ranges: None, .. })));
}

#[test]
fn t_player_host_26_silent_worker_is_killed_and_restarted() {
    let h = harness();
    h.open(true);
    h.script.go_silent.store(true, Ordering::Relaxed);
    assert!(wait_for(8000, || h.script.spawns.load(Ordering::SeqCst) >= 2 || h.has_state(EventState::Restarting)), "{:?}", h.states());
    assert!(h.has_state(EventState::Restarting));
}

#[test]
fn t_player_host_27_resize_creates_a_new_epoch_once_per_interval_latest_wins() {
    let h = harness();
    h.open(false);
    let first = h.host.current_epoch();
    h.host.set_render_size(1280, 720).unwrap(); // 같은 크기 → 변경 없음
    assert_eq!(h.host.current_epoch(), first);
    // 연속 변경 — 마지막(960x540)만 최종 적용
    std::thread::sleep(Duration::from_millis(80));
    h.host.set_render_size(1000, 600).unwrap();
    h.host.set_render_size(960, 540).unwrap();
    assert!(wait_for(2000, || h.script.commands().iter().any(|c| matches!(c, Command::Attach { w: 960, h: 540, .. }))), "{:?}", h.script.commands());
    let attaches = h.script.count(|c| matches!(c, Command::Attach { .. }));
    assert!(attaches <= 3, "epoch 교체는 합쳐진다: {attaches}");
    let now = h.host.current_epoch();
    assert!(now > first);
    // 이전 epoch 의 버퍼는 Attach 성공 뒤에 놓는다
    assert!(wait_for(1000, || h.surface.retired.lock().unwrap().contains(&first)));
    // 이전 epoch 의 release 는 워커로 가지 않는다
    h.host.release_slots(first, &[(0, 1)]);
    h.host.seek(0).unwrap();
    assert_eq!(h.script.count(|c| matches!(c, Command::Release { epoch, .. } if *epoch == first)), 0);
}

#[test]
fn t_player_host_28_worker_error_during_playback_emits_an_error_event_and_keeps_the_session() {
    let h = harness();
    h.open(true);
    h.host.inject(h.host.current_inc(), Message::Error { code: crate::protocol::WorkerErrorCode::OpenFailed, msg: "read failed".into() });
    h.host.seek(0).unwrap();
    let ev = h.events.lock().unwrap().iter().find(|e| e.state == EventState::Error).cloned().expect("error event");
    assert_eq!(ev.error_code, Some(PlayerErrorCode::OpenFailed));
    assert!(!ev.error.unwrap().contains("clip"), "경로는 싣지 않는다");
}

#[test]
fn t_player_host_29_ended_buffering_and_playing_transitions_are_emitted() {
    let h = harness();
    h.open(true);
    let inc = h.host.current_inc();
    h.host.inject(inc, Message::Buffering { on: true });
    h.host.inject(inc, Message::Ended { time_ms: 29_967 });
    // 끝난 뒤 토글은 Play (워커가 처음부터 재생한다) — 큐 순서상 Ended 처리 뒤에 토글이 처리된다
    h.host.control(ControlAction::Toggle).unwrap();
    assert!(h.script.commands().iter().rev().take(2).any(|c| matches!(c, Command::Play)), "{:?}", h.script.commands());
    let ev = h.events.lock().unwrap().clone();
    assert!(ev.iter().any(|e| e.buffering), "buffering 전이");
    let ended = ev.iter().find(|e| e.state == EventState::Ended).expect("ended");
    assert_eq!(ended.time_ms, 29_967);
}

#[test]
fn t_player_host_30_shutdown_stops_the_worker_and_later_calls_fail() {
    let h = harness();
    h.open(true);
    h.host.shutdown(Duration::from_secs(2));
    assert!(h.script.commands().contains(&Command::Quit));
    assert!(matches!(h.host.seek(1), Err(PlayerError::HostGone | PlayerError::Timeout)));
}

#[test]
fn t_player_host_31_surface_failure_fails_the_open_and_keeps_the_host_usable() {
    let h = harness();
    h.surface.fail_create.store(true, Ordering::Relaxed);
    let e = h.host.open(req(true)).unwrap_err();
    assert!(e.is_engine_unavailable(), "{e}");
    h.surface.fail_create.store(false, Ordering::Relaxed);
    assert!(h.host.open(req(true)).is_ok());
}
