//! 줄 단위 JSON 직렬화 — 빈 줄·깨진 JSON·과도하게 긴 줄은 오류로 돌려주고 패닉하지 않는다.

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Read};

/// 줄 하나의 최대 바이트 (Attach 핸들 8개·구간 1000개도 이 안에 들어온다)
pub const MAX_LINE_BYTES: usize = 256 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WireError {
    #[error("empty line")]
    Empty,
    #[error("line too long")]
    TooLong,
    #[error("invalid json: {0}")]
    Json(String),
}

/// 한 줄 JSON 으로 직렬화 (끝에 `\n` 포함)
pub fn encode_line<T: Serialize>(v: &T) -> Result<String, WireError> {
    let mut s = serde_json::to_string(v).map_err(|e| WireError::Json(e.to_string()))?;
    s.push('\n');
    Ok(s)
}

/// 한 줄을 역직렬화한다 (`\r\n` 허용)
pub fn decode_line<'a, T: Deserialize<'a>>(line: &'a str) -> Result<T, WireError> {
    let t = line.trim();
    if t.is_empty() {
        return Err(WireError::Empty);
    }
    if t.len() > MAX_LINE_BYTES {
        return Err(WireError::TooLong);
    }
    serde_json::from_str(t).map_err(|e| WireError::Json(e.to_string()))
}

/// 줄 하나를 `max` 바이트까지만 읽는다. 반환 0 = EOF. 개행 없이 `max` 를 넘기면 `InvalidData` (무한히 쌓지 않는다)
pub fn read_bounded_line<R: BufRead>(r: &mut R, buf: &mut String, max: usize) -> io::Result<usize> {
    let mut bytes = Vec::new();
    let n = r.by_ref().take(max as u64 + 1).read_until(b'\n', &mut bytes)?;
    if n > max && bytes.last() != Some(&b'\n') {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
    }
    buf.push_str(&String::from_utf8_lossy(&bytes));
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // 분류: Happy / Boundary(빈 줄·긴 줄) / Validation(깨진 JSON)

    #[test]
    fn t_player_proto_01_round_trip_is_one_line() {
        let line = encode_line(&serde_json::json!({"a": 1})).unwrap();
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
        let back: serde_json::Value = decode_line(&line).unwrap();
        assert_eq!(back["a"], 1);
    }

    #[test]
    fn t_player_proto_02_malformed_lines_are_errors_not_panics() {
        assert_eq!(decode_line::<serde_json::Value>(""), Err(WireError::Empty));
        assert_eq!(decode_line::<serde_json::Value>("  \r\n"), Err(WireError::Empty));
        assert!(matches!(decode_line::<serde_json::Value>("{nope"), Err(WireError::Json(_))));
        let long = format!("\"{}\"", "x".repeat(MAX_LINE_BYTES + 1));
        assert_eq!(decode_line::<serde_json::Value>(&long), Err(WireError::TooLong));
        let ok: serde_json::Value = decode_line("{\"a\":2}\r\n").unwrap();
        assert_eq!(ok["a"], 2);
    }

    #[test]
    fn t_player_proto_03_bounded_reader_caps_runaway_lines() {
        // Arrange: 개행 없는 긴 입력 / 정상 두 줄
        let mut huge = Cursor::new(vec![b'a'; 100]);
        let mut buf = String::new();
        // Act / Assert
        assert_eq!(read_bounded_line(&mut huge, &mut buf, 10).unwrap_err().kind(), io::ErrorKind::InvalidData);
        let mut two = Cursor::new(b"one\ntwo\n".to_vec());
        buf.clear();
        assert_eq!(read_bounded_line(&mut two, &mut buf, 10).unwrap(), 4);
        assert_eq!(buf, "one\n");
        buf.clear();
        read_bounded_line(&mut two, &mut buf, 10).unwrap();
        assert_eq!(buf, "two\n");
        buf.clear();
        assert_eq!(read_bounded_line(&mut two, &mut buf, 10).unwrap(), 0, "EOF");
        // 정확히 max 바이트(개행 포함)는 허용
        let mut edge = Cursor::new(b"0123456789\n".to_vec());
        buf.clear();
        assert!(read_bounded_line(&mut edge, &mut buf, 11).is_ok());
    }
}
