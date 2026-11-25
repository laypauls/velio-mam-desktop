//! `player://state` 이벤트 모델과 중계 제한(throttle) — 계약 §2.
//! 전이(transition)는 절대 버리거나 늦추지 않는다. 재생 중 시각 갱신은 ≤ 5Hz·값이 바뀐 때만, seek/step 완료는 ≤ 1/33ms(마지막 값 우선),
//! 정지 중에는 주기 이벤트가 없다. 같은 페이로드가 연속되면 한 번만 보낸다. 프레임은 이벤트로 보내지 않는다.
//! 시계는 호출자가 `now` 로 넘긴다 (시험에서 고정).

use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EventState {
    Opening,
    Playing,
    Paused,
    Ended,
    Error,
    Restarting,
    RestartFailed,
}

/// 이벤트에 실리는 오류 코드 (앱의 `ErrorCode` 와 같은 snake_case 문자열 — src-tauri 시험이 일치를 검증한다)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerErrorCode {
    OpenFailed,
    PlayerWorkerCrashed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RestartInfo {
    pub attempt: u8,
    pub max: u8,
}

/// `player://state` 페이로드 — 경로·비밀을 싣지 않는다. `time_ms` 는 소스 타임라인(0 = 파일 첫 프레임)의 보정된 값
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerEvent {
    pub session_id: u64,
    pub asset_id: i64,
    pub state: EventState,
    pub time_ms: i64,
    /// `null` 도 직렬화한다
    pub length_ms: Option<i64>,
    pub fps: Option<f64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub buffering: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<PlayerErrorCode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart: Option<RestartInfo>,
}

impl PlayerEvent {
    pub fn new(
        session_id: u64,
        asset_id: i64,
        state: EventState,
        time_ms: i64,
        length_ms: Option<i64>,
        fps: Option<f64>,
    ) -> Self {
        Self {
            session_id,
            asset_id,
            state,
            time_ms,
            length_ms,
            fps,
            buffering: false,
            error: None,
            error_code: None,
            restart: None,
        }
    }
}

pub type EventSink = Arc<dyn Fn(PlayerEvent) + Send + Sync>;

/// 이벤트 종류별 중계 규칙
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emit {
    /// 상태 전이 — 즉시, 항상
    Transition,
    /// 재생 중 시각 갱신 — 5Hz, 값이 바뀐 때만
    Time,
    /// seek/step 완료 — 33ms 에 한 번, 넘치면 마지막 값을 trailing 으로
    Completion,
}

const TIME_INTERVAL: Duration = Duration::from_millis(200);
const COMPLETION_INTERVAL: Duration = Duration::from_millis(33);

pub struct EventBridge {
    sink: EventSink,
    last: Option<PlayerEvent>,
    last_time_at: Option<Instant>,
    last_completion_at: Option<Instant>,
    trailing: Option<PlayerEvent>,
}

impl EventBridge {
    pub fn new(sink: EventSink) -> Self {
        Self {
            sink,
            last: None,
            last_time_at: None,
            last_completion_at: None,
            trailing: None,
        }
    }

    /// 새 세션·닫기 — 대기 중인 trailing 과 중복 기준을 버린다 (닫은 뒤에는 이전 세션 이벤트가 나가지 않는다)
    pub fn reset(&mut self) {
        self.last = None;
        self.trailing = None;
        self.last_time_at = None;
        self.last_completion_at = None;
    }

    /// 열기 응답이 이미 알려 준 상태를 중복 기준으로만 기록한다 (이벤트는 보내지 않는다)
    pub fn seed(&mut self, ev: PlayerEvent) {
        self.last = Some(ev);
    }

    pub fn last(&self) -> Option<&PlayerEvent> {
        self.last.as_ref()
    }

    fn send(&mut self, ev: PlayerEvent) {
        if self.last.as_ref() == Some(&ev) {
            return;
        }
        self.last = Some(ev.clone());
        (self.sink)(ev);
    }

    pub fn emit(&mut self, ev: PlayerEvent, kind: Emit, now: Instant) {
        match kind {
            Emit::Transition => {
                self.trailing = None; // 더 새로운 전이가 trailing 을 대체한다
                self.send(ev);
            }
            Emit::Time => {
                let changed = self.last.as_ref().is_none_or(|l| l.time_ms != ev.time_ms);
                let due = self
                    .last_time_at
                    .is_none_or(|t| now.saturating_duration_since(t) >= TIME_INTERVAL);
                if ev.state == EventState::Playing && changed && due {
                    self.last_time_at = Some(now);
                    self.send(ev);
                }
            }
            Emit::Completion => {
                if self
                    .last_completion_at
                    .is_none_or(|t| now.saturating_duration_since(t) >= COMPLETION_INTERVAL)
                {
                    self.last_completion_at = Some(now);
                    self.trailing = None;
                    self.send(ev);
                } else {
                    self.trailing = Some(ev);
                }
            }
        }
    }

    /// 대기 중인 trailing 이 때가 됐으면 보낸다 (감독자 루프가 주기적으로 부른다)
    pub fn poll(&mut self, now: Instant) {
        if let Some(t) = self.last_completion_at
            && self.trailing.is_some()
            && now.saturating_duration_since(t) >= COMPLETION_INTERVAL
        {
            self.last_completion_at = Some(now);
            if let Some(ev) = self.trailing.take() {
                self.send(ev);
            }
        }
    }

    /// trailing 이 있으면 보낼 시각 (루프의 대기 상한 계산용)
    pub fn next_due(&self) -> Option<Instant> {
        self.trailing
            .as_ref()
            .and(self.last_completion_at)
            .map(|t| t + COMPLETION_INTERVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // 분류: Business rule(throttle) / Boundary(dedupe·trailing) / Time(주입 시계) / Validation(직렬화 모양 = golden)

    fn bridge() -> (EventBridge, Arc<Mutex<Vec<PlayerEvent>>>) {
        let got: Arc<Mutex<Vec<PlayerEvent>>> = Arc::default();
        let g = Arc::clone(&got);
        (
            EventBridge::new(Arc::new(move |e| g.lock().unwrap().push(e))),
            got,
        )
    }

    fn ev(state: EventState, t: i64) -> PlayerEvent {
        PlayerEvent::new(5, 7, state, t, Some(120_000), Some(25.0))
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn fixture() -> serde_json::Value {
        let raw =
            include_str!("../../../../src/test/fixtures/player-wire.json").replace("\r\n", "\n");
        serde_json::from_str(&raw).expect("fixture")
    }

    #[test]
    fn t_ipc_player_mpv_20_event_payloads_match_the_golden_fixture() {
        let fx = fixture();
        let se = &fx["stateEvents"];
        let playing = ev(EventState::Playing, 1500);
        assert_eq!(serde_json::to_value(&playing).unwrap(), se["playing"]);
        let mut unknown = PlayerEvent::new(5, 7, EventState::Paused, 0, None, None);
        assert_eq!(
            serde_json::to_value(&unknown).unwrap(),
            se["pausedUnknownFps"],
            "null 도 직렬화"
        );
        let mut b = ev(EventState::Playing, 61_000);
        b.buffering = true;
        assert_eq!(serde_json::to_value(&b).unwrap(), se["buffering"]);
        let mut r = ev(EventState::Restarting, 12_000);
        r.restart = Some(RestartInfo { attempt: 1, max: 3 });
        assert_eq!(serde_json::to_value(&r).unwrap(), se["restarting"]);
        let mut rf = ev(EventState::RestartFailed, 12_000);
        rf.error = Some("player worker stopped repeatedly".into());
        rf.error_code = Some(PlayerErrorCode::PlayerWorkerCrashed);
        rf.restart = Some(RestartInfo { attempt: 3, max: 3 });
        assert_eq!(serde_json::to_value(&rf).unwrap(), se["restartFailed"]);
        let mut e = PlayerEvent::new(5, 7, EventState::Error, 0, None, None);
        e.error = Some("cannot open media".into());
        e.error_code = Some(PlayerErrorCode::OpenFailed);
        assert_eq!(serde_json::to_value(&e).unwrap(), se["error"]);
        unknown.buffering = false; // 필드가 false 면 생략
        assert!(
            serde_json::to_value(&unknown)
                .unwrap()
                .get("buffering")
                .is_none()
        );
    }

    #[test]
    fn t_player_host_ev_01_transitions_are_never_dropped_but_identical_ones_are_deduped() {
        let (mut b, got) = bridge();
        let t0 = Instant::now();
        b.emit(ev(EventState::Playing, 0), Emit::Transition, t0);
        b.emit(ev(EventState::Playing, 0), Emit::Transition, t0);
        b.emit(ev(EventState::Paused, 0), Emit::Transition, t0);
        b.emit(ev(EventState::Playing, 0), Emit::Transition, t0);
        assert_eq!(
            got.lock()
                .unwrap()
                .iter()
                .map(|e| e.state)
                .collect::<Vec<_>>(),
            vec![EventState::Playing, EventState::Paused, EventState::Playing]
        );
    }

    #[test]
    fn t_player_host_ev_02_time_updates_are_5hz_changed_only_and_playing_only() {
        let (mut b, got) = bridge();
        let t0 = Instant::now();
        b.emit(ev(EventState::Playing, 0), Emit::Transition, t0);
        // 100ms 간격 10개 → 5Hz 로 솎아낸다
        for i in 1..=10u64 {
            b.emit(
                ev(EventState::Playing, i as i64 * 100),
                Emit::Time,
                t0 + ms(i * 100),
            );
        }
        let n = got.lock().unwrap().len();
        assert_eq!(
            n,
            1 + 5,
            "{:?}",
            got.lock()
                .unwrap()
                .iter()
                .map(|e| e.time_ms)
                .collect::<Vec<_>>()
        );
        // 값이 같으면 보내지 않고, 정지 중에는 주기 이벤트가 없다
        let last_t = got.lock().unwrap().last().unwrap().time_ms;
        b.emit(ev(EventState::Playing, last_t), Emit::Time, t0 + ms(5000));
        b.emit(ev(EventState::Paused, 99), Emit::Time, t0 + ms(6000));
        assert_eq!(got.lock().unwrap().len(), n);
    }

    #[test]
    fn t_player_host_ev_03_completions_are_33ms_with_trailing_latest_value() {
        let (mut b, got) = bridge();
        let t0 = Instant::now();
        b.emit(ev(EventState::Paused, 100), Emit::Completion, t0);
        b.emit(ev(EventState::Paused, 133), Emit::Completion, t0 + ms(10));
        b.emit(ev(EventState::Paused, 167), Emit::Completion, t0 + ms(20));
        assert_eq!(got.lock().unwrap().len(), 1, "33ms 안의 완료는 보류");
        assert!(b.next_due().is_some());
        b.poll(t0 + ms(25));
        assert_eq!(got.lock().unwrap().len(), 1, "아직 때가 아니다");
        b.poll(t0 + ms(40));
        let times: Vec<i64> = got.lock().unwrap().iter().map(|e| e.time_ms).collect();
        assert_eq!(times, vec![100, 167], "trailing 은 마지막 값");
        assert!(b.next_due().is_none());
    }

    #[test]
    fn t_player_host_ev_04_a_transition_supersedes_a_pending_trailing_completion_and_reset_drops_it()
     {
        let (mut b, got) = bridge();
        let t0 = Instant::now();
        b.emit(ev(EventState::Paused, 100), Emit::Completion, t0);
        b.emit(ev(EventState::Paused, 133), Emit::Completion, t0 + ms(5));
        b.emit(ev(EventState::Playing, 133), Emit::Transition, t0 + ms(6));
        b.poll(t0 + ms(100));
        assert_eq!(
            got.lock()
                .unwrap()
                .iter()
                .map(|e| (e.state, e.time_ms))
                .collect::<Vec<_>>(),
            vec![(EventState::Paused, 100), (EventState::Playing, 133)]
        );
        // reset(닫기·새 세션) 뒤에는 이전 세션의 trailing 이 나가지 않는다
        b.emit(ev(EventState::Playing, 140), Emit::Completion, t0 + ms(110));
        b.emit(ev(EventState::Playing, 150), Emit::Completion, t0 + ms(115));
        b.reset();
        b.poll(t0 + ms(500));
        assert_eq!(got.lock().unwrap().len(), 3);
        // reset 뒤 같은 페이로드도 다시 보낸다 (중복 기준 초기화)
        b.emit(ev(EventState::Playing, 140), Emit::Transition, t0 + ms(600));
        assert_eq!(got.lock().unwrap().len(), 4);
    }

    #[test]
    fn t_player_host_ev_05_seeded_state_dedupes_the_first_identical_transition() {
        let (mut b, got) = bridge();
        let t0 = Instant::now();
        b.seed(ev(EventState::Paused, 0));
        b.emit(ev(EventState::Paused, 0), Emit::Transition, t0);
        assert!(got.lock().unwrap().is_empty(), "열기 응답이 이미 알렸다");
        b.emit(ev(EventState::Playing, 0), Emit::Transition, t0);
        assert_eq!(got.lock().unwrap().len(), 1);
    }
}
