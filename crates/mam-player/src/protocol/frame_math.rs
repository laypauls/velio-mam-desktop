//! 렌더 크기·프레임 시각·음량 곡선 등 순수 계산 (계약 §1.5, §3.6, §13).

/// 렌더 해상도 상한 (캔버스 장치 픽셀을 이 안으로 줄인다) — 4K 전체 화면 RGB 는 60fps 에서 약 2GB/s 이기 때문
pub const RENDER_CAP: (u32, u32) = (1920, 1080);

/// bgr0(픽셀당 4바이트) stride — 64 의 배수 (libmpv render.h 의 SIMD 빠른 경로 권장)
pub fn stride_for(w: u32) -> u32 {
    w.saturating_mul(4).div_ceil(64).saturating_mul(64)
}

/// 캔버스 장치 픽셀 크기 → 렌더 크기. 종횡비를 유지한 채 `cap` 안으로 줄이고 최소 2×2. 줄일 필요가 없으면 그대로
pub fn fit_render_size(w: u32, h: u32, cap: (u32, u32)) -> (u32, u32) {
    let (w, h) = (w.max(2), h.max(2));
    let scale = (cap.0 as f64 / w as f64)
        .min(cap.1 as f64 / h as f64)
        .min(1.0);
    if scale >= 1.0 {
        return (w, h);
    }
    (
        ((w as f64 * scale).round() as u32).max(2),
        ((h as f64 * scale).round() as u32).max(2),
    )
}

/// 초 → 정수 ms (반올림). 유한하지 않으면 0
pub fn ms_from_secs(s: f64) -> i64 {
    if s.is_finite() {
        (s * 1000.0).round() as i64
    } else {
        0
    }
}

/// 시각(초, 파일 pts 영역)을 가장 가까운 프레임 시작 시각 `start + k/fps` 로 맞춘다. `start` = 첫 프레임의 pts (mpv `demuxer-start-time`;
/// 방송·HDR 파일은 0 이 아니다 — 실파일: 0.105s 에서 0 기준으로 맞추면 seek 가 일정하게 +21.8ms 어긋났다).
/// UI 가 보내는 값은 ms 로 반올림된 프레임 시작이라 그대로 seek 하면 프레임 사이에 떨어진다.
/// fps 를 모르거나 유효하지 않으면(VFR 포함) 맞추지 않는다
pub fn snap_to_frame_start(t_s: f64, fps: Option<f64>, start_s: f64) -> f64 {
    let start = if start_s.is_finite() { start_s } else { 0.0 };
    match fps {
        Some(f) if f.is_finite() && f > 0.0 && t_s.is_finite() && t_s >= start => {
            start + ((t_s - start) * f).round() / f
        }
        _ => t_s,
    }
}

/// 마지막 프레임 시각 보정 — mpv 는 `keep-open` EOF 에서 `time-pos` 를 파일 길이로 바꾼다(PoC: 899번 프레임 → 30.0).
/// `time-pos >= duration` 이면 지금까지 본 `duration` 미만 최대값을 돌려주고, 본 적이 없으면 `duration - 1/fps`
#[derive(Debug, Default, Clone)]
pub struct LastFrameGuard {
    max_below: Option<f64>,
}

impl LastFrameGuard {
    /// 파일을 새로 열거나 타임라인(EDL)이 바뀌면 이전 관측을 버린다
    pub fn reset(&mut self) {
        self.max_below = None;
    }

    /// EOF(`eof-reached`)에서 보고할 마지막 프레임 시각 — 본 값 중 최대, 본 적이 없으면 `duration - 1/fps`.
    /// 첫 프레임 pts 가 0 이 아닌 파일은 EOF 의 `time-pos` 가 `duration` 과 달라질 수 있어 호출자가 `eof-reached` 로 직접 부른다
    pub fn eof_time(&self, duration: Option<f64>, fps: Option<f64>) -> f64 {
        self.max_below.unwrap_or_else(|| match (duration, fps) {
            (Some(d), Some(f)) if d.is_finite() && f.is_finite() && f > 0.0 => {
                (d - 1.0 / f).max(0.0)
            }
            (Some(d), _) if d.is_finite() => d.max(0.0),
            _ => 0.0,
        })
    }

    /// mpv `time-pos`(초)를 보정한다. `duration` 은 같은 타임라인(EDL 이면 EDL 길이)의 값이다
    pub fn correct(&mut self, pos: f64, duration: Option<f64>, fps: Option<f64>) -> f64 {
        if !pos.is_finite() {
            return 0.0;
        }
        let pos = pos.max(0.0);
        match duration {
            Some(d) if d.is_finite() && d > 0.0 && pos >= d - 1e-6 => self.eof_time(duration, fps),
            _ => {
                self.max_below = Some(self.max_below.map_or(pos, |m| m.max(pos)));
                pos
            }
        }
    }
}

/// 와이어 음량 0..=100 → mpv `volume`. mpv 는 3제곱 곡선(진폭 = (v/100)³)이라 `100·∛(v/100)` 로 바꿔 저장된 음량의 체감을 유지한다
pub fn volume_to_mpv(v: u8) -> f64 {
    100.0 * (f64::from(v.min(100)) / 100.0).cbrt()
}

/// 뒤로 한 프레임 캐시에 둘 프레임 수 — 메모리 상한 안에서 렌더 크기로 정한다. 2장 미만이면 0(캐시 사용 안 함)
pub fn back_cache_frames(cap_bytes: u64, stride: u32, h: u32) -> usize {
    let frame = stride as u64 * h as u64;
    if frame == 0 {
        return 0;
    }
    let n = (cap_bytes / frame).min(120) as usize;
    if n < 2 { 0 } else { n }
}

/// 장GOP 코덱인가 (mpv `video-format` 값). 뒤로 이동 캐시는 장GOP 에만 쓴다 — 인트라(ProRes·MJPEG·DNxHD)·짧은 GOP(MPEG-2)는
/// PoC 에서 이미 20ms 대라 캐시(프레임당 수 MB 복사)가 이득 없이 CPU 만 쓴다
pub fn is_long_gop(video_format: &str) -> bool {
    matches!(
        video_format.to_ascii_lowercase().as_str(),
        "h264" | "avc" | "hevc" | "h265" | "vp8" | "vp9" | "av1" | "mpeg4" | "vc1" | "wmv3"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Boundary(0·극단·EOF) / Time(마지막 프레임 규칙) / Business rule(곡선·캐시 수)

    #[test]
    fn t_player_geom_01_stride_is_64_aligned_and_covers_row() {
        for w in [1u32, 15, 16, 17, 640, 1280, 1920, 1921] {
            let s = stride_for(w);
            assert_eq!(s % 64, 0, "w={w}");
            assert!(s >= w * 4 && s < w * 4 + 64, "w={w}");
        }
        assert_eq!(stride_for(1280), 5120, "golden fixture 와 같은 값");
        let _ = stride_for(u32::MAX); // 넘쳐도 패닉하지 않는다
    }

    #[test]
    fn t_player_geom_02_render_size_capped_with_aspect_kept() {
        let cap = RENDER_CAP;
        assert_eq!(fit_render_size(1280, 720, cap), (1280, 720));
        assert_eq!(fit_render_size(3840, 2160, cap), (1920, 1080));
        assert_eq!(fit_render_size(2560, 1080, cap), (1920, 810));
        assert_eq!(fit_render_size(1000, 3000, cap), (360, 1080));
        assert_eq!(fit_render_size(1920, 1080, cap), (1920, 1080));
        assert_eq!(fit_render_size(0, 0, cap), (2, 2));
        let (w, h) = fit_render_size(20000, 1, cap);
        assert!(w <= 1920 && h >= 2);
    }

    #[test]
    fn t_player_time_01_snap_to_frame_start() {
        // UI 는 ms 로 반올림한 프레임 시작을 보낸다 (30fps 의 1번 프레임 = 33.33ms → 0.033)
        assert!((snap_to_frame_start(0.033, Some(30.0), 0.0) - 1.0 / 30.0).abs() < 1e-12);
        assert!(
            (snap_to_frame_start(1.0, Some(29.97002997), 0.0) - 30.0 / 29.97002997).abs() < 1e-9,
            "NTSC"
        );
        assert_eq!(snap_to_frame_start(0.0, Some(30.0), 0.0), 0.0);
        // 프레임 사이는 가까운 쪽으로
        assert!((snap_to_frame_start(0.0499, Some(30.0), 0.0) - 1.0 / 30.0).abs() < 1e-12);
        assert!((snap_to_frame_start(0.0501, Some(30.0), 0.0) - 2.0 / 30.0).abs() < 1e-12);
    }

    #[test]
    fn t_player_time_07_snap_is_relative_to_the_stream_start_time() {
        // Arrange: 실파일(HDR HEVC) — 첫 프레임 pts 0.105s, 프레임은 start + k/fps 격자 위에 있다
        let (fps, start) = (Some(25.0), 0.105);
        // Act / Assert: 프레임 10 의 시작(0.105 + 0.4)을 ms 로 반올림해 보내도 같은 프레임으로 돌아온다
        assert!((snap_to_frame_start(0.505, fps, start) - 0.505).abs() < 1e-9);
        // 0 기준으로 맞추면 0.52 (= 13/25) 가 되어 +15ms 어긋난다 — 이 회귀를 막는다
        assert!((snap_to_frame_start(0.505, fps, 0.0) - 0.52).abs() < 1e-9);
        // 프레임 사이는 격자 기준으로 가까운 쪽
        assert!((snap_to_frame_start(0.105 + 0.021, fps, start) - 0.105 - 0.04).abs() < 1e-9);
        // 시작 이전(음수 상대 시각)은 건드리지 않고, 첫 프레임은 그대로
        assert_eq!(snap_to_frame_start(0.05, fps, start), 0.05);
        assert!((snap_to_frame_start(0.105, fps, start) - 0.105).abs() < 1e-12);
        // 유효하지 않은 start 는 0 으로 본다
        assert!((snap_to_frame_start(0.04, fps, f64::NAN) - 0.04).abs() < 1e-12);
    }

    #[test]
    fn t_player_time_02_unknown_or_invalid_fps_does_not_snap() {
        for fps in [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(snap_to_frame_start(1.2345, fps, 0.0), 1.2345, "{fps:?}");
        }
        assert_eq!(
            snap_to_frame_start(-1.0, Some(30.0), 0.0),
            -1.0,
            "음수는 건드리지 않는다"
        );
        assert!(snap_to_frame_start(f64::NAN, Some(30.0), 0.0).is_nan());
    }

    #[test]
    fn t_player_time_03_last_frame_reports_the_largest_time_below_duration() {
        // Arrange: 900 프레임 30fps, 길이 30.0 — 마지막 프레임(29.9667)에서 mpv 는 time-pos 를 30.0 으로 바꾼다
        let mut g = LastFrameGuard::default();
        let fps = Some(30.0);
        // Act / Assert
        assert_eq!(g.correct(29.9, Some(30.0), fps), 29.9);
        assert!((g.correct(29.9667, Some(30.0), fps) - 29.9667).abs() < 1e-9);
        assert!(
            (g.correct(30.0, Some(30.0), fps) - 29.9667).abs() < 1e-9,
            "EOF 에서는 본 값 중 최대"
        );
        assert!(
            (g.correct(30.0, Some(30.0), fps) - 29.9667).abs() < 1e-9,
            "반복 호출도 같은 값"
        );
    }

    #[test]
    fn t_player_time_04_last_frame_without_history_uses_duration_minus_one_frame() {
        let mut g = LastFrameGuard::default();
        assert!(
            (g.correct(30.0, Some(30.0), Some(30.0)) - (30.0 - 1.0 / 30.0)).abs() < 1e-9,
            "마지막 프레임으로 바로 seek 한 경우"
        );
        let mut g = LastFrameGuard::default();
        assert_eq!(
            g.correct(30.0, Some(30.0), None),
            30.0,
            "fps 를 모르면 보정 불가"
        );
        // reset 은 이전 파일의 관측을 버린다
        let mut g = LastFrameGuard::default();
        g.correct(10.0, Some(30.0), Some(30.0));
        g.reset();
        assert!((g.correct(30.0, Some(30.0), Some(30.0)) - (30.0 - 1.0 / 30.0)).abs() < 1e-9);
    }

    #[test]
    fn t_player_time_08_eof_flag_reports_the_last_frame_even_when_time_pos_is_not_at_duration() {
        // 첫 프레임 pts 가 0.105 인 파일 — 상대 시각으로는 EOF 에서도 duration 미만일 수 있다
        let mut g = LastFrameGuard::default();
        g.correct(29.8, Some(30.0), Some(30.0));
        assert_eq!(g.eof_time(Some(30.0), Some(30.0)), 29.8, "본 최대값");
        assert!(
            (LastFrameGuard::default().eof_time(Some(30.0), Some(25.0)) - 29.96).abs() < 1e-9,
            "본 적 없으면 duration - 1 프레임"
        );
        assert_eq!(LastFrameGuard::default().eof_time(None, None), 0.0);
    }

    #[test]
    fn t_player_time_05_invalid_input_never_panics_or_goes_negative() {
        let mut g = LastFrameGuard::default();
        assert_eq!(g.correct(f64::NAN, Some(30.0), Some(30.0)), 0.0);
        assert_eq!(g.correct(-1.0, Some(30.0), Some(30.0)), 0.0);
        assert_eq!(g.correct(1.0, None, None), 1.0, "길이를 모르면 그대로");
        assert_eq!(g.correct(5.0, Some(0.0), None), 5.0, "길이 0 은 무효");
        assert_eq!(
            LastFrameGuard::default().correct(0.5, Some(0.3), Some(30.0)),
            0.3 - 1.0 / 30.0,
            "길이보다 큰 값도 EOF 로 본다"
        );
    }

    #[test]
    fn t_player_time_06_ms_rounding() {
        assert_eq!(ms_from_secs(1.0005), 1001);
        assert_eq!(ms_from_secs(0.0333), 33);
        assert_eq!(ms_from_secs(f64::NAN), 0);
        assert_eq!(ms_from_secs(f64::INFINITY), 0);
    }

    #[test]
    fn t_player_vol_01_curve_preserves_loudness_and_endpoints() {
        assert_eq!(volume_to_mpv(0), 0.0);
        assert!((volume_to_mpv(100) - 100.0).abs() < 1e-9);
        // mpv 는 진폭 = (volume/100)^3 — 변환 후 진폭이 와이어 값/100 과 같아야 한다
        for v in [1u8, 10, 30, 50, 75, 99] {
            let amp = (volume_to_mpv(v) / 100.0).powi(3);
            assert!((amp - f64::from(v) / 100.0).abs() < 1e-9, "v={v}");
        }
        assert_eq!(volume_to_mpv(250), volume_to_mpv(100), "범위 밖은 100 으로");
    }

    #[test]
    fn t_player_cache_01_frame_count_follows_render_size_and_cap() {
        let cap = 256 * 1024 * 1024;
        assert_eq!(back_cache_frames(cap, 7680, 1080), 32, "1080p");
        assert_eq!(back_cache_frames(cap, 5120, 720), 72, "720p");
        assert_eq!(back_cache_frames(cap, 256, 64), 120, "상한 120장");
        assert_eq!(back_cache_frames(cap, 0, 1080), 0);
        assert_eq!(
            back_cache_frames(8 * 1024 * 1024, 7680, 1080),
            0,
            "한 장 크기 이하면 사용 안 함"
        );
        assert_eq!(back_cache_frames(cap, 7680 * 4, 4320), 2, "8K 는 2장");
        assert_eq!(
            back_cache_frames(cap, 7680 * 8, 4320),
            0,
            "2장 미만이면 사용 안 함"
        );
    }

    #[test]
    fn t_player_cache_02_only_long_gop_codecs_use_the_cache() {
        for c in ["h264", "hevc", "H264", "vp9", "av1"] {
            assert!(is_long_gop(c), "{c}");
        }
        for c in ["prores", "mpeg2video", "mjpeg", "dnxhd", "", "rawvideo"] {
            assert!(!is_long_gop(c), "{c}");
        }
    }
}
