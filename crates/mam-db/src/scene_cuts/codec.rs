//! 컷 배열 BLOB 코덱과 순수 정규화 함수 (설계 §1 ADR-1, §4.2).
//!
//! BLOB = 버전 바이트 `0x01` + u32 LE ms 엄격 오름차순. 길이 = 1 + 4n. 빈 배열은 NULL 로 저장한다.

/// BLOB 형식 버전 (바이트 0)
pub const CODEC_VERSION: u8 = 0x01;
/// 한 BLOB 에 담을 수 있는 최대 컷 수 (열별 상한의 최댓값)
pub const CODEC_MAX_CUTS: usize = 20_000;
/// 자동 컷 상한 — 초과 시 점수 상위만 남긴다
pub const MAX_AUTO_CUTS: usize = 20_000;
/// 사용자 컷 상한 — 초과 추가는 거부한다
pub const MAX_USER_CUTS: usize = 5_000;
/// 숨김 목록 상한
pub const MAX_SUPPRESSED: usize = 20_000;
/// 시작·끝에서 이 시간 안의 컷은 버린다 (ms)
pub const EDGE_MARGIN_MS: i64 = 500;
/// 자동 컷 최소 간격 — 이보다 가까운 연속 검출은 점수 높은 하나만 남긴다 (ms)
pub const MIN_GAP_MS: i64 = 1000;

/// BLOB 디코드·인코드 실패 (값 자체는 싣지 않는다)
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("unknown cut blob version")]
    UnknownVersion,
    #[error("cut blob length is not 1 + 4n")]
    BadLength,
    #[error("cuts are not strictly ascending")]
    NotAscending,
    #[error("too many cuts")]
    TooMany,
    #[error("cut out of u32 range")]
    OutOfRange,
}

/// 컷 → BLOB. 비어 있으면 `None`(NULL). 엄격 오름차순·`0..=u32::MAX`·`CODEC_MAX_CUTS` 이하만 허용.
pub fn encode(cuts: &[i64]) -> Result<Option<Vec<u8>>, DecodeError> {
    if cuts.is_empty() {
        return Ok(None);
    }
    if cuts.len() > CODEC_MAX_CUTS {
        return Err(DecodeError::TooMany);
    }
    let mut out = Vec::with_capacity(1 + 4 * cuts.len());
    out.push(CODEC_VERSION);
    let mut prev: Option<i64> = None;
    for &ms in cuts {
        let v = u32::try_from(ms).map_err(|_| DecodeError::OutOfRange)?;
        if prev.is_some_and(|p| ms <= p) {
            return Err(DecodeError::NotAscending);
        }
        prev = Some(ms);
        out.extend_from_slice(&v.to_le_bytes());
    }
    Ok(Some(out))
}

/// BLOB → 컷. NULL(`None`) 은 빈 목록. 길이·버전·정렬·상한을 검증한다.
pub fn decode(blob: Option<&[u8]>) -> Result<Vec<i64>, DecodeError> {
    let Some(b) = blob else {
        return Ok(Vec::new());
    };
    let Some((&version, body)) = b.split_first() else {
        return Err(DecodeError::BadLength);
    };
    if version != CODEC_VERSION {
        return Err(DecodeError::UnknownVersion);
    }
    if body.len() % 4 != 0 {
        return Err(DecodeError::BadLength);
    }
    if body.len() / 4 > CODEC_MAX_CUTS {
        return Err(DecodeError::TooMany);
    }
    let mut out = Vec::with_capacity(body.len() / 4);
    for ch in body.chunks_exact(4) {
        let ms = i64::from(u32::from_le_bytes([ch[0], ch[1], ch[2], ch[3]]));
        if out.last().is_some_and(|&p| ms <= p) {
            return Err(DecodeError::NotAscending);
        }
        out.push(ms);
    }
    Ok(out)
}

fn valid_fps(fps: Option<f64>) -> Option<f64> {
    fps.filter(|f| f.is_finite() && *f > 0.0)
}

/// 같은 컷으로 볼 허용 오차(ms): fps 가 유효하면 `max(1, round(1000/fps))`, 아니면 100 (D6)
pub fn tolerance_ms(fps: Option<f64>) -> i64 {
    match valid_fps(fps) {
        Some(f) => ((1000.0 / f).round() as i64).max(1),
        None => 100,
    }
}

/// 이동 시 더하는 반 프레임(ms): fps 유효하면 `max(1, round(500/fps))`, 아니면 20 (D8)
pub fn half_frame_ms(fps: Option<f64>) -> i64 {
    match valid_fps(fps) {
        Some(f) => ((500.0 / f).round() as i64).max(1),
        None => 20,
    }
}

/// 원시 검출 `(ms, score)` → 저장할 자동 컷 (시간 오름차순).
///
/// ① `ms < 500` 및 `ms > duration - 500`(길이를 알 때) 버림, 비유한 점수·음수 시각 버림
/// ② 점수 내림차순(동점은 시각 오름차순)으로 훑으며 이미 채택한 컷과 1000ms 미만이면 버림
/// ③ 채택이 `MAX_AUTO_CUTS` 를 넘으면 점수 상위만 ④ 시각 오름차순 정렬
pub fn select_auto(raw: &[(i64, f32)], duration_ms: Option<i64>) -> Vec<i64> {
    let duration = duration_ms.filter(|d| *d > 0);
    let mut cand: Vec<(i64, f32)> = raw
        .iter()
        .copied()
        .filter(|&(ms, s)| {
            s.is_finite()
                && ms >= EDGE_MARGIN_MS
                && ms <= i64::from(u32::MAX)
                && duration.is_none_or(|d| ms <= d - EDGE_MARGIN_MS)
        })
        .collect();
    cand.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    // 채택 집합은 시각순 벡터로 유지해 이진 탐색으로 이웃만 검사한다 (O(n log n))
    let mut accepted: Vec<i64> = Vec::new();
    let mut order: Vec<i64> = Vec::new(); // 채택 순서(= 점수 순)
    for (ms, _) in cand {
        let pos = accepted.partition_point(|&x| x < ms);
        let near_prev = pos > 0 && ms - accepted[pos - 1] < MIN_GAP_MS;
        let near_next = pos < accepted.len() && accepted[pos] - ms < MIN_GAP_MS;
        if near_prev || near_next {
            continue;
        }
        accepted.insert(pos, ms);
        order.push(ms);
    }
    if order.len() > MAX_AUTO_CUTS {
        tracing::warn!(
            count = order.len(),
            "scene auto cuts over limit; keeping top scores"
        );
        order.truncate(MAX_AUTO_CUTS);
    }
    order.sort_unstable();
    order
}

/// `sorted` 에서 `ms` 와 `±tol` 안에 있는 가장 가까운 값의 인덱스 (동거리면 앞쪽)
pub(crate) fn nearest_within(sorted: &[i64], ms: i64, tol: i64) -> Option<usize> {
    let pos = sorted.partition_point(|&x| x < ms);
    let mut best: Option<(i64, usize)> = None;
    for i in [pos.checked_sub(1), (pos < sorted.len()).then_some(pos)]
        .into_iter()
        .flatten()
    {
        let d = (sorted[i] - ms).abs();
        if d <= tol && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// `sorted` 안에 `ms ± tol` 에 드는 값이 있는가
pub(crate) fn has_within(sorted: &[i64], ms: i64, tol: i64) -> bool {
    nearest_within(sorted, ms, tol).is_some()
}
