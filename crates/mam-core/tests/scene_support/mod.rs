//! scene_exec*.rs 공용 환경 — 장면 검출만 정교하게 흉내 내는 실행기 대역 + 처리 환경 + 실행기 등록.
#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mam_core::jobs::JobKind;
use mam_core::scene::SceneExecutor;
use mam_db::Priority;
use mam_db::scene_cuts::{self, RequestOptions, SceneRow};
use mam_media::error::{MediaError, Result as MediaResult};
use mam_media::tool::{CommandOutput, CommandRunner, MediaTools, RunOptions, Tool};

use crate::common::env::wait_until;
use crate::common::proc_env::{
    ProcEnv, ok_output, png_bytes, probe_audio_json, probe_json, wav_bytes,
};

/// 장면 검출(scdet) 호출에 대한 응답.
#[derive(Clone)]
pub enum Scene {
    /// 정상 종료 + 이 표준출력
    Cuts(String),
    /// 실행기 오류
    Err(fn() -> MediaError),
    /// 0 이 아닌 종료 코드 + stderr
    Exit(i32, &'static str),
    /// 취소될 때까지 실행 중인 척한다 — 모드가 바뀌면 그 응답으로 끝난다
    Block,
}

/// 메타·썸네일은 `stub_tools` 처럼 응답하고, `scdet` 호출만 [`Scene`] 모드를 따른다.
pub struct SceneRunner {
    mode: Mutex<Scene>,
    /// scdet 호출 시작 횟수
    pub calls: AtomicUsize,
    /// 취소로 "죽인" 횟수
    pub killed: AtomicUsize,
    /// scdet 호출의 `-vf` 인자 (임계값 확인용)
    pub filters: Mutex<Vec<String>>,
}

impl SceneRunner {
    pub fn new(mode: Scene) -> Arc<Self> {
        Arc::new(Self {
            mode: Mutex::new(mode),
            calls: AtomicUsize::new(0),
            killed: AtomicUsize::new(0),
            filters: Mutex::new(Vec::new()),
        })
    }

    pub fn set(&self, mode: Scene) {
        *self.mode.lock().unwrap() = mode;
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn killed(&self) -> usize {
        self.killed.load(Ordering::SeqCst)
    }

    fn mode(&self) -> Scene {
        self.mode.lock().unwrap().clone()
    }
}

fn scdet_filter(args: &[OsString]) -> Option<String> {
    args.iter()
        .map(|a| a.to_string_lossy().into_owned())
        .find(|a| a.contains("scdet="))
}

pub struct Shared(pub Arc<SceneRunner>);

impl CommandRunner for Shared {
    fn run(
        &self,
        tool: Tool,
        _program: &Path,
        args: &[OsString],
        _opts: &RunOptions,
    ) -> MediaResult<CommandOutput> {
        match tool {
            Tool::Ffprobe => {
                let wav = args
                    .last()
                    .is_some_and(|a| a.to_string_lossy().ends_with(".wav"));
                ok_output(if wav {
                    probe_audio_json()
                } else {
                    probe_json()
                })
            }
            Tool::Ffmpeg => ok_output(png_bytes(64, 36)),
        }
    }

    fn run_cancellable(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
        cancel: &dyn Fn() -> bool,
    ) -> MediaResult<CommandOutput> {
        let Some(filter) = scdet_filter(args) else {
            return self.run(tool, program, args, opts);
        };
        let r = &self.0;
        if cancel() {
            return Err(MediaError::Cancelled { tool: "ffmpeg" });
        }
        r.calls.fetch_add(1, Ordering::SeqCst);
        r.filters.lock().unwrap().push(filter);
        loop {
            match r.mode() {
                Scene::Block => {
                    if cancel() {
                        r.killed.fetch_add(1, Ordering::SeqCst);
                        return Err(MediaError::Cancelled { tool: "ffmpeg" });
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Scene::Cuts(out) => return ok_output(out.into_bytes()),
                Scene::Err(make) => return Err(make()),
                Scene::Exit(code, stderr) => {
                    return Ok(CommandOutput {
                        code: Some(code),
                        stdout: Vec::new(),
                        stderr: stderr.to_string(),
                    });
                }
            }
        }
    }
}

/// (시각 초, 점수) 목록 → `metadata=print` 표준출력 묶음.
pub fn cut_text(cuts: &[(f64, f32)]) -> String {
    cuts.iter()
        .enumerate()
        .map(|(i, (secs, score))| {
            format!(
                "frame:{i} pts:0 pts_time:{secs}\nlavfi.scd.mafd={score}\nlavfi.scd.score={score}\nlavfi.scd.time={secs}\n"
            )
        })
        .collect()
}

pub fn runner_tools(runner: &Arc<SceneRunner>) -> Arc<MediaTools> {
    Arc::new(MediaTools::with_paths(
        Some(PathBuf::from("ffprobe-stub")),
        Some(PathBuf::from("ffmpeg-stub")),
        Arc::new(Shared(Arc::clone(runner))),
    ))
}

pub struct Fixture {
    pub p: ProcEnv,
    pub runner: Arc<SceneRunner>,
    /// `on_updated` 로 알린 자산 id
    pub updated: Arc<Mutex<Vec<i64>>>,
}

impl Fixture {
    /// 영상·이미지·음성이 든 라이브러리 + 처리 환경. `with_scene` 이면 장면 실행기를 꽂는다. 아직 시작하지 않는다.
    pub fn new(mode: Scene, with_scene: bool) -> Self {
        let runner = SceneRunner::new(mode);
        let p = ProcEnv::new(runner_tools(&runner));
        p.put("lib/v.mp4", b"not a real video; tools are stubbed");
        p.put("lib/a.png", &png_bytes(200, 100));
        p.put("lib/s.wav", &wav_bytes());
        let updated: Arc<Mutex<Vec<i64>>> = Arc::default();
        if with_scene {
            let sink = Arc::clone(&updated);
            p.env.ingest.register_executor(
                JobKind::Scene,
                Arc::new(SceneExecutor::new(
                    Arc::clone(&p.proc),
                    Arc::new(move |id| sink.lock().unwrap().push(id)),
                )),
            );
        }
        Self { p, runner, updated }
    }

    /// 설정 값을 JSON 원문 그대로 저장한다 (예: `true`, `"low"`, `"yes"`).
    pub fn set_setting(&self, key: &'static str, json: &'static str) {
        self.p
            .env
            .pool
            .write(Priority::High, move |c| {
                c.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    [key, json],
                )?;
                Ok(())
            })
            .unwrap();
    }

    /// 폴더 등록 + 실행기 시작. 장면 작업을 뺀 처리(메타·썸네일·해시)가 끝날 때까지 기다린다.
    pub fn start(&self) {
        self.p
            .env
            .ingest
            .add_folder(self.p.env.path("lib").to_str().unwrap())
            .unwrap();
        self.p.env.ingest.start().unwrap();
        assert!(self.wait_except_scene(), "{}", self.p.jobs_dump());
    }

    pub fn wait_except_scene(&self) -> bool {
        wait_until(Duration::from_secs(120), || {
            self.p.env.count(
                "SELECT COUNT(*) FROM jobs WHERE kind <> 'scene' AND status IN ('pending','running')",
            ) == 0
        })
    }

    pub fn video(&self) -> i64 {
        self.p.asset_of("v.mp4")
    }

    /// 사용자 요청 등록 (`scene_detect_request` 와 같은 mam-db 경로)
    pub fn request(&self, ids: &[i64], opts: RequestOptions, priority: i64) {
        let (ids, now) = (ids.to_vec(), self.p.env.clock_now());
        self.p
            .env
            .pool
            .write(Priority::High, move |c| {
                Ok(scene_cuts::request_tx(c, &ids, &opts, priority, now))
            })
            .unwrap()
            .unwrap();
        self.p.env.ingest.jobs().notify();
    }

    pub fn row(&self, asset_id: i64) -> Option<SceneRow> {
        self.p.env.read(|c| scene_cuts::get(c, asset_id).unwrap())
    }

    pub fn scene_jobs(&self) -> i64 {
        self.p
            .env
            .count("SELECT COUNT(*) FROM jobs WHERE kind = 'scene'")
    }

    pub fn updated_ids(&self) -> Vec<i64> {
        self.updated.lock().unwrap().clone()
    }
}
