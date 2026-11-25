//! 동영상 장면전환(컷) 검출 — FFmpeg `scdet` 필터 (설계 §4.1, 1단계 PoC 로 명령·임계값 확정).
//!
//! 영상 전체를 디코딩하므로 길이 비례 타임아웃·취소를 쓴다. 출력은 `metadata=mode=print:file=-` 로 stdout 파이프에만
//! 받는다 — 원본 폴더는 물론 어디에도 파일을 쓰지 않는다. 검출 명령·임계값 대응이 바뀌면 mam-db 의
//! `SCENE_DETECTOR_VERSION` 을 올린다.

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use crate::error::{MediaError, Result};
use crate::tool::{MediaTools, RunOptions, Tool};

/// 민감도 낮음 — 큰 장면 변화만
pub const THRESHOLD_LOW: f32 = 20.0;
/// 민감도 보통 (FFmpeg 기본값)
pub const THRESHOLD_MEDIUM: f32 = 10.0;
/// 민감도 높음 — 작은 변화도
pub const THRESHOLD_HIGH: f32 = 5.0;

/// 표준출력 상한 (컷 수천 개도 수십 KB — 이상 동작 방어)
pub const MAX_SCENE_OUTPUT: usize = 16 * 1024 * 1024;

const TIMEOUT_MIN: Duration = Duration::from_secs(120);
const TIMEOUT_MAX: Duration = Duration::from_secs(6 * 3600);
const TIMEOUT_UNKNOWN: Duration = Duration::from_secs(2 * 3600);
/// 측정 최악 배속(길이 × 0.77)의 약 4배 여유
const TIMEOUT_FACTOR: i64 = 3;
const TIMEOUT_BASE_MS: i64 = 60_000;
/// 컷 시각 상한 (mam-db 컷 배열이 u32 ms) — 비정상 값 방어
const MAX_CUT_MS: i64 = u32::MAX as i64;
/// 이 문구가 stderr 에 있으면 영상 스트림이 없는 파일이다 (`[0:V:0]` 이 아무 스트림과도 맞지 않음, 커버 아트 제외 —
/// n8.1 실측: `Stream specifier ':V:0' in filtergraph description ... matches no streams.`)
const NO_VIDEO_STDERR: &str = "matches no streams";

#[derive(Debug, Clone, Copy)]
pub struct SceneDetectOptions {
    /// `scdet` 임계값 (0 초과 100 이하)
    pub threshold: f32,
    /// 길이(ms) — 타임아웃 계산용. 모르면 `None`
    pub duration_ms: Option<i64>,
    /// 표준출력 상한 (바이트)
    pub max_output: usize,
}

impl SceneDetectOptions {
    pub fn new(threshold: f32, duration_ms: Option<i64>) -> Self {
        Self {
            threshold,
            duration_ms,
            max_output: MAX_SCENE_OUTPUT,
        }
    }
}

/// 검출된 컷 1개 — 새 장면 첫 프레임의 시각(ms)과 점수.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawCut {
    pub ms: i64,
    pub score: f32,
}

/// 길이 비례 타임아웃: `clamp(길이×3 + 60초, 120초, 6시간)`, 길이를 모르거나 0 이하면 2시간.
pub fn scene_timeout(duration_ms: Option<i64>) -> Duration {
    let Some(d) = duration_ms.filter(|d| *d > 0) else {
        return TIMEOUT_UNKNOWN;
    };
    let ms = d
        .saturating_mul(TIMEOUT_FACTOR)
        .saturating_add(TIMEOUT_BASE_MS);
    let ms = u64::try_from(ms).unwrap_or(0);
    Duration::from_millis(ms).clamp(TIMEOUT_MIN, TIMEOUT_MAX)
}

fn scene_args(path: &Path, threshold: f32, threads: u32) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-nostdin", "-hide_banner", "-v", "error", "-threads"]
        .into_iter()
        .map(Into::into)
        .collect();
    args.push(threads.max(1).to_string().into());
    args.extend(["-noautorotate", "-i"].map(OsString::from));
    args.push(path.as_os_str().to_os_string());
    args.extend(["-an", "-sn", "-dn", "-filter_complex"].map(OsString::from));
    args.push(scene_graph(threshold).into());
    args.extend(["-map", "[k]", "-f", "null", "-"].map(OsString::from));
    args
}

/// 한 번의 디코딩에서 두 검출 체인(일반 / `normalize` 선행)을 돌리고 합집합을 stdout 으로 받는 필터 그래프 (v2).
///
/// - `[0:V:0]` — 첫 영상 스트림(커버 아트 제외). 없으면 `Stream specifier ... matches no streams` 로 실패한다.
/// - `normalize=smoothing=0:independence=0` — 프레임별 대비 확장이라 어두운 장면의 컷도 점수가 오른다(대신 일부 밝은 컷을 잃어 합집합).
/// - 두 체인의 `metadata print` 는 `direct=1`(비버퍼) 이어야 한다 — 버퍼 출력은 파이프에서 줄 중간에 섞인다(실측).
/// - 체인 출력 `[p]`·`[n]` 은 `nullsink`, 실제 출력은 첫 프레임만 통과하는 `[k]` 하나다. 컷이 없는 체인이 있는 채로 출력 스트림이
///   프레임 0개이면 `Could not open encoder before EOF` 로 실패하고, 출력이 모두 끝나면 FFmpeg 가 조기 종료하기 때문이다.
fn scene_graph(threshold: f32) -> String {
    // select 단계가 없으면 모든 프레임이 출력된다 (PoC)
    let chain = format!(
        "scdet=threshold={threshold},metadata=mode=select:key=lavfi.scd.time,\
         metadata=mode=print:file=-:direct=1"
    );
    format!(
        "[0:V:0]scale=w=320:h=-2:flags=fast_bilinear,split=3[a][b][k0];\
         [a]format=gray,{chain}[p];\
         [b]normalize=smoothing=0:independence=0,format=gray,{chain}[n];\
         [k0]select='eq(n,0)'[k];[p]nullsink;[n]nullsink"
    )
}

/// `metadata=mode=print` 출력(4줄 묶음: `frame:` / `lavfi.scd.mafd=` / `lavfi.scd.score=` / `lavfi.scd.time=`)을 해석한다.
/// `lavfi.scd.time` 이 컷 시각이고 점수는 직전 `score` 줄에서 가져온다(없으면 0). `frame:N` 은 출력 순번이라 무시한다.
/// 해석 불가·비유한·음수·범위 밖 값은 버리고, CRLF·줄 누락을 허용한다. 결과는 시각 오름차순, 같은 ms 는 최고 점수 하나.
/// v2 는 두 체인의 출력이 한 stdout 에 시각 순서와 무관하게 섞여 들어오므로(블록 단위), 입력 순서에 의존하지 않고 합집합을 만든다.
pub fn parse_scdet_output(stdout: &str) -> Vec<RawCut> {
    let mut cuts: Vec<RawCut> = Vec::new();
    let mut score: Option<f32> = None;
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("frame:") {
            score = None;
        } else if let Some(v) = line.strip_prefix("lavfi.scd.score=") {
            score = v.trim().parse::<f32>().ok().filter(|s| s.is_finite());
        } else if let Some(v) = line.strip_prefix("lavfi.scd.time=") {
            let ms = v
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|t| t.is_finite() && *t >= 0.0)
                .map(|t| (t * 1000.0).round())
                .filter(|ms| *ms <= MAX_CUT_MS as f64)
                .map(|ms| ms as i64);
            if let Some(ms) = ms {
                cuts.push(RawCut {
                    ms,
                    score: score.take().unwrap_or(0.0).max(0.0),
                });
            }
            score = None;
        }
    }
    cuts.sort_by(|a, b| a.ms.cmp(&b.ms).then(b.score.total_cmp(&a.score)));
    cuts.dedup_by_key(|c| c.ms);
    cuts
}

/// 영상의 장면전환(컷) 후보를 검출한다. `cancel()` 이 true 가 되면 FFmpeg 를 죽이고 `Cancelled` (일시정지·종료).
/// 영상 스트림이 없으면 `Unsupported`, 실행 파일이 없으면 `ToolNotFound`, 제한 시간·출력 초과는 각각 `Timeout`·`OutputTooLarge`.
pub fn detect_scene_cuts(
    tools: &MediaTools,
    path: &Path,
    opts: &SceneDetectOptions,
    cancel: &dyn Fn() -> bool,
) -> Result<Vec<RawCut>> {
    if !(opts.threshold.is_finite() && opts.threshold > 0.0 && opts.threshold <= 100.0) {
        return Err(MediaError::InvalidOutput(
            "scene threshold out of range".into(),
        ));
    }
    let args = scene_args(path, opts.threshold, tools.threads);
    let run = RunOptions {
        timeout: scene_timeout(opts.duration_ms),
        max_stdout: opts.max_output,
    };
    match tools.run_with(Tool::Ffmpeg, &args, run, cancel) {
        Ok(stdout) => Ok(parse_scdet_output(&String::from_utf8_lossy(&stdout))),
        Err(MediaError::ToolFailed { stderr, .. }) if stderr.contains(NO_VIDEO_STDERR) => {
            Err(MediaError::Unsupported("no video stream"))
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    //! 분류: Happy / Boundary / Validation / Time (순수 함수)
    use super::*;

    const SAMPLE: &str = "frame:0    pts:1000    pts_time:1\nlavfi.scd.mafd=99.609\nlavfi.scd.score=99.609\nlavfi.scd.time=1\n\
frame:1    pts:2200    pts_time:2.2\nlavfi.scd.mafd=49.609\nlavfi.scd.score=49.609\nlavfi.scd.time=2.2\n";

    #[test]
    fn t_scene_20_parses_poc_output_groups() {
        // Act
        let cuts = parse_scdet_output(SAMPLE);
        // Assert
        assert_eq!(
            cuts,
            vec![
                RawCut {
                    ms: 1000,
                    score: 99.609
                },
                RawCut {
                    ms: 2200,
                    score: 49.609
                }
            ]
        );
    }

    #[test]
    fn t_scene_20_tolerates_crlf_and_missing_lines() {
        // Arrange - 점수 줄 없음 + CRLF + 시각 줄만
        let text = "frame:0 pts:5\r\nlavfi.scd.time=0.5\r\nlavfi.scd.time=1.2345\r\n";
        // Act
        let cuts = parse_scdet_output(text);
        // Assert
        assert_eq!(
            cuts,
            vec![
                RawCut {
                    ms: 500,
                    score: 0.0
                },
                RawCut {
                    ms: 1235,
                    score: 0.0
                }
            ]
        );
    }

    #[test]
    fn t_scene_20_empty_and_garbage_yield_nothing() {
        for text in [
            "",
            "\n\n",
            "hello\nworld",
            "lavfi.scd.time=abc",
            "lavfi.scd.time=NaN",
            "lavfi.scd.time=inf",
            "lavfi.scd.time=-1",
            "lavfi.scd.time=9999999999",
            "lavfi.scd.time=",
        ] {
            assert!(parse_scdet_output(text).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn t_scene_20_sorts_and_keeps_highest_score_for_same_ms() {
        // Arrange - 역순 + 같은 ms 두 번 + 점수가 비유한인 묶음은 점수 0
        let text = "lavfi.scd.score=30\nlavfi.scd.time=3\n\
                    lavfi.scd.score=12\nlavfi.scd.time=1\n\
                    lavfi.scd.score=40\nlavfi.scd.time=1.0004\n\
                    lavfi.scd.score=nan\nlavfi.scd.time=2\n";
        // Act
        let cuts = parse_scdet_output(text);
        // Assert
        assert_eq!(
            cuts.iter().map(|c| (c.ms, c.score)).collect::<Vec<_>>(),
            vec![(1000, 40.0), (2000, 0.0), (3000, 30.0)]
        );
    }

    #[test]
    fn t_scene_20_score_does_not_leak_into_next_group() {
        // Arrange - 첫 묶음에 time 줄이 없고 다음 묶음에는 score 줄이 없다
        let text = "frame:0\nlavfi.scd.score=77\nframe:1\nlavfi.scd.time=2\n";
        // Act / Assert
        assert_eq!(
            parse_scdet_output(text),
            vec![RawCut {
                ms: 2000,
                score: 0.0
            }]
        );
    }

    #[test]
    fn t_scene_24_union_of_two_chains_interleaved_blocks_dedups_keeping_max() {
        // Arrange - 체인 A(일반)·B(normalize) 블록이 섞여 있고 frame 번호가 각자 0부터 다시 시작한다.
        // 1.0s 는 양쪽(A 34.3 / B 33.0), 3.0s 는 B 만(어두운 컷), 2.0s 는 A 만
        let text = "frame:0 pts:1000 pts_time:1\nlavfi.scd.mafd=36.4\nlavfi.scd.score=34.299\nlavfi.scd.time=1\n\
                    frame:0 pts:1000 pts_time:1\nlavfi.scd.mafd=34.2\nlavfi.scd.score=32.991\nlavfi.scd.time=1\n\
                    frame:1 pts:3000 pts_time:3\nlavfi.scd.mafd=30.5\nlavfi.scd.score=30.518\nlavfi.scd.time=3\n\
                    frame:1 pts:2000 pts_time:2\nlavfi.scd.mafd=35.8\nlavfi.scd.score=35.872\nlavfi.scd.time=2\n";
        // Act
        let cuts = parse_scdet_output(text);
        // Assert - 시각 오름차순, 중복 1.0s 는 최고 점수 하나
        assert_eq!(
            cuts.iter().map(|c| (c.ms, c.score)).collect::<Vec<_>>(),
            vec![(1000, 34.299), (2000, 35.872), (3000, 30.518)]
        );
    }

    #[test]
    fn t_scene_21_graph_runs_both_chains_on_one_decode_with_direct_prints() {
        // Act
        let g = scene_graph(10.0);
        // Assert - 디코딩 1회(split), 체인 2개, 비버퍼 출력, 첫 프레임만 통과하는 단일 출력
        assert!(g.starts_with("[0:V:0]scale=w=320:h=-2:flags=fast_bilinear,split=3"));
        assert_eq!(g.matches("scdet=threshold=10,").count(), 2);
        assert_eq!(g.matches("metadata=mode=print:file=-:direct=1").count(), 2);
        assert_eq!(
            g.matches("normalize=smoothing=0:independence=0,format=gray")
                .count(),
            1
        );
        assert!(g.contains("select='eq(n,0)'[k]"));
        assert_eq!(g.matches("nullsink").count(), 2);
    }

    #[test]
    fn t_scene_22_timeout_formula_and_bounds() {
        let s = Duration::from_secs;
        assert_eq!(scene_timeout(None), s(7200));
        assert_eq!(scene_timeout(Some(0)), s(7200));
        assert_eq!(scene_timeout(Some(-5)), s(7200));
        // 10초 영상: 3*10+60 = 90 -> 하한 120
        assert_eq!(scene_timeout(Some(10_000)), s(120));
        // 경계: 3*d + 60 = 120 -> d = 20초
        assert_eq!(scene_timeout(Some(20_000)), s(120));
        assert_eq!(scene_timeout(Some(20_001)), Duration::from_millis(120_003));
        // 1시간 영상: 3*3600+60
        assert_eq!(scene_timeout(Some(3_600_000)), s(10_860));
        // 상한 6시간
        assert_eq!(scene_timeout(Some(100 * 3_600_000)), s(21_600));
        assert_eq!(scene_timeout(Some(i64::MAX)), s(21_600));
    }
}
