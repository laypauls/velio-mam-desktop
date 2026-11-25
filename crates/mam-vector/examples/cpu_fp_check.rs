//! 코어별 256비트 부동소수 진단 — 논리 CPU 마다 스레드를 고정하고 `kernel::dot_f32`(자동 선택 커널)를
//! 고정 누적 순서의 스칼라 기준값과 반복 대조한다. 불일치하는 코어가 있으면 그 코어는 같은 입력에도 다른 값을 낸다
//! (이 경우 `MAM_VECTOR_KERNEL=baseline` 으로 128비트 커널을 강제해 점수 흔들림을 피하고, 하드웨어 점검을 권한다).
//!
//! 실행: `cargo run --release -p mam-vector --example cpu_fp_check` (Windows 전용 스레드 고정)

#[cfg(windows)]
fn main() {
    use mam_vector::kernel::dot_f32;
    use std::hint::black_box;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadAffinityMask};

    // 고정 순서 기준값: 16 레인 누적 -> 레인 순 합산 (커널 문서의 순서)
    fn reference(q: &[f32], d: &[f32]) -> f32 {
        let mut acc = [0f32; 16];
        for (x, y) in q.chunks_exact(16).zip(d.chunks_exact(16)) {
            for i in 0..16 {
                acc[i] += black_box(x[i]) * y[i];
            }
        }
        acc.iter().sum()
    }

    let cpus = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(64);
    let mut s = 12345u64;
    let mut next = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((s >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    };
    let q: Vec<f32> = (0..512).map(|_| next()).collect();
    let rows: Vec<Vec<f32>> = (0..300)
        .map(|_| (0..512).map(|_| next()).collect())
        .collect();
    let mut bad = Vec::new();
    for cpu in 0..cpus {
        // SAFETY: 현재 스레드 핸들에 유효한 마스크를 지정한다.
        let prev = unsafe { SetThreadAffinityMask(GetCurrentThread(), 1usize << cpu) };
        if prev == 0 {
            println!("cpu {cpu}: affinity failed");
            continue;
        }
        let mismatches = rows
            .iter()
            .filter(|d| {
                let want = reference(&q, d).to_bits();
                (0..20).any(|_| dot_f32(black_box(&q), black_box(d)).to_bits() != want)
            })
            .count();
        println!("cpu {cpu}: {mismatches}/{} rows differ", rows.len());
        if mismatches > 0 {
            bad.push(cpu);
        }
    }
    println!(
        "{}",
        if bad.is_empty() {
            "all cores consistent".to_string()
        } else {
            format!("INCONSISTENT cores: {bad:?}")
        }
    );
}

#[cfg(not(windows))]
fn main() {
    println!("windows only");
}
