//! `tracing` WARN 이상 로그를 메모리에 모아 측정 끝에 요약한다 — "오류 0" 판정이 로그에 숨은 경고·오류를 놓치지 않게 한다.
//! 제품의 로그 파일 설정(`logging.rs`)과는 별개다 (측정은 로그 파일을 만들지 않는다).

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};

use tracing_subscriber::fmt::MakeWriter;

type Store = Arc<Mutex<Vec<String>>>;

static STORE: OnceLock<Store> = OnceLock::new();
/// 메모리 상한 (줄 수) — 반복 경고가 폭주해도 측정이 죽지 않게 한다
const MAX_LINES: usize = 20_000;

struct Cap(Store);

struct Line {
    store: Store,
    buf: Vec<u8>,
}

impl Write for Line {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for Line {
    fn drop(&mut self) {
        let text = String::from_utf8_lossy(&self.buf).trim().to_string();
        let mut g = self.store.lock().unwrap_or_else(|e| e.into_inner());
        if !text.is_empty() && g.len() < MAX_LINES {
            g.push(text);
        }
    }
}

impl<'a> MakeWriter<'a> for Cap {
    type Writer = Line;

    fn make_writer(&'a self) -> Line {
        Line {
            store: Arc::clone(&self.0),
            buf: Vec::new(),
        }
    }
}

fn subscriber(store: Store) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::fmt()
        .with_writer(Cap(store))
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::WARN)
        .finish()
}

/// 전역 구독자를 한 번 설치한다 (이미 다른 구독자가 있으면 조용히 건너뛴다).
pub fn init() {
    STORE.get_or_init(|| {
        let store: Store = Arc::default();
        let _ = tracing::subscriber::set_global_default(subscriber(Arc::clone(&store)));
        store
    });
}

/// 줄들을 앞 260자로 묶어 (줄, 횟수) 로 — 많은 순.
fn group(lines: Vec<String>) -> Vec<(String, usize)> {
    let mut groups: BTreeMap<String, usize> = BTreeMap::new();
    for l in lines {
        *groups.entry(l.chars().take(260).collect()).or_default() += 1;
    }
    let mut v: Vec<(String, usize)> = groups.into_iter().collect();
    v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    v
}

/// 지금까지 모은 WARN/ERROR 를 묶어 돌려주고 비운다.
pub fn take_summary() -> Vec<(String, usize)> {
    let Some(store) = STORE.get() else {
        return Vec::new();
    };
    group(std::mem::take(
        &mut *store.lock().unwrap_or_else(|e| e.into_inner()),
    ))
}

pub fn print_summary(label: &str) {
    let s = take_summary();
    let total: usize = s.iter().map(|(_, n)| n).sum();
    println!(
        "\n### {label} — 로그 WARN/ERROR {total} 줄 ({} 종류)",
        s.len()
    );
    for (line, n) in s.iter().take(12) {
        println!("- ×{n}: {line}");
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (경고를 모은다) / Boundary (INFO 는 모으지 않고 같은 줄은 묶는다)
    use super::*;

    #[test]
    fn t_p4_logs_01_warnings_are_collected_grouped_and_info_is_ignored() {
        // Arrange: 전역이 아닌 지역 구독자 (다른 시험과 간섭하지 않는다)
        let store: Store = Arc::default();
        // Act
        tracing::subscriber::with_default(subscriber(Arc::clone(&store)), || {
            tracing::warn!("p4 test warning");
            tracing::warn!("p4 test warning");
            tracing::info!("p4 info is dropped");
        });
        let grouped = group(std::mem::take(&mut *store.lock().unwrap()));
        // Assert
        assert_eq!(grouped.len(), 1, "{grouped:?}");
        assert!(grouped[0].0.contains("WARN") && grouped[0].0.contains("p4 test warning"));
        assert_eq!(grouped[0].1, 2);
    }
}
