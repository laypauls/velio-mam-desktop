// Categories: Happy (T-ipc-sub-01 계약 모양) / Validation (02 인자·오류 코드) / 원본 보호 (03 임시 파일 수명·위치, 원본·자막 불변) /
// Concurrency (04 동시 호출·LRU 4)
// 엔진 없는 PlayerService 만 쓴다 - 실제 자막 렌더링은 mam-player-worker 의 라이브 시험이 맡는다.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::*;
use crate::player_commands::tests::{canvas, code_of, fingerprint, fixture, no_engine, set_volume_offline};

const KO: &str = "1\n00:00:01,000 --> 00:00:02,500\n안녕 &lt;b&gt;& 세계\n\n2\n00:00:03,000 --> 00:00:04,000\n둘째\n\n";
const EN: &str = "1\n00:00:00,500 --> 00:00:01,500\nhello\n\n";

fn put(media: &Path, name: &str, body: &str) -> PathBuf {
    let p = media.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn names(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .map(|rd| rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

#[test]
fn t_ipc_sub_01_tracks_and_cues_have_the_contract_shape() {
    // Arrange
    let fx = fixture();
    put(&fx.media, "clip.ko.srt", KO);
    put(&fx.media, "clip.eng.srt", EN);
    let svc = SubtitleService::default();

    // Act
    let tracks = svc.tracks(&fx.reuse, fx.wav_id).unwrap();
    let cues = svc.cues(&fx.reuse, fx.wav_id, "clip.ko.srt").unwrap();

    // Assert: camelCase, 형식 소문자, 언어 코드 오름차순(en < ko), 줄바꿈 \n
    assert_eq!(
        serde_json::to_value(&tracks).unwrap(),
        serde_json::json!([
            {"id": "clip.eng.srt", "language": "en", "format": "srt", "fileName": "clip.eng.srt"},
            {"id": "clip.ko.srt", "language": "ko", "format": "srt", "fileName": "clip.ko.srt"},
        ])
    );
    assert_eq!(
        serde_json::to_value(&cues).unwrap(),
        serde_json::json!([
            {"startMs": 1000, "endMs": 2500, "text": "안녕 <b>& 세계"},
            {"startMs": 3000, "endMs": 4000, "text": "둘째"},
        ])
    );
    // 자막이 없는 자산은 오류가 아니라 빈 목록
    std::fs::remove_file(fx.media.join("clip.ko.srt")).unwrap();
    std::fs::remove_file(fx.media.join("clip.eng.srt")).unwrap();
    assert!(svc.tracks(&fx.reuse, fx.wav_id).unwrap().is_empty());
}

#[test]
fn t_ipc_sub_02_validation_and_error_codes_without_paths() {
    // Arrange
    let fx = fixture();
    put(&fx.media, "clip.ko.srt", KO);
    let svc = SubtitleService::default();
    let (player, _events) = no_engine(&fx);

    // Act / Assert: trackId 검증 - 빈 문자열·513자는 invalid_input (자산 해석보다 먼저), 경계 512자는 통과해 not_found
    for bad in [String::new(), "x".repeat(513)] {
        assert_eq!(code_of(&svc.cues(&fx.reuse, fx.wav_id, &bad).unwrap_err()), "invalid_input");
        assert_eq!(code_of(&svc.cues(&fx.reuse, 9_999_999, &bad).unwrap_err()), "invalid_input");
    }
    assert_eq!(code_of(&svc.cues(&fx.reuse, fx.wav_id, &"x".repeat(512)).unwrap_err()), "not_found");
    assert_eq!(code_of(&svc.cues(&fx.reuse, fx.wav_id, &"한".repeat(512)).unwrap_err()), "not_found");
    assert_eq!(code_of(&svc.cues(&fx.reuse, fx.wav_id, "nope.srt").unwrap_err()), "not_found");
    // 알 수 없는 자산 -> asset_offline
    assert_eq!(code_of(&svc.tracks(&fx.reuse, 9_999_999).unwrap_err()), "asset_offline");
    assert_eq!(code_of(&svc.cues(&fx.reuse, 9_999_999, "clip.ko.srt").unwrap_err()), "asset_offline");
    // 원본이 사라짐 -> asset_offline
    let (original, backup) = (fx.media.join("clip.wav"), fx.media.join("clip.bak"));
    std::fs::rename(&original, &backup).unwrap();
    assert_eq!(code_of(&svc.tracks(&fx.reuse, fx.wav_id).unwrap_err()), "asset_offline");
    std::fs::rename(&backup, &original).unwrap();
    // 볼륨 오프라인 -> asset_offline, 메시지에 경로를 싣지 않는다
    let fresh = SubtitleService::default(); // 캐시가 없어야 자산 해석을 거친다
    set_volume_offline(&fx);
    let err = fresh.cues(&fx.reuse, fx.wav_id, "clip.ko.srt").unwrap_err();
    assert_eq!(code_of(&err), "asset_offline");
    assert!(!serde_json::to_string(&err).unwrap().contains("clip"));
    assert_eq!(code_of(&fresh.tracks(&fx.reuse, fx.wav_id).unwrap_err()), "asset_offline");

    // 열린 세션이 없으면 선택·끄기 모두 Ok (엔진 없음 포함), 끄기는 자산을 조회하지 않는다
    player.set_subtitle(fx.wav_id, None).unwrap();
    player.set_subtitle(1, Some(("clip.ko.srt".into(), Vec::new()))).unwrap();
}

#[test]
fn t_ipc_sub_03_temp_srt_lives_only_in_the_cache_dir_and_originals_are_untouched() {
    // Arrange: 이전 실행이 남긴 파일 + 원본·자막
    let fx = fixture();
    let ko = put(&fx.media, "clip.ko.srt", KO);
    let en = put(&fx.media, "clip.eng.srt", EN);
    let dir = fx.data.join("cache").join("player-subtitles");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("stale.srt"), "x").unwrap();
    let before_media = names(&fx.media);
    let before = [fx.media.join("clip.wav"), ko.clone(), en.clone()].map(|p| (p.clone(), fingerprint(&p)));
    let (player, _events) = no_engine(&fx);
    let subs = SubtitleService::default();
    let cues = |id: &str| subs.cues(&fx.reuse, fx.wav_id, id).unwrap();

    // Assert: 시작 때 정리된다 (with_manager)
    assert!(names(&dir).is_empty(), "stale temp file survived startup");

    // Act: 선택 -> 임시 SRT 는 캐시 폴더에만, 같은 트랙은 재사용, 다른 트랙은 새 파일
    player.set_subtitle(fx.wav_id, Some(("clip.ko.srt".into(), cues("clip.ko.srt")))).unwrap();
    let first = names(&dir);
    assert_eq!(first.len(), 1);
    player.set_subtitle(fx.wav_id, Some(("clip.ko.srt".into(), cues("clip.ko.srt")))).unwrap();
    assert_eq!(names(&dir), first, "the same (asset, track) must reuse its file");
    player.set_subtitle(fx.wav_id, Some(("clip.eng.srt".into(), cues("clip.eng.srt")))).unwrap();
    assert_eq!(names(&dir).len(), 2);
    for n in names(&dir) {
        assert!(n.starts_with("sub-") && n.ends_with(".srt"), "{n}");
    }
    // P6: 꺾쇠는 낫표 부호로 치환되고 & 는 그대로, UTF-8 BOM
    let body = std::fs::read_to_string(dir.join(first.iter().next().unwrap())).unwrap();
    assert!(body.starts_with('\u{feff}'));
    assert!(body.contains("안녕 \u{2039}b\u{203a}& 세계"), "{body}");
    // 원본 폴더에는 아무것도 생기지 않았다
    assert_eq!(names(&fx.media), before_media);

    // 닫기 -> 삭제
    player.close().unwrap();
    assert!(names(&dir).is_empty(), "close left temp files");
    // 열기는 (엔진 없음으로) 실패해도 지운다
    player.set_subtitle(fx.wav_id, Some(("clip.ko.srt".into(), cues("clip.ko.srt")))).unwrap();
    assert_eq!(names(&dir).len(), 1);
    assert_eq!(code_of(&player.open(&fx.reuse, fx.wav_id, canvas(), true, None, None, None).unwrap_err()), "player_unavailable");
    assert!(names(&dir).is_empty(), "a failed open left temp files");
    // 종료 -> 폴더째 삭제
    player.set_subtitle(fx.wav_id, Some(("clip.ko.srt".into(), cues("clip.ko.srt")))).unwrap();
    player.shutdown();
    assert!(!dir.exists());

    // 원본 보호: 영상·자막의 내용과 수정 시각 불변
    for (p, fp) in before {
        assert_eq!(fingerprint(&p), fp, "{} changed", p.display());
    }
}

#[test]
fn t_ipc_sub_04_concurrent_calls_agree_and_the_cache_keeps_four_assets() {
    // Arrange
    let fx = fixture();
    put(&fx.media, "clip.ko.srt", KO);
    put(&fx.media, "clip.eng.srt", EN);
    let svc = Arc::new(SubtitleService::default());
    let video = fx.media.join("clip.wav");

    // Act: 같은 자산 동시 호출
    let results: Vec<_> = (0..4)
        .map(|_| {
            let (svc, reuse, id) = (Arc::clone(&svc), Arc::clone(&fx.reuse), fx.wav_id);
            std::thread::spawn(move || (svc.tracks(&reuse, id).unwrap(), svc.cues(&reuse, id, "clip.ko.srt").unwrap()))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();

    // Assert: 모두 같은 결과, 캐시에는 한 번만
    assert!(results.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(svc.cached_assets(), vec![fx.wav_id]);

    // LRU 4: 5번째 자산이 들어오면 가장 오래된 것이 빠지고, 조회(`cues`)는 가장 최근으로 올린다
    let svc = SubtitleService::default();
    for id in 1..=5 {
        svc.rescan(id, &video);
    }
    assert_eq!(svc.cached_assets(), vec![5, 4, 3, 2]);
    svc.cues_cached_for_test(3);
    assert_eq!(svc.cached_assets(), vec![3, 5, 4, 2]);
    svc.rescan(6, &video);
    assert_eq!(svc.cached_assets(), vec![6, 3, 5, 4]);

    // subtitle_tracks 는 항상 다시 탐색한다: 자막을 추가하면 다음 호출에 보인다
    let fresh = SubtitleService::default();
    assert_eq!(fresh.tracks(&fx.reuse, fx.wav_id).unwrap().len(), 2);
    put(&fx.media, "clip.jpn.srt", EN);
    assert_eq!(fresh.tracks(&fx.reuse, fx.wav_id).unwrap().len(), 3);
    assert_eq!(fresh.cached_assets(), vec![fx.wav_id]);
}

impl SubtitleService {
    /// 시험 전용: 자산 해석 없이 캐시 조회만 한다 (캐시 적중이면 경로 클로저는 불리지 않는다)
    fn cues_cached_for_test(&self, asset_id: i64) {
        self.cached_or_scan(asset_id, || panic!("cache miss")).unwrap();
    }
}

#[test]
fn t_ipc_sub_05_request_for_another_open_asset_skips_discovery_and_temp_files() {
    // Arrange: 다른 자산이 열려 있다고 가정 (엔진 없이 상태만)
    let fx = fixture();
    put(&fx.media, "clip.ko.srt", KO);
    let (player, _events) = no_engine(&fx);
    let subs = SubtitleService::default();
    let dir = fx.data.join("cache").join("player-subtitles");
    player.set_current_asset_for_test(Some(fx.wav_id + 1000));

    // Act / Assert: 다른 자산 -> Ok, 탐색(캐시)도 임시 파일도 없다
    set_subtitle_for(&player, &subs, &fx.reuse, fx.wav_id, Some("clip.ko.srt".into())).unwrap();
    assert!(subs.cached_assets().is_empty());
    assert!(names(&dir).is_empty());
    // 인자 검증은 그래도 먼저다
    assert_eq!(code_of(&set_subtitle_for(&player, &subs, &fx.reuse, fx.wav_id, Some(String::new())).unwrap_err()), "invalid_input");
    // 같은 자산이거나 열린 자산을 모르면 평소대로 처리한다 (탐색 + 임시 파일)
    for current in [Some(fx.wav_id), None] {
        player.set_current_asset_for_test(current);
        set_subtitle_for(&player, &subs, &fx.reuse, fx.wav_id, Some("clip.ko.srt".into())).unwrap();
        assert_eq!(subs.cached_assets(), vec![fx.wav_id]);
        assert_eq!(names(&dir).len(), 1);
        player.close().unwrap();
        assert!(names(&dir).is_empty());
    }
}
