//! 부동소수 결과가 비트까지 같아야 하는 테스트용 가드 — 256비트 부동소수가 오동작하는 코어를 프로세스에서 제외한다.
//!
//! 일부 PC 는 특정 물리 코어에서 같은 입력이 호출마다 다른 부동소수 결과를 낸다(하드웨어 결함, 코드 결함 아님).
//! 그런 코어에 스레드가 배치되면 정확 일치 테스트가 무작위로 실패하므로, 코어별 자체 점검으로 불일치 코어를 찾아
//! **프로세스 affinity 를 정상 코어로만** 제한한다. 모든 코어가 정상이면 아무것도 바꾸지 않는다(소프트웨어 비결정성을 가리지 않는다).
//! 스캔 스레드는 `std::thread` 로 만들며(rayon 미사용) 프로세스 마스크를 상속하고, 기존 테스트 스레드도 마스크 변경으로 갱신된다.

use std::sync::OnceLock;

/// 테스트 시작 시 호출한다. 프로세스당 한 번만 실행되고, 다른 테스트는 끝날 때까지 기다린다. Windows 가 아니면 no-op.
pub fn exclude_inconsistent_cores() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(imp::run);
}

#[cfg(not(windows))]
mod imp {
    pub fn run() {}
}

#[cfg(windows)]
mod imp {
    use mam_vector::kernel::{dot_f16, dot_f32};
    use std::hint::black_box;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThread, SetProcessAffinityMask, SetThreadAffinityMask,
    };

    const ROWS: usize = 40;
    const REPEATS: usize = 8;

    /// 고정 순서 기준값(커널 문서의 순서): 16 레인 누적 -> 레인 순 합산. 128비트 경로로 컴파일된다.
    fn reference(q: &[f32], d: &[f32]) -> f32 {
        let mut acc = [0f32; 16];
        for (x, y) in q.chunks_exact(16).zip(d.chunks_exact(16)) {
            for i in 0..16 {
                acc[i] += black_box(x[i]) * y[i];
            }
        }
        acc.iter().sum()
    }

    struct Probe {
        q: Vec<f32>,
        rows: Vec<Vec<f32>>,
        halves: Vec<Vec<u16>>,
    }

    fn probe() -> Probe {
        let mut s = 0x5eed_u64;
        let mut next = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 40) as f32 / (1u64 << 23) as f32) - 1.0
        };
        let q: Vec<f32> = (0..512).map(|_| next()).collect();
        let rows: Vec<Vec<f32>> = (0..ROWS)
            .map(|_| (0..512).map(|_| next()).collect())
            .collect();
        let halves = rows
            .iter()
            .map(|r| {
                r.iter()
                    .map(|&x| half::f16::from_f32(x).to_bits())
                    .collect()
            })
            .collect();
        Probe { q, rows, halves }
    }

    /// 현재 스레드가 고정된 코어에서 커널이 기준과 항상 같은 비트를 내는가.
    fn core_consistent(p: &Probe) -> bool {
        p.rows.iter().zip(&p.halves).all(|(d, h)| {
            let want32 = reference(&p.q, d).to_bits();
            // f16 기준값: 같은 변환을 거친 f32 로 계산
            let conv: Vec<f32> = h
                .iter()
                .map(|&b| half::f16::from_bits(b).to_f32())
                .collect();
            let want16 = reference(&p.q, &conv).to_bits();
            (0..REPEATS).all(|_| {
                dot_f32(black_box(&p.q), black_box(d)).to_bits() == want32
                    && dot_f16(black_box(&p.q), black_box(h)).to_bits() == want16
            })
        })
    }

    /// 논리 CPU 마다 스레드를 고정해 점검하고 불일치 코어 목록을 돌려준다. 호출 스레드의 affinity 는 복원한다.
    fn inconsistent_cores(allowed: usize, cpus: usize) -> Vec<usize> {
        let p = probe();
        let mut bad = Vec::new();
        for cpu in 0..cpus {
            if allowed & (1usize << cpu) == 0 {
                continue;
            }
            // SAFETY: 현재 스레드 핸들에 단일 CPU 마스크를 지정하고, 반환된 이전 마스크로 복원한다.
            let prev = unsafe { SetThreadAffinityMask(GetCurrentThread(), 1usize << cpu) };
            if prev == 0 {
                continue; // 고정 불가 코어는 점검하지 못한다
            }
            let ok = core_consistent(&p);
            // SAFETY: 위에서 받은 이전 마스크다.
            unsafe { SetThreadAffinityMask(GetCurrentThread(), prev) };
            if !ok {
                bad.push(cpu);
            }
        }
        bad
    }

    pub fn run() {
        let cpus = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(usize::BITS as usize);
        let all = if cpus >= usize::BITS as usize {
            usize::MAX
        } else {
            (1usize << cpus) - 1
        };
        let bad = inconsistent_cores(all, cpus);
        if bad.is_empty() {
            return;
        }
        let good = bad.iter().fold(all, |m, &c| m & !(1usize << c));
        if good == 0 {
            eprintln!(
                "WARNING: every core failed the FP self-check {bad:?}; leaving affinity unchanged"
            );
            return;
        }
        // SAFETY: 현재 프로세스 핸들, 비어 있지 않은 부분집합 마스크.
        let ok = unsafe { SetProcessAffinityMask(GetCurrentProcess(), good) };
        eprintln!(
            "WARNING: logical CPUs {bad:?} give inconsistent 256-bit float results (hardware fault); \
             restricting this test process to mask {good:#x} (set ok: {})",
            ok != 0
        );
        // 제한 뒤 현재 스레드가 정상 코어에서 도는지 재점검한다
        let still = inconsistent_cores(good, cpus);
        if !still.is_empty() {
            eprintln!("WARNING: cores {still:?} still inconsistent after restriction");
        }
    }
}
