// Categories: Happy (기본값·키·동의) / Validation (범위·알 수 없는 필드) / Business rule (동의 전 외부 호출 0회, 키 비노출,
// 철회 시 대기 외부 단계 건너뜀, 자동·예약 등록) / External (연결 확인 실패) / Time (365일 정리, 예약 시간대)
use std::io::Write;
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::super::AiFailure;
use super::super::test_support::{Fx, StubTransport};
use super::*;

fn count(fx: &Fx, sql: &str) -> i64 {
    fx.pool
        .read(|c| Ok(c.query_row(sql, [], |r| r.get(0)).unwrap()))
        .unwrap()
}

#[derive(Clone, Default)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
    type Writer = LogBuf;
    fn make_writer(&'a self) -> LogBuf {
        self.clone()
    }
}

#[test]
fn t_p3w_set_01_defaults_match_the_contract() {
    // Arrange + Act
    let fx = Fx::new();
    let s = fx.service.get_settings();
    // Assert
    assert_eq!(s.default_provider, None);
    assert!(!s.consent.given && s.consent.at.is_none());
    assert_eq!(s.language, "ko");
    assert!(!s.auto_analyze_new && !s.background_analyze);
    assert_eq!(
        serde_json::to_value(&s).unwrap(),
        json!({
            "defaultProvider": null,
            "consent": {"given": false, "at": null},
            "language": "ko",
            "autoAnalyzeNew": false,
            "autoSubtitleNew": null,
            "backgroundAnalyze": false,
            "schedule": {"enabled": false, "startHour": 0, "endHour": 0},
            "externalKinds": ["describe", "keywords", "ocr"],
            "transfer": {"imageMaxEdge": 1024, "videoFrames": 5, "includeMetadata": true},
            "localStt": {"enabled": true, "model": "base"},
            "semanticSearch": {"enabled": false}
        })
    );
}

#[test]
fn t_p3w_set_02_patch_ranges_and_unknown_fields_are_rejected_without_change() {
    let fx = Fx::new();
    fx.service.give_consent().unwrap();
    let before = fx.service.get_settings();
    let bad = [
        json!({"transfer": {"imageMaxEdge": 255}}),
        json!({"transfer": {"imageMaxEdge": 2049}}),
        json!({"transfer": {"videoFrames": 0}}),
        json!({"transfer": {"videoFrames": 11}}),
        json!({"schedule": {"startHour": 24}}),
        json!({"schedule": {"endHour": -1}}),
        json!({"externalKinds": ["describe", "stt"]}),
        json!({"language": "ko; drop"}),
        json!({"localStt": {"model": "large"}}),
        json!({"consent": {"given": true}}),
        json!({"nonsense": 1}),
        // 한 필드가 틀리면 같은 패치의 다른 필드도 적용되지 않는다
        json!({"language": "en", "transfer": {"videoFrames": 99}}),
    ];
    for patch in bad {
        let err = fx.service.update_settings(patch.clone()).unwrap_err();
        assert!(matches!(err, AiFailure::Invalid(_)), "{patch}: {err:?}");
    }
    assert_eq!(fx.service.get_settings(), before);
}

#[test]
fn t_p3w_set_03_boundaries_accepted_and_persist_across_restart() {
    let fx = Fx::new();
    let s = fx
        .service
        .update_settings(json!({
            "transfer": {"imageMaxEdge": 256, "videoFrames": 10},
            "schedule": {"enabled": true, "startHour": 22, "endHour": 6},
            "externalKinds": ["summary", "summary", "describe"],
            "language": "en-US",
            "localStt": {"model": "small"}
        }))
        .unwrap();
    assert_eq!(s.transfer.image_max_edge, 256);
    assert_eq!(s.transfer.video_frames, 10);
    assert_eq!(s.external_kinds.len(), 2, "중복 제거");
    let again = fx.restart().get_settings();
    assert_eq!(again, s);
}

#[test]
fn t_p3w_set_04_automation_needs_consent_and_semantic_needs_models() {
    let fx = Fx::new();
    // 동의 전: 자동·백그라운드 분석을 켤 수 없다
    for patch in [
        json!({"autoAnalyzeNew": true}),
        json!({"backgroundAnalyze": true}),
    ] {
        assert!(matches!(
            fx.service.update_settings(patch),
            Err(AiFailure::ConsentRequired)
        ));
    }
    // 모델이 없으면 의미 검색을 켤 수 없다
    assert!(matches!(
        fx.service
            .update_settings(json!({"semanticSearch": {"enabled": true}})),
        Err(AiFailure::Invalid(_))
    ));
    // 동의 후에는 켤 수 있고, 철회하면 다시 꺼진다
    fx.service.give_consent().unwrap();
    let s = fx
        .service
        .update_settings(json!({"autoAnalyzeNew": true, "backgroundAnalyze": true}))
        .unwrap();
    assert!(s.auto_analyze_new && s.background_analyze);
    let s = fx.service.revoke_consent().unwrap();
    assert!(!s.consent.given && !s.auto_analyze_new && !s.background_analyze);
}

#[test]
fn t_p3w_set_05_default_provider_can_be_cleared_with_null() {
    let fx = Fx::new();
    fx.service
        .update_settings(json!({"defaultProvider": "claude"}))
        .unwrap();
    assert_eq!(
        fx.service.get_settings().default_provider,
        Some(mam_ai::external::Provider::Claude)
    );
    let s = fx
        .service
        .update_settings(json!({"defaultProvider": null}))
        .unwrap();
    assert_eq!(s.default_provider, None);
    assert!(
        fx.service
            .update_settings(json!({"defaultProvider": "bard"}))
            .is_err()
    );
}

#[test]
fn t_p3w_consent_01_no_network_before_consent_and_gate_after() {
    // Arrange
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    fx.service
        .set_api_key("openai", "sk-test-SECRET-123456", false)
        .unwrap();
    // Act + Assert: 연결 확인·외부 분석·전체 분석 모두 동의 전에는 거절, 전송 0회
    assert!(matches!(
        fx.service.check_provider("openai"),
        Err(AiFailure::ConsentRequired)
    ));
    assert!(matches!(
        fx.service
            .analyze_assets(&[1], Some(&["describe".to_string()]), false),
        Err(AiFailure::ConsentRequired)
    ));
    assert!(matches!(
        fx.service.analyze_all(true),
        Err(AiFailure::ConsentRequired)
    ));
    assert_eq!(fx.transport.count(), 0);
    // 로컬 단계만은 동의 없이도 등록된다
    let r = fx
        .service
        .analyze_assets(&[1], Some(&["embedding".to_string()]), false)
        .unwrap();
    assert_eq!(r.queued, 1);
    assert_eq!(fx.transport.count(), 0);
}

#[test]
fn t_p3w_key_01_key_never_in_responses_logs_or_settings() {
    // Arrange
    let log = LogBuf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(log.clone())
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let secret = "sk-test-SECRET-123456";
    let fx = Fx::with(
        StubTransport::new(401, "{\"error\":\"bad key sk-test-SECRET-123456\"}"),
        1_800_000_000_000,
    );
    let mut responses: Vec<String> = Vec::new();
    // Act
    tracing::subscriber::with_default(subscriber, || {
        fx.service.give_consent().unwrap();
        let st = fx.service.set_api_key("openai", secret, true).unwrap();
        responses.push(serde_json::to_string(&st).unwrap());
        let check = fx.service.check_provider("openai").unwrap();
        responses.push(serde_json::to_string(&check).unwrap());
        responses.push(serde_json::to_string(&fx.service.list_providers()).unwrap());
        responses.push(serde_json::to_string(&fx.service.get_settings()).unwrap());
        let err: crate::error::CommandError = fx
            .service
            .set_api_key("openai", "bad key with space", false)
            .unwrap_err()
            .into();
        responses.push(serde_json::to_string(&err).unwrap());
        fx.service.clear_api_key("openai").unwrap();
    });
    // Assert: 응답·로그·DB 설정 문서 어디에도 평문 키가 없다
    for r in &responses {
        assert!(!r.contains(secret) && !r.contains("SECRET"), "{r}");
    }
    let logged = String::from_utf8_lossy(&log.0.lock().unwrap()).into_owned();
    assert!(!logged.contains("SECRET"), "{logged}");
    let all_settings: String = fx
        .pool
        .read(|c| {
            let mut s = c.prepare("SELECT key || value FROM settings")?;
            let rows = s.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.map(Result::unwrap).collect::<Vec<_>>().join("\n"))
        })
        .unwrap();
    assert!(!all_settings.contains("SECRET"));
    let usage: i64 = count(&fx, "SELECT COUNT(*) FROM ai_usage WHERE kind = 'check'");
    assert_eq!(usage, 1, "연결 확인 사용량 기록");
}

#[cfg(windows)]
#[test]
fn t_p3w_key_02_persisted_key_survives_restart_and_session_key_does_not() {
    // Arrange
    let fx = Fx::new();
    fx.service
        .set_api_key("openai", "sk-persisted-111111", true)
        .unwrap();
    fx.service
        .set_api_key("gemini", "AIza-session-222222", false)
        .unwrap();
    let status = |s: &AiService, p: &str| {
        s.list_providers()
            .into_iter()
            .find(|x| x.provider.as_str() == p)
            .unwrap()
            .key_storage
    };
    assert_eq!(status(&fx.service, "openai"), KeyStorage::Persisted);
    assert_eq!(status(&fx.service, "gemini"), KeyStorage::Session);
    // Act: 재시작
    let again = fx.restart();
    assert_eq!(
        status(&again, "openai"),
        KeyStorage::None,
        "복원 전에는 없음"
    );
    again.load_persisted_keys();
    // Assert
    assert_eq!(status(&again, "openai"), KeyStorage::Persisted);
    assert_eq!(status(&again, "gemini"), KeyStorage::None);
    // 지우면 저장된 암호문도 사라진다
    again.clear_api_key("openai").unwrap();
    assert_eq!(
        count(
            &fx,
            "SELECT COUNT(*) FROM settings WHERE key LIKE 'ai\\_key\\_%' ESCAPE '\\'"
        ),
        0
    );
}

#[test]
fn t_p3w_key_03_invalid_keys_and_providers_are_rejected() {
    let fx = Fx::new();
    for key in ["", "   ", "has space", "tab\tkey"] {
        assert!(
            matches!(
                fx.service.set_api_key("openai", key, false),
                Err(AiFailure::Invalid(_))
            ),
            "{key:?}"
        );
    }
    assert!(matches!(
        fx.service.set_api_key("bard", "k", false),
        Err(AiFailure::Invalid(_))
    ));
    assert!(
        fx.service
            .list_providers()
            .iter()
            .all(|p| p.key_storage == KeyStorage::None)
    );
}

#[test]
fn t_p3w_key_04_first_key_becomes_default_provider_and_model_is_validated() {
    let fx = Fx::new();
    fx.service
        .set_api_key("claude", "sk-ant-xyz", false)
        .unwrap();
    assert_eq!(
        fx.service.get_settings().default_provider,
        Some(mam_ai::external::Provider::Claude)
    );
    assert!(
        fx.service
            .set_provider_model("claude", Some("x/../y".into()))
            .is_err()
    );
    let st = fx
        .service
        .set_provider_model("claude", Some(" claude-haiku-4-5 ".into()))
        .unwrap();
    assert_eq!(st.model.as_deref(), Some("claude-haiku-4-5"));
    let st = fx
        .service
        .set_provider_model("claude", Some("  ".into()))
        .unwrap();
    assert_eq!(st.model, None);
}

#[test]
fn t_p3w_check_01_invalid_key_reports_auth_failed_with_one_request() {
    // Arrange
    let fx = Fx::with(StubTransport::new(401, "{}"), 1_800_000_000_000);
    fx.consent_and_key();
    // Act
    let r = fx.service.check_provider("openai").unwrap();
    // Assert
    assert!(!r.ok);
    assert_eq!(r.error_code.as_deref(), Some("auth_failed"));
    assert_eq!(fx.transport.count(), 1);
    let last = fx.service.list_providers()[0].last_check.clone().unwrap();
    assert!(!last.ok && last.error_code.as_deref() == Some("auth_failed"));
}

#[test]
fn t_p3w_check_02_missing_key_makes_no_request() {
    let fx = Fx::new();
    fx.service.give_consent().unwrap();
    let r = fx.service.check_provider("gemini").unwrap();
    assert_eq!(r.error_code.as_deref(), Some("auth_failed"));
    assert_eq!(fx.transport.count(), 0);
}

#[test]
fn t_p3w_revoke_01_pending_external_steps_skipped_and_results_kept() {
    // Arrange
    let fx = Fx::new();
    fx.consent_and_key();
    fx.insert_asset(1, "video", "a.mp4");
    fx.service.analyze_assets(&[1], None, false).unwrap();
    // 이미 받은 결과
    let c = Arc::clone(&fx.service.clock);
    fx.pool
        .write(mam_db::Priority::Low, move |conn| {
            mam_core::ai_store::save_analysis(
                conn,
                &*c,
                1,
                &mam_core::ai_store::NewAnalysis {
                    fields: mam_core::ai_store::AnalysisFields {
                        description: Some("kept".into()),
                        ..Default::default()
                    },
                    language: "ko".into(),
                    provider: "openai".into(),
                    model: "m".into(),
                },
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
    // Act
    fx.service.revoke_consent().unwrap();
    // Assert
    let ai = fx.service.get_asset_ai(1).unwrap();
    assert_eq!(ai.analysis.unwrap().description, "kept");
    let ext: Vec<_> = ai
        .status
        .iter()
        .filter(|s| s.kind.is_external() && s.state != "none")
        .collect();
    assert!(!ext.is_empty());
    assert!(
        ext.iter()
            .all(|s| s.state == "skipped" && s.error_code.as_deref() == Some("consent_required"))
    );
    let stt = ai.status.iter().find(|s| s.kind == StepKind::Stt).unwrap();
    assert_eq!(stt.state, "queued", "로컬 단계는 유지");
}

#[test]
fn t_p3w_analyze_01_input_validation() {
    let fx = Fx::new();
    fx.service.give_consent().unwrap();
    assert!(matches!(
        fx.service.analyze_assets(&[], None, false),
        Err(AiFailure::Invalid(_))
    ));
    assert!(matches!(
        fx.service.analyze_assets(&[0], None, false),
        Err(AiFailure::Invalid(_))
    ));
    let many: Vec<i64> = (1..=10_001).collect();
    assert!(matches!(
        fx.service.analyze_assets(&many, None, false),
        Err(AiFailure::Invalid(_))
    ));
    assert!(matches!(
        fx.service.analyze_assets(&[1], Some(&[]), false),
        Err(AiFailure::Invalid(_))
    ));
    assert!(matches!(
        fx.service
            .analyze_assets(&[1], Some(&["bogus".to_string()]), false),
        Err(AiFailure::Invalid(_))
    ));
    assert!(matches!(
        fx.service.get_asset_ai(999),
        Err(AiFailure::NotFound)
    ));
    // 없는 자산은 오류가 아니라 skipped
    let r = fx.service.analyze_assets(&[999], None, false).unwrap();
    assert_eq!((r.queued, r.skipped), (0, 1));
}

#[test]
fn t_p3w_edit_01_edit_is_high_priority_and_validated() {
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    let view = fx
        .service
        .edit_analysis(
            1,
            &EditInput {
                description: Some("직접 쓴 설명".into()),
                keywords: Some(vec!["a".into(), " a ".into()]),
            },
        )
        .unwrap();
    assert!(view.edited);
    assert_eq!(view.keywords, vec!["a"]);
    assert!(matches!(
        fx.service.edit_analysis(
            404,
            &EditInput {
                description: Some("x".into()),
                keywords: None
            }
        ),
        Err(AiFailure::NotFound)
    ));
    assert!(matches!(
        fx.service.edit_analysis(
            1,
            &EditInput {
                description: Some("가".repeat(20_001)),
                keywords: None
            }
        ),
        Err(AiFailure::Invalid(_))
    ));
    let hist = fx.service.list_history(1).unwrap();
    assert!(hist.is_empty());
}

#[test]
fn t_p3w_usage_01_purge_keeps_365_days_boundary() {
    // Arrange: now = 고정, 366일·365일·10일 전 행
    let now = 1_800_000_000_000_i64;
    let fx = Fx::with(StubTransport::new(200, "{}"), now);
    let day = 86_400_000_i64;
    fx.pool
        .write(mam_db::Priority::Low, move |c| {
            for (i, at) in [now - 366 * day, now - 365 * day, now - 10 * day].into_iter().enumerate() {
                c.execute(
                    "INSERT INTO ai_usage (provider, model, kind, ok, at) VALUES ('openai', 'm', 'describe', 1, ?1)",
                    [at],
                )?;
                c.execute(
                    "INSERT INTO analysis_runs (asset_id, kind, provider, ok, message, attempts, at)
                     VALUES (?1, 'describe', 'openai', 1, '', 1, ?2)",
                    rusqlite::params![i as i64 + 1, at],
                )
                .ok();
            }
            Ok(())
        })
        .unwrap();
    // Act
    let removed = fx.service.purge_old().unwrap();
    // Assert: 365일 전 정각은 남고 366일 전만 지워진다
    assert!(removed >= 1);
    assert_eq!(count(&fx, "SELECT COUNT(*) FROM ai_usage"), 2);
    assert_eq!(count(&fx, "SELECT COUNT(*) FROM ai_usage WHERE at < 0"), 0);
    // 멱등
    assert_eq!(fx.service.purge_old().unwrap(), 0);
}

#[test]
fn t_p3w_usage_02_list_usage_validates_days() {
    let fx = Fx::new();
    assert!(matches!(
        fx.service.list_usage("2026-13-01", "2026-12-01"),
        Err(AiFailure::Invalid(_))
    ));
    assert!(matches!(
        fx.service.list_usage("2026-02-01", "2026-01-01"),
        Err(AiFailure::Invalid(_))
    ));
    assert!(
        fx.service
            .list_usage("2026-01-01", "2026-01-31")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn t_p3w_sched_01_new_assets_follow_watermark_only_when_enabled() {
    // Arrange
    let fx = Fx::new();
    fx.insert_asset(1, "image", "old.jpg");
    assert_eq!(fx.service.tick_new_assets().unwrap(), 0); // 워터마크 시작 = 1
    fx.service.give_consent().unwrap();
    // 꺼진 동안 생긴 자산은 대상이 아니다
    fx.insert_asset(2, "image", "while_off.jpg");
    assert_eq!(fx.service.tick_new_assets().unwrap(), 0);
    fx.service
        .update_settings(json!({"autoAnalyzeNew": true}))
        .unwrap();
    assert_eq!(
        fx.service.tick_new_assets().unwrap(),
        0,
        "켜기 전 자산 제외"
    );
    // Act: 켠 뒤 새 자산
    fx.insert_asset(3, "video", "new.mp4"); // 키가 없으면 이미지에는 할 단계가 없다 (stt 는 영상·음성만)
    let queued = fx.service.tick_new_assets().unwrap();
    // Assert
    assert_eq!(queued, 1);
    let st = fx.service.get_asset_ai(3).unwrap().status;
    assert!(st.iter().any(|s| s.state == "queued"));
    assert!(
        fx.service
            .get_asset_ai(2)
            .unwrap()
            .status
            .iter()
            .all(|s| s.state == "none")
    );
    // 키가 없으므로 외부 단계는 등록하지 않는다 (건너뜀 표식 대량 생성 방지)
    assert!(
        st.iter()
            .all(|s| !s.kind.is_external() || s.state == "none")
    );
    assert_eq!(fx.service.tick_new_assets().unwrap(), 0, "멱등");
}

#[test]
fn t_p3w_sched_02_background_sweep_respects_schedule_window() {
    // now = 2027-01-15T15:00:00Z, 오프셋 +9h -> 로컬 00:00
    let now = 1_800_025_200_000_i64;
    let fx = Fx::with(StubTransport::new(200, "{}"), now);
    fx.consent_and_key();
    fx.insert_asset(1, "image", "a.jpg");
    fx.service
        .update_settings(json!({"backgroundAnalyze": true, "schedule": {"enabled": true, "startHour": 22, "endHour": 6}}))
        .unwrap();
    // 로컬 시각 안 (00시) -> 등록
    let r = fx.service.tick_background().unwrap();
    assert_eq!(r.queued, 1);
    // 시간대 밖 (12시)
    let fx2 = Fx::with(StubTransport::new(200, "{}"), now + 12 * 3_600_000);
    fx2.consent_and_key();
    fx2.insert_asset(1, "image", "a.jpg");
    fx2.service
        .update_settings(json!({"backgroundAnalyze": true, "schedule": {"enabled": true, "startHour": 22, "endHour": 6}}))
        .unwrap();
    let r = fx2.service.tick_background().unwrap();
    assert_eq!((r.queued, r.skipped), (0, 0));
}

#[test]
fn t_backfill_09_tick_queues_embedding_only_when_semantic_on_without_consent_or_key() {
    // Arrange: 동의·키 없음, 의미 검색 꺼짐
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    fx.insert_asset(2, "video", "b.mp4");

    // Act / Assert: 꺼짐 -> 0건
    assert_eq!(fx.service.tick_embedding_backfill().unwrap().queued, 0);

    // 켜짐(모델 준비 검사는 설정 API 에서 끝났다고 보고 저장소에 직접 반영) -> 임베딩만 등록, 재호출은 멱등
    fx.service
        .store
        .update(|d| {
            d.settings.semantic_search.enabled = true;
            Ok(())
        })
        .unwrap();
    assert_eq!(fx.service.tick_embedding_backfill().unwrap().queued, 2);
    assert_eq!(fx.service.tick_embedding_backfill().unwrap().queued, 0);
    for id in [1, 2] {
        for s in fx.service.get_asset_ai(id).unwrap().status {
            assert_eq!(
                s.state == "queued",
                s.kind == mam_core::ai_jobs::StepKind::Embedding,
                "asset {id} {:?}",
                s.kind
            );
        }
    }
}

struct FixedText;

impl mam_core::embedding::TextEncoder for FixedText {
    fn model(&self) -> mam_core::embedding::ModelInfo {
        mam_core::embedding::ModelInfo {
            model_id: "m".into(),
            model_version: "1".into(),
            dimension: 2,
        }
    }
    fn encode_texts(&self, t: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Ok(vec![vec![0.6, 0.8]; t.len()])
    }
}

#[test]
fn t_p3w_vec_01_index_generation_is_built_then_extended_incrementally() {
    // Arrange
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    fx.insert_asset(2, "image", "b.jpg");
    let key = ModelKey {
        kind: VectorKind::Semantic,
        model_id: "m".into(),
        model_version: "1".into(),
        dimension: 2,
    };
    let put = |asset: i64| {
        let store = Arc::clone(&fx.service.embed_store);
        fx.pool
            .write(Priority::Low, move |c| {
                mam_core::embedding::embed_semantic(c, &store, &FixedText, asset, &["x"], 1)
                    .unwrap();
                Ok(())
            })
            .unwrap();
    };
    let open = || {
        Generations::new(&fx.service.embed_store, &key)
            .unwrap()
            .open_active(&key, OpenMode::View)
            .unwrap()
    };
    // Act + Assert: 벡터가 없으면 활성 세대도 만들지 않는다
    assert!(open().is_none());
    put(1);
    let t0 = std::time::Instant::now();
    assert!(fx.service.sync_key_at(&key, t0).unwrap());
    assert_eq!(mam_vector::AnnIndex::len(&open().unwrap()), 1);
    // 새 벡터가 없으면 아무것도 하지 않는다 (멱등)
    assert!(!fx.service.sync_key_at(&key, t0).unwrap());
    put(2);
    // 저장 제한 간격(최대 10분)이 지난 뒤의 틱
    assert!(fx.service.sync_key_at(&key, later(t0)).unwrap());
    assert_eq!(mam_vector::AnnIndex::len(&open().unwrap()), 2);
}

fn later(t: std::time::Instant) -> std::time::Instant {
    t + std::time::Duration::from_secs(3_600)
}

/// 분류: Business rule / Time — 저장 빈도 제한 (적응형)
#[test]
fn t_p3w_vec_02_throttled_delta_is_persisted_on_a_later_due_tick() {
    // Arrange: 첫 틱(이력 없음)은 즉시 저장
    let fx = Fx::new();
    fx.insert_asset(1, "image", "a.jpg");
    fx.insert_asset(2, "image", "b.jpg");
    let key = ModelKey {
        kind: VectorKind::Semantic,
        model_id: "m".into(),
        model_version: "1".into(),
        dimension: 2,
    };
    let put = |asset: i64| {
        let store = Arc::clone(&fx.service.embed_store);
        fx.pool
            .write(Priority::Low, move |c| {
                mam_core::embedding::embed_semantic(c, &store, &FixedText, asset, &["x"], 1)
                    .unwrap();
                Ok(())
            })
            .unwrap();
    };
    let gen_dir = fx.service.embed_store.key_dir(&key).join("gen").join("g1");
    let len = || {
        let g = Generations::new(&fx.service.embed_store, &key).unwrap();
        mam_vector::AnnIndex::len(&g.open_active(&key, OpenMode::View).unwrap().unwrap())
    };
    let t0 = std::time::Instant::now();
    put(1);
    assert!(fx.service.sync_key_at(&key, t0).unwrap());
    // 재구축이 이미 썼고 더 읽은 게 없으므로 다시 쓰지 않는다 (T-3b)
    assert_eq!(mam_vector::data_file_versions(&gen_dir), vec![0]);

    // Act: 새 벡터 + 간격 안의 틱 -> 읽지도 저장하지도 않고 반영 표시도 없다
    put(2);
    assert!(!fx.service.sync_key_at(&key, t0).unwrap());

    // Assert: 검색이 보는 세대는 그대로
    assert_eq!(len(), 1);
    assert_eq!(mam_vector::data_file_versions(&gen_dir), vec![0]);

    // Act: 간격이 지난 틱 -> 누락 없이 반영·저장
    assert!(fx.service.sync_key_at(&key, later(t0)).unwrap());

    // Assert
    assert_eq!(len(), 2);
    assert_eq!(mam_vector::data_file_versions(&gen_dir), vec![0, 1]);
    // 같은 시각의 다음 틱은 할 일이 없다
    assert!(!fx.service.sync_key_at(&key, later(t0)).unwrap());
}
