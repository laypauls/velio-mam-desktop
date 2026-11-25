//! 크기 상한이 있는 줄 읽기 — 워커가 개행 없이 끝없이 쓰거나 거대한 줄을 보내도 호스트 메모리가 `max + 버퍼` 를 넘지 않는다.

use std::io::Read;
use std::sync::mpsc::SyncSender;

#[derive(Debug, PartialEq, Eq)]
pub enum LineEvent {
    /// 개행·`\r` 을 뺀 한 줄
    Line(Vec<u8>),
    /// 한 줄이 `max` 바이트를 넘었다 — 이후 읽지 않는다
    Oversize,
    /// 스트림 끝(워커 종료) 또는 읽기 오류
    Eof,
}

const CHUNK: usize = 16 * 1024;

/// `r` 에서 줄을 읽어 `tx` 로 보낸다. 수신자가 사라지거나 Oversize/Eof 를 보내면 끝난다.
/// 채널이 가득 차면 `send` 가 막혀 워커의 쓰기가 파이프 버퍼에서 멈춘다(배압) — 호출 타임아웃이 종료시킨다.
pub fn pump_lines<R: Read>(mut r: R, max: usize, tx: &SyncSender<LineEvent>) {
    let mut line: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) | Err(_) => {
                let _ = tx.send(LineEvent::Eof);
                return;
            }
            Ok(n) => n,
        };
        for piece in buf[..n].split_inclusive(|b| *b == b'\n') {
            let complete = piece.last() == Some(&b'\n');
            let body = if complete { &piece[..piece.len() - 1] } else { piece };
            if line.len() + body.len() > max {
                let _ = tx.send(LineEvent::Oversize);
                return;
            }
            line.extend_from_slice(body);
            if complete {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if tx.send(LineEvent::Line(std::mem::take(&mut line))).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::sync_channel;

    // 분류: Happy / Boundary(상한 정확히·+1, 개행 없는 끝) / External(끝없는 출력)

    fn run(data: &[u8], max: usize) -> Vec<LineEvent> {
        let (tx, rx) = sync_channel(64);
        pump_lines(data, max, &tx);
        drop(tx);
        rx.iter().collect()
    }

    #[test]
    fn t_phost_line_01_splits_lines_and_strips_crlf() {
        let ev = run(b"a\r\nbb\n\nccc\n", 16);
        assert_eq!(ev, vec![LineEvent::Line(b"a".to_vec()), LineEvent::Line(b"bb".to_vec()), LineEvent::Line(vec![]), LineEvent::Line(b"ccc".to_vec()), LineEvent::Eof]);
    }

    #[test]
    fn t_phost_line_02_boundary_exact_max_ok_plus_one_is_oversize() {
        assert_eq!(run(b"12345\n", 5)[0], LineEvent::Line(b"12345".to_vec()));
        assert_eq!(run(b"123456\n", 5), vec![LineEvent::Oversize]);
    }

    #[test]
    fn t_phost_line_03_endless_line_without_newline_is_oversize_and_bounded() {
        // Arrange: 개행 없는 1 MiB, 상한 4 KiB
        let data = vec![b'x'; 1 << 20];
        // Act / Assert
        assert_eq!(run(&data, 4096), vec![LineEvent::Oversize]);
    }

    #[test]
    fn t_phost_line_04_partial_last_line_without_newline_is_dropped_at_eof() {
        assert_eq!(run(b"ok\npartial", 64), vec![LineEvent::Line(b"ok".to_vec()), LineEvent::Eof]);
    }
}
