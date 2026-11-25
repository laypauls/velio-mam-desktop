//! 워커 재시작 후 "열린 파일·위치·상태 복원" — 순수 로직 (계약 §7).
//!
//! 호스트는 사용자 명령과 워커 이벤트를 `Remembered` 에 계속 기록한다. 워커가 죽으면 `RestorePlan` 이 새 워커에 보낼 명령을 단계별로 만든다:
//! `Open`(정지 + 구간) → 배속·음량·자막 → `FileLoaded` 에서 `Seek` → `PlaybackRestart` 의 `time_ms` 가 목표에서 20ms 이내면
//! 재생 중이었다면 `Play`, 완료. 모든 시각은 소스 타임라인이다 (EDL 변환은 워커 몫).

use std::path::PathBuf;

use crate::protocol::{Command, Message, RangeMs};

/// 복원 목표 위치 허용 오차 (ms)
pub const POSITION_TOLERANCE_MS: i64 = 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Remembered {
    pub session_id: u64,
    pub asset_id: i64,
    /// 호스트가 해석한 원본 경로 (프론트는 경로를 보내지 않는다). 세션이 없으면 None
    pub path: Option<PathBuf>,
    pub paused: bool,
    pub pos_ms: i64,
    pub length_ms: Option<i64>,
    pub fps: Option<f64>,
    pub speed: f64,
    pub volume: u8,
    pub muted: bool,
    pub ranges: Option<Vec<RangeMs>>,
    pub duration_hint_ms: Option<i64>,
    /// 이번 열기에만 쓰는 시작 위치 - 워커 재시작 복원은 `pos_ms` 를 쓰므로 여기 값을 다시 보내지 않는다
    pub start_ms: Option<i64>,
    pub subtitle: Option<PathBuf>,
    /// 마지막으로 연결한 렌더 크기
    pub render_size: (u32, u32),
    pub render_active: bool,
    pub buffering: bool,
}

impl Default for Remembered {
    fn default() -> Self {
        Self {
            session_id: 0,
            asset_id: 0,
            path: None,
            paused: true,
            pos_ms: 0,
            length_ms: None,
            fps: None,
            speed: 1.0,
            volume: 100,
            muted: false,
            ranges: None,
            duration_hint_ms: None,
            start_ms: None,
            subtitle: None,
            render_size: (0, 0),
            render_active: true,
            buffering: false,
        }
    }
}

impl Remembered {
    /// 열려 있는 세션이 있는가
    pub fn has_session(&self) -> bool {
        self.path.is_some()
    }

    /// 워커 이벤트를 반영한다 (복원 중에는 호출하지 않는다 — 복원이 끝나기 전의 값이 목표를 덮지 않도록)
    pub fn on_message(&mut self, m: &Message) {
        match m {
            Message::FileLoaded { fps, duration_ms, .. } => {
                self.fps = *fps;
                self.length_ms = *duration_ms;
            }
            Message::PlaybackRestart { time_ms } | Message::Time { ms: time_ms } | Message::Ended { time_ms } => self.pos_ms = *time_ms,
            Message::Paused { paused, time_ms } => {
                self.paused = *paused;
                self.pos_ms = *time_ms;
            }
            Message::Buffering { on } => self.buffering = *on,
            _ => {}
        }
    }

    /// 복원 계획 — 세션이 없으면 None
    pub fn plan(&self) -> Option<RestorePlan> {
        self.path.as_ref()?;
        Some(RestorePlan { rem: self.clone(), stage: Stage::NeedStart })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    NeedStart,
    WaitLoaded,
    WaitPosition,
    Done,
}

#[derive(Debug, Clone)]
pub struct RestorePlan {
    rem: Remembered,
    pub stage: Stage,
}

impl RestorePlan {
    pub fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }

    pub fn target_ms(&self) -> i64 {
        self.rem.pos_ms
    }

    /// 새 워커에 처음 보낼 명령들 (`Attach` 가 성공한 뒤에 보낸다). 열기는 항상 정지 상태다
    pub fn start(&mut self) -> Vec<Command> {
        self.stage = Stage::WaitLoaded;
        let r = &self.rem;
        let mut v = Vec::new();
        if !r.render_active {
            v.push(Command::SetRenderActive { active: false });
        }
        v.push(Command::Open {
            path: r.path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            paused: true,
            ranges: r.ranges.clone(),
            duration_ms: r.duration_hint_ms,
            start_ms: None,
        });
        if (r.speed - 1.0).abs() > 1e-9 {
            v.push(Command::SetSpeed { speed: r.speed });
        }
        if r.volume != 100 || r.muted {
            v.push(Command::SetVolume { volume: r.volume, mute: r.muted });
        }
        if let Some(sub) = &r.subtitle {
            v.push(Command::SubSet { path: Some(sub.to_string_lossy().into_owned()) });
        }
        v
    }

    /// 워커 이벤트에 반응해 다음 명령을 돌려준다
    pub fn on_message(&mut self, m: &Message) -> Vec<Command> {
        match (self.stage, m) {
            (Stage::WaitLoaded, Message::FileLoaded { .. }) => {
                self.stage = Stage::WaitPosition;
                if self.rem.pos_ms > 0 { vec![Command::Seek { ms: self.rem.pos_ms }] } else { vec![] }
            }
            (Stage::WaitPosition, Message::PlaybackRestart { time_ms }) if (time_ms - self.rem.pos_ms).abs() <= POSITION_TOLERANCE_MS => {
                self.stage = Stage::Done;
                if self.rem.paused { vec![] } else { vec![Command::Play] }
            }
            _ => vec![],
        }
    }

    /// 복원 도중 들어온 사용자 설정 변경을 목표에 반영한다 (seek·재생/정지·배속·음량·구간·자막)
    pub fn remembered_mut(&mut self) -> &mut Remembered {
        &mut self.rem
    }

    pub fn remembered(&self) -> &Remembered {
        &self.rem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Business rule(복원 순서) / Boundary(위치 0·세션 없음·허용 오차)

    fn session() -> Remembered {
        Remembered { path: Some(PathBuf::from(r"C:\media\a.mp4")), asset_id: 7, session_id: 3, ..Remembered::default() }
    }

    fn loaded() -> Message {
        Message::FileLoaded { w: None, h: None, fps: Some(30.0), duration_ms: Some(30_000), video_format: None }
    }

    #[test]
    fn t_player_host_restore_01_no_session_means_nothing_to_restore() {
        assert!(Remembered::default().plan().is_none());
        assert!(session().plan().is_some());
    }

    #[test]
    fn t_player_host_restore_02_full_sequence_restores_playing_state_at_position() {
        // Arrange: 재생 중이던 세션 — 배속 1.5·음량 40·자막·구간·12.5초
        let mut r = session();
        r.paused = false;
        r.speed = 1.5;
        r.volume = 40;
        r.subtitle = Some(PathBuf::from(r"C:\cache\sub.srt"));
        r.ranges = Some(vec![(0, 20_000)]);
        r.duration_hint_ms = Some(30_000);
        r.on_message(&Message::Time { ms: 12_500 });
        let mut p = r.plan().unwrap();
        // Act 1: 열기는 정지 상태 + 구간, 이어서 설정
        let first = p.start();
        assert!(matches!(&first[0], Command::Open { paused: true, ranges: Some(_), duration_ms: Some(30_000), .. }), "{first:?}");
        assert!(first.contains(&Command::SetSpeed { speed: 1.5 }));
        assert!(first.contains(&Command::SetVolume { volume: 40, mute: false }));
        assert!(first.iter().any(|c| matches!(c, Command::SubSet { path: Some(_) })));
        // Act 2: 로드되면 기억한 위치로 seek
        assert_eq!(p.on_message(&loaded()), vec![Command::Seek { ms: 12_500 }]);
        // 열기 자체의 PlaybackRestart(0ms) 는 완료가 아니다
        assert!(p.on_message(&Message::PlaybackRestart { time_ms: 0 }).is_empty() && !p.is_done());
        // 목표에 도달하면 재생 중이었으므로 Play 하고 완료
        assert_eq!(p.on_message(&Message::PlaybackRestart { time_ms: 12_510 }), vec![Command::Play]);
        assert!(p.is_done());
    }

    #[test]
    fn t_player_host_restore_03_paused_session_is_not_resumed_and_position_zero_skips_seek() {
        let mut p = session().plan().unwrap();
        p.start();
        assert!(p.on_message(&loaded()).is_empty(), "위치 0 이면 seek 하지 않는다");
        assert!(p.on_message(&Message::PlaybackRestart { time_ms: 0 }).is_empty(), "정지였으므로 Play 를 보내지 않는다");
        assert!(p.is_done());
    }

    #[test]
    fn t_player_host_restore_04_position_tolerance_is_20ms() {
        let mut r = session();
        r.pos_ms = 5000;
        let mut p = r.plan().unwrap();
        p.start();
        p.on_message(&loaded());
        p.on_message(&Message::PlaybackRestart { time_ms: 5021 });
        assert!(!p.is_done(), "21ms 는 밖");
        p.on_message(&Message::PlaybackRestart { time_ms: 4979 });
        assert!(!p.is_done());
        p.on_message(&Message::PlaybackRestart { time_ms: 4985 });
        assert!(p.is_done());
        assert!(p.on_message(&Message::PlaybackRestart { time_ms: 0 }).is_empty(), "완료 뒤 이벤트는 무시");
    }

    #[test]
    fn t_player_host_restore_05_events_before_the_stage_are_ignored_and_inactive_render_is_restored() {
        let mut r = session();
        r.render_active = false;
        r.muted = true;
        let mut p = r.plan().unwrap();
        assert!(p.on_message(&loaded()).is_empty(), "start 전의 FileLoaded 는 무시");
        let cmds = p.start();
        assert_eq!(cmds[0], Command::SetRenderActive { active: false }, "숨김 상태가 먼저 복원된다");
        assert!(cmds.contains(&Command::SetVolume { volume: 100, mute: true }));
        assert!(p.on_message(&Message::PlaybackRestart { time_ms: 0 }).is_empty(), "로드 전의 restart 는 무시");
        assert!(!p.is_done());
    }

    #[test]
    fn t_player_host_restore_06_state_tracking_from_worker_events() {
        let mut r = session();
        r.on_message(&Message::Paused { paused: false, time_ms: 100 });
        assert!(!r.paused && r.pos_ms == 100);
        r.on_message(&Message::PlaybackRestart { time_ms: 3000 });
        r.on_message(&Message::Buffering { on: true });
        assert!(r.pos_ms == 3000 && r.buffering);
        r.on_message(&loaded());
        assert_eq!((r.fps, r.length_ms), (Some(30.0), Some(30_000)));
        r.on_message(&Message::Ended { time_ms: 29_967 });
        assert_eq!(r.pos_ms, 29_967);
    }
}
