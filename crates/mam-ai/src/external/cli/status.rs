//! CLI 버전·로그인 상태 해석. 상태 명령이 실패하면 **알 수 없음(`None`)** 이다 — 로그아웃으로 단정하지 않는다
//! (구버전은 `auth status` 가 없어 도움말을 출력하고 실패하는데, 그 CLI 는 로그인된 채 정상 동작할 수 있다).

use serde::Serialize;
use serde_json::Value;

/// 지원하는 최소 CLI 버전.
///
/// 근거: 쓰는 플래그(`--tools ""`, `--setting-sources`, `--json-schema`, stream-json 입력, `auth status --json`)를
/// 선례(mymy-v4 nle-sidecar)가 2.1.280 이상에서 검증했고, MAM 은 2.1.318 에서 실측했다(2.1.280~2.1.317 구간은 미실측).
pub const MIN_VERSION: Version = Version(2, 1, 280);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// `claude --version` 출력(예: `2.1.318 (Claude Code)`)에서 첫 `x.y.z` 를 뽑는다.
pub fn parse_version(out: &str) -> Option<Version> {
    out.split_whitespace().find_map(|tok| {
        let tok = tok.trim_start_matches('v');
        let mut it = tok.split('.');
        let a = it.next()?.parse().ok()?;
        let b = it.next()?.parse().ok()?;
        let c_raw = it.next()?;
        let digits: String = c_raw.chars().take_while(char::is_ascii_digit).collect();
        let c = digits.parse().ok()?;
        Some(Version(a, b, c))
    })
}

/// `claude auth status --json` 의 계정 정보 (UI `CliAccount`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliAccount {
    pub logged_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// `subscriptionType` (예: `max`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
}

/// UI `CliStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    pub found: bool,
    pub version: Option<String>,
    /// 최소 지원 버전 이상
    pub supported: bool,
    /// `None` = 알 수 없음
    pub account: Option<CliAccount>,
    /// 이 프로세스 환경에 `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` 이 있다 (값은 읽지 않는다)
    pub api_key_env: bool,
}

/// `auth status --json` 출력 -> 계정. JSON 이 아니거나 `loggedIn` 이 불리언이 아니면 `None`.
pub fn parse_auth_status(stdout: &str) -> Option<CliAccount> {
    let v: Value = serde_json::from_str(stdout.trim()).ok()?;
    let logged_in = v.get("loggedIn")?.as_bool()?;
    let text = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(CliAccount {
        logged_in,
        email: text("email"),
        plan: text("subscriptionType"),
        auth_method: text("authMethod"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Boundary / Validation
    #[test]
    fn t_aicli_ver_01_parses_cli_version_output() {
        assert_eq!(
            parse_version("2.1.318 (Claude Code)\n"),
            Some(Version(2, 1, 318))
        );
        assert_eq!(parse_version("v2.1.280"), Some(Version(2, 1, 280)));
        assert_eq!(parse_version("2.1.318-beta.1"), Some(Version(2, 1, 318)));
    }

    #[test]
    fn t_aicli_ver_02_rejects_non_version_text() {
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("error: unknown option"), None);
        assert_eq!(parse_version("2.1"), None);
    }

    #[test]
    fn t_aicli_ver_03_minimum_version_boundary() {
        assert!(Version(2, 1, 280) >= MIN_VERSION);
        assert!(Version(2, 1, 279) < MIN_VERSION);
        assert!(Version(2, 2, 0) >= MIN_VERSION);
        assert!(Version(1, 9, 999) < MIN_VERSION);
    }

    #[test]
    fn t_aicli_auth_01_parses_logged_in_account() {
        let out = r#"{"loggedIn":true,"authMethod":"claude.ai","subscriptionType":"max","email":"a@b.c","orgName":"x"}"#;
        let acc = parse_auth_status(out).unwrap();
        assert!(acc.logged_in);
        assert_eq!(acc.plan.as_deref(), Some("max"));
        assert_eq!(acc.email.as_deref(), Some("a@b.c"));
        assert_eq!(acc.auth_method.as_deref(), Some("claude.ai"));
    }

    #[test]
    fn t_aicli_auth_02_explicit_logged_out_is_not_unknown() {
        let acc = parse_auth_status(r#"{"loggedIn":false}"#).unwrap();
        assert!(!acc.logged_in);
        assert_eq!(acc.email, None);
    }

    #[test]
    fn t_aicli_auth_03_unparseable_output_is_unknown_not_logged_out() {
        for bad in [
            "",
            "Usage: claude [options]",
            r#"{"loggedIn":"yes"}"#,
            r#"{"x":1}"#,
            "[]",
        ] {
            assert_eq!(parse_auth_status(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn t_aicli_auth_04_serializes_camel_case_and_skips_missing_fields() {
        let acc = parse_auth_status(r#"{"loggedIn":true,"subscriptionType":"pro"}"#).unwrap();
        let v = serde_json::to_value(&acc).unwrap();
        assert_eq!(v, serde_json::json!({"loggedIn": true, "plan": "pro"}));
    }
}
