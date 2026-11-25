//! 뒤로 한 프레임 이동용 최근 프레임 캐시 (계획서 11단계, 선택).
//! 앞으로 진행하며 렌더된 프레임을 최근 N장 보관하고, 캐시에 이전 프레임이 있으면 mpv 의 느린 `frame-back-step`
//! (키프레임부터 다시 디코드) 없이 바로 보여 준다. 순수 로직 — 단위 테스트 대상.

use std::collections::VecDeque;

/// 보관한 프레임 한 장 (슬롯과 같은 bgr0 레이아웃)
pub struct Entry {
    pub hash: u64,
    pub w: u32,
    pub h: u32,
    pub stride: usize,
    pub data: Vec<u8>,
}

pub struct FrameCache {
    cap: usize,
    frames: VecDeque<Entry>,
    hits: u64,
}

/// 프레임 내용 해시 (FNV-1a). 모든 픽셀을 읽지 않고 행·열을 건너뛰며 표본만 본다 — 같은 프레임을 다시 그린(redraw) 것을 걸러내는 용도
pub fn hash_pixels(data: &[u8], stride: usize, w: u32, h: u32) -> u64 {
    let mut x: u64 = 0xcbf29ce484222325;
    let (rs, cs) = ((h as usize / 61).max(1), (w as usize / 61).max(1));
    let mut y = 0;
    while y < h as usize {
        let mut c = 0;
        while c < w as usize {
            let o = y * stride + c * 4;
            if let Some(p) = data.get(o..o + 3) {
                for b in p {
                    x = (x ^ *b as u64).wrapping_mul(0x100000001b3);
                }
            }
            c += cs;
        }
        y += rs;
    }
    x
}

impl FrameCache {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            frames: VecDeque::new(),
            hits: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// seek/열기 등으로 프레임 순서가 끊기면 비운다
    pub fn clear(&mut self) {
        self.frames.clear();
    }

    /// 새로 렌더된 프레임을 맨 뒤(현재 화면)에 넣는다. 직전과 같은 내용이면(redraw) false. 크기가 바뀌면 이전 것을 버린다
    pub fn push(&mut self, hash: u64, w: u32, h: u32, stride: usize, data: &[u8]) -> bool {
        if self
            .frames
            .back()
            .is_some_and(|e| e.hash == hash && e.w == w && e.h == h)
        {
            return false;
        }
        if self.frames.back().is_some_and(|e| e.w != w || e.h != h) {
            self.frames.clear();
        }
        if self.frames.len() == self.cap {
            self.frames.pop_front();
        }
        self.frames.push_back(Entry {
            hash,
            w,
            h,
            stride,
            data: data.to_vec(),
        });
        true
    }

    /// 뒤로 한 프레임: 현재 화면(맨 뒤)을 버리고 그 앞 프레임을 돌려준다. 앞 프레임이 없으면 None (현재 프레임은 그대로 둔다)
    pub fn step_back(&mut self) -> Option<&Entry> {
        if self.frames.len() < 2 {
            return None;
        }
        self.frames.pop_back();
        self.hits += 1;
        self.frames.back()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Boundary(빈·1장·용량 초과) / Business rule(redraw 중복·크기 변경)

    fn frame(v: u8) -> Vec<u8> {
        vec![v; 64 * 4 * 4]
    }

    #[test]
    fn t_player_worker_cache_01_back_step_walks_history_backwards() {
        let mut c = FrameCache::new(8);
        for i in 0..4u8 {
            assert!(c.push(hash_pixels(&frame(i), 256, 64, 4), 64, 4, 256, &frame(i)));
        }
        assert_eq!(c.step_back().unwrap().data[0], 2);
        assert_eq!(c.step_back().unwrap().data[0], 1);
        assert_eq!(c.step_back().unwrap().data[0], 0);
        assert!(c.step_back().is_none(), "더 이전 프레임이 없으면 miss");
        assert_eq!(c.len(), 1, "miss 는 현재 프레임을 건드리지 않는다");
        assert_eq!(c.hits(), 3);
    }

    #[test]
    fn t_player_worker_cache_02_redraw_of_same_frame_is_not_stored_twice() {
        let mut c = FrameCache::new(8);
        let f = frame(7);
        let h = hash_pixels(&f, 256, 64, 4);
        assert!(c.push(h, 64, 4, 256, &f));
        assert!(!c.push(h, 64, 4, 256, &f), "같은 내용의 redraw");
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn t_player_worker_cache_03_capacity_drops_oldest() {
        let mut c = FrameCache::new(3);
        for i in 0..5u8 {
            c.push(i as u64 + 100, 64, 4, 256, &frame(i));
        }
        assert_eq!(c.len(), 3);
        c.step_back();
        c.step_back();
        assert_eq!(
            c.step_back().map(|e| e.data[0]),
            None,
            "가장 오래된 두 장(0,1)은 밀려났다"
        );
    }

    #[test]
    fn t_player_worker_cache_04_clear_and_size_change_invalidate_history() {
        let mut c = FrameCache::new(4);
        c.push(1, 64, 4, 256, &frame(1));
        c.push(2, 64, 4, 256, &frame(2));
        // 렌더 크기가 바뀐 프레임이 오면 이전 크기의 이력은 쓸 수 없다
        c.push(3, 32, 4, 128, &vec![3; 128 * 4]);
        assert_eq!(c.len(), 1);
        assert!(c.step_back().is_none());
        c.push(4, 32, 4, 128, &vec![4; 128 * 4]);
        c.clear();
        assert!(c.is_empty() && c.step_back().is_none());
    }

    #[test]
    fn t_player_worker_cache_05_hash_distinguishes_frames_and_ignores_stride_padding() {
        let a = frame(10);
        let mut b = frame(10);
        assert_eq!(hash_pixels(&a, 256, 64, 4), hash_pixels(&b, 256, 64, 4));
        b[0] = 99;
        assert_ne!(hash_pixels(&a, 256, 64, 4), hash_pixels(&b, 256, 64, 4));
        // 너무 짧은 슬라이스도 패닉하지 않는다
        let _ = hash_pixels(&a[..10], 256, 64, 4);
        assert_eq!(hash_pixels(&[], 256, 0, 0), 0xcbf29ce484222325);
    }
}
