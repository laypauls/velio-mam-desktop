//! 워커가 `mam.host.call` 로 부르는 호스트 쪽 구현 자리. 실제 함수(asset/kv/tag)와 권한 검사는 게이트웨이 crate — 여기서는 trait·호출 컨텍스트만 둔다.
//!
//! 주의: `call` 은 감독자 락을 쥔 채 호출된다. 같은 `PluginHost` 를 다시 부르면 교착한다. 또 호출이 오래 걸려도 감독자가
//! 중간에 끊을 수 없다(호스트 함수는 짧게, DB 대기는 `CallCx.scope.deadline` 까지만 — 오래 걸리는 일은 작업 대기열로).

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::time::Instant;

use mam_plugin_manifest::PluginId;
use serde_json::Value;

use crate::protocol::LogLevel;

/// 호출 1회가 다룰 수 있는 자산 수 상한 (선택 `assetIds` 상한과 같다)
pub const MAX_SCOPE_ASSETS: usize = 10_000;

/// 호스트 함수가 돌려주는 오류 코드. 워커가 직접 내는 `bad_request`·`too_large`(`io.rs`)와 같은 이름을 쓴다
pub mod codes {
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const BAD_REQUEST: &str = "bad_request";
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    pub const NOT_FOUND: &str = "not_found";
    pub const TOO_LARGE: &str = "too_large";
    pub const QUOTA_EXCEEDED: &str = "quota_exceeded";
    pub const UNAVAILABLE: &str = "unavailable";
    pub const BUSY: &str = "busy";
    pub const INTERNAL: &str = "internal";
    pub const HOST_PANIC: &str = "host_panic";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    pub code: String,
    pub message: String,
}

impl HostError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

/// `assets` 가 `MAX_SCOPE_ASSETS` 를 넘었다
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invoke scope exceeds {MAX_SCOPE_ASSETS} assets")]
pub struct ScopeTooLarge;

/// 호출 1회의 범위·마감·예산. **호출자(메뉴·작업 실행기)가 만든다 — 플러그인이 아니다.**
/// `deadline` 은 감독자가 호출 타임아웃으로 다시 찍는다(`PluginHost::invoke*`).
#[derive(Debug)]
pub struct InvokeScope {
    /// 이 호출에서 읽을 수 있는 자산 id — 정렬·중복 없음
    pub assets: Arc<[i64]>,
    pub deadline: Instant,
    /// 이 호출에서 돌려준 파생 데이터 누적 바이트
    pub derived_bytes: AtomicU64,
    /// 이 호출에서 새로 만든 태그 이름 수
    pub new_tags: AtomicU32,
}

impl InvokeScope {
    /// 정렬·중복 제거 후 보관한다. 호출 밖(init 등)은 `empty()`
    pub fn new(mut assets: Vec<i64>) -> Result<Self, ScopeTooLarge> {
        assets.sort_unstable();
        assets.dedup();
        if assets.len() > MAX_SCOPE_ASSETS {
            return Err(ScopeTooLarge);
        }
        Ok(Self {
            assets: assets.into(),
            deadline: Instant::now(),
            derived_bytes: AtomicU64::new(0),
            new_tags: AtomicU32::new(0),
        })
    }

    pub fn empty() -> Self {
        Self {
            assets: Arc::from(Vec::new()),
            deadline: Instant::now(),
            derived_bytes: AtomicU64::new(0),
            new_tags: AtomicU32::new(0),
        }
    }

    pub fn with_deadline(mut self, deadline: Instant) -> Self {
        self.deadline = deadline;
        self
    }

    /// 이진 탐색
    pub fn contains(&self, asset_id: i64) -> bool {
        self.assets.binary_search(&asset_id).is_ok()
    }
}

/// 호스트 호출·로그가 받는 호출 컨텍스트. 플러그인 id 는 호스트가 연결에서 정한 값이다 — 워커가 보낸 메시지의 값을 쓰지 않는다
#[derive(Clone, Copy)]
pub struct CallCx<'a> {
    pub plugin: &'a PluginId,
    pub scope: &'a InvokeScope,
}

pub trait HostApi: Send + Sync {
    fn call(&self, cx: &CallCx<'_>, method: &str, params: Value) -> Result<Value, HostError>;

    /// 워커 `mam.log.*` — 길이는 호출자(감독자)가 이미 잘랐다. 키·토큰 마스킹은 구현의 몫
    fn log(&self, _cx: &CallCx<'_>, _level: LogLevel, _message: &str) {}
}
