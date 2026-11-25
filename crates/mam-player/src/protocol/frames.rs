//! WebView ↔ 호스트 프레임 메시지 (`"mam": "player.v1"`) — 계약 §3.
//! 호스트→JS: `BufferInfo`(SharedBuffer 의 additionalData), `FrameMsgOut`. JS→호스트: `ReleaseMsgIn` (Tauri command `player_release_slots` 의 인자).
//! `validate_release_value` 는 command 인자(JSON 값)를, `parse_release` 는 JSON 문자열 본문을 검증하는 순수 함수다.

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use super::frame_math::stride_for;

/// 모든 메시지의 `mam` 값
pub const PLAYER_WIRE_TAG: &str = "player.v1";
/// 버퍼 3개 고정
pub const SLOT_COUNT: u32 = 3;
/// 한 번의 release 메시지가 담을 수 있는 슬롯 수
pub const MAX_RELEASE_SLOTS: usize = 8;
/// release 메시지 본문 최대 바이트
pub const MAX_RELEASE_BODY: usize = 1024;

/// `"mam": "player.v1"` — 직렬화는 항상 이 값, 역직렬화는 다른 값을 거부한다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WireTag;

impl Serialize for WireTag {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(PLAYER_WIRE_TAG)
    }
}

impl<'de> Deserialize<'de> for WireTag {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = String::deserialize(d)?;
        if v == PLAYER_WIRE_TAG {
            Ok(WireTag)
        } else {
            Err(de::Error::custom("unknown wire tag"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PixelFormat {
    Bgr0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum FrameKind {
    #[serde(rename = "frame")]
    Frame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ReleaseKind {
    #[serde(rename = "release")]
    Release,
}

/// SharedBuffer 한 개의 `additionalDataAsJson` (버퍼 3개를 같은 epoch 로 한 번씩 게시한다)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferInfo {
    pub mam: WireTag,
    pub epoch: u64,
    pub slot: u32,
    pub count: u32,
    pub size: u64,
    pub w: u32,
    pub h: u32,
    /// 64 의 배수, `>= w * 4`
    pub stride: u32,
    pub fmt: PixelFormat,
}

impl BufferInfo {
    /// 해상도에서 stride·size 를 계산한다
    pub fn new(epoch: u64, slot: u32, w: u32, h: u32) -> Self {
        let stride = stride_for(w);
        Self {
            mam: WireTag,
            epoch,
            slot,
            count: SLOT_COUNT,
            size: stride as u64 * h as u64,
            w,
            h,
            stride,
            fmt: PixelFormat::Bgr0,
        }
    }
}

/// 호스트 → JS 프레임 알림 (`PostWebMessageAsJson`)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FrameMsgOut {
    pub mam: WireTag,
    k: FrameKind,
    pub epoch: u64,
    pub slot: u32,
    pub seq: u64,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    #[serde(rename = "renderDoneMs")]
    pub render_done_ms: f64,
}

impl FrameMsgOut {
    pub fn new(
        epoch: u64,
        slot: u32,
        seq: u64,
        w: u32,
        h: u32,
        stride: u32,
        render_done_ms: f64,
    ) -> Self {
        Self {
            mam: WireTag,
            k: FrameKind::Frame,
            epoch,
            slot,
            seq,
            w,
            h,
            stride,
            render_done_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotRef {
    pub slot: u32,
    pub seq: u64,
}

/// JS → 호스트 슬롯 반환
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseMsgIn {
    pub mam: WireTag,
    k: ReleaseKind,
    pub epoch: u64,
    pub slots: Vec<SlotRef>,
}

impl ReleaseMsgIn {
    pub fn new(epoch: u64, slots: Vec<SlotRef>) -> Self {
        Self {
            mam: WireTag,
            k: ReleaseKind::Release,
            epoch,
            slots,
        }
    }
}

/// 거부 사유 — 호출자는 조용히 버리고 개수만 센다 (로그에 본문을 남기지 않는다)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReject {
    /// 본문이 `MAX_RELEASE_BODY` 를 넘는다
    TooLarge,
    /// JSON 객체가 아니다 (문자열 메시지·배열·숫자 등 — Tauri IPC 문자열 경로와 섞이지 않게 거부)
    NotObject,
    /// `mam`/`k` 불일치, 필드 누락·알 수 없는 필드, 타입 오류
    Malformed,
    /// 슬롯 수가 1..=8 이 아니거나 슬롯 번호가 범위 밖
    BadSlots,
}

/// JSON 본문(문자열)을 검증해 release 메시지로 돌려준다. epoch 검사는 호출자 몫이다
pub fn parse_release(body: &str) -> Result<ReleaseMsgIn, ReleaseReject> {
    if body.len() > MAX_RELEASE_BODY {
        return Err(ReleaseReject::TooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| ReleaseReject::Malformed)?;
    validate_release_value(value)
}

/// 이미 파싱된 JSON 값(Tauri command 인자)을 검증한다 — 객체 여부·`mam`/`k`·알 수 없는 필드·슬롯 수·슬롯 번호. epoch 검사는 호출자 몫이다
pub fn validate_release_value(value: serde_json::Value) -> Result<ReleaseMsgIn, ReleaseReject> {
    if !value.is_object() {
        return Err(ReleaseReject::NotObject);
    }
    let msg: ReleaseMsgIn = serde_json::from_value(value).map_err(|_| ReleaseReject::Malformed)?;
    if msg.slots.is_empty()
        || msg.slots.len() > MAX_RELEASE_SLOTS
        || msg.slots.iter().any(|s| s.slot >= SLOT_COUNT)
    {
        return Err(ReleaseReject::BadSlots);
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    // 분류: Happy(golden) / Validation(거부 목록) / Boundary(크기·슬롯 수)

    fn fixture() -> Value {
        let raw =
            include_str!("../../../../src/test/fixtures/player-wire.json").replace("\r\n", "\n");
        serde_json::from_str(&raw).expect("fixture json")
    }

    #[test]
    fn t_player_wire_01_golden_shapes_match_the_shared_fixture() {
        // Arrange
        let fx = fixture();
        // Act / Assert: serde 가 만드는 모양 = 프론트(vitest)와 공유하는 fixture
        let info = BufferInfo {
            mam: WireTag,
            epoch: 3,
            slot: 1,
            count: 3,
            size: 3_686_400,
            w: 1280,
            h: 720,
            stride: 5120,
            fmt: PixelFormat::Bgr0,
        };
        assert_eq!(serde_json::to_value(info).unwrap(), fx["bufferInfo"]);
        assert_eq!(
            BufferInfo::new(3, 1, 1280, 720),
            info,
            "stride/size 계산이 fixture 와 같다"
        );
        let frame = FrameMsgOut::new(3, 1, 42, 1280, 720, 5120, 1_763_550_000_123.5);
        assert_eq!(serde_json::to_value(frame).unwrap(), fx["frameMsg"]);
        let release = ReleaseMsgIn::new(
            3,
            vec![SlotRef { slot: 0, seq: 41 }, SlotRef { slot: 1, seq: 42 }],
        );
        assert_eq!(serde_json::to_value(&release).unwrap(), fx["releaseMsg"]);
        // 반대 방향: fixture 가 그대로 역직렬화된다
        assert_eq!(
            serde_json::from_value::<BufferInfo>(fx["bufferInfo"].clone()).unwrap(),
            info
        );
        assert_eq!(
            serde_json::from_value::<FrameMsgOut>(fx["frameMsg"].clone()).unwrap(),
            frame
        );
        assert_eq!(
            parse_release(&fx["releaseMsg"].to_string()).unwrap(),
            release
        );
    }

    #[test]
    fn t_player_wire_02_invalid_release_messages_from_the_fixture_are_rejected() {
        let fx = fixture();
        let list = fx["invalidReleaseMessages"].as_array().expect("list");
        assert!(list.len() >= 5);
        for (i, v) in list.iter().enumerate() {
            // WebView2 는 JS 가 보낸 값을 JSON 으로 돌려준다 — 문자열 메시지는 JSON 문자열이 된다
            let body = serde_json::to_string(v).unwrap();
            assert!(
                parse_release(&body).is_err(),
                "#{i} must be rejected: {body}"
            );
        }
        // 이유별 확인
        assert_eq!(
            parse_release(&serde_json::to_string(&list[0]).unwrap()),
            Err(ReleaseReject::NotObject),
            "문자열 메시지"
        );
        assert_eq!(
            parse_release(&serde_json::to_string(&list[1]).unwrap()),
            Err(ReleaseReject::Malformed),
            "mam 불일치"
        );
        assert_eq!(
            parse_release(&serde_json::to_string(&list[2]).unwrap()),
            Err(ReleaseReject::Malformed),
            "알 수 없는 k"
        );
        assert_eq!(
            parse_release(&serde_json::to_string(&list[3]).unwrap()),
            Err(ReleaseReject::BadSlots),
            "slot 범위 밖"
        );
        assert_eq!(
            parse_release(&serde_json::to_string(&list[4]).unwrap()),
            Err(ReleaseReject::BadSlots),
            "9 슬롯"
        );
    }

    #[test]
    fn t_player_wire_03_oversize_unknown_keys_and_non_objects_are_rejected() {
        // 크기
        let big = format!(
            r#"{{"mam":"player.v1","k":"release","epoch":1,"slots":[{{"slot":0,"seq":1}}],"pad":"{}"}}"#,
            "x".repeat(MAX_RELEASE_BODY)
        );
        assert_eq!(parse_release(&big), Err(ReleaseReject::TooLarge));
        // 알 수 없는 키 (최상위·슬롯)
        assert_eq!(
            parse_release(
                r#"{"mam":"player.v1","k":"release","epoch":1,"slots":[{"slot":0,"seq":1}],"extra":1}"#
            ),
            Err(ReleaseReject::Malformed)
        );
        assert_eq!(
            parse_release(
                r#"{"mam":"player.v1","k":"release","epoch":1,"slots":[{"slot":0,"seq":1,"x":2}]}"#
            ),
            Err(ReleaseReject::Malformed)
        );
        // 객체가 아닌 값
        for v in ["[]", "null", "42", "true", r#""release""#] {
            assert_eq!(parse_release(v), Err(ReleaseReject::NotObject), "{v}");
        }
        // 깨진 JSON·음수 epoch·소수 seq
        for v in [
            "",
            "{",
            r#"{"mam":"player.v1","k":"release","epoch":-1,"slots":[{"slot":0,"seq":1}]}"#,
            r#"{"mam":"player.v1","k":"release","epoch":1,"slots":[{"slot":0,"seq":1.5}]}"#,
        ] {
            assert_eq!(parse_release(v), Err(ReleaseReject::Malformed), "{v}");
        }
    }

    #[test]
    fn t_player_wire_04_slot_count_boundaries() {
        let mk = |n: usize| {
            let slots: Vec<Value> = (0..n)
                .map(|i| serde_json::json!({"slot": i as u32 % 3, "seq": i + 1}))
                .collect();
            serde_json::json!({"mam":"player.v1","k":"release","epoch":1,"slots":slots}).to_string()
        };
        assert_eq!(parse_release(&mk(0)), Err(ReleaseReject::BadSlots));
        assert_eq!(parse_release(&mk(1)).unwrap().slots.len(), 1);
        assert_eq!(parse_release(&mk(8)).unwrap().slots.len(), 8);
        assert_eq!(parse_release(&mk(9)), Err(ReleaseReject::BadSlots));
    }

    #[test]
    fn t_player_wire_13_validate_release_value_matches_the_string_parser() {
        let fx = fixture();
        let ok = validate_release_value(fx["releaseMsg"].clone()).unwrap();
        assert_eq!(ok, parse_release(&fx["releaseMsg"].to_string()).unwrap());
        for v in fx["invalidReleaseMessages"].as_array().unwrap() {
            assert!(validate_release_value(v.clone()).is_err(), "{v}");
        }
        assert_eq!(
            validate_release_value(Value::String("x".into())),
            Err(ReleaseReject::NotObject)
        );
    }
}
