//! host → worker 명령. 모든 시각은 **소스 타임라인**(ms)이다 — EDL 변환은 워커 안에서만 한다.

use super::frames::SlotRef;
use serde::{Deserialize, Serialize};

/// 재생 구간 `(start_ms, end_ms)` — 정렬·병합 끝난 값
pub type RangeMs = (i64, i64);

/// `player_control` 동작 (프론트 `PlayerControlAction`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ControlAction {
    Play,
    Pause,
    Toggle,
    StepForward,
    StepBack,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// 파일 열기. `ranges` 가 있으면 워커가 EDL 로 만든다. `duration_ms` = 소스 길이 힌트(없으면 워커가 먼저 열어 길이를 얻는다). `start_ms` = 열기 직후 시작할 소스 시각(첫 프레임 = 0, 없으면 처음) - 첫 프레임을 보이기 전에 이동한다
    Open {
        path: String,
        paused: bool,
        ranges: Option<Vec<RangeMs>>,
        duration_ms: Option<i64>,
        start_ms: Option<i64>,
    },
    /// 열린 채로 구간 교체 (`None` = 구간 해제). 위치·재생 상태를 유지한다
    SetRanges {
        ranges: Option<Vec<RangeMs>>,
        duration_ms: Option<i64>,
    },
    Play,
    Pause,
    /// 소스 시각(ms) — 워커가 프레임 시작 시각으로 맞춰 `absolute+exact` 로 이동한다. 재생/정지 상태는 바꾸지 않는다
    Seek {
        ms: i64,
    },
    FrameStep,
    FrameBackStep,
    SetSpeed {
        speed: f64,
    },
    /// `volume` 0..=100 (와이어 값) — 워커가 mpv 곡선으로 바꾼다. `mute` 이거나 0 이면 무음
    SetVolume {
        volume: u8,
        mute: bool,
    },
    /// 외부 자막 선택(`sub-add <path> select`) / `None` = 끔
    SubSet {
        path: Option<String>,
    },
    /// `false` 면 렌더·프레임 게시를 멈춘다 (창 최소화·보기 숨김)
    SetRenderActive {
        active: bool,
    },
    /// 프레임 슬롯 연결 — `handles` 는 **워커 프로세스 안에서 유효한** 파일 매핑 핸들(호스트가 DuplicateHandle 로 복제).
    /// 이전 연결은 해제되고, 새 연결 직후 현재 프레임을 한 번 다시 그려 게시한다
    Attach {
        epoch: u64,
        w: u32,
        h: u32,
        stride: u32,
        size: u64,
        handles: Vec<u64>,
    },
    Detach,
    /// JS 가 돌려준 슬롯들 (id=0 으로 보내 응답을 받지 않는다)
    Release {
        epoch: u64,
        slots: Vec<SlotRef>,
    },
    /// 진단용 — 워커가 본 mpv 상태
    Probe,
    Close,
    Quit,
}

/// 명령 봉투 — `{"id":1,"cmd":"open",...}`. `id == 0` 은 응답을 받지 않는 고빈도 명령(Release)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub command: Command,
}

#[cfg(test)]
mod tests {
    use super::super::wire::{decode_line, encode_line};
    use super::*;

    // 분류: Happy / Boundary(선택 필드 생략) / Validation(필수 누락·알 수 없는 명령)

    #[test]
    fn t_player_proto_10_request_round_trip() {
        let reqs = [
            Request {
                id: 1,
                command: Command::Open {
                    path: r"C:\a b\한글.mp4".into(),
                    paused: true,
                    ranges: Some(vec![(0, 1000), (5000, 6000)]),
                    duration_ms: Some(30_000),
                    start_ms: Some(12_345),
                },
            },
            Request {
                id: 2,
                command: Command::Seek { ms: 33 },
            },
            Request {
                id: 3,
                command: Command::SubSet { path: None },
            },
            Request {
                id: 4,
                command: Command::SetVolume {
                    volume: 0,
                    mute: true,
                },
            },
            Request {
                id: 0,
                command: Command::Release {
                    epoch: 2,
                    slots: vec![SlotRef { slot: 1, seq: 77 }],
                },
            },
            Request {
                id: 5,
                command: Command::Attach {
                    epoch: 2,
                    w: 1280,
                    h: 720,
                    stride: 5120,
                    size: 3_686_400,
                    handles: vec![0x1a4, 0x1a8, 0x1ac],
                },
            },
        ];
        for r in reqs {
            let line = encode_line(&r).unwrap();
            assert_eq!(decode_line::<Request>(&line).unwrap(), r);
        }
    }

    #[test]
    fn t_player_proto_11_wire_shape_is_stable() {
        let line = encode_line(&Request {
            id: 7,
            command: Command::FrameStep,
        })
        .unwrap();
        assert_eq!(line.trim(), r#"{"id":7,"cmd":"frame_step"}"#);
        // 선택 필드는 생략해도 된다 (구버전 호스트와의 호환)
        let r: Request =
            decode_line(r#"{"id":1,"cmd":"open","path":"a.mp4","paused":false}"#).unwrap();
        assert!(matches!(
            r.command,
            Command::Open {
                ranges: None,
                duration_ms: None,
                start_ms: None,
                ..
            }
        ));
    }

    #[test]
    fn t_player_proto_12_malformed_commands_are_errors() {
        assert!(decode_line::<Request>(r#"{"id":1,"cmd":"no_such"}"#).is_err());
        assert!(
            decode_line::<Request>(r#"{"id":1,"cmd":"seek"}"#).is_err(),
            "필수 필드 누락"
        );
        assert!(
            decode_line::<Request>(r#"{"cmd":"play"}"#).is_err(),
            "id 누락"
        );
        assert!(
            decode_line::<Request>(r#"{"id":1,"cmd":"set_volume","volume":300,"mute":false}"#)
                .is_err(),
            "u8 범위 밖"
        );
        assert!(decode_line::<Request>(r#"{"id":1,"cmd":"seek","ms":"3"}"#).is_err());
    }

    #[test]
    fn t_player_proto_13_control_action_wire_names() {
        for (s, a) in [
            ("play", ControlAction::Play),
            ("stepForward", ControlAction::StepForward),
            ("stepBack", ControlAction::StepBack),
            ("toggle", ControlAction::Toggle),
            ("pause", ControlAction::Pause),
        ] {
            assert_eq!(
                serde_json::from_value::<ControlAction>(serde_json::json!(s)).unwrap(),
                a
            );
        }
        assert!(serde_json::from_value::<ControlAction>(serde_json::json!("fastForward")).is_err());
    }
}
