//! 프레임 슬롯 소유권 상태 기계 — 워커가 권위를 가진다 (순수 로직, libmpv·WebView 불필요).
//!
//! 슬롯 = SharedBuffer 1개. `FREE → WRITING → READY(seq) → (JS 소유) → release → FREE`.
//! JS 의 ArrayBuffer 는 SharedArrayBuffer 가 아니라 Atomics 를 쓸 수 없으므로 소유권은 메시지로만 넘어간다:
//! 워커가 `publish` 하면 JS 소유, JS 가 `release` 를 보내야만 워커가 다시 쓸 수 있다.
//! 워커는 JS 를 기다리지 않는다 — FREE 가 없으면 `acquire` 가 None 이고 호출자는 그 프레임을 건너뛴다(`skipped`).
//! 버퍼를 다시 만들면(해상도 변경·재시작) 새 epoch 의 새 테이블이 되고, 이전 epoch 의 release 는 무시한다(`ignored`).
//! 다시 쓸 수 있는 유일한 경로는 `release` 다 — 앞선 READY 를 워커가 회수하면 JS 읽기와 경합한다.

/// 슬롯 하나의 상태
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    Free,
    Writing,
    /// 게시했다 (JS 소유). 값 = 프레임 seq
    Ready(u64),
}

/// `release` 처리 결과
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    /// 이전 epoch(교체 전 버퍼)의 메시지 — 무시
    StaleEpoch,
    /// 슬롯 번호·상태·seq 가 맞지 않음 — 무시
    Mismatch,
}

#[derive(Debug)]
pub struct SlotTable {
    epoch: u64,
    slots: Vec<SlotState>,
    next_seq: u64,
    /// 다음 acquire 탐색 시작 위치 (한 슬롯만 계속 쓰지 않도록 순환)
    cursor: usize,
    skipped: u64,
    ignored: u64,
}

impl SlotTable {
    pub fn new(epoch: u64, count: usize) -> Self {
        Self { epoch, slots: vec![SlotState::Free; count], next_seq: 0, cursor: 0, skipped: 0, ignored: 0 }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn state(&self, slot: usize) -> Option<SlotState> {
        self.slots.get(slot).copied()
    }

    /// 건너뛴 프레임 수 (FREE 슬롯이 없었던 횟수)
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// 무시된(오래된 epoch·불일치) release 수
    pub fn ignored(&self) -> u64 {
        self.ignored
    }

    /// FREE 슬롯을 WRITING 으로 잡는다. 없으면 None(대기하지 않음)이고 건너뜀으로 센다
    pub fn acquire(&mut self) -> Option<usize> {
        let n = self.slots.len();
        for i in 0..n {
            let s = (self.cursor + i) % n;
            if self.slots[s] == SlotState::Free {
                self.slots[s] = SlotState::Writing;
                self.cursor = (s + 1) % n;
                return Some(s);
            }
        }
        self.skipped += 1;
        None
    }

    /// 렌더 완료: WRITING → READY. 새 seq(epoch 안에서 단조 증가, 1부터)를 돌려준다
    pub fn publish(&mut self, slot: usize) -> Option<u64> {
        match self.slots.get(slot) {
            Some(SlotState::Writing) => {
                self.next_seq += 1;
                self.slots[slot] = SlotState::Ready(self.next_seq);
                Some(self.next_seq)
            }
            _ => None,
        }
    }

    /// 렌더 실패: WRITING → FREE (seq 를 소비하지 않는다)
    pub fn abort(&mut self, slot: usize) {
        if self.slots.get(slot) == Some(&SlotState::Writing) {
            self.slots[slot] = SlotState::Free;
        }
    }

    /// JS 가 슬롯을 돌려줌: READY(seq) → FREE
    pub fn release(&mut self, epoch: u64, slot: usize, seq: u64) -> Outcome {
        if epoch != self.epoch {
            self.ignored += 1;
            return Outcome::StaleEpoch;
        }
        match self.slots.get(slot) {
            Some(SlotState::Ready(s)) if *s == seq => {
                self.slots[slot] = SlotState::Free;
                Outcome::Applied
            }
            _ => {
                self.ignored += 1;
                Outcome::Mismatch
            }
        }
    }
}

/// (슬롯, seq)
pub type SlotSeq = (usize, u64);
/// 그릴 프레임과 그리지 않고 돌려줄 프레임 목록
pub type DisplayPlan = (Option<SlotSeq>, Vec<SlotSeq>);

/// JS 표시 정책의 기준 구현 (`frameQueue.ts` 가 같은 규칙을 따른다 — 시험에서 쌍둥이 비교): 가장 새 READY 만 그리고 나머지는 버린다
pub fn plan_display(ready: &[SlotSeq]) -> DisplayPlan {
    let newest = ready.iter().copied().max_by_key(|&(_, seq)| seq);
    let dropped = ready.iter().copied().filter(|&r| Some(r) != newest).collect();
    (newest, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Boundary(전부 사용 중) / Business rule(supersede) / Concurrency(seq 단조·stale release)

    #[test]
    fn t_player_slot_01_normal_cycle() {
        let mut t = SlotTable::new(1, 3);
        let s = t.acquire().unwrap();
        assert_eq!(t.state(s), Some(SlotState::Writing));
        let seq = t.publish(s).unwrap();
        assert_eq!(t.state(s), Some(SlotState::Ready(seq)));
        assert_eq!(t.release(1, s, seq), Outcome::Applied);
        assert_eq!(t.state(s), Some(SlotState::Free));
    }

    #[test]
    fn t_player_slot_02_newer_ready_supersedes_undisplayed_older() {
        let mut t = SlotTable::new(1, 3);
        let (a, b) = (t.acquire().unwrap(), t.acquire().unwrap());
        let (sa, sb) = (t.publish(a).unwrap(), t.publish(b).unwrap());
        let (show, drop) = plan_display(&[(a, sa), (b, sb)]);
        assert_eq!(show, Some((b, sb)));
        assert_eq!(drop, vec![(a, sa)]);
        // 버려진 a 는 JS 가 release 하기 전에는 워커가 다시 쓸 수 없다
        assert_eq!(t.state(a), Some(SlotState::Ready(sa)));
        assert_eq!(t.release(1, a, sa), Outcome::Applied);
        assert_eq!(t.state(a), Some(SlotState::Free));
    }

    #[test]
    fn t_player_slot_03_all_busy_never_blocks_and_counts_skips() {
        let mut t = SlotTable::new(1, 3);
        for _ in 0..3 {
            let s = t.acquire().unwrap();
            t.publish(s).unwrap();
        }
        assert_eq!(t.acquire(), None);
        assert_eq!(t.acquire(), None);
        assert_eq!(t.skipped(), 2);
        assert_eq!(t.release(1, 0, 1), Outcome::Applied);
        assert!(t.acquire().is_some(), "하나가 돌아오면 다시 렌더할 수 있다");
    }

    #[test]
    fn t_player_slot_04_seq_is_monotonic_and_abort_does_not_consume_a_seq() {
        let mut t = SlotTable::new(1, 2);
        let mut last = 0;
        for i in 0..20 {
            let s = t.acquire().unwrap();
            if i % 5 == 4 {
                t.abort(s);
                continue;
            }
            let seq = t.publish(s).unwrap();
            assert_eq!(seq, last + 1, "seq 는 빠짐없이 1씩 증가");
            last = seq;
            assert_eq!(t.release(1, s, seq), Outcome::Applied);
        }
    }

    #[test]
    fn t_player_slot_05_stale_epoch_release_is_ignored() {
        let mut old = SlotTable::new(1, 3);
        let s = old.acquire().unwrap();
        let seq = old.publish(s).unwrap();
        let mut new = SlotTable::new(2, 3);
        let s2 = new.acquire().unwrap();
        let seq2 = new.publish(s2).unwrap();
        assert_eq!(new.release(1, s, seq), Outcome::StaleEpoch);
        assert_eq!(new.state(s2), Some(SlotState::Ready(seq2)), "새 epoch 슬롯은 그대로");
        assert_eq!(new.ignored(), 1);
    }

    #[test]
    fn t_player_slot_06_invalid_transitions_are_rejected() {
        let mut t = SlotTable::new(1, 3);
        assert_eq!(t.publish(0), None, "WRITING 이 아닌 슬롯은 publish 불가");
        assert_eq!(t.publish(99), None);
        let s = t.acquire().unwrap();
        let seq = t.publish(s).unwrap();
        assert_eq!(t.release(1, s, seq + 1), Outcome::Mismatch, "seq 불일치");
        assert_eq!(t.release(1, 99, seq), Outcome::Mismatch, "없는 슬롯");
        assert_eq!(t.release(1, s, seq), Outcome::Applied);
        assert_eq!(t.release(1, s, seq), Outcome::Mismatch, "중복 release");
    }

    #[test]
    fn t_player_slot_07_acquire_rotates_over_slots() {
        let mut t = SlotTable::new(1, 3);
        let mut seen = vec![];
        for _ in 0..6 {
            let s = t.acquire().unwrap();
            seen.push(s);
            let q = t.publish(s).unwrap();
            t.release(1, s, q);
        }
        assert_eq!(seen, vec![0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn t_player_slot_08_plan_display_empty_single_and_many() {
        assert_eq!(plan_display(&[]), (None, vec![]));
        assert_eq!(plan_display(&[(2, 7)]), (Some((2, 7)), vec![]));
        let (show, drop) = plan_display(&[(0, 5), (1, 9), (2, 7)]);
        assert_eq!(show, Some((1, 9)));
        assert_eq!(drop.len(), 2);
    }
}
