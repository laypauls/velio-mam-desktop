//! 열린 파일 하나의 상태와 시각 변환 (순수 로직 — mpv 없이 시험한다).
//!
//! 시각 규칙 (계약 §1.4·§1.5, 실파일 발견):
//! - IPC 의 모든 시각은 **소스 타임라인, 첫 프레임 = 0** 이다 (호스트·UI 는 start 오프셋을 모른다)
//! - `start_s` = 첫 프레임의 pts (mpv `time-pos`). 방송·HDR 파일은 0 이 아니다 (0.105s 에서 0 기준으로 맞추면 seek 가 +21.8ms 어긋났다)
//! - 프레임 격자는 `start + k/fps`. seek 는 이 격자에 맞춰 `absolute+exact` 로 보낸다
//! - EDL(구간)이면 mpv 의 시각은 EDL 타임라인(0 에서 시작)이고, `EdlMap` 이 소스 상대 시각과 오간다

use mam_player::protocol::{EdlMap, LastFrameGuard, Landing, RangeMs, ms_from_secs, snap_to_frame_start};
use std::time::Instant;

/// 비동기로 이어지는 전환 (이벤트 스레드가 다음 단계를 진행한다)
#[derive(Debug, Clone)]
pub enum Pending {
    /// 구간 열기의 1단계: 일반 파일로 먼저 열어 첫 프레임 pts(start)와 길이를 얻는다
    Probe,
    /// 구간 열기의 2단계: EDL 로 다시 열었고 첫 재시작을 기다린다
    EdlLoad,
    /// 열린 채 구간 교체(재열기) — 로드되면 목표로 seek, 도달하면 재생 상태를 되돌린다
    Reload { target_s: f64, resume_play: bool, seeked: bool, past_end: bool, deadline: Instant },
}

#[derive(Debug, Default)]
pub struct Session {
    pub open: bool,
    pub path: String,
    pub edl: Option<EdlMap>,
    /// 소스 전체 길이 (EDL 이어도 합계가 아니다) — 힌트 또는 탐색값
    pub source_duration_ms: Option<i64>,
    /// 첫 프레임 pts (초). `start_known` 이전에는 0
    pub start_s: f64,
    pub start_known: bool,
    pub fps: Option<f64>,
    /// mpv `duration` (현재 타임라인의 길이, 초)
    pub duration_s: Option<f64>,
    pub video_format: Option<String>,
    pub guard: LastFrameGuard,
    pub pending: Option<Pending>,
    /// 사용자 기준 정지 상태 (열기·전환 중의 내부 정지는 반영하지 않는다)
    pub paused: bool,
    pub eof: bool,
    /// 열기 진행 중 — `Paused` 이벤트를 내보내지 않는다
    pub opening: bool,
    /// 현재 로드(일반·EDL)의 `FILE_LOADED` 를 봤는가 — 그 전에 오는 재시작 이벤트는 이전 파일의 것이라 무시한다 (start 오판 방지)
    pub loaded: bool,
    /// 구간 열기의 시작 위치 보정 seek 를 이미 한 번 보냈는가
    pub edl_rewound: bool,
    pub autoplay_after_open: bool,
    /// 마지막으로 알린 소스 시각 (ms)
    pub shown_ms: i64,
    /// 프레임 이동이 시작된 시각 (완료 전까지 `Paused` 중간 전이를 감춘다)
    pub step: Option<Instant>,
    /// 전환(열기·구간 재열기) 중에 들어온 자막 요청 — 전환이 끝난 뒤 적용한다 (`Some(None)` = 끄기)
    pub deferred_sub: Option<Option<String>>,
    /// 열기 중에 들어온 seek — 첫 프레임 pts(start)를 읽은 뒤 적용한다 (먼저 이동하면 start 가 틀어진다)
    pub deferred_seek: Option<i64>,
    /// 구간 없는 열기의 시작 위치(이어보기) - 첫 프레임 pts 를 읽은 직후 한 번 이동하고, 도착한 재시작에서 열기를 끝낸다 (그 전까지 프레임을 게시하지 않는다)
    pub open_seek: Option<i64>,
    /// 캐시로 응답한 뒤 mpv 가 따라잡는 동안 들어온 프레임 이동 (true = 앞으로) — 따라잡기가 끝난 뒤 차례로 실행한다.
    /// mpv 는 이동 중에 들어온 같은 종류의 이동을 합쳐 버려 위치가 한 칸 모자라게 된다
    pub queued_steps: std::collections::VecDeque<bool>,
}

/// seek 목표 — mpv 타임라인의 초
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeekTarget {
    pub mpv_s: f64,
    pub landing: Landing,
}

impl Session {
    /// 새 파일 열기 — 이전 상태를 모두 버린다
    pub fn begin(path: String, edl: Option<EdlMap>, duration_hint_ms: Option<i64>, paused_after: bool) -> Self {
        Self { open: true, path, edl, source_duration_ms: duration_hint_ms, opening: true, paused: paused_after, autoplay_after_open: !paused_after, ..Self::default() }
    }

    /// mpv `time-pos`(현재 타임라인 초) → 보정된 소스 시각 (ms, 첫 프레임 = 0). `eof` 면 마지막 프레임 시각
    pub fn source_ms(&mut self, pos_s: f64, eof: bool) -> i64 {
        let fps = self.fps;
        let ms = match &self.edl {
            Some(edl) => {
                let total_s = edl.total_ms() as f64 / 1000.0;
                let c = if eof { self.guard.eof_time(Some(total_s), fps) } else { self.guard.correct(pos_s, Some(total_s), fps) };
                edl.edl_to_source(c * 1000.0)
            }
            None => {
                let rel = (pos_s - self.start_s).max(0.0);
                let c = if eof { self.guard.eof_time(self.duration_s, fps) } else { self.guard.correct(rel, self.duration_s, fps) };
                c * 1000.0
            }
        };
        ms_from_secs(ms / 1000.0).max(0)
    }

    /// 소스 시각(ms, 첫 프레임 = 0) → mpv seek 목표. 프레임 격자(`start + k/fps`)에 맞춘다 (fps 를 모르면 맞추지 않는다)
    pub fn seek_target(&self, ms: i64) -> SeekTarget {
        let want_rel = (ms.max(0)) as f64 / 1000.0;
        // 격자는 파일 pts 영역(start 기준)에서 맞추고, 상대 시각으로 되돌린다
        let snapped_rel = snap_to_frame_start(self.start_s + want_rel, self.fps, self.start_s) - self.start_s;
        match &self.edl {
            Some(edl) => {
                let (edl_ms, landing) = edl.source_to_edl(snapped_rel * 1000.0);
                SeekTarget { mpv_s: edl_ms / 1000.0, landing }
            }
            None => {
                let limit = self.duration_s.map(|d| d.max(0.0));
                let rel = limit.map_or(snapped_rel, |l| snapped_rel.min(l));
                SeekTarget { mpv_s: self.start_s + rel, landing: Landing::Inside }
            }
        }
    }

    /// 처음(첫 구간 시작 또는 파일 시작)으로 가는 seek 목표 — 끝난 뒤 다시 재생할 때
    pub fn rewind_target(&self) -> f64 {
        if self.edl.is_some() { 0.0 } else { self.start_s }
    }

    /// 구간을 바꾼다 (재열기 전). 보정 기록을 버린다
    pub fn set_edl(&mut self, edl: Option<EdlMap>) {
        self.edl = edl;
        self.guard.reset();
    }

    pub fn ranges(&self) -> Option<&[RangeMs]> {
        self.edl.as_ref().map(EdlMap::ranges)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Time(시작 오프셋·EOF·EDL) / Boundary(fps 미상) / Business rule(구간 사이 이동)

    fn plain(start: f64, fps: Option<f64>, dur: f64) -> Session {
        let mut s = Session::begin("a.mp4".into(), None, None, true);
        s.start_s = start;
        s.start_known = true;
        s.fps = fps;
        s.duration_s = Some(dur);
        s
    }

    #[test]
    fn t_player_time_10_times_are_relative_to_the_first_frame() {
        // Arrange: 첫 프레임 pts 0.105 (HDR HEVC 실파일)
        let mut s = plain(0.105, Some(25.0), 10.0);
        // Act / Assert: 첫 프레임 = 0, 이후는 start 를 뺀 값
        assert_eq!(s.source_ms(0.105, false), 0);
        assert_eq!(s.source_ms(1.105, false), 1000);
        assert_eq!(s.source_ms(0.0, false), 0, "start 이전은 0 으로");
    }

    #[test]
    fn t_player_time_11_seek_target_lands_on_the_start_offset_grid() {
        // 실파일 회귀: 0 기준으로 맞추면 상수 오프셋(+21.8ms)으로 어긋났다
        let s = plain(0.105, Some(25.0), 10.0);
        // 상대 0.5s(프레임 12.5 → 13 이 아니라 격자 위 12 또는 13): UI 가 보내는 값은 ms 로 반올림된 프레임 시작 (프레임 12 = 0.48s)
        let t = s.seek_target(480);
        assert!((t.mpv_s - (0.105 + 12.0 / 25.0)).abs() < 1e-9, "{t:?}");
        assert!((s.seek_target(0).mpv_s - 0.105).abs() < 1e-12, "첫 프레임");
        // start=0 파일은 기존과 같다
        let z = plain(0.0, Some(30.0), 10.0);
        assert!((z.seek_target(33).mpv_s - 1.0 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn t_player_time_12_seek_without_fps_is_not_snapped_and_clamped_to_the_length() {
        let s = plain(0.5, None, 10.0);
        assert!((s.seek_target(1234).mpv_s - 1.734).abs() < 1e-9);
        assert!((s.seek_target(99_000).mpv_s - 10.5).abs() < 1e-9, "길이를 넘지 않는다");
        assert!((s.seek_target(-5).mpv_s - 0.5).abs() < 1e-12, "음수는 0");
    }

    #[test]
    fn t_player_time_13_last_frame_with_offset_uses_the_eof_rule() {
        // 900 프레임 30fps, 첫 프레임 0.105 — EOF 에서는 본 최대값
        let mut s = plain(0.105, Some(30.0), 30.0);
        for f in [0, 100, 899] {
            s.source_ms(0.105 + f as f64 / 30.0, false);
        }
        assert_eq!(s.source_ms(0.0, true), ms_from_secs(899.0 / 30.0));
        // 본 적 없이 EOF: duration - 1 프레임
        let mut s2 = plain(0.105, Some(30.0), 30.0);
        assert_eq!(s2.source_ms(0.0, true), ms_from_secs(30.0 - 1.0 / 30.0));
    }

    #[test]
    fn t_player_time_14_edl_maps_between_the_edl_and_the_source_timeline() {
        // Arrange: 구간 [1s,2s) [5s,6s), 30fps, start 0.105 — EDL 시각은 0 에서 시작
        let mut s = Session::begin("a.mp4".into(), EdlMap::new(&[(1000, 2000), (5000, 6000)]), Some(30_000), true);
        s.start_s = 0.105;
        s.start_known = true;
        s.fps = Some(30.0);
        s.duration_s = Some(2.0);
        // Act / Assert: EDL 0.5s = 소스 1.5s, EDL 1.5s = 소스 5.5s
        assert_eq!(s.source_ms(0.5, false), 1500);
        assert_eq!(s.source_ms(1.5, false), 5500);
        // seek: 소스 5.5s → EDL 1.5s (프레임 격자: 5.5s = 프레임 165 → 그대로)
        let t = s.seek_target(5500);
        assert_eq!(t.landing, Landing::Inside);
        assert!((t.mpv_s - 1.5).abs() < 1e-6, "{t:?}");
        // 구간 사이로의 seek 는 다음 구간 시작, 마지막 구간 뒤는 EDL 끝
        let gap = s.seek_target(3000);
        assert_eq!((gap.landing, gap.mpv_s), (Landing::NextRange, 1.0));
        let past = s.seek_target(9000);
        assert_eq!((past.landing, past.mpv_s), (Landing::PastEnd, 2.0));
        // 처음으로: EDL 0
        assert_eq!(s.rewind_target(), 0.0);
    }

    #[test]
    fn t_player_time_15_begin_resets_everything_and_plain_rewind_is_the_start() {
        let mut old = plain(0.3, Some(30.0), 5.0);
        old.eof = true;
        old.shown_ms = 4000;
        let fresh = Session::begin("b.mp4".into(), None, Some(1000), false);
        assert!(fresh.open && fresh.opening && !fresh.start_known && !fresh.eof && fresh.shown_ms == 0);
        assert!(fresh.autoplay_after_open && !fresh.paused);
        assert_eq!(plain(0.3, None, 1.0).rewind_target(), 0.3);
    }
}
