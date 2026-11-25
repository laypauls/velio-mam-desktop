//! 측정 도구 공용 코드 - `#[path = "common/mod.rs"] mod common;` 로 가져다 쓴다 (mam-vector 의 P1-3 도구에서 복사).
#![allow(dead_code)]

pub mod stats;
pub mod synth;
pub mod sysinfo;

/// 간단한 `--key value` 인자 파서.
pub struct Args(Vec<String>);

impl Args {
    pub fn parse() -> Self {
        Self(std::env::args().skip(1).collect())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        let flag = format!("--{key}");
        self.0
            .iter()
            .position(|a| *a == flag)
            .and_then(|i| self.0.get(i + 1))
            .map(String::as_str)
    }

    pub fn has(&self, key: &str) -> bool {
        let flag = format!("--{key}");
        self.0.contains(&flag)
    }

    pub fn num<T: std::str::FromStr>(&self, key: &str, default: T) -> T {
        self.get(key).map_or(default, |v| {
            v.parse()
                .unwrap_or_else(|_| fail(&format!("--{key}: invalid value {v}")))
        })
    }

    pub fn require(&self, key: &str) -> &str {
        self.get(key)
            .unwrap_or_else(|| fail(&format!("--{key} is required")))
    }
}

pub fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(2);
}

pub fn print_conditions() -> sysinfo::Conditions {
    let c = sysinfo::conditions();
    println!("CPU: {}", c.cpu);
    println!("RAM: {} GB", c.ram_gb);
    println!("OS: {}", c.os);
    println!("Power plan: {}", c.power_plan);
    println!("Disks: {}", c.disks);
    c
}
