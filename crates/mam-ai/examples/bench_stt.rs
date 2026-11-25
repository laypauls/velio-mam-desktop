//! STT 실시간 배율(RTF)·RSS·정확도(CER/WER) 측정 - 제품 코드(`SttEngine`)를 그대로 쓰고 실행기만 측정용으로 바꾼다.
//!
//! `bench_stt --whisper <whisper-cli.exe> --ffmpeg <ffmpeg.exe> --model <ggml.bin> --dir <wav/txt 폴더> --lang ko|en
//!   [--threads 4] [--concurrency 1] [--long] [--language auto|ko|en] [--limit N]`
//!
//! `<dir>` 에는 `ko01.wav` + `ko01.txt`(정답 원문) 쌍이 있어야 한다 (Windows SAPI 합성 음성 - 실제 사람 음성보다 쉽다).
//! RSS 는 whisper-cli **자식 프로세스**의 최대 작업 집합(`PeakWorkingSetSize`)을 20ms 간격으로 조회해 얻는다.

#[path = "common/mod.rs"]
mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Args, fail, print_conditions, stats, sysinfo};
use mam_ai::AiError;
use mam_ai::local::stt::{
    CommandOutput, CommandRunner, Language, RunOptions, SttConfig, SttEngine, SttPaths, Tool,
};

#[cfg(windows)]
fn peak_ws(pid: u32) -> u64 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: 핸들을 열어 카운터만 읽고 즉시 닫는다. 구조체는 0 초기화 후 cb 를 채운다.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return 0;
        }
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = GetProcessMemoryInfo(h, &mut c, c.cb);
        CloseHandle(h);
        if ok != 0 {
            c.PeakWorkingSetSize as u64
        } else {
            0
        }
    }
}

/// 제품 실행기와 같은 방식(낮은 우선순위, 타임아웃)이되 자식의 최대 RSS 와 소요 시간을 기록한다.
#[derive(Default)]
struct Measuring {
    log: Mutex<Vec<(Tool, Duration, u64)>>,
}

impl CommandRunner for Measuring {
    fn run(
        &self,
        tool: Tool,
        program: &Path,
        args: &[OsString],
        opts: &RunOptions,
    ) -> Result<CommandOutput, AiError> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000 | if opts.low_priority { 0x4000 } else { 0 });
        }
        let started = Instant::now();
        let mut child = cmd.spawn().map_err(|e| AiError::Io {
            path: program.into(),
            source: e,
        })?;
        let mut peak = 0u64;
        let status = loop {
            if let Some(s) = child.try_wait().map_err(|e| AiError::Io {
                path: program.into(),
                source: e,
            })? {
                break s;
            }
            peak = peak.max(peak_ws(child.id()));
            if started.elapsed() > opts.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AiError::Timeout {
                    tool: tool.name(),
                    timeout: opts.timeout,
                });
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        self.log
            .lock()
            .unwrap()
            .push((tool, started.elapsed(), peak));
        Ok(CommandOutput {
            code: status.code(),
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

/// (이름, 오디오 초, 처리 초, 감지 언어, 전사문)
type Row = (String, f64, f64, String, String);

fn norm(s: &str) -> Vec<char> {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn edit<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1)
                .min(cur[j - 1] + 1)
                .min(prev[j - 1] + (a[i - 1] != b[j - 1]) as usize);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn wav_seconds(p: &Path) -> f64 {
    let b = std::fs::read(p).unwrap_or_else(|e| fail(&format!("{}: {e}", p.display())));
    let byte_rate = u32::from_le_bytes([b[28], b[29], b[30], b[31]]) as f64;
    (b.len() - 44) as f64 / byte_rate
}

fn main() {
    let a = Args::parse();
    let lang = a.require("lang").to_string();
    let threads: usize = a.num("threads", 4);
    let conc: usize = a.num("concurrency", 1);
    println!(
        "\n== bench_stt lang={lang} threads={threads} concurrency={conc} model={} ==",
        Path::new(a.require("model"))
            .file_name()
            .unwrap()
            .to_string_lossy()
    );
    let cond = print_conditions();

    let dir = PathBuf::from(a.require("dir"));
    let mut names: Vec<String> = if a.has("long") {
        vec![format!("{lang}long")]
    } else {
        (1..=10).map(|i| format!("{lang}{i:02}")).collect()
    };
    names.truncate(a.num("limit", usize::MAX));

    let runner = Arc::new(Measuring::default());
    let language = match a.get("language").unwrap_or("auto") {
        "auto" => Language::Auto,
        l => Language::Fixed(l.to_string()),
    };
    let engine = Arc::new(SttEngine::new(
        runner.clone(),
        SttPaths {
            whisper_cli: a.require("whisper").into(),
            ffmpeg: a.require("ffmpeg").into(),
            model: a.require("model").into(),
        },
        SttConfig {
            threads,
            language,
            ..Default::default()
        },
    ));
    let work = std::env::temp_dir().join("mam-ai-bench-stt");
    std::fs::create_dir_all(&work).expect("work");

    // 워밍업(모델 파일을 OS 캐시에 올린다) - 결과는 버린다
    let first = dir.join(format!("{}.wav", names[0]));
    engine.transcribe(&first, &work, None).expect("warmup");
    runner.log.lock().unwrap().clear();

    let queue = Arc::new(Mutex::new(names.clone()));
    let results: Arc<Mutex<Vec<Row>>> = Arc::default(); // (name, secs, wall, lang, text)
    let started = Instant::now();
    let workers: Vec<_> = (0..conc)
        .map(|_| {
            let (queue, results, engine, work, dir) = (
                queue.clone(),
                results.clone(),
                engine.clone(),
                work.clone(),
                dir.clone(),
            );
            std::thread::spawn(move || {
                loop {
                    let Some(name) = queue.lock().unwrap().pop() else {
                        break;
                    };
                    let wav = dir.join(format!("{name}.wav"));
                    let t = Instant::now();
                    let tr = engine
                        .transcribe(&wav, &work, None)
                        .unwrap_or_else(|e| fail(&e.to_string()));
                    results.lock().unwrap().push((
                        name,
                        wav_seconds(&wav),
                        t.elapsed().as_secs_f64(),
                        tr.language.clone(),
                        tr.full_text(),
                    ));
                }
            })
        })
        .collect();
    for w in workers {
        w.join().expect("worker");
    }
    let total = started.elapsed().as_secs_f64();

    let mut rs = results.lock().unwrap().clone();
    rs.sort_by(|a, b| a.0.cmp(&b.0));
    let (mut cer_e, mut cer_n, mut wer_e, mut wer_n, mut lang_ok) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    println!("\n| 파일 | 길이(s) | 처리(s) | RTF | 감지 언어 |\n|---|---:|---:|---:|---|");
    for (name, secs, wall, l, text) in &rs {
        let truth = std::fs::read_to_string(dir.join(format!("{name}.txt"))).expect("truth");
        let (t, h) = (norm(&truth), norm(text));
        cer_e += edit(&t, &h);
        cer_n += t.len();
        let (tw, hw) = (words(&truth), words(text));
        wer_e += edit(&tw, &hw);
        wer_n += tw.len();
        lang_ok += (l == &lang) as usize;
        println!(
            "| {name} | {secs:.1} | {wall:.2} | {:.3} | {l} |",
            wall / secs
        );
    }
    let rtf: Vec<Duration> = rs
        .iter()
        .map(|r| Duration::from_secs_f64(r.2 / r.1 * 1000.0))
        .collect(); // RTF x1e6 ms 로 스케일해 stats 재사용
    let s = stats::summarize(&rtf);
    let audio: f64 = rs.iter().map(|r| r.1).sum();
    let log = runner.log.lock().unwrap();
    let whisper_peak = log
        .iter()
        .filter(|l| l.0 == Tool::WhisperCli)
        .map(|l| l.2)
        .max()
        .unwrap_or(0);
    let whisper_secs: f64 = log
        .iter()
        .filter(|l| l.0 == Tool::WhisperCli)
        .map(|l| l.1.as_secs_f64())
        .sum();
    let ffmpeg_secs: f64 = log
        .iter()
        .filter(|l| l.0 == Tool::Ffmpeg)
        .map(|l| l.1.as_secs_f64())
        .sum();
    println!("\n| 지표 | 값 |\n|---|---:|");
    println!("| 파일 수 / 총 오디오 길이 | {} / {audio:.1}s |", rs.len());
    println!(
        "| RTF (처리시간/오디오) P50 / P95 / max | {:.3} / {:.3} / {:.3} |",
        s.p50_ms / 1e6,
        s.p95_ms / 1e6,
        s.max_ms / 1e6
    );
    println!(
        "| 전체 처리량 (오디오초/벽시계초) | {:.2}x (벽시계 {total:.1}s, 동시 {conc}) |",
        audio / total
    );
    println!(
        "| whisper-cli 누적 {whisper_secs:.1}s / ffmpeg 변환 누적 {ffmpeg_secs:.1}s | 변환 비중 {:.1}% |",
        ffmpeg_secs / (whisper_secs + ffmpeg_secs) * 100.0
    );
    println!(
        "| whisper-cli 프로세스 최대 RSS (MB, 프로세스 1개 기준) | {:.0} |",
        sysinfo::mb(whisper_peak)
    );
    println!(
        "| 동시 {conc} 개 최대 RSS 합 상한 (MB) | {:.0} |",
        sysinfo::mb(whisper_peak) * conc as f64
    );
    println!(
        "| CER (공백·기호 제외) | {:.3} |",
        cer_e as f64 / cer_n as f64
    );
    println!("| WER (공백 단위) | {:.3} |", wer_e as f64 / wer_n as f64);
    println!("| 언어 감지 정확 | {lang_ok}/{} |", rs.len());
    println!(
        "JSON: {{\"tool\":\"bench_stt\",\"lang\":\"{lang}\",\"model\":{:?},\"threads\":{threads},\"concurrency\":{conc},\"files\":{},\"audio_s\":{audio:.1},\"rtf_p50\":{:.3},\"rtf_p95\":{:.3},\"throughput_x\":{:.2},\"whisper_peak_mb\":{:.0},\"cer\":{:.4},\"wer\":{:.4},\"lang_ok\":{lang_ok},\"cpu\":{:?}}}",
        Path::new(a.require("model"))
            .file_name()
            .unwrap()
            .to_string_lossy(),
        rs.len(),
        s.p50_ms / 1e6,
        s.p95_ms / 1e6,
        audio / total,
        sysinfo::mb(whisper_peak),
        cer_e as f64 / cer_n as f64,
        wer_e as f64 / wer_n as f64,
        cond.cpu
    );
}
