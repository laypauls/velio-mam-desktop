//! 감사 로그 이어 읽기 — 파일별 바이트 오프셋 커서, 로테이션, 읽기 1 MiB 상한. watch_folder `ddpaudit/source.go` 의 `tick`·`readFile` 규칙.
//!
//! 오프셋은 파일 이름별로 센다(시각·줄 수로 하면 같은 줄을 두 번 읽거나 건너뛴다).
//! [`LogTail::poll`] 은 **상태를 바꾸지 않고** 읽은 줄과 다음 커서를 돌려준다 — 호출자가 줄을 변경 대기열에 쓴 **같은 트랜잭션**에서
//! 커서를 저장한 뒤에만 [`LogTail::commit`] 한다. 기록이 실패하면 커서는 그대로라 다음 주기에 같은 오프셋부터 다시 읽는다.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::runner::{RemoteRunner, RunError};

/// 한 번에 가져올 최대 바이트 — 로그 하나가 약 1000줄에서 갈려 평소엔 수십 KB 다. 넘기면(오래 꺼져 있었거나 폭주) 끝으로 건너뛰고
/// 호출자가 재스캔으로 만회한다.
pub const MAX_FETCH: u64 = 1 << 20;
/// 한 주기에 가져올 총량 — 파일이 많이 밀렸어도 한 주기에 몰아 끌어오지 않는다 (남은 것은 다음 주기가 이어 읽는다)
const POLL_BUDGET: u64 = 8 * MAX_FETCH;

/// 영속 커서 (`watch_cursors.cursor` 의 JSON). 모르는 필드는 무시한다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    #[serde(default)]
    pub offsets: BTreeMap<String, u64>,
}

impl Cursor {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }

    /// 해석할 수 없는 저장값은 `None` — 처음 시작처럼 끝에서부터 따라간다(재스캔이 만회한다).
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

/// [`LogTail::poll`] 한 번의 결과.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Polled {
    /// 줄바꿈까지 온 완결된 줄들 (파일 순서대로)
    pub lines: Vec<String>,
    /// 줄을 처리한 뒤 저장할 커서
    pub next: Cursor,
    /// 폭주 방지로 건너뛴 바이트
    pub skipped_bytes: u64,
    /// 놓쳤을 수 있다 — 첫 기준선(과거를 알 수 없음)·건너뜀·파일이 줄어듦(교체). 호출자가 `Rescan` 한다
    pub gap: bool,
}

pub struct LogTail {
    dir: String,
    cursor: Cursor,
    /// 기준선을 잡았는가 (저장된 커서에서 이어 시작하면 처음부터 true)
    primed: bool,
}

impl LogTail {
    pub fn new(dir: &str, resumed: Option<Cursor>) -> Self {
        Self {
            dir: dir.trim_end_matches('/').to_string(),
            primed: resumed.is_some(),
            cursor: resumed.unwrap_or_default(),
        }
    }

    pub fn cursor(&self) -> &Cursor {
        &self.cursor
    }

    pub fn commit(&mut self, next: Cursor) {
        self.cursor = next;
        self.primed = true;
    }

    /// 로그 폴더를 한 번 읽는다 (상태 불변).
    pub fn poll(&self, run: &mut dyn RemoteRunner) -> Result<Polled, RunError> {
        let sizes = list_sizes(run, &self.dir)?;
        let mut names: Vec<&String> = sizes.keys().collect();
        names.sort_by(|a, b| log_cmp(a, b));
        let mut out = Polled::default();
        if !self.primed {
            // 첫 폴링은 지금까지의 줄을 읽지 않는다 — 읽으면 앱을 켤 때마다 지난 1000줄이 "방금 바뀜" 으로 쏟아진다.
            // 그 공백은 재스캔이 맞춘다.
            out.next.offsets = sizes.clone();
            out.gap = true;
            return Ok(out);
        }
        let mut budget = POLL_BUDGET;
        for name in names {
            let size = sizes[name];
            let known = self.cursor.offsets.get(name).copied();
            let mut off = known.unwrap_or(0); // 새로 생긴 파일(로테이션)은 처음부터
            if size < off {
                // 줄어듦 = 갈아치워짐 — 처음부터 다시 보고, 그 사이를 못 봤을 수 있다
                off = 0;
                out.gap = true;
            }
            if size == off {
                out.next.offsets.insert(name.clone(), size);
                continue;
            }
            if size - off > MAX_FETCH {
                out.skipped_bytes += size - off;
                out.next.offsets.insert(name.clone(), size);
                out.gap = true;
                continue;
            }
            if budget < size - off {
                // 이번 주기 몫을 다 썼다 — 읽지 않고 오프셋을 그대로 둔다 (새 파일은 기록하지 않아 다음에 처음부터)
                if let Some(k) = known {
                    out.next.offsets.insert(name.clone(), k);
                }
                continue;
            }
            let bytes = run.run(&cmd::tail_from(&self.dir, name, off))?;
            budget = budget.saturating_sub(bytes.len() as u64);
            // 마지막 줄이 아직 쓰이는 중일 수 있다 — 줄바꿈까지 온 것만 처리하고 오프셋은 거기까지만 올린다
            let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
                out.next.offsets.insert(name.clone(), off);
                continue;
            };
            out.next.offsets.insert(name.clone(), off + end as u64 + 1);
            out.lines.extend(
                String::from_utf8_lossy(&bytes[..=end])
                    .split('\n')
                    .map(|l| l.trim_end_matches('\r'))
                    .filter(|l| !l.trim().is_empty())
                    .map(str::to_string),
            );
        }
        // 사라진 파일(4일 뒤 지워짐)의 오프셋은 next 에 넣지 않아 버려진다
        Ok(out)
    }
}

/// 원격에 보내는 명령 — **읽기 전용** (`wc -c`, `tail -c`). 다른 명령 문자열은 이 crate 어디에도 없다.
pub(super) mod cmd {
    /// 홑따옴표로 감싸 셸이 건드리지 못하게 한다 (한글·공백·`'` 포함 경로).
    pub fn sh_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', r"'\''"))
    }

    /// 로그 폴더의 `audit-log.*` 와 크기. 폴더가 없으면 종료 코드 9. stat 은 배포판마다 옵션이 달라 `wc` 를 쓴다(busybox 에서도 돈다).
    pub fn list_sizes(dir: &str) -> String {
        format!(
            "cd {} 2>/dev/null || exit 9\nfor f in audit-log.*; do\n  [ -f \"$f\" ] || continue\n  printf '%s %s\\n' \"$(wc -c < \"$f\" | tr -d ' ')\" \"$f\"\ndone",
            sh_quote(dir)
        )
    }

    /// 파일의 `off` 바이트 뒤 (`tail -c +N` 은 N 번째 바이트 **부터** 라 1 을 더한다).
    pub fn tail_from(dir: &str, name: &str, off: u64) -> String {
        format!("tail -c +{} {}", off + 1, sh_quote(&format!("{dir}/{name}")))
    }
}

/// 원격이 알려 준 파일 이름이 안전한가 — `audit-log.` 로 시작하고 영숫자·`.`·`_`·`-` 만 (경로 구분자·셸 문자 차단).
fn valid_log_name(n: &str) -> bool {
    n.len() <= 64
        && n.starts_with("audit-log.")
        && n.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// `audit-log.*` 이름 → 크기. 비어 있으면 `NoAuditFiles`, 폴더가 없으면 `NoLogDir`. 접속 시험도 같은 함수를 쓴다.
pub fn list_sizes(run: &mut dyn RemoteRunner, dir: &str) -> Result<BTreeMap<String, u64>, RunError> {
    let out = run.run(&cmd::list_sizes(dir))?; // 종료 코드 9(폴더 없음)는 실행기가 `NoLogDir` 로 바꿔 준다
    let sizes: BTreeMap<String, u64> = String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| {
            let (size, name) = l.trim().split_once(' ')?;
            let name = name.trim();
            if !valid_log_name(name) {
                return None;
            }
            Some((name.to_string(), size.parse().ok()?))
        })
        .collect();
    if sizes.is_empty() {
        return Err(RunError::NoAuditFiles);
    }
    Ok(sizes)
}

fn log_seq(name: &str) -> Option<u64> {
    name.rsplit_once('.')?.1.parse().ok()
}

/// `audit-log.2` 가 `audit-log.10` 보다 앞서도록 숫자로 본다 (둘 다 숫자일 때만, 아니면 글자 순).
pub fn log_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (log_seq(a), log_seq(b)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => a.cmp(b),
    }
}

#[cfg(test)]
#[path = "tail_tests.rs"]
mod tests;
