//! mpv 이벤트 스레드 — mpv 이벤트를 프로토콜 메시지로 바꾸고, 열기·구간 재열기의 다음 단계를 진행한다.
//! 표시 시각은 `PLAYBACK_RESTART` 시점에 갱신한다 (PoC: seek 직후 옛 프레임이 먼저 그려지는 경우가 20–97% — 재시작 이벤트는 맞는 프레임 뒤에 온다).
//! 이 스레드만 `time-pos` 를 조회한다 (렌더 스레드는 mpv 속성 API 를 부르지 않는다).

use super::mpv_api::*;
use super::player::{Inner, OBS_CACHE_PAUSE, OBS_EOF, OBS_PAUSE, OBS_TIME};
use super::session::Pending;
use mam_player::protocol::{Message, WorkerErrorCode, ms_from_secs};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 프레임 이동이 이 시간 안에 `PLAYBACK_RESTART` 를 내지 않으면(첫/마지막 프레임 no-op) 현재 위치로 완료를 알린다
const STEP_FALLBACK: Duration = Duration::from_millis(300);
/// 위치 일치 허용 오차 (초)
const NEAR_S: f64 = 0.02;

pub fn spawn(inner: Arc<Inner>, log: bool) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mpv = inner.mpv.clone();
        let mut last_time = Instant::now() - Duration::from_secs(1);
        // 100ms 안에 들어온 time-pos 는 버리지 않고 보류했다가 보낸다 (프레임 이동 직후의 마지막 값이 사라져 시간 표시가 낡는 것 방지)
        let mut pending_time: Option<f64> = None;
        let mut last_stats = Instant::now();
        while !inner.shared.stop.load(Ordering::Acquire) {
            let ev = unsafe { &*(mpv.api.wait_event)(mpv.handle.0, 0.1) };
            match ev.event_id {
                EV_SHUTDOWN => break,
                EV_LOG_MESSAGE if log && !ev.data.is_null() => {
                    let l = unsafe { &*(ev.data as *const EventLog) };
                    eprintln!(
                        "[mpv {}] {}: {}",
                        unsafe { lossy(l.level) },
                        unsafe { lossy(l.prefix) },
                        unsafe { lossy(l.text) }.trim_end()
                    );
                }
                EV_FILE_LOADED => on_file_loaded(&inner),
                EV_PLAYBACK_RESTART => on_restart(&inner),
                EV_END_FILE if !ev.data.is_null() => {
                    let e = unsafe { &*(ev.data as *const EventEndFile) };
                    if e.reason == END_FILE_ERROR {
                        on_read_error(&inner);
                    }
                }
                EV_PROPERTY_CHANGE if !ev.data.is_null() => {
                    let p = unsafe { &*(ev.data as *const EventProperty) };
                    if p.data.is_null() {
                        continue; // 속성을 쓸 수 없는 상태 (FORMAT_NONE)
                    }
                    match (ev.reply_userdata, p.format) {
                        (OBS_PAUSE, FORMAT_FLAG) => {
                            on_pause(&inner, unsafe { *(p.data as *const i32) } != 0)
                        }
                        (OBS_EOF, FORMAT_FLAG) => {
                            on_eof(&inner, unsafe { *(p.data as *const i32) } != 0)
                        }
                        (OBS_CACHE_PAUSE, FORMAT_FLAG) => inner.send(Message::Buffering {
                            on: unsafe { *(p.data as *const i32) } != 0,
                        }),
                        (OBS_TIME, FORMAT_DOUBLE) => {
                            let pos = unsafe { *(p.data as *const f64) };
                            if !on_step_time(&inner, pos) {
                                pending_time = Some(pos);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
            if let Some(pos) = pending_time
                && last_time.elapsed() >= Duration::from_millis(100)
            {
                last_time = Instant::now();
                pending_time = None;
                on_time(&inner, pos);
            }
            tick(&inner);
            if last_stats.elapsed() >= Duration::from_millis(500) {
                last_stats = Instant::now();
                // 하트비트 — 세션·슬롯이 없어도 보낸다 (호스트의 침묵 감시)
                let sh = &inner.shared;
                inner.send(Message::Stats {
                    rendered: sh.rendered.load(Ordering::Relaxed),
                    skipped: sh.skipped.load(Ordering::Relaxed),
                    ignored: sh.ignored.load(Ordering::Relaxed),
                    cache_hits: sh
                        .cache
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .as_ref()
                        .map_or(0, |c| c.hits()),
                });
            }
        }
    })
}

fn on_file_loaded(i: &Inner) {
    let m = &i.mpv;
    let (w, h) = (m.get_int("width"), m.get_int("height"));
    let fps = m
        .get_double("container-fps")
        .filter(|f| f.is_finite() && *f > 0.0);
    let dur = m
        .get_double("duration")
        .filter(|d| d.is_finite() && *d >= 0.0);
    let fmt = m.get_string("video-format");
    let mut s = i.lock();
    s.loaded = true;
    if fps.is_some() {
        s.fps = fps;
    }
    s.duration_s = dur;
    if fmt.is_some() {
        s.video_format = fmt.clone();
    }
    match s.pending.clone() {
        Some(Pending::Probe) => {
            // 구간 열기 1단계 — 소스 길이를 얻는다 (힌트가 없을 때). 호스트에는 알리지 않는다
            if s.source_duration_ms.is_none() {
                s.source_duration_ms = dur.map(ms_from_secs);
            }
        }
        Some(Pending::Reload { target_s, .. }) => {
            if let Some(Pending::Reload { seeked, .. }) = s.pending.as_mut() {
                *seeked = true;
            }
            drop(s);
            let _ = m.command(&["seek", &format!("{target_s}"), "absolute+exact"]);
        }
        _ => {
            let duration_ms = if s.edl.is_some() {
                s.source_duration_ms
            } else {
                dur.map(ms_from_secs)
            };
            let video_format = s.video_format.clone();
            drop(s);
            i.send(Message::FileLoaded {
                w,
                h,
                fps,
                duration_ms,
                video_format,
            });
            i.configure_cache();
        }
    }
}

fn on_restart(i: &Inner) {
    // 열기 단계에서는 이번 로드의 FILE_LOADED 뒤에 온 재시작만 본다
    {
        let s = i.lock();
        if (s.opening || matches!(s.pending, Some(Pending::Probe | Pending::EdlLoad))) && !s.loaded
        {
            return;
        }
    }
    let Some(pos) = i.mpv.get_double("time-pos") else {
        return;
    };
    let mut s = i.lock();
    match s.pending.clone() {
        Some(Pending::Probe) => {
            // 첫 프레임 pts = 프레임 격자의 시작. 이제 EDL 로 다시 연다
            s.start_s = pos;
            s.start_known = true;
            s.pending = Some(Pending::EdlLoad);
            drop(s);
            if let Err((code, msg)) = i.load_edl() {
                fail_open(i, code, msg);
            }
        }
        Some(Pending::EdlLoad) => {
            // EDL 은 0 에서 시작해야 한다. 부하 중에 다른 위치(예: 둘째 구간 시작)로 보고되면 한 번 되감고 다음 재시작을 기다린다
            if pos > 0.05 && !s.edl_rewound {
                s.edl_rewound = true;
                drop(s);
                let _ = i.mpv.command(&["seek", "0", "absolute+exact"]);
                return;
            }
            complete_open(i, s, pos)
        }
        Some(Pending::Reload {
            target_s, seeked, ..
        }) => {
            if seeked && (pos - target_s).abs() <= NEAR_S {
                complete_reload(i, s, pos);
            }
        }
        None if s.opening => {
            if !s.start_known {
                s.start_s = pos;
                s.start_known = true;
            }
            if let Some(ms) = s.open_seek.take() {
                // 끝 근처(0.5초 이내)는 이동하지 않는다 - EOF 에 닿으면 재시작 이벤트가 오지 않아 열기가 멈춘다
                let target = s.seek_target(ms).mpv_s;
                let near_end = s.duration_s.is_some_and(|d| ms as f64 / 1000.0 >= d - 0.5);
                if !near_end {
                    drop(s);
                    if i.mpv
                        .command(&["seek", &format!("{target}"), "absolute+exact"])
                        .is_ok()
                    {
                        return; // 도착 재시작에서 열기를 끝낸다
                    }
                    s = i.lock();
                }
            }
            complete_open(i, s, pos);
        }
        None => {
            let eof = s.eof;
            let ms = s.source_ms(pos, eof);
            s.shown_ms = ms;
            let stepped = s.step.take().is_some();
            drop(s);
            // 캐시로 이미 응답한 뒤 mpv 가 따라잡은 프레임 이동 — 이미 알렸다
            if i.shared
                .suppress
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
                .is_ok()
            {
                i.drain_steps();
                return;
            }
            i.send(Message::PlaybackRestart { time_ms: ms });
            if stepped {
                i.send(Message::Paused {
                    paused: true,
                    time_ms: ms,
                });
            }
        }
    }
}

/// 열기 마무리 — 첫 프레임이 화면에 올랐다. 자동 재생이면 이제 재생을 시작한다
fn complete_open(i: &Inner, mut s: std::sync::MutexGuard<'_, super::session::Session>, pos: f64) {
    s.opening = false;
    s.pending = None;
    let ms = s.source_ms(pos, false);
    s.shown_ms = ms;
    let autoplay = s.autoplay_after_open;
    s.paused = !autoplay;
    let deferred = s.deferred_sub.take();
    let deferred_seek = s.deferred_seek.take();
    drop(s);
    // 구간 열기는 중간 프레임을 붙들어 두었으므로 현재 프레임을 한 번 게시한다 (일반 열기는 이미 게시됐다)
    if i.shared.hold.swap(false, Ordering::AcqRel) {
        i.shared.redraw.store(true, Ordering::Release);
        i.shared.notify();
    }
    i.shared.has_video.store(true, Ordering::Release);
    if let Some(sub) = deferred {
        let _ = i.sub_set(sub);
    }
    if autoplay {
        let _ = i.mpv.set_prop("pause", "no");
    }
    i.send(Message::PlaybackRestart { time_ms: ms });
    i.send(Message::Paused {
        paused: !autoplay,
        time_ms: ms,
    });
    if let Some(t) = deferred_seek {
        let _ = i.seek(t);
    }
}

/// 구간 재열기 마무리 — 목표 위치에 도달했다. 재생 중이었다면 이어서 재생한다
fn complete_reload(i: &Inner, mut s: std::sync::MutexGuard<'_, super::session::Session>, pos: f64) {
    let resume = matches!(
        s.pending,
        Some(Pending::Reload {
            resume_play: true,
            ..
        })
    );
    s.pending = None;
    let ms = s.source_ms(pos, false);
    s.shown_ms = ms;
    s.paused = !resume;
    let deferred = s.deferred_sub.take();
    drop(s);
    if let Some(sub) = deferred {
        let _ = i.sub_set(sub);
    }
    i.shared.hold.store(false, Ordering::Release);
    i.shared.redraw.store(true, Ordering::Release);
    i.shared.notify();
    if resume {
        let _ = i.mpv.set_prop("pause", "no");
    }
    i.send(Message::PlaybackRestart { time_ms: ms });
    i.send(Message::Paused {
        paused: !resume,
        time_ms: ms,
    });
}

fn fail_open(i: &Inner, code: WorkerErrorCode, msg: String) {
    {
        let mut s = i.lock();
        s.pending = None;
        s.opening = false;
        s.open = false;
    }
    i.shared.hold.store(false, Ordering::Release);
    i.send(Message::Error { code, msg });
}

fn on_read_error(i: &Inner) {
    let was_opening = {
        let s = i.lock();
        s.opening || s.pending.is_some()
    };
    if was_opening {
        fail_open(
            i,
            WorkerErrorCode::OpenFailed,
            "cannot open the media".into(),
        );
    } else {
        i.send(Message::Error {
            code: WorkerErrorCode::OpenFailed,
            msg: "cannot read the media".into(),
        });
    }
}

fn on_pause(i: &Inner, paused: bool) {
    let mut s = i.lock();
    // 열기·전환·프레임 이동 중의 내부 정지 전이는 사용자에게 알리지 않는다 (완료 시 한 번에 알린다)
    if s.opening || s.pending.is_some() || s.step.is_some() || !s.open {
        return;
    }
    s.paused = paused;
    let ms = s.shown_ms;
    drop(s);
    i.send(Message::Paused {
        paused,
        time_ms: ms,
    });
}

/// 프레임 이동 중 `time-pos` 가 바뀌었다 — 새 프레임이 화면에 올라온 것이다 (앞으로 이동은 `PLAYBACK_RESTART` 가 없고 시각 변화만 있다).
/// 완료를 즉시 알린다 (100ms 시각 제한을 거치지 않는다)
fn on_step_time(i: &Inner, pos: f64) -> bool {
    let mut s = i.lock();
    if s.step.is_none()
        || s.pending.is_some()
        || s.opening
        || i.shared.suppress.load(Ordering::Acquire) > 0
    {
        return false;
    }
    s.step = None;
    let eof = s.eof;
    let ms = s.source_ms(pos, eof);
    s.shown_ms = ms;
    drop(s);
    i.send(Message::PlaybackRestart { time_ms: ms });
    i.send(Message::Paused {
        paused: true,
        time_ms: ms,
    });
    true
}

fn on_eof(i: &Inner, eof: bool) {
    let mut s = i.lock();
    if !s.open {
        return;
    }
    if !eof {
        s.eof = false;
        return;
    }
    if let Some(Pending::Reload { .. }) = s.pending {
        // 마지막 구간 끝으로의 재열기 — 도착으로 본다
        complete_reload(i, s, 0.0);
        s = i.lock();
    }
    if s.opening {
        return;
    }
    s.eof = true;
    let ms = s.source_ms(0.0, true);
    s.shown_ms = ms;
    drop(s);
    i.send(Message::Ended { time_ms: ms });
}

fn on_time(i: &Inner, pos: f64) {
    if i.shared.suppress.load(Ordering::Acquire) > 0 {
        return;
    }
    let mut s = i.lock();
    if s.opening || s.pending.is_some() || s.step.is_some() || !s.open || s.eof {
        return;
    }
    let ms = s.source_ms(pos, false);
    s.shown_ms = ms;
    drop(s);
    i.send(Message::Time { ms });
}

/// 주기 점검 — 프레임 이동 무반응 보정, 구간 재열기 제한 시간
fn tick(i: &Inner) {
    // 안전장치: 따라잡기 신호가 오지 않아 suppress 가 풀린 뒤에도 대기 이동을 비운다
    i.drain_steps();
    let mut s = i.lock();
    if let Some(t) = s.step
        && t.elapsed() >= STEP_FALLBACK
    {
        s.step = None;
        let ms = s.shown_ms;
        drop(s);
        // 첫/마지막 프레임에서의 이동은 mpv 가 아무것도 하지 않는다 — 그래도 완료를 알린다 (UI 가 시각을 갱신한다)
        i.send(Message::PlaybackRestart { time_ms: ms });
        i.send(Message::Paused {
            paused: true,
            time_ms: ms,
        });
        return;
    }
    if let Some(Pending::Reload {
        deadline, target_s, ..
    }) = s.pending
        && Instant::now() >= deadline
    {
        complete_reload(i, s, target_s);
    }
}
