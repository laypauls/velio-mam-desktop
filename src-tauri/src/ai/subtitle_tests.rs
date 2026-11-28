// T-subtitle-30~ - 새 파일 자동 자막 생성 설정(`autoSubtitleNew`)·실효값·등록 틱·종류별 정리·일괄 생성 command 계약.
// Categories: Happy / Validation (동의 없이 패치, 알 수 없는 필드) / Business rule (실효값, 워터마크, 종류별 정리, 동의 철회) /
// External (자막 스위치 꺼짐 거부) / Concurrency (멱등)
// `stt` = 자막 (저장·교환 값은 `stt` 유지)
use serde_json::json;

use super::AiFailure;
use super::settings::AiSettings;
use super::test_support::Fx;
use crate::error::{CommandError, ErrorCode};
use mam_core::ai_jobs::StepKind;

/// 자산의 자막(stt) 단계 상태 ("none" = 행 없음).
fn stt_state(fx: &Fx, id: i64) -> String {
    fx.service
        .get_asset_ai(id)
        .unwrap()
        .status
        .into_iter()
        .find(|s| s.kind == StepKind::Stt)
        .map_or("none".into(), |s| s.state)
}

/// 워터마크를 시작(= 현재 최대 id)시키고 동의를 준다.
fn started(fx: &Fx) {
    fx.insert_asset(1, "image", "old.jpg");
    assert_eq!(fx.service.tick_new_assets().unwrap(), 0);
}

#[test]
fn t_subtitle_30_effective_value_follows_analyze_and_consent_unless_explicit() {
    // Arrange
    let mut s = AiSettings::default();
    assert_eq!(s.auto_subtitle_new, None);

    // Act / Assert: 값 없음 = 자동 분석 && 동의
    assert!(!s.auto_subtitle_effective());
    s.auto_analyze_new = true;
    assert!(!s.auto_subtitle_effective(), "동의 없음");
    s.consent.given = true;
    assert!(s.auto_subtitle_effective());
    // 명시값 우선 (동의·자동 분석과 무관)
    s.auto_subtitle_new = Some(false);
    assert!(!s.auto_subtitle_effective());
    s.auto_analyze_new = false;
    s.consent.given = false;
    s.auto_subtitle_new = Some(true);
    assert!(s.auto_subtitle_effective());
    // 런타임 스냅샷에도 실효값이 실린다
    assert!(s.runtime().auto_subtitle_new);
}

#[test]
fn t_subtitle_31_old_settings_document_without_the_field_loads_as_null() {
    // Arrange: 필드가 없는 구 문서 + null 직렬화
    let old = json!({"language": "en", "autoAnalyzeNew": true});

    // Act
    let s: AiSettings = serde_json::from_value(old).unwrap();

    // Assert
    assert_eq!(s.auto_subtitle_new, None);
    assert_eq!(s.language, "en");
    assert_eq!(
        serde_json::to_value(&s).unwrap()["autoSubtitleNew"],
        json!(null)
    );
    let mut on = s.clone();
    on.auto_subtitle_new = Some(false);
    assert_eq!(
        serde_json::to_value(&on).unwrap()["autoSubtitleNew"],
        json!(false)
    );
    let back: AiSettings = serde_json::from_value(serde_json::to_value(&on).unwrap()).unwrap();
    assert_eq!(back.auto_subtitle_new, Some(false));
}

#[test]
fn t_subtitle_32_patch_works_without_consent_and_unknown_fields_are_still_rejected() {
    // Arrange: 동의 전
    let fx = Fx::new();
    assert!(!fx.service.get_settings().consent.given);

    // Act
    let on = fx
        .service
        .update_settings(json!({"autoSubtitleNew": true}))
        .unwrap();
    let off = fx
        .service
        .update_settings(json!({"autoSubtitleNew": false}))
        .unwrap();
    let untouched = fx
        .service
        .update_settings(json!({"language": "en"}))
        .unwrap();

    // Assert
    assert_eq!(on.auto_subtitle_new, Some(true));
    assert_eq!(off.auto_subtitle_new, Some(false));
    assert_eq!(
        untouched.auto_subtitle_new,
        Some(false),
        "다른 패치는 값을 바꾸지 않는다"
    );
    assert!(matches!(
        fx.service
            .update_settings(json!({"autoSubtitleNew": "yes"})),
        Err(AiFailure::Invalid(_))
    ));
    assert!(matches!(
        fx.service.update_settings(json!({"autoSubtitle": true})),
        Err(AiFailure::Invalid(_))
    ));
    // 동의가 필요한 자동 분석은 여전히 거부
    assert!(matches!(
        fx.service.update_settings(json!({"autoAnalyzeNew": true})),
        Err(AiFailure::ConsentRequired)
    ));
    // 재시작해도 유지
    assert_eq!(fx.restart().get_settings().auto_subtitle_new, Some(false));
}

#[test]
fn t_subtitle_33_tick_without_consent_queues_only_stt_for_new_video_and_audio() {
    // Arrange
    let fx = Fx::new();
    started(&fx);
    fx.service
        .update_settings(json!({"autoSubtitleNew": true}))
        .unwrap();
    fx.insert_asset(2, "video", "v.mp4");
    fx.insert_asset(3, "image", "i.jpg");
    fx.insert_asset(4, "audio", "a.wav");

    // Act
    let queued = fx.service.tick_new_assets().unwrap();

    // Assert: 영상·음성만, 자막만 (이미지 제외)
    assert_eq!(queued, 2);
    assert_eq!(stt_state(&fx, 2), "queued");
    assert_eq!(stt_state(&fx, 4), "queued");
    assert_eq!(stt_state(&fx, 3), "none");
    let video = fx.service.get_asset_ai(2).unwrap().status;
    assert!(
        video
            .iter()
            .all(|s| s.kind == StepKind::Stt || s.state == "none")
    );
    assert_eq!(fx.service.tick_new_assets().unwrap(), 0, "멱등");
}

#[test]
fn t_subtitle_34_files_added_while_off_are_not_queued_after_turning_on() {
    // Arrange
    let fx = Fx::new();
    started(&fx);
    fx.insert_asset(2, "video", "while_off.mp4");
    assert_eq!(fx.service.tick_new_assets().unwrap(), 0); // 꺼진 동안 워터마크만 따라간다

    // Act
    fx.service
        .update_settings(json!({"autoSubtitleNew": true}))
        .unwrap();
    assert_eq!(fx.service.tick_new_assets().unwrap(), 0);
    fx.insert_asset(3, "video", "after_on.mp4");
    let queued = fx.service.tick_new_assets().unwrap();

    // Assert
    assert_eq!(queued, 1);
    assert_eq!(stt_state(&fx, 2), "none");
    assert_eq!(stt_state(&fx, 3), "queued");
}

#[test]
fn t_subtitle_35_stt_switch_off_means_the_tick_only_follows_the_watermark() {
    // Arrange: 자동 자막 켬 + 자막 스위치 끔
    let fx = Fx::new();
    started(&fx);
    fx.service
        .update_settings(json!({"autoSubtitleNew": true, "localStt": {"enabled": false}}))
        .unwrap();
    fx.insert_asset(2, "video", "v.mp4");

    // Act
    let off = fx.service.tick_new_assets().unwrap();
    fx.service
        .update_settings(json!({"localStt": {"enabled": true}}))
        .unwrap();
    let on_same = fx.service.tick_new_assets().unwrap();

    // Assert: 꺼진 동안 들어온 파일은 켠 뒤에도 대상이 아니다
    assert_eq!((off, on_same), (0, 0));
    assert_eq!(stt_state(&fx, 2), "none");
}

#[test]
fn t_subtitle_36_unset_value_keeps_the_old_behavior_of_auto_analyze_with_consent() {
    // Arrange: 자동 분석 + 동의, 자막 값은 건드리지 않음 (null)
    let fx = Fx::new();
    started(&fx);
    fx.service.give_consent().unwrap();
    fx.service
        .update_settings(json!({"autoAnalyzeNew": true}))
        .unwrap();
    fx.insert_asset(2, "video", "v.mp4");
    fx.insert_asset(3, "video", "w.mp4");

    // Act: 값이 없으면 자동 분석 + 동의를 따라 자막이 등록된다
    let first = fx.service.tick_new_assets().unwrap();
    assert_eq!(first, 2);
    assert_eq!(stt_state(&fx, 2), "queued");
    assert_eq!(stt_state(&fx, 3), "queued");
    // 명시적으로 끄면 실효값 true -> false 이므로 대기분이 정리되고 새 파일에도 등록되지 않는다
    fx.service
        .update_settings(json!({"autoSubtitleNew": false}))
        .unwrap();
    fx.insert_asset(4, "video", "x.mp4");
    let second = fx.service.tick_new_assets().unwrap();

    // Assert
    assert_eq!(stt_state(&fx, 2), "none");
    assert_eq!(stt_state(&fx, 3), "none");
    assert_eq!(second, 0, "자막 명시 OFF + 키 없음 -> 등록할 종류가 없다");
    assert_eq!(stt_state(&fx, 4), "none");
}

#[test]
fn t_subtitle_37_turning_subtitle_off_deletes_only_queued_auto_new_stt() {
    // Arrange: 1=자동 등록 stt(queued), 5=사용자가 요청한 stt
    let fx = Fx::new();
    started(&fx);
    fx.service
        .update_settings(json!({"autoSubtitleNew": true}))
        .unwrap();
    fx.insert_asset(2, "video", "v.mp4");
    fx.insert_asset(5, "video", "m.mp4");
    fx.service
        .analyze_assets(&[5], Some(&["stt".into()]), false)
        .unwrap();
    fx.service.tick_new_assets().unwrap();
    assert_eq!(stt_state(&fx, 2), "queued");

    // Act
    fx.service
        .update_settings(json!({"autoSubtitleNew": false}))
        .unwrap();
    // 이미 꺼진 상태에서 다시 꺼도(값 동일) 영향 없다
    fx.service
        .update_settings(json!({"autoSubtitleNew": false}))
        .unwrap();

    // Assert
    assert_eq!(stt_state(&fx, 2), "none");
    assert_eq!(stt_state(&fx, 5), "queued", "manual 은 보존");
}

#[test]
fn t_subtitle_38_turning_auto_analyze_off_keeps_queued_auto_stt() {
    // Arrange
    let fx = Fx::new();
    started(&fx);
    fx.service.give_consent().unwrap();
    fx.service
        .update_settings(json!({"autoAnalyzeNew": true, "autoSubtitleNew": true}))
        .unwrap();
    fx.insert_asset(2, "video", "v.mp4");
    assert_eq!(fx.service.tick_new_assets().unwrap(), 1);

    // Act
    fx.service
        .update_settings(json!({"autoAnalyzeNew": false}))
        .unwrap();

    // Assert
    assert_eq!(stt_state(&fx, 2), "queued");
}

#[test]
fn t_subtitle_39_revoking_consent_keeps_stt_only_when_subtitle_is_explicitly_on() {
    for explicit in [true, false] {
        // Arrange
        let fx = Fx::new();
        started(&fx);
        fx.service.give_consent().unwrap();
        let mut patch = json!({"autoAnalyzeNew": true});
        if explicit {
            patch["autoSubtitleNew"] = json!(true);
        }
        fx.service.update_settings(patch).unwrap();
        fx.insert_asset(2, "video", "v.mp4");
        assert_eq!(fx.service.tick_new_assets().unwrap(), 1);
        assert_eq!(stt_state(&fx, 2), "queued");

        // Act
        let s = fx.service.revoke_consent().unwrap();

        // Assert: 명시 ON 이면 유지, 값이 없으면(분석+동의를 따름) 함께 정리된다
        assert!(!s.consent.given && !s.auto_analyze_new);
        assert_eq!(
            stt_state(&fx, 2),
            if explicit { "queued" } else { "none" },
            "explicit={explicit}"
        );
        // 멱등
        fx.service.revoke_consent().unwrap();
        assert_eq!(stt_state(&fx, 2), if explicit { "queued" } else { "none" });
    }
}

#[test]
fn t_subtitle_40_generate_all_rejects_disabled_switch_and_needs_no_consent() {
    // Arrange: 동의 없음
    let fx = Fx::new();
    fx.insert_asset(1, "video", "a.mp4");
    fx.insert_asset(2, "image", "b.jpg");
    fx.service
        .update_settings(json!({"localStt": {"enabled": false}}))
        .unwrap();

    // Act / Assert: 꺼짐 -> 거부, 아무것도 등록되지 않는다
    assert!(matches!(
        fx.service.generate_subtitles_all(),
        Err(AiFailure::Invalid("localStt.enabled"))
    ));
    assert_eq!(stt_state(&fx, 1), "none");

    // 켜면 동의 없이도 동영상만 등록되고, 다시 실행해도 늘지 않는다
    fx.service
        .update_settings(json!({"localStt": {"enabled": true}}))
        .unwrap();
    let r = fx.service.generate_subtitles_all().unwrap();
    let again = fx.service.generate_subtitles_all().unwrap();
    assert_eq!(r.queued, 1);
    assert_eq!(again.queued, 0);
    assert_eq!(stt_state(&fx, 1), "queued");
    assert_eq!(stt_state(&fx, 2), "none");
}

#[test]
fn t_ipc_subtitle_01_command_contract_queue_result_shape_and_disabled_error_code() {
    // Arrange
    let fx = Fx::new();
    fx.insert_asset(1, "video", "a.mp4");

    // Act
    let ok = serde_json::to_value(fx.service.generate_subtitles_all().unwrap()).unwrap();
    fx.service
        .update_settings(json!({"localStt": {"enabled": false}}))
        .unwrap();
    let err = CommandError::from(fx.service.generate_subtitles_all().unwrap_err());

    // Assert: `{queued, skipped}` camelCase, 꺼짐은 기존 invalid_input
    assert_eq!(ok, json!({"queued": 1, "skipped": 0}));
    assert_eq!(err.code(), ErrorCode::InvalidInput);
    assert_eq!(
        serde_json::to_value(&err).unwrap()["code"],
        json!("invalid_input")
    );
}
