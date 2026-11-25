//! 자산 처리(P2-3) 통합 테스트 환경 - `Env`(실제 SQLite·임시 볼륨) + 처리 환경 + 외부 도구 대역.
#![allow(dead_code)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use image::{ImageFormat, Rgb, RgbImage};
use mam_core::Blake3Hasher;
use mam_core::processing::ProcessingEnv;
use mam_media::error::{MediaError, Result as MediaResult};
use mam_media::tool::{CommandOutput, CommandRunner, MediaTools, RunOptions, Tool};

use super::env::{Env, wait_until};

/// 테스트가 만든 PNG/JPEG 바이트 (왼쪽 빨강 / 오른쪽 파랑).
pub fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(w, h, |x, _| {
        if x < w / 2 {
            Rgb([255, 0, 0])
        } else {
            Rgb([0, 0, 255])
        }
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("mam-media")
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// 최소 유효 WAV (무음 0.1초) - 음성 자산용.
pub fn wav_bytes() -> Vec<u8> {
    let data = vec![0u8; 1600];
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&8000u32.to_le_bytes());
    v.extend_from_slice(&16000u32.to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(&data);
    v
}

pub type RunnerFn = dyn Fn(Tool, &[OsString]) -> MediaResult<CommandOutput> + Send + Sync;

/// 대역 실행기: 호출마다 클로저가 결과를 정한다 (실제 ffmpeg 없이 성공·실패·시간 초과를 재현).
pub struct StubRunner(pub Box<RunnerFn>);

impl CommandRunner for StubRunner {
    fn run(
        &self,
        tool: Tool,
        _program: &Path,
        args: &[OsString],
        _opts: &RunOptions,
    ) -> MediaResult<CommandOutput> {
        (self.0)(tool, args)
    }
}

pub fn ok_output(stdout: Vec<u8>) -> MediaResult<CommandOutput> {
    Ok(CommandOutput {
        code: Some(0),
        stdout,
        stderr: String::new(),
    })
}

/// 10초 H.264/AAC 640x360 mp4 를 흉내 내는 ffprobe JSON.
pub fn probe_audio_json() -> Vec<u8> {
    br#"{"streams":[{"codec_type":"audio","codec_name":"pcm_s16le","sample_rate":"8000","channels":1}],
        "format":{"format_name":"wav","duration":"0.1","size":"1644","bit_rate":"128000"}}"#
        .to_vec()
}

pub fn probe_json() -> Vec<u8> {
    br#"{"streams":[{"codec_type":"video","codec_name":"h264","width":640,"height":360,"avg_frame_rate":"30/1"},
        {"codec_type":"audio","codec_name":"aac","sample_rate":"44100","channels":2}],
        "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10.0","size":"1000","bit_rate":"800000",
        "tags":{"creation_time":"2024-05-01T12:34:56.000000Z"}}}"#
        .to_vec()
}

/// 정상 동작하는 대역 도구: ffprobe -> `probe_json`, ffmpeg -> 작은 PNG 프레임. `on_ffmpeg` 로 호출 중 부수 동작을 끼운다.
pub fn stub_tools(on_ffmpeg: impl Fn() + Send + Sync + 'static) -> Arc<MediaTools> {
    let runner = StubRunner(Box::new(move |tool, args| match tool {
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
        Tool::Ffmpeg => {
            on_ffmpeg();
            ok_output(png_bytes(64, 36))
        }
    }));
    Arc::new(MediaTools::with_paths(
        Some(PathBuf::from("ffprobe-stub")),
        Some(PathBuf::from("ffmpeg-stub")),
        Arc::new(runner),
    ))
}

/// 모든 호출이 실패하는 대역 도구.
pub fn failing_tools(make: impl Fn() -> MediaError + Send + Sync + 'static) -> Arc<MediaTools> {
    let runner = StubRunner(Box::new(move |_, _| Err(make())));
    Arc::new(MediaTools::with_paths(
        Some(PathBuf::from("ffprobe-stub")),
        Some(PathBuf::from("ffmpeg-stub")),
        Arc::new(runner),
    ))
}

/// ffmpeg·ffprobe 가 아예 없는 상태 (영상·음성 메타는 ToolNotFound).
pub fn no_tools() -> Arc<MediaTools> {
    Arc::new(MediaTools::with_paths(
        None,
        None,
        Arc::new(StubRunner(Box::new(|_, _| unreachable!()))),
    ))
}

/// PATH 의 실제 ffmpeg/ffprobe. 없으면 `None` (해당 테스트는 건너뛴다).
pub fn real_tools() -> Option<Arc<MediaTools>> {
    use mam_media::tool::{ProcessRunner, ToolConfig};
    let tools = MediaTools::discover(&ToolConfig::default(), Arc::new(ProcessRunner));
    (tools.has_ffmpeg() && tools.has_ffprobe()).then(|| Arc::new(tools))
}

pub struct ProcEnv {
    pub env: Env,
    pub proc: Arc<ProcessingEnv>,
    pub cache: PathBuf,
}

impl ProcEnv {
    /// 처리를 켠 환경. 아직 `ingest.start()` 하지 않는다.
    pub fn new(tools: Arc<MediaTools>) -> Self {
        Self::with_env(Env::new(), tools)
    }

    pub fn with_env(env: Env, tools: Arc<MediaTools>) -> Self {
        Self::with_hasher(env, tools, Arc::new(Blake3Hasher))
    }

    /// 해시 계산기를 주입한다 (호출 횟수 세기·실패 주입 시험용).
    pub fn with_hasher(
        env: Env,
        tools: Arc<MediaTools>,
        hasher: Arc<dyn mam_core::ContentHasher + Send + Sync>,
    ) -> Self {
        let cache = env.data.join("cache");
        fs::create_dir_all(&cache).unwrap();
        let clock: mam_core::SharedClock = env.clock.clone();
        let proc = ProcessingEnv::new(Arc::clone(&env.pool), clock, tools, hasher, cache.clone());
        env.ingest.enable_processing(Arc::clone(&proc));
        Self { env, proc, cache }
    }

    /// 볼륨에 파일을 쓴다 (원하는 바이트 그대로).
    pub fn put(&self, rel: &str, bytes: &[u8]) -> PathBuf {
        let p = self.env.path(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, bytes).unwrap();
        p
    }

    /// 대기·실행 중인 처리 작업(meta/thumb/hash)이 없어질 때까지 기다린다.
    ///
    /// 일시 오류(부하 시 시간 초과 등)로 백오프에 들어간 작업은 수동 시계를 그 시각까지 당겨 곧바로 다시 실행시킨다
    /// (실제 시간을 기다리지 않는다). 종결 상태(완료·`failed`)만 기다리므로 부하와 무관하게 결정적이다.
    /// 상한(5분)은 멈춤 감지용일 뿐 정상 경로의 대기 시간이 아니다.
    pub fn wait_idle(&self) -> bool {
        wait_until(Duration::from_secs(300), || {
            let busy = self
                .env
                .count("SELECT COUNT(*) FROM jobs WHERE status IN ('pending','running')");
            if busy == 0 {
                return true;
            }
            let running = self
                .env
                .count("SELECT COUNT(*) FROM jobs WHERE status = 'running'");
            if running == 0 {
                let now = self.env.clock_now();
                let next = self.env.count(
                    "SELECT COALESCE(MAX(not_before), 0) FROM jobs WHERE status = 'pending'",
                );
                if next > now {
                    self.env.clock.advance(next - now);
                }
            }
            false
        })
    }

    pub fn deriv(&self, asset_id: i64, kind: &str) -> Option<(String, Option<String>)> {
        self.env.read(|c| {
            c.query_row(
                "SELECT state, error FROM asset_derivatives WHERE asset_id = ?1 AND kind = ?2",
                (asset_id, kind),
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
        })
    }

    pub fn asset_of(&self, name: &str) -> i64 {
        self.env.count(&format!(
            "SELECT asset_id FROM file_locations WHERE rel_path LIKE '%{name}' AND detached_at IS NULL"
        ))
    }

    pub fn revision(&self, asset_id: i64) -> i64 {
        self.env.count(&format!(
            "SELECT revision FROM media_assets WHERE asset_id = {asset_id}"
        ))
    }

    pub fn shutdown(&self) {
        self.env.ingest.shutdown();
    }
}

impl ProcEnv {
    /// 남은 작업 행 (진단용: 대기가 끝나지 않을 때 원인을 출력한다).
    pub fn jobs_dump(&self) -> String {
        self.env.read(|c| {
            let mut stmt = c
                .prepare("SELECT kind, status, attempts, not_before, last_error FROM jobs")
                .unwrap();
            let rows = stmt
                .query_map([], |r| {
                    Ok(format!(
                        "{}/{} attempts={} not_before={} err={:?}",
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, Option<String>>(4)?
                    ))
                })
                .unwrap();
            rows.map(Result::unwrap).collect::<Vec<_>>().join("\n")
        })
    }
}
