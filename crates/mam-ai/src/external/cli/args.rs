//! CLI 인자·입력 조립. 인자 조합은 Phase 0 실측(2.1.318)으로 고정한다 — 바꾸면 도구 비활성 보장이 깨질 수 있다.
//!
//! - `--tools ""` + `--disallowedTools ...` + `--permission-mode dontAsk` + `--strict-mcp-config`: 파일·셸·웹 도구 0개
//!   (`--allowedTools` 만으로는 사용자 전역 `bypassPermissions` 설정에 뚫리는 것이 선례에서 확인됨)
//! - `--setting-sources project`: 사용자 전역 설정(훅·권한)을 읽지 않는다. 작업 폴더는 빈 폴더라 프로젝트 설정도 없다
//! - `--no-session-persistence`: 세션 기록을 디스크에 남기지 않는다
//! - `--safe-mode` + `--system-prompt <고정 문구>`: **사용자 컨텍스트가 요청에 섞여 외부(Anthropic)로 나가는 것을 막는다.**
//!   실측(2.1.318, 구독 로그인): 위 두 플래그 없이는 cwd 가 %TEMP% 아래여도 사용자 전역 `~/.claude/CLAUDE.md` 가 로드됐고,
//!   cwd 가 저장소 안이면 프로젝트 CLAUDE.md·자동 메모리 색인·git status 까지 실려 cache-creation 토큰이 925 -> 21,706 이 됐다.
//!   두 플래그를 함께 쓰고 cwd 를 OS 임시 폴더 아래 빈 폴더로 두면 모델이 CLAUDE.md·메모리·git 상태를 전혀 보지 못했고,
//!   이미지 분석·`--json-schema`·구독 인증(`apiKeySource: none`)은 그대로 동작했다. 둘 중 하나라도 빼지 말 것.
//! - `--bare` 는 구독 OAuth 를 읽지 않아 쓸 수 없다
//! - 이미지는 argv 가 아니라 stdin 의 stream-json 메시지(base64 블록)로만 전달한다 — 원본 경로가 인자에 들어가지 않는다

use std::ffi::OsString;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::json;

use crate::external::payload::PreparedImage;

/// 모델 미지정 시 CLI 별칭.
pub const DEFAULT_CLI_MODEL: &str = "haiku";

/// 고정 시스템 프롬프트 — CLI 기본 시스템 프롬프트(환경·git·메모리 절 포함)를 대체한다.
/// 사용자 정보·경로를 넣지 않는다.
pub const SYSTEM_PROMPT: &str = "You are an assistant embedded in a media asset manager. Describe the supplied media and reply only in the requested format. No tools are available.";

const DISALLOWED_TOOLS: &str =
    "Bash,Read,Write,Edit,Glob,Grep,WebFetch,WebSearch,Agent,NotebookEdit";

/// 분석 출력 스키마 (`prompt::Parsed` 와 같은 필드). Phase 0 에서 5/5 `structured_output` 으로 수신됐다.
pub const OUTPUT_SCHEMA: &str = r#"{"type":"object","properties":{"description":{"type":"string"},"keywords":{"type":"array","items":{"type":"string"}},"ocrText":{"type":"string"},"summary":{"type":"string"},"language":{"type":"string"}},"required":["description","keywords","ocrText","summary","language"]}"#;

/// 요청 실행 인자. `schema` 가 `Some` 이면 구조화 출력(분석), `None` 이면 일반 텍스트(연결 확인).
pub fn build_args(model: &str, schema: Option<&str>) -> Vec<OsString> {
    let mut a: Vec<OsString> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--tools",
        "",
        "--strict-mcp-config",
        "--permission-mode",
        "dontAsk",
        "--setting-sources",
        "project",
        "--disallowedTools",
        DISALLOWED_TOOLS,
        "--model",
        model,
        "--no-session-persistence",
        "--safe-mode",
        "--system-prompt",
        SYSTEM_PROMPT,
    ]
    .iter()
    .map(OsString::from)
    .collect();
    if let Some(s) = schema {
        a.push("--json-schema".into());
        a.push(s.into());
    }
    a
}

/// stdin 으로 보낼 한 줄 JSON 메시지 (텍스트 블록 + 이미지 base64 블록).
pub fn request_line(prompt: &str, images: &[PreparedImage]) -> Vec<u8> {
    let mut content = vec![json!({"type": "text", "text": prompt})];
    for img in images {
        content.push(json!({
            "type": "image",
            "source": {"type": "base64", "media_type": img.mime, "data": B64.encode(&img.bytes)}
        }));
    }
    let msg = json!({"type": "user", "message": {"role": "user", "content": content}});
    let mut line = msg.to_string().into_bytes();
    line.push(b'\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(a: &[OsString]) -> Vec<String> {
        a.iter().map(|s| s.to_string_lossy().into_owned()).collect()
    }

    // 분류: Happy / Business rule
    #[test]
    fn t_aicli_args_01_isolation_flags_are_fixed_and_tools_are_empty() {
        let a = strings(&build_args("haiku", Some(OUTPUT_SCHEMA)));
        let pos = a.iter().position(|s| s == "--tools").unwrap();
        assert_eq!(a[pos + 1], "", "--tools 값은 빈 문자열이어야 한다");
        for flag in [
            "--strict-mcp-config",
            "--no-session-persistence",
            "--verbose",
        ] {
            assert!(a.contains(&flag.to_string()), "{flag}");
        }
        let pm = a.iter().position(|s| s == "--permission-mode").unwrap();
        assert_eq!(a[pm + 1], "dontAsk");
        let ss = a.iter().position(|s| s == "--setting-sources").unwrap();
        assert_eq!(a[ss + 1], "project");
        let dt = a.iter().position(|s| s == "--disallowedTools").unwrap();
        for t in ["Bash", "Read", "Write", "Edit", "WebFetch", "Agent"] {
            assert!(a[dt + 1].split(',').any(|x| x == t), "{t}");
        }
        // 사용자 컨텍스트 차단 (실측 근거는 모듈 문서)
        assert!(a.contains(&"--safe-mode".to_string()));
        let sp = a.iter().position(|s| s == "--system-prompt").unwrap();
        assert_eq!(a[sp + 1], SYSTEM_PROMPT);
        assert!(!a[sp + 1].is_empty());
        assert!(!a.contains(&"--bare".to_string()));
        assert!(!a.contains(&"--allowedTools".to_string()));
    }

    #[test]
    fn t_aicli_args_02_model_and_schema_are_passed_only_when_given() {
        let with = strings(&build_args("claude-haiku-5-5", Some(OUTPUT_SCHEMA)));
        let m = with.iter().position(|s| s == "--model").unwrap();
        assert_eq!(with[m + 1], "claude-haiku-5-5");
        assert!(with.contains(&"--json-schema".to_string()));
        let without = strings(&build_args("haiku", None));
        assert!(!without.contains(&"--json-schema".to_string()));
    }

    #[test]
    fn t_aicli_args_03_schema_is_valid_json_with_all_output_fields() {
        let v: serde_json::Value = serde_json::from_str(OUTPUT_SCHEMA).unwrap();
        for f in ["description", "keywords", "ocrText", "summary", "language"] {
            assert!(v["properties"].get(f).is_some(), "{f}");
        }
    }

    #[test]
    fn t_aicli_args_04_request_line_carries_text_and_base64_images_on_one_line() {
        let img = PreparedImage {
            bytes: vec![1, 2, 3, 4],
            mime: "image/jpeg",
        };
        let line = request_line("describe", &[img.clone(), img]);
        assert_eq!(line.last(), Some(&b'\n'));
        assert_eq!(line.iter().filter(|b| **b == b'\n').count(), 1);
        let v: serde_json::Value = serde_json::from_slice(&line).unwrap();
        let content = v["message"]["content"].as_array().unwrap();
        assert_eq!(content.len(), 3);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["source"]["media_type"], "image/jpeg");
        assert_eq!(content[1]["source"]["data"], B64.encode([1, 2, 3, 4]));
    }
}
