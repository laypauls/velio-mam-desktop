//! 수집기의 보조 상태 — 깨우기 제한(`KickGate`)과 증거로 남기는 집계(`Counts`).

use std::collections::HashMap;

/// 폴더마다 "마지막으로 재스캔을 요청한 시각" 을 기억해 너무 잦은 요청을 막는다 (watch_folder `kickGate`, 5초).
///
/// 이것이 없으면 기능을 켠 것이 오히려 손해가 된다 — 100GB 를 복사하는 동안 줄이 끊임없이 들어오는데 줄마다 깨우면 스캔이 쉴 틈 없이 돈다.
/// 시각은 호출자가 넘긴다(시계 주입).
#[derive(Debug)]
pub struct KickGate {
    every_ms: i64,
    last: HashMap<i64, i64>,
}

impl KickGate {
    pub fn new(every_ms: i64) -> Self {
        Self { every_ms, last: HashMap::new() }
    }

    /// 지금 요청해도 되면 true 로 돌려주고 시각을 기록한다.
    pub fn allow(&mut self, folder_id: i64, now_ms: i64) -> bool {
        match self.last.get(&folder_id) {
            Some(t) if now_ms.saturating_sub(*t) < self.every_ms => false,
            _ => {
                self.last.insert(folder_id, now_ms);
                true
            }
        }
    }
}

/// 증거로 남길 집계 — 구간(10분)과 전체를 같은 모양으로 센다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// 읽어 본 횟수
    pub polls: u64,
    /// 읽은 줄
    pub lines: u64,
    /// 경로 변환에 성공한 바뀐 경로
    pub changes: u64,
    /// 그중 어느 폴더에 보고한 것
    pub reported: u64,
    /// 그중 어느 폴더 루트에도 들지 않던 것
    pub outside: u64,
    /// 변환 규칙이 없거나 위험해 버린 경로
    pub unmapped: u64,
    /// 형식이 맞지 않던 줄
    pub bad: u64,
    /// 읽기·접속 실패 횟수
    pub fails: u64,
    /// 폭주 방지로 건너뛴 바이트
    pub skipped_bytes: u64,
    /// 밀린 행이 상한을 넘어 읽지 않고 건너뛴 행 수 (`ses_queue`)
    pub skipped_rows: u64,
}

impl Counts {
    pub fn add(&mut self, o: &Counts) {
        self.polls += o.polls;
        self.lines += o.lines;
        self.changes += o.changes;
        self.reported += o.reported;
        self.outside += o.outside;
        self.unmapped += o.unmapped;
        self.bad += o.bad;
        self.fails += o.fails;
        self.skipped_bytes += o.skipped_bytes;
        self.skipped_rows += o.skipped_rows;
    }
}
