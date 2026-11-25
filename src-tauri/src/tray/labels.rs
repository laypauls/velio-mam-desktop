//! 트레이 문구 상태 — 프론트(i18n)가 번역한 문구를 보관한다. 시작 직후(프론트가 넘기기 전) 기본값은 영어.

use std::sync::{LazyLock, Mutex};

use serde::Deserialize;

pub const TOOLTIP: &str = crate::native::APP_DISPLAY_NAME;

/// 문구 한 개의 길이 상한(문자 수) — 메뉴가 깨지지 않게 자른다
const MAX_LABEL_CHARS: usize = 200;

/// 트레이 문구 묶음 (프론트 `TrayLabels` 와 동일한 camelCase 키)
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayLabels {
    pub open: String,
    pub pause_jobs: String,
    pub resume_jobs: String,
    pub quit: String,
    /// 보간이 끝난 종료 대기 툴팁 문구
    pub ai_waiting: String,
}

impl Default for TrayLabels {
    fn default() -> Self {
        Self {
            open: "Open".into(),
            pause_jobs: "Pause jobs".into(),
            resume_jobs: "Resume jobs".into(),
            quit: "Quit".into(),
            ai_waiting: concat!(
                crate::app_display_name!(),
                " - Waiting for AI requests to finish…"
            )
            .into(),
        }
    }
}

/// 입력 한 필드 정리: 앞뒤 공백 제거, 비면 기존 값 유지, 200자(문자 단위) 상한
fn sanitize_field(incoming: &str, current: &str) -> String {
    let trimmed = incoming.trim();
    if trimmed.is_empty() {
        return current.to_string();
    }
    trimmed.chars().take(MAX_LABEL_CHARS).collect()
}

impl TrayLabels {
    /// 들어온 문구를 정리해 `self`(기존 문구) 위에 반영한 새 묶음을 만든다
    pub fn merged_with(&self, incoming: &TrayLabels) -> TrayLabels {
        TrayLabels {
            open: sanitize_field(&incoming.open, &self.open),
            pause_jobs: sanitize_field(&incoming.pause_jobs, &self.pause_jobs),
            resume_jobs: sanitize_field(&incoming.resume_jobs, &self.resume_jobs),
            quit: sanitize_field(&incoming.quit, &self.quit),
            ai_waiting: sanitize_field(&incoming.ai_waiting, &self.ai_waiting),
        }
    }

    /// 일시정지 상태에 맞는 토글 항목 문구
    pub fn toggle(&self, paused: bool) -> &str {
        if paused { &self.resume_jobs } else { &self.pause_jobs }
    }
}

static CURRENT: LazyLock<Mutex<TrayLabels>> = LazyLock::new(|| Mutex::new(TrayLabels::default()));

/// 현재 보관된 문구 복사본
pub fn current() -> TrayLabels {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// 정리해 보관하고 반영된 문구를 돌려준다
pub fn store(incoming: &TrayLabels) -> TrayLabels {
    let mut guard = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    let merged = guard.merged_with(incoming);
    *guard = merged.clone();
    merged
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Boundary / Validation / Business rule
    use super::*;

    fn all(s: &str) -> TrayLabels {
        TrayLabels {
            open: s.into(),
            pause_jobs: s.into(),
            resume_jobs: s.into(),
            quit: s.into(),
            ai_waiting: s.into(),
        }
    }

    #[test]
    fn t_tray_01_defaults_are_english() {
        // Arrange / Act
        let d = TrayLabels::default();
        // Assert
        assert_eq!(d.open, "Open");
        assert_eq!(d.pause_jobs, "Pause jobs");
        assert_eq!(d.resume_jobs, "Resume jobs");
        assert_eq!(d.quit, "Quit");
        assert!(d.ai_waiting.starts_with(TOOLTIP));
        assert!(d.ai_waiting.ends_with("Waiting for AI requests to finish…"));
    }

    #[test]
    fn t_tray_02_empty_and_blank_fields_keep_the_existing_value() {
        // Arrange
        let base = TrayLabels::default();
        let mut incoming = all("   ");
        incoming.open = "열기".into();
        incoming.quit = String::new();
        // Act
        let merged = base.merged_with(&incoming);
        // Assert
        assert_eq!(merged.open, "열기");
        assert_eq!(merged.quit, base.quit);
        assert_eq!(merged.pause_jobs, base.pause_jobs);
        assert_eq!(merged.resume_jobs, base.resume_jobs);
        assert_eq!(merged.ai_waiting, base.ai_waiting);
    }

    #[test]
    fn t_tray_03_fields_are_trimmed() {
        let merged = TrayLabels::default().merged_with(&all("  \t작업 재개 \n"));
        assert_eq!(merged, all("작업 재개"));
    }

    #[test]
    fn t_tray_04_cap_is_character_based_not_byte_based() {
        // Arrange: 3바이트 문자 250개 -> 바이트 슬라이스였다면 경계에서 패닉/깨짐
        let long: String = "한".repeat(250);
        // Act
        let merged = TrayLabels::default().merged_with(&all(&long));
        // Assert
        assert_eq!(merged.open.chars().count(), 200);
        assert!(merged.open.chars().all(|c| c == '한'));
        // Boundary: 정확히 200자는 그대로
        let exact = "é".repeat(200);
        assert_eq!(TrayLabels::default().merged_with(&all(&exact)).quit, exact);
    }

    #[test]
    fn t_tray_05_toggle_label_follows_the_paused_state_with_custom_labels() {
        // Arrange
        let custom = TrayLabels {
            pause_jobs: "Pausar".into(),
            resume_jobs: "Reanudar".into(),
            ..TrayLabels::default()
        };
        // Act / Assert
        assert_eq!(custom.toggle(false), "Pausar");
        assert_eq!(custom.toggle(true), "Reanudar");
        assert_ne!(TrayLabels::default().toggle(false), TrayLabels::default().toggle(true));
    }

    #[test]
    fn t_tray_06_deserializes_camel_case_keys() {
        let json = r#"{"open":"a","pauseJobs":"b","resumeJobs":"c","quit":"d","aiWaiting":"e"}"#;
        let parsed: Result<TrayLabels, _> = serde_json::from_str(json);
        assert_eq!(
            parsed.ok(),
            Some(TrayLabels {
                open: "a".into(),
                pause_jobs: "b".into(),
                resume_jobs: "c".into(),
                quit: "d".into(),
                ai_waiting: "e".into(),
            })
        );
        // Validation: 키 누락은 거부
        assert!(serde_json::from_str::<TrayLabels>(r#"{"open":"a"}"#).is_err());
    }
}
