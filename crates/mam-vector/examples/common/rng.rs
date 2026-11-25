//! 시드 고정 난수(SplitMix64) — mam-search `examples/common/rng.rs` 에서 필요한 부분만 복사했다
//! (작업 문서 "측정 도구 중복": 세 번째 PoC 에서도 필요하면 공용 crate 로 합친다).

#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// 자산 번호마다 독립된 결정적 스트림 (스레드 수와 무관하게 같은 벡터가 나온다).
    pub fn for_item(seed: u64, item: u64) -> Self {
        let mut r = Self(seed ^ item.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// `[0, 1)`
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// `[0, n)`. n == 0 이면 0.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }

    /// 앞쪽 인덱스가 더 자주 나오는 치우친 선택 (`u²` 분포) — 클러스터 인기도.
    pub fn skewed(&mut self, n: usize) -> usize {
        let u = self.unit();
        ((u * u * n as f64) as usize).min(n - 1)
    }

    /// 표준정규 (Box-Muller, 한 번에 하나만 쓴다).
    pub fn gauss(&mut self) -> f32 {
        let u1 = self.unit().max(1e-12);
        let u2 = self.unit();
        ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
    }
}
