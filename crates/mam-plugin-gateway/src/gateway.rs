//! 게이트웨이 본체 — `HostApi` 구현: 메서드 이름 정확 일치 -> 권한 검사 -> 메서드 실행. 오류는 정적 메시지로만 나간다.
//!
//! 플러그인 id 는 `CallCx` 에서만 읽는다(params 를 신뢰하지 않음). 게이트웨이는 `PluginHost` 를 참조하지 않는다(재진입 없음).
//! 호출 1회가 쓰는 DB 접근은 문장 1개 또는 작은 트랜잭션 1개이며 대기는 호출 마감(`scope.deadline`)을 넘지 않는다.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_core::userdata::UserData;
use mam_db::DbPool;
use mam_plugin_host::{CallCx, HostApi, HostError, LogLevel};
use mam_plugin_manifest::Permission;
use serde_json::Value;

use crate::errors::GwError;
use crate::grants::GrantRegistry;
use crate::logs::{Admit, LogLimiter, LogSink, TracingSink, sanitize};
use crate::quota::Meters;

/// 제안 기본값(미측정) — 시험은 작은 값으로 바꿔 쓴다
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// 썸네일이 있는 MAM 캐시 루트 (`mam_media::thumbnail_path` 의 기준)
    pub cache_root: PathBuf,
    /// 플러그인당 KV 총량(바이트, 행 비용 = 키 + 값 + 64)
    pub kv_quota_bytes: u64,
    /// 값 1개의 compact JSON 직렬화 최대 바이트
    pub kv_value_max_bytes: usize,
    /// 썸네일 1개 최대 바이트
    pub derived_max_bytes: u64,
    /// 호출 1회에서 돌려줄 수 있는 파생 데이터 누적 바이트
    pub derived_budget_bytes: u64,
    /// DB 읽기·쓰기 대기 상한 (호출 마감이 더 이르면 마감)
    pub read_wait: Duration,
    pub write_wait: Duration,
    /// 자산당 태그 수 상한, 호출당 새 태그 이름 수 상한
    pub max_asset_tags: i64,
    pub max_new_tags_per_call: u32,
    /// 로그 토큰 버킷 (초당, burst)
    pub log_per_sec: u32,
    pub log_burst: u32,
    /// debug 로그는 디버그 로깅일 때만 남긴다
    pub debug_logging: bool,
}

impl GatewayConfig {
    pub fn new(cache_root: PathBuf) -> Self {
        Self {
            cache_root,
            kv_quota_bytes: 256 << 20,
            kv_value_max_bytes: 65_536,
            derived_max_bytes: 1 << 20,
            derived_budget_bytes: 32 << 20,
            read_wait: Duration::from_secs(2),
            write_wait: Duration::from_secs(3),
            max_asset_tags: 100,
            max_new_tags_per_call: 200,
            log_per_sec: 20,
            log_burst: 100,
            debug_logging: false,
        }
    }
}

pub struct Gateway {
    pub(crate) pool: Arc<DbPool>,
    pub(crate) clock: SharedClock,
    pub(crate) userdata: UserData,
    pub(crate) cfg: GatewayConfig,
    pub(crate) grants: Arc<GrantRegistry>,
    pub(crate) meters: Meters,
    limiter: LogLimiter,
    sink: Arc<dyn LogSink>,
}

impl Gateway {
    pub fn new(pool: Arc<DbPool>, clock: SharedClock, cfg: GatewayConfig) -> Self {
        let userdata = UserData::new(Arc::clone(&pool), Arc::clone(&clock));
        let limiter = LogLimiter::new(cfg.log_per_sec, cfg.log_burst);
        Self {
            pool,
            clock,
            userdata,
            cfg,
            grants: Arc::new(GrantRegistry::default()),
            meters: Meters::default(),
            limiter,
            sink: Arc::new(TracingSink),
        }
    }

    /// 로그 출력 대상을 바꾼다 (시험·진단)
    #[must_use]
    pub fn with_log_sink(mut self, sink: Arc<dyn LogSink>) -> Self {
        self.sink = sink;
        self
    }

    pub fn grants(&self) -> &GrantRegistry {
        &self.grants
    }

    /// 플러그인 제거 뒤 KV 계량기를 비운다 (재설치 시 낡은 사용량이 남지 않게 — 다음 `kv.set` 이 DB 합계로 다시 센다)
    pub fn forget_plugin(&self, plugin: &str) {
        self.meters.forget(plugin);
    }

    /// 시험 전용(`test-util` feature) — 계량기가 초기화됐으면 현재 사용량(바이트)
    #[cfg(any(test, feature = "test-util"))]
    #[doc(hidden)]
    pub fn kv_meter_used(&self, plugin: &str) -> Option<u64> {
        let m = self.meters.of(plugin);
        m.ready().then(|| m.used())
    }

    pub(crate) fn read_deadline(&self, cx: &CallCx<'_>) -> Instant {
        (Instant::now() + self.cfg.read_wait).min(cx.scope.deadline)
    }

    pub(crate) fn write_deadline(&self, cx: &CallCx<'_>) -> Instant {
        (Instant::now() + self.cfg.write_wait).min(cx.scope.deadline)
    }

    /// 호출 범위 안의 자산인가 — DB 접근 전에 한다. 범위 밖과 없는 자산은 같은 `not_found`
    pub(crate) fn in_scope(cx: &CallCx<'_>, asset_id: i64) -> Result<(), GwError> {
        if cx.scope.contains(asset_id) {
            Ok(())
        } else {
            Err(GwError::NotFound)
        }
    }

    fn dispatch(&self, cx: &CallCx<'_>, method: &str, params: Value) -> Result<Value, GwError> {
        let perm = match method {
            "asset.get" => Permission::AssetReadMetadata,
            "asset.getDerived" => Permission::AssetReadDerived,
            "kv.get" | "kv.list" => Permission::KvRead,
            "kv.set" | "kv.delete" => Permission::KvWrite,
            "tag.add" => Permission::TagWrite,
            _ => return Err(GwError::UnknownMethod),
        };
        if !self.grants.allows(cx.plugin.as_str(), perm) {
            return Err(GwError::PermissionDenied(perm));
        }
        match method {
            "asset.get" => self.asset_get(cx, params),
            "asset.getDerived" => self.asset_get_derived(cx, params),
            "kv.get" => self.kv_get(cx, params),
            "kv.list" => self.kv_list(cx, params),
            "kv.set" => self.kv_set(cx, params),
            "kv.delete" => self.kv_delete(cx, params),
            _ => self.tag_add(cx, params),
        }
    }
}

impl HostApi for Gateway {
    fn call(&self, cx: &CallCx<'_>, method: &str, params: Value) -> Result<Value, HostError> {
        self.dispatch(cx, method, params)
            .map_err(|e| e.into_host(cx.plugin.as_str(), method))
    }

    fn log(&self, cx: &CallCx<'_>, level: LogLevel, message: &str) {
        let level = match level {
            LogLevel::Debug if !self.cfg.debug_logging => return,
            LogLevel::Error => LogLevel::Warn, // 레벨 상한 warn
            other => other,
        };
        let id = cx.plugin.as_str();
        match self.limiter.admit(id, Instant::now()) {
            Admit::Drop => return,
            Admit::Pass(Some(n)) => {
                self.sink
                    .emit(id, LogLevel::Warn, &format!("{n} messages dropped"))
            }
            Admit::Pass(None) => {}
        }
        self.sink.emit(id, level, &sanitize(message));
    }
}
