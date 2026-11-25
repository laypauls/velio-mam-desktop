//! T-aij-35~39 - 영상은 P2-3 프레임 스트립의 칸으로 외부 분석·시각 임베딩: 개수 제한, 프레임별 벡터, 미생성 재시도, 대체.
//! 분류: Happy / Boundary(개수 보정) / Business rule(원본 비접근) / External(미생성·미지원).

#[path = "common/ai_env.rs"]
mod ai_env;
mod common;

use ai_env::AiEnv;
use mam_core::ai_jobs::{EmbedMode, StepKind};
use mam_core::jobs::JobOutcome;
use mam_media::strip_path;

#[test]
fn t_aij_35_video_sends_strip_frames_up_to_video_frames_and_embeds_each() {
    // Arrange: 기본 videoFrames = 5, 스트립 칸 = 5
    let e = AiEnv::new();
    e.enable_vectors();
    let id = e.add_asset("video");
    let before = common::env::snapshot(&e.env.vol);
    e.queue_all(id, None, false);

    // Act
    let out = e.run_next(&|| false).unwrap();

    // Assert
    assert_eq!(out, Ok(JobOutcome::Done));
    let ext = e.ext.calls.lock().unwrap();
    assert_eq!(ext.len(), 1);
    let idx: Vec<_> = ext[0]
        .images
        .iter()
        .map(|i| i.frame.map(|f| (f.index, f.cells)))
        .collect();
    assert_eq!(
        idx,
        (0..5).map(|i| Some((i, 5))).collect::<Vec<_>>(),
        "스트립 칸 5개를 위치 순으로"
    );
    let strip = strip_path(&e.cache, id);
    assert!(
        ext[0]
            .images
            .iter()
            .all(|i| i.path == strip && i.path.starts_with(&e.cache)),
        "프레임은 MAMData 스트립뿐 - 원본 경로 없음"
    );
    drop(ext);
    let emb = e.emb.calls.lock().unwrap();
    assert_eq!((emb[0].mode, emb[0].images.len()), (EmbedMode::Full, 5));
    drop(emb);
    assert_eq!(
        e.visual_sources(id),
        (0..5)
            .map(|i| ("frame".to_string(), Some(i)))
            .collect::<Vec<_>>(),
        "프레임마다 Frame 벡터 + frame_index"
    );
    assert_eq!(common::env::snapshot(&e.env.vol), before, "원본 보호");
}

#[test]
fn t_aij_36_video_frames_setting_limits_and_clamps_the_frame_count() {
    for (want, expect) in [
        (3u8, vec![0u32, 2, 4]),
        (1, vec![2]),
        (9, vec![0, 1, 2, 3, 4]),
    ] {
        // Arrange
        let e = AiEnv::new();
        e.enable_vectors();
        let id = e.add_asset("video");
        e.set(|s| s.transfer.video_frames = want);
        e.queue_all(id, None, false);

        // Act
        e.run_next(&|| false).unwrap().unwrap();

        // Assert: 외부 전송과 시각 벡터가 같은 칸 집합
        let sent: Vec<u32> = e.ext.calls.lock().unwrap()[0]
            .images
            .iter()
            .map(|i| i.frame.unwrap().index)
            .collect();
        let stored: Vec<Option<i64>> = e.visual_sources(id).into_iter().map(|s| s.1).collect();
        assert_eq!(sent, expect, "videoFrames={want}");
        assert_eq!(
            stored,
            expect
                .iter()
                .map(|i| Some(i64::from(*i)))
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn t_aij_37_missing_strip_is_retryable_and_sends_nothing() {
    // done 인데 파일 없음 - 재시도 대상 (행 없음/pending 은 시도 없이 기다린다: T-aij-40)
    // Arrange
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.set_strip(id, Some("done"), false);
    e.set(|s| s.stt_enabled = false);
    e.queue_all(id, Some(&[StepKind::Describe, StepKind::Embedding]), false);

    // Act
    let err = e.run_next(&|| false).unwrap().unwrap_err();

    // Assert
    assert!(err.retryable);
    assert!(e.ext.calls.lock().unwrap().is_empty());
    assert!(e.emb.calls.lock().unwrap().is_empty());
    for k in ["describe", "embedding"] {
        assert_eq!(
            e.step(id, k),
            Some(("queued".into(), Some("thumbnail_missing".into()))),
            "{k}"
        );
    }
}

#[test]
fn t_aij_38_unsupported_or_failed_strip_falls_back_to_the_single_thumbnail() {
    for state in ["unsupported", "failed"] {
        // Arrange
        let e = AiEnv::new();
        e.enable_vectors();
        let id = e.add_asset("video");
        e.set_strip(id, Some(state), false);
        e.queue_all(id, None, false);

        // Act
        let out = e.run_next(&|| false).unwrap();

        // Assert: 외부·임베딩 모두 썸네일 1장 (프레임 아님), 벡터는 Image 1개
        assert_eq!(out, Ok(JobOutcome::Done), "{state}");
        let ext = e.ext.calls.lock().unwrap();
        assert_eq!(ext[0].images.len(), 1, "{state}");
        assert!(ext[0].images[0].frame.is_none() && ext[0].images[0].path.starts_with(&e.cache));
        drop(ext);
        assert_eq!(e.visual_sources(id), vec![("image".to_string(), None)]);
    }
}

#[test]
fn t_aij_39_fallback_without_thumbnail_and_plain_image_paths() {
    // 스트립 미지원 + 썸네일도 없음 -> 재시도
    let e = AiEnv::new();
    let id = e.add_asset("video");
    e.set_strip(id, Some("unsupported"), false);
    e.remove_thumb(id);
    e.queue_all(id, Some(&[StepKind::Describe]), false);
    let err = e.run_next(&|| false).unwrap().unwrap_err();
    assert!(err.retryable);
    assert_eq!(
        e.step(id, "describe"),
        Some(("queued".into(), Some("thumbnail_missing".into())))
    );

    // 이미지는 스트립과 무관하게 썸네일 1장
    let img = e.add_asset("image");
    e.queue_all(img, Some(&[StepKind::Describe]), false);
    e.run_next(&|| false).unwrap().unwrap();
    let ext = e.ext.calls.lock().unwrap();
    let last = ext.last().unwrap();
    assert_eq!((last.asset_id, last.images.len()), (img, 1));
    assert!(last.images[0].frame.is_none());
}
