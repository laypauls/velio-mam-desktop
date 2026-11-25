//! P1-2 측정 도구 공용 코드 — `#[path = "common/mod.rs"] mod common;` 로 가져다 쓴다.
//! 도구마다 쓰는 함수가 달라 미사용 경고를 끈다.
#![allow(dead_code)]

pub mod args;
pub mod load;
pub mod rng;
pub mod stats;
pub mod synth;
pub mod sysinfo;
pub mod vocab;

use std::path::{Path, PathBuf};

use mam_search::{FilenameStrategy, IndexOptions};

/// 데이터셋 폴더 배치: `<root>/dataset.db`, `<root>/index-a|b|c`.
pub struct Layout {
    pub dir: PathBuf,
}

impl Layout {
    /// 기본 위치는 워크스페이스 `target/bench-p1-2/<size>/` (cwd 와 무관).
    pub fn new(size: u64, out: Option<PathBuf>) -> Self {
        let dir = out.unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/bench-p1-2")
                .join(size.to_string())
        });
        Self { dir }
    }

    pub fn db(&self) -> PathBuf {
        self.dir.join("dataset.db")
    }

    pub fn index(&self, strategy: char) -> PathBuf {
        self.dir.join(format!("index-{strategy}"))
    }
}

/// `a|b|c` -> (문자, 질의 전략, 색인 옵션).
pub fn parse_strategy(s: &str) -> (char, FilenameStrategy, IndexOptions) {
    match s.to_ascii_lowercase().as_str() {
        "a" => ('a', FilenameStrategy::A, IndexOptions::strategy_a()),
        "b" => ('b', FilenameStrategy::B, IndexOptions::strategy_b()),
        "c" => ('c', FilenameStrategy::C, IndexOptions::strategy_c()),
        other => args::fail(&format!("--strategy must be a|b|c, got '{other}'")),
    }
}

pub fn open_db(path: &Path) -> rusqlite::Connection {
    mam_db::open(path).unwrap_or_else(|e| args::fail(&format!("open {}: {e}", path.display())))
}

/// 데이터셋 생성 시 `settings` 에 남긴 값.
pub fn setting(conn: &rusqlite::Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

pub fn dir_size(path: &Path) -> u64 {
    std::fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

/// commit 된 세그먼트 수 (`meta.json` 기준 — 쓰기 잠금 없이 읽는다).
pub fn segment_count(dir: &Path) -> usize {
    tantivy::Index::open_in_dir(dir)
        .and_then(|i| i.searchable_segment_metas())
        .map(|m| m.len())
        .unwrap_or(0)
}

pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 실행 조건을 한 줄 JSON 객체로.
pub fn conditions_json(c: &sysinfo::Conditions) -> String {
    format!(
        "{{\"cpu\":{},\"ram_gb\":{},\"os\":{},\"power_plan\":{},\"disks\":{}}}",
        json_str(&c.cpu),
        json_str(&c.ram_gb),
        json_str(&c.os),
        json_str(&c.power_plan),
        json_str(&c.disks)
    )
}

pub fn print_conditions(c: &sysinfo::Conditions) {
    println!("CPU: {}", c.cpu);
    println!("RAM: {} GB", c.ram_gb);
    println!("OS: {}", c.os);
    println!("Power plan: {}", c.power_plan);
    println!("Disks: {}", c.disks);
}

/// 인덱스 디렉터리의 파일 목록 시그니처(이름+크기) — 병합이 끝났는지 판단하는 데 쓴다.
fn dir_signature(dir: &Path) -> Vec<(String, u64)> {
    let mut v: Vec<(String, u64)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| {
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        e.metadata().map(|m| m.len()).unwrap_or(0),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// 백그라운드 병합이 끝나기를 기다린다.
///
/// `SearchIndex` 가 `wait_merging_threads` 를 노출하지 않아 디렉터리 시그니처(파일명+크기)가
/// `quiet` 동안 변하지 않으면 끝난 것으로 본다 — 휴리스틱이다. 반환은 (마지막 변화까지 걸린 시간 = 병합 종료 추정, 타임아웃 여부).
pub fn wait_for_merges(
    dir: &Path,
    quiet: std::time::Duration,
    cap: std::time::Duration,
) -> (std::time::Duration, bool) {
    let start = std::time::Instant::now();
    let mut last = dir_signature(dir);
    let mut stable_since = std::time::Instant::now();
    let mut last_change = std::time::Duration::ZERO;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let now = dir_signature(dir);
        if now != last {
            last = now;
            stable_since = std::time::Instant::now();
            last_change = start.elapsed();
        } else if stable_since.elapsed() >= quiet {
            return (last_change, false);
        }
        if start.elapsed() >= cap {
            return (start.elapsed(), true);
        }
    }
}
