//! 경로 겹침 판정 — 사용자 지정 경로가 MAM 산출물 위치(MAMData·백업 폴더)가 될 때 관리 폴더(원본)와 겹치지 않게 한다.
//!
//! 겹침 = 같거나 어느 한쪽이 다른 쪽의 하위. 비교는 대소문자 무시, 구분자·`\?\` 접두·`..` 정규화,
//! 존재하는 가장 깊은 상위는 `canonicalize`(심볼릭 링크·정션·8.3 이름 해소) 한 뒤 나머지를 덧붙인다.

use std::path::{Path, PathBuf};

const DATA_DIR_NAME: &str = "MAMData";

/// 존재하는 가장 깊은 상위를 canonicalize 하고 존재하지 않는 꼬리를 덧붙인다.
fn canonical_prefix(path: &Path) -> PathBuf {
    let mut tail = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(c) = std::fs::canonicalize(&cur) {
            let mut out = c;
            out.extend(tail.iter().rev());
            return out;
        }
        match (cur.file_name().map(ToOwned::to_owned), cur.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name);
                cur = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn components(path: &Path) -> Vec<String> {
    let text = canonical_prefix(path).to_string_lossy().replace('/', "\\");
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let mut out: Vec<String> = Vec::new();
    for part in text.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            p => out.push(p.to_lowercase()),
        }
    }
    out
}

/// 두 경로가 같거나 한쪽이 다른 쪽을 포함하는가.
pub fn paths_overlap(a: &Path, b: &Path) -> bool {
    let (a, b) = (components(a), components(b));
    let n = a.len().min(b.len());
    a[..n] == b[..n]
}

/// `inner` 가 `outer` 와 같거나 그 하위인가.
pub fn is_same_or_inside(inner: &Path, outer: &Path) -> bool {
    let (i, o) = (components(inner), components(outer));
    i.len() >= o.len() && i[..o.len()] == o[..]
}

/// 사용자가 고른 폴더 -> 실제 MAMData 루트 (D1). 고른 폴더가 이미 MAMData 루트(`database/mam.sqlite` 가 있음)면 그대로,
/// 아니면 항상 고정 하위 폴더 `MAMData` 를 붙인다 — 기존 폴더(원본 트리)에 `database`·`cache` 가 섞여 들어가지 않게 한다.
pub fn resolve_data_root(chosen: &Path) -> PathBuf {
    if chosen.join("database").join("mam.sqlite").is_file() {
        chosen.to_path_buf()
    } else {
        chosen.join(DATA_DIR_NAME)
    }
}
