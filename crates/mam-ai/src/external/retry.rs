//! 재시도 정책 (PL-A4): 429 는 `Retry-After` 존중, 5xx·timeout 은 지수 백오프, 최대 3회. 4xx(429 제외)는 즉시 실패.
//! 시계와 대기를 주입할 수 있어 테스트에서 실제로 기다리지 않는다.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::error::ExternalError;
use super::transport::{HttpRequest, HttpResponse, HttpTransport, TransportError};

pub trait Clock: Send + Sync {
    /// Unix ms.
    fn now_ms(&self) -> i64;
}

pub trait Sleeper: Send + Sync {
    fn sleep(&self, d: Duration);
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

pub struct ThreadSleeper;
impl Sleeper for ThreadSleeper {
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

#[derive(Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    /// 백오프와 `Retry-After` 대기 모두의 상한.
    pub max_delay: Duration,
    pub clock: Arc<dyn Clock>,
    pub sleeper: Arc<dyn Sleeper>,
}

impl RetryPolicy {
    pub fn system() -> Self {
        Self::with(Arc::new(SystemClock), Arc::new(ThreadSleeper))
    }

    pub fn with(clock: Arc<dyn Clock>, sleeper: Arc<dyn Sleeper>) -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            clock,
            sleeper,
        }
    }

    fn backoff(&self, retry_index: u32) -> Duration {
        let factor = 1u32.checked_shl(retry_index).unwrap_or(u32::MAX);
        self.base_delay.saturating_mul(factor).min(self.max_delay)
    }

    /// 요청을 보내고 재시도 규칙에 따라 성공 응답(2xx) 또는 오류를 돌려준다.
    pub fn send(
        &self,
        transport: &dyn HttpTransport,
        req: &HttpRequest,
    ) -> Result<HttpResponse, ExternalError> {
        let mut retries = 0u32;
        loop {
            let (err, wait) = match transport.send(req) {
                Ok(resp) if (200..300).contains(&resp.status) => return Ok(resp),
                Ok(resp) => match classify(&resp, self.clock.now_ms()) {
                    Classified::Fatal(e) => return Err(e),
                    Classified::Retry(e, hint) => (e, hint),
                },
                Err(TransportError::Timeout) => (ExternalError::Timeout, None),
                Err(TransportError::Offline) => return Err(ExternalError::Offline),
                Err(TransportError::Other) => {
                    return Err(ExternalError::BadResponse("transport failure"));
                }
            };
            if retries >= self.max_retries {
                return Err(err);
            }
            let delay = wait
                .map(|d| d.min(self.max_delay))
                .unwrap_or_else(|| self.backoff(retries));
            self.sleeper.sleep(delay);
            retries += 1;
        }
    }
}

enum Classified {
    Fatal(ExternalError),
    Retry(ExternalError, Option<Duration>),
}

fn classify(resp: &HttpResponse, now_ms: i64) -> Classified {
    match resp.status {
        429 => {
            let wait = resp
                .header("retry-after")
                .and_then(|v| parse_retry_after(v, now_ms));
            Classified::Retry(
                ExternalError::RateLimited {
                    retry_after_secs: wait.map(|d| d.as_secs()),
                },
                wait,
            )
        }
        500..=599 => Classified::Retry(
            ExternalError::Server {
                status: resp.status,
            },
            None,
        ),
        401 | 403 => Classified::Fatal(ExternalError::AuthFailed),
        s => Classified::Fatal(ExternalError::BadRequest { status: s }),
    }
}

/// `Retry-After`: 정수 초 또는 HTTP-date(IMF-fixdate). 해석 불가·과거 날짜는 `None`/0.
pub fn parse_retry_after(value: &str, now_ms: i64) -> Option<Duration> {
    let v = value.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let target_ms = parse_http_date(v)?;
    Some(Duration::from_millis((target_ms - now_ms).max(0) as u64))
}

/// `Sun, 06 Nov 1994 08:49:37 GMT` -> Unix ms.
fn parse_http_date(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 6 || parts[5] != "GMT" {
        return None;
    }
    let day: i64 = parts[1].parse().ok()?;
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS.iter().position(|m| *m == parts[2])? as i64 + 1;
    let year: i64 = parts[3].parse().ok()?;
    let mut t = parts[4].split(':');
    let (h, m, sec): (i64, i64, i64) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    if !(1..=31).contains(&day) || h > 23 || m > 59 || sec > 60 {
        return None;
    }
    // days-from-civil (Howard Hinnant)
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + m) * 60_000 + sec * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_date_matches_rfc_example() {
        // RFC 7231 예: 784111777 (1994-11-06T08:49:37Z)
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777_000)
        );
    }

    #[test]
    fn retry_after_past_date_is_zero_and_garbage_none() {
        let d = parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT", 784_111_900_000);
        assert_eq!(d, Some(Duration::ZERO));
        assert_eq!(parse_retry_after("soon", 0), None);
        assert_eq!(parse_retry_after("7", 0), Some(Duration::from_secs(7)));
    }
}
