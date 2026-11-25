//! QuickJS 런타임 1개 = 플러그인 1개. 등록하는 것은 `mam` 전역(boot.js)뿐이다 — `std`·`os` 모듈, 파일·네트워크·프로세스 기능 없음,
//! 모듈 로더 없음(`import` 는 항상 실패). 한도(힙·스택)는 런타임에, CPU 한도는 interrupt handler 에 건다.

use std::rc::Rc;

use mam_plugin_host::protocol::{CODE_LIMIT, CODE_SCRIPT, CODE_TOO_LARGE, InvokeKind, Limits, LogLevel};
use rquickjs::{Context, Ctx, Error, Function, Object, Persistent, Runtime};
use serde_json::Value;

use crate::io::Io;
use crate::meter::Meter;

const BOOT: &str = include_str!("boot.js");
/// 호출 뒤 대기 중인 Promise 작업을 비우는 상한 (무한 마이크로태스크 방지)
const MAX_JOBS: usize = 10_000;

pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

pub struct Engine {
    io: Rc<Io>,
    rt: Runtime,
    ctx: Context,
    meter: Rc<Meter>,
    internal: Persistent<Object<'static>>,
    pub commands: Vec<String>,
    pub providers: Vec<String>,
}

fn describe(ctx: &Ctx<'_>, e: Error) -> String {
    if !matches!(e, Error::Exception) {
        return e.to_string();
    }
    let v = ctx.catch();
    if let Some(ex) = v.as_exception() {
        return ex.message().unwrap_or_else(|| "exception".to_owned());
    }
    v.as_string().and_then(|s| s.to_string().ok()).unwrap_or_else(|| "exception".to_owned())
}

impl Engine {
    pub fn boot(io: &Rc<Io>, source: &str, limits: &Limits) -> Result<Self, String> {
        let rt = Runtime::new().map_err(|e| e.to_string())?;
        let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
        let meter = Rc::new(Meter::new(limits.cpu_ms));
        if limits.memory_bytes > 0 {
            rt.set_memory_limit(usize::try_from(limits.memory_bytes).unwrap_or(usize::MAX));
        }
        if limits.stack_bytes > 0 {
            rt.set_max_stack_size(usize::try_from(limits.stack_bytes).unwrap_or(usize::MAX));
        }
        let m = Rc::clone(&meter);
        rt.set_interrupt_handler(Some(Box::new(move || m.check())));
        meter.begin();

        let (call_io, call_meter, log_io) = (Rc::clone(io), Rc::clone(&meter), Rc::clone(io));
        let (internal, names) = ctx.with(|ctx| -> Result<(Persistent<Object<'static>>, String), String> {
            let call = Function::new(ctx.clone(), move |method: String, params: String| -> String { call_io.host_call(&call_meter, &method, &params) }).map_err(|e| describe(&ctx, e))?;
            let log = Function::new(ctx.clone(), move |level: String, msg: String| {
                let level = match level.as_str() {
                    "debug" => LogLevel::Debug,
                    "warn" => LogLevel::Warn,
                    "error" => LogLevel::Error,
                    _ => LogLevel::Info,
                };
                log_io.log(level, &msg);
            })
            .map_err(|e| describe(&ctx, e))?;
            let boot: Function = ctx.eval(BOOT).map_err(|e| describe(&ctx, e))?;
            let internal: Object = boot.call((call, log)).map_err(|e| describe(&ctx, e))?;
            ctx.eval::<(), _>(source).map_err(|e| describe(&ctx, e))?;
            let names_fn: Function = internal.get("names").map_err(|e| describe(&ctx, e))?;
            let names: String = names_fn.call(()).map_err(|e| describe(&ctx, e))?;
            Ok((Persistent::save(&ctx, internal), names))
        })?;
        let parsed: Value = serde_json::from_str(&names).map_err(|e| e.to_string())?;
        let list = |k: &str| -> Vec<String> { parsed[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default() };
        let (commands, providers) = (list("commands"), list("providers"));
        Ok(Self { io: Rc::clone(io), rt, ctx, meter, internal, commands, providers })
    }

    pub fn invoke(&self, kind: InvokeKind, target: &str, params: &Value) -> Result<Value, Failure> {
        self.meter.begin();
        let kind_str = if kind == InvokeKind::Command { "command" } else { "provider" };
        let out = self.ctx.with(|ctx| -> Result<String, String> {
            let internal = self.internal.clone().restore(&ctx).map_err(|e| describe(&ctx, e))?;
            let dispatch: Function = internal.get("dispatch").map_err(|e| describe(&ctx, e))?;
            dispatch.call((kind_str, target.to_owned(), params.to_string())).map_err(|e| describe(&ctx, e))
        });
        // 이 호출이 남긴 Promise 작업(예: 거부된 `import()`)을 같은 한도 안에서 비운다
        for _ in 0..MAX_JOBS {
            if !matches!(self.rt.execute_pending_job(), Ok(true)) {
                break;
            }
        }
        match out {
            // 파싱 전에 크기부터 — 상한을 넘는 결과는 Value 로 부풀리지 않고 `too_large` 로 즉시 거절
            Ok(json) if self.io.max_line() > 0 && json.len() > self.io.max_line() => {
                Err(Failure { code: CODE_TOO_LARGE, message: format!("result of {} bytes exceeds the {} byte limit", json.len(), self.io.max_line()) })
            }
            Ok(json) => serde_json::from_str(&json).map_err(|e| Failure { code: CODE_SCRIPT, message: format!("result is not JSON: {e}") }),
            Err(_) if self.meter.tripped() => Err(Failure { code: CODE_LIMIT, message: "cpu time limit exceeded".to_owned() }),
            Err(m) if m.contains("out of memory") => Err(Failure { code: CODE_LIMIT, message: "memory limit exceeded".to_owned() }),
            Err(m) => Err(Failure { code: CODE_SCRIPT, message: m }),
        }
    }
}
