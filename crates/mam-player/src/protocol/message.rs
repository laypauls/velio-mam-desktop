//! worker → host 메시지 (`ev` 태그, kebab-case). 시각은 모두 **소스 타임라인** ms 정수다 (보정 끝난 값).

use serde::{Deserialize, Serialize};

/// 워커가 거절·실패를 알릴 때의 분류 (호스트가 `open_failed` 등으로 바꾼다)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerErrorCode {
    OpenFailed,
    InvalidArgument,
    NoMedia,
    Unsupported,
    Internal,
}

/// 워커가 고른 렌더 경로 — R2 = D3D11 렌더 + d3d11va, R1 = SW 렌더
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RenderPath {
    R1,
    R2,
}

/// `probe` 응답 — 시험·진단용 mpv 상태
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProbeData {
    /// mpv `time-pos` (초, 보정 전)
    pub time_pos: Option<f64>,
    pub paused: Option<bool>,
    pub hwdec: Option<String>,
    pub speed: Option<f64>,
    /// mpv `volume` (곡선 적용 후 0..100)
    pub volume: Option<f64>,
    pub mute: Option<bool>,
    pub audio_delay: Option<f64>,
    pub video_format: Option<String>,
    /// 뒤로 이동 캐시가 보관 중인 프레임 수 (켜지지 않았으면 0)
    pub back_cache_len: u64,
    /// EDL(구간) 로 열려 있는가
    pub edl: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "kebab-case")]
pub enum Message {
    /// 파이프 연결 + libmpv 초기화 직후 1회
    Ready {
        mpv_api: u64,
        protocol: u32,
        render_path: RenderPath,
    },
    /// 명령 응답 (`id != 0` 인 명령만)
    Reply {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<WorkerErrorCode>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<ProbeData>,
    },
    /// 파일 로드 완료 — `duration_ms` 는 **소스 전체 길이** (EDL 이어도 합계가 아니다)
    FileLoaded {
        w: Option<i64>,
        h: Option<i64>,
        fps: Option<f64>,
        duration_ms: Option<i64>,
        video_format: Option<String>,
    },
    /// seek/열기/프레임 이동 후 재생 재시작 (mpv PLAYBACK_RESTART) — 화면에 그려진 프레임의 보정된 소스 시각
    PlaybackRestart { time_ms: i64 },
    /// 일시정지 상태 변화
    Paused { paused: bool, time_ms: i64 },
    /// 끝(EOF, 마지막 구간 끝) 도달
    Ended { time_ms: i64 },
    /// 위치 갱신 (재생 중 최대 10Hz)
    Time { ms: i64 },
    /// 네트워크/느린 저장소로 재생이 멈췄다(true)/재개됐다(false)
    Buffering { on: bool },
    /// 읽기·디코드 오류 등 (워커 stderr 문구가 아니라 분류된 문구만 싣는다)
    Error { code: WorkerErrorCode, msg: String },
    /// 슬롯에 프레임 렌더 완료 — JS 가 release 할 때까지 워커가 그 슬롯을 쓰지 않는다
    FrameReady {
        epoch: u64,
        slot: u32,
        seq: u64,
        w: u32,
        h: u32,
        stride: u32,
        #[serde(rename = "renderDoneMs")]
        render_done_ms: f64,
    },
    /// 하트비트 + 슬롯 통계 (약 0.5초마다, 세션이 없어도)
    Stats {
        rendered: u64,
        skipped: u64,
        ignored: u64,
        cache_hits: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::super::wire::{decode_line, encode_line};
    use super::*;

    // 분류: Happy / Validation(알 수 없는 태그) / Boundary(선택 필드 생략)

    #[test]
    fn t_player_proto_20_message_round_trip() {
        let msgs = [
            Message::Ready {
                mpv_api: 0x2_0000,
                protocol: 1,
                render_path: RenderPath::R2,
            },
            Message::Reply {
                id: 9,
                ok: true,
                code: None,
                error: None,
                data: Some(ProbeData {
                    time_pos: Some(2.5),
                    paused: Some(true),
                    edl: true,
                    ..ProbeData::default()
                }),
            },
            Message::Reply {
                id: 10,
                ok: false,
                code: Some(WorkerErrorCode::OpenFailed),
                error: Some("cannot open".into()),
                data: None,
            },
            Message::FileLoaded {
                w: Some(1920),
                h: Some(1080),
                fps: Some(29.97),
                duration_ms: Some(30_000),
                video_format: Some("h264".into()),
            },
            Message::PlaybackRestart { time_ms: 1500 },
            Message::Paused {
                paused: true,
                time_ms: 0,
            },
            Message::Ended { time_ms: 29_967 },
            Message::Time { ms: 12 },
            Message::Buffering { on: true },
            Message::Error {
                code: WorkerErrorCode::Internal,
                msg: "x".into(),
            },
            Message::FrameReady {
                epoch: 2,
                slot: 0,
                seq: 5,
                w: 1280,
                h: 720,
                stride: 5120,
                render_done_ms: 1.7e12,
            },
            Message::Stats {
                rendered: 10,
                skipped: 2,
                ignored: 0,
                cache_hits: 3,
            },
        ];
        for m in msgs {
            assert_eq!(
                decode_line::<Message>(&encode_line(&m).unwrap()).unwrap(),
                m
            );
        }
    }

    #[test]
    fn t_player_proto_21_wire_names_are_stable() {
        let line = encode_line(&Message::FrameReady {
            epoch: 2,
            slot: 0,
            seq: 5,
            w: 4,
            h: 4,
            stride: 64,
            render_done_ms: 1.0,
        })
        .unwrap();
        assert!(
            line.contains(r#""ev":"frame-ready""#) && line.contains(r#""renderDoneMs""#),
            "{line}"
        );
        let line = encode_line(&Message::PlaybackRestart { time_ms: 3 }).unwrap();
        assert_eq!(line.trim(), r#"{"ev":"playback-restart","time_ms":3}"#);
        let line = encode_line(&Message::Ready {
            mpv_api: 1,
            protocol: 1,
            render_path: RenderPath::R1,
        })
        .unwrap();
        assert!(line.contains(r#""render_path":"r1""#), "{line}");
    }

    #[test]
    fn t_player_proto_22_unknown_or_truncated_messages_are_errors() {
        assert!(decode_line::<Message>(r#"{"ev":"mystery"}"#).is_err());
        assert!(
            decode_line::<Message>(r#"{"ev":"playback-restart"}"#).is_err(),
            "필수 필드 누락"
        );
        assert!(
            decode_line::<Message>(r#"{"ev":"reply","id":1,"ok":true}"#).is_ok(),
            "선택 필드는 생략 가능"
        );
    }
}
