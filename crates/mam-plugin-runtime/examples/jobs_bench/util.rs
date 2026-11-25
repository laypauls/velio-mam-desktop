//! 측정 공용 — 지연 표본(P50/P95/P99), 크기 표기, 부하 스레드, 간단한 난수.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 지연 표본(ms)
#[derive(Default, Clone)]
pub struct Lat(pub Vec<f64>);

impl Lat {
    pub fn push(&mut self, d: Duration) {
        self.0.push(d.as_secs_f64() * 1000.0);
    }

    pub fn sorted(&self) -> Vec<f64> {
        let mut v = self.0.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v
    }

    pub fn pct(&self, p: f64) -> f64 {
        let v = self.sorted();
        if v.is_empty() {
            return 0.0;
        }
        v[((v.len() as f64 - 1.0) * p).round() as usize]
    }

    pub fn sum_ms(&self) -> f64 {
        self.0.iter().sum()
    }

    pub fn line(&self, label: &str) -> String {
        format!(
            "{label}: n={} P50={:.2}ms P95={:.2}ms P99={:.2}ms max={:.2}ms",
            self.0.len(),
            self.pct(0.5),
            self.pct(0.95),
            self.pct(0.99),
            self.pct(1.0)
        )
    }
}

pub fn say(msg: impl AsRef<str>) {
    println!("{}", msg.as_ref());
}

pub fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

pub fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map_or(0, |m| m.len())
}

/// DB 파일 + WAL + SHM 크기
pub fn db_files_bytes(db: &Path) -> u64 {
    let mut total = file_len(db);
    for ext in ["-wal", "-shm"] {
        let mut s = db.as_os_str().to_owned();
        s.push(ext);
        total += file_len(Path::new(&s));
    }
    total
}

pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            total += if p.is_dir() {
                dir_size(&p)
            } else {
                e.metadata().map_or(0, |m| m.len())
            };
        }
    }
    total
}

/// xorshift64* — 재현 가능한 난수
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// 일정 간격으로 `f` 를 반복하며 각 호출 지연을 모으는 부하 스레드
pub struct Load {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<(Lat, u64)>,
}

impl Load {
    /// `f` 는 한 번 호출이 성공이면 `true`. 실패 수도 센다.
    pub fn spawn(pause: Duration, mut f: impl FnMut() -> bool + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let (mut lat, mut failed) = (Lat::default(), 0u64);
            while !flag.load(Ordering::Relaxed) {
                let t = Instant::now();
                if f() {
                    lat.push(t.elapsed());
                } else {
                    failed += 1;
                }
                std::thread::sleep(pause);
            }
            (lat, failed)
        });
        Self { stop, handle }
    }

    pub fn finish(self) -> (Lat, u64) {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.join().unwrap_or_default()
    }
}

/// 부하 한 쌍(High 읽기 + High 쓰기)의 결과를 한 줄씩 적는다
pub fn report_pair(phase: &str, read: Load, write: Load) {
    let (r, rf) = read.finish();
    let (w, wf) = write.finish();
    say(r.line(&format!(
        "  [{phase}] High read  hydrate_page(50 random ids) (fail {rf})"
    )));
    say(w.line(&format!(
        "  [{phase}] High write update_asset_tags (fail {wf})"
    )));
}
