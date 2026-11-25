//! 전수 비교용 내적 커널 — f32 / f16(비트) / i8.
//!
//! 기본 빌드(x86-64 baseline = SSE2)에서도 AVX2+FMA 를 쓰도록 `#[target_feature]` 래퍼를 두고
//! 실행 시 CPU 기능을 검사해 고른다. 본문은 `#[inline(always)]` 일반 함수라 같은 코드가 두 번 컴파일된다.

/// f16 비트 -> f32. 부호 분리 후 지수 비트 이동 + 2^112 곱으로 정규/비정규수를 분기 없이 변환한다
/// (무한대·NaN 은 저장하지 않으므로 처리하지 않는다 — 입력 검증이 막는다).
#[inline(always)]
pub fn f16_bits_to_f32(h: u16) -> f32 {
    let h = h as u32;
    let sign = (h & 0x8000) << 16;
    let mag = f32::from_bits((h & 0x7fff) << 13) * f32::from_bits(0x7780_0000); // 2^112
    f32::from_bits(mag.to_bits() | sign)
}

#[inline(always)]
fn dot_f32_generic(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 16];
    let (ca, cb) = (a.chunks_exact(16), b.chunks_exact(16));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..16 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

#[inline(always)]
fn dot_f16_generic(q: &[f32], d: &[u16]) -> f32 {
    let mut acc = [0f32; 16];
    let (cq, cd) = (q.chunks_exact(16), d.chunks_exact(16));
    let (rq, rd) = (cq.remainder(), cd.remainder());
    for (x, y) in cq.zip(cd) {
        for i in 0..16 {
            acc[i] += x[i] * f16_bits_to_f32(y[i]);
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in rq.iter().zip(rd) {
        s += x * f16_bits_to_f32(*y);
    }
    s
}

#[inline(always)]
fn dot_i8_generic(a: &[i8], b: &[i8]) -> i32 {
    let mut acc = [0i32; 16];
    let (ca, cb) = (a.chunks_exact(16), b.chunks_exact(16));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..16 {
            acc[i] += x[i] as i32 * y[i] as i32;
        }
    }
    let mut s: i32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += *x as i32 * *y as i32;
    }
    s
}

#[cfg(target_arch = "x86_64")]
mod avx2 {
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
        super::dot_f32_generic(a, b)
    }
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn dot_f16(q: &[f32], d: &[u16]) -> f32 {
        super::dot_f16_generic(q, d)
    }
    #[target_feature(enable = "avx2,fma")]
    pub unsafe fn dot_i8(a: &[i8], b: &[i8]) -> i32 {
        super::dot_i8_generic(a, b)
    }
}

#[cfg(target_arch = "x86_64")]
fn has_avx2() -> bool {
    use std::sync::OnceLock;
    static V: OnceLock<bool> = OnceLock::new();
    // MAM_VECTOR_KERNEL=baseline: 128비트(SSE2) 커널 강제. 점수는 같은 누적 순서라 정상 CPU 에서 비트까지 같다.
    // 256비트 부동소수 연산이 오동작하는 코어(진단: `cargo run -p mam-vector --example cpu_fp_check`)에서
    // 호출마다 점수가 흔들리는 것을 피하는 용도다. 기본값(자동 선택)은 변하지 않는다.
    *V.get_or_init(|| {
        std::env::var("MAM_VECTOR_KERNEL").map_or(true, |v| v != "baseline")
            && is_x86_feature_detected!("avx2")
            && is_x86_feature_detected!("fma")
    })
}

/// 길이가 같은 두 f32 벡터의 내적.
pub fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    #[cfg(target_arch = "x86_64")]
    if has_avx2() {
        // SAFETY: avx2+fma 지원을 실행 시 확인했다.
        return unsafe { avx2::dot_f32(a, b) };
    }
    dot_f32_generic(a, b)
}

/// f32 질의와 f16(비트) 저장 벡터의 내적.
pub fn dot_f16(q: &[f32], d: &[u16]) -> f32 {
    debug_assert_eq!(q.len(), d.len());
    #[cfg(target_arch = "x86_64")]
    if has_avx2() {
        // SAFETY: avx2+fma 지원을 실행 시 확인했다.
        return unsafe { avx2::dot_f16(q, d) };
    }
    dot_f16_generic(q, d)
}

/// i8 두 벡터의 정수 내적 (스케일은 호출자가 곱한다).
pub fn dot_i8(a: &[i8], b: &[i8]) -> i32 {
    debug_assert_eq!(a.len(), b.len());
    #[cfg(target_arch = "x86_64")]
    if has_avx2() {
        // SAFETY: avx2+fma 지원을 실행 시 확인했다.
        return unsafe { avx2::dot_i8(a, b) };
    }
    dot_i8_generic(a, b)
}

/// 벡터별 스케일 i8 양자화: `q = round(x / max|x| * 127)`, 스케일 = `max|x| / 127`.
/// 반환 스케일을 곱하면 원래 값을 근사한다. 영벡터는 호출 전에 걸러진다.
pub fn quantize_i8(v: &[f32], out: &mut [i8]) -> f32 {
    let max = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    if max == 0.0 {
        out.fill(0);
        return 0.0;
    }
    let inv = 127.0 / max;
    for (o, x) in out.iter_mut().zip(v) {
        *o = (x * inv).round().clamp(-127.0, 127.0) as i8;
    }
    max / 127.0
}
