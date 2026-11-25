//! 샘플 플러그인 `examples/plugins/exif-summary` 시험 — 저장소의 실제 폴더를 실제 설치 경로(`inspect_dir` -> `install`)로
//! 임시 MAMData 에 설치하고, 실제 워커·실제 SQLite 에서 메뉴 명령과 자동 provider(변경 피드 경유)를 돌린다.
//!
//! 분류: Happy(설치·명령·provider 의 KV·태그 기록, 로케일 폴백) / Boundary(크기 정보 없는 자산, 범위 안의 없는 자산·영상 건너뜀) /
//! Business rule(최소 권한 — kv.write 없이는 명령이 쓰지 못한다, 원본 폴더 불변). 대기는 채널·큐 condvar 뿐이다.

mod common_integ;
mod common_rt;

use std::path::{Path, PathBuf};

use common_integ::Integ;
use common_rt::{EN, file_list, snapshot};
use mam_core::jobs::JobOutcome;
use mam_plugin_runtime::{InstallChange, Permission, PluginStatus};
use serde_json::{Value, json};

const ID: &str = "com.example.exif-summary";

fn sample_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("examples").join("plugins").join("exif-summary")
}

fn perms(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn declared() -> Vec<String> {
    perms(&["asset.read.metadata", "kv.write", "tag.write"])
}

fn set_size(it: &Integ, asset: i64, w: i64, h: i64) {
    it.rt.exec(&format!("INSERT INTO media_metadata (asset_id, width, height) VALUES ({asset}, {w}, {h})"));
}

/// 플러그인 KV 값 (`asset` 이 `None` 이면 전역 키)
fn kv(it: &Integ, asset: Option<i64>, key: &str) -> Option<Value> {
    let key = key.to_owned();
    it.rt
        .pool
        .read(move |c| {
            let text: Option<String> = match asset {
                Some(a) => c.query_row("SELECT value FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = ?1 AND asset_id = ?2 AND key = ?3", (ID, a, &key), |r| r.get(0)).ok(),
                None => c.query_row("SELECT value FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = ?1 AND asset_id IS NULL AND key = ?2", (ID, &key), |r| r.get(0)).ok(),
            };
            Ok(text)
        })
        .unwrap()
        .map(|t| serde_json::from_str(&t).unwrap())
}

fn kv_rows(it: &Integ) -> i64 {
    it.count(&format!("SELECT COUNT(*) FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id = '{ID}'"))
}

#[test]
fn t_psample_01_installs_through_the_real_path_with_minimal_permissions_and_leaves_the_source_untouched() {
    // Arrange
    let it = Integ::new();
    let dir = sample_dir();
    let before = snapshot(&dir);

    // Act
    let preview = it.rt.rt.inspect_dir(&dir, EN).expect("the sample manifest must validate");
    let info = it.rt.rt.install(&dir, &preview.id, &preview.version, declared(), EN).expect("install the sample");

    // Assert — 미리보기: 필요한 권한만 요청, 서명 없음, 새 설치
    assert_eq!((preview.id.as_str(), preview.version.as_str(), preview.display_name.as_str()), (ID, "1.0.0", "EXIF Summary"));
    assert_eq!(preview.permissions, vec![Permission::AssetReadMetadata, Permission::KvWrite, Permission::TagWrite]);
    assert!(preview.unsigned && preview.change == InstallChange::Install);
    assert_eq!(preview.commands.len(), 1);
    assert_eq!(preview.providers.len(), 1);
    // 설치 결과 — 복사된 것은 manifest·main·로케일뿐
    assert_eq!((info.status, info.enabled), (PluginStatus::Ok, true));
    let installed = it.rt.plugins_dir.join(ID).join("1.0.0");
    assert_eq!(file_list(&installed), vec!["l10n/en.json", "l10n/ko.json", "main.js", "plugin.json"]);
    // 원본 보호 — 내용·수정 시각 불변
    assert_eq!(snapshot(&dir), before);
}

#[test]
fn t_psample_02_menu_entry_follows_the_locale_fallback_current_then_en() {
    // Arrange
    let it = Integ::new();
    it.rt.rt.install(&sample_dir(), ID, "1.0.0", declared(), EN).unwrap();

    // Act
    let ko = it.rt.rt.contributions("ko").menu_items;
    let fr = it.rt.rt.contributions("fr").menu_items;

    // Assert — 한국어 파일 -> 한국어, 없는 로케일 -> en (id 가 아니다)
    assert_eq!(ko.len(), 1);
    assert_eq!((ko[0].command_id.as_str(), ko[0].title.as_str()), ("summarize", "선택한 이미지 요약"));
    assert_eq!(ko[0].when.as_deref(), Some("selection.count >= 1 && asset.kind == 'image'"));
    assert_eq!(fr[0].title, "Summarize selected images");
}

#[test]
fn t_psample_03_command_summarizes_images_skips_others_and_writes_the_global_kv() {
    // Arrange — 가로·세로 이미지, 영상, 범위 안이지만 없는 자산
    let it = Integ::new();
    it.rt.rt.install(&sample_dir(), ID, "1.0.0", declared(), EN).unwrap();
    let (land, port, video) = (it.rt.add_asset("a.jpg"), it.rt.add_asset("b.jpg"), it.rt.add_asset("c.mp4"));
    set_size(&it, land, 4000, 3000);
    set_size(&it, port, 100, 200);
    it.rt.exec(&format!("UPDATE media_assets SET media_type = 'video' WHERE asset_id = {video}"));
    let missing = video + 1000;

    // Act
    let out = it.rt.rt.run_command(ID, "summarize", vec![land, port, video, missing]).expect("command runs");

    // Assert
    assert_eq!(out.message.as_deref(), Some("Summarized 2 image(s): 1 landscape, 1 portrait, 0 square, 2 skipped"));
    assert_eq!(kv(&it, None, "lastSummary"), Some(json!({ "images": 2, "landscape": 1, "portrait": 1, "square": 0, "unknown": 0 })));
    assert_eq!(kv_rows(&it), 1, "the command writes only the global summary");
}

#[test]
fn t_psample_04_command_with_no_images_writes_nothing() {
    // Arrange
    let it = Integ::new();
    it.rt.rt.install(&sample_dir(), ID, "1.0.0", declared(), EN).unwrap();
    let video = it.rt.add_asset("c.mp4");
    it.rt.exec(&format!("UPDATE media_assets SET media_type = 'video' WHERE asset_id = {video}"));

    // Act
    let out = it.rt.rt.run_command(ID, "summarize", vec![video]).unwrap();

    // Assert
    assert_eq!(out.message.as_deref(), Some("No images to summarize"));
    assert_eq!(kv_rows(&it), 0);
}

#[test]
fn t_psample_05_command_cannot_write_without_the_kv_write_grant() {
    // Arrange — 사용자가 kv.write 를 허용하지 않았다 (게이트: 허용 ∩ 선언)
    let it = Integ::new();
    it.rt.rt.install(&sample_dir(), ID, "1.0.0", perms(&["asset.read.metadata", "tag.write"]), EN).unwrap();
    let a = it.rt.add_asset("a.jpg");
    set_size(&it, a, 10, 10);

    // Act
    let out = it.rt.rt.run_command(ID, "summarize", vec![a]);

    // Assert — 읽기는 되지만 쓰기에서 permission_denied 로 명령이 실패하고 아무것도 남지 않는다
    let err = out.expect_err("kv.set must be refused").to_string();
    assert!(err.contains("permission denied: kv.write"), "{err}");
    assert_eq!(kv_rows(&it), 0);
}

#[test]
fn t_psample_06_provider_records_kv_summary_and_orientation_tags_through_the_feed() {
    // Arrange
    let it = Integ::new();
    it.rt.rt.install(&sample_dir(), ID, "1.0.0", declared(), EN).unwrap();
    let a = it.rt.add_asset("photo.jpg");
    set_size(&it, a, 4000, 3000);

    // Act — 사용자 편집 -> outbox -> 피드 -> 실제 러너 -> 실제 워커 provider
    it.edit(a, "user-tag");
    it.jobs.feed_tick();
    let done = it.run(1);

    // Assert
    assert!(matches!(done[0], Ok(JobOutcome::Done)), "{:?}", done[0]);
    let s = kv(&it, Some(a), "summary").expect("summary stored for the asset");
    assert_eq!((s["fileName"].as_str(), s["width"].as_i64(), s["height"].as_i64()), (Some("photo.jpg"), Some(4000), Some(3000)));
    assert_eq!((s["orientation"].as_str(), s["megapixels"].as_f64(), s["capturedAt"].is_null()), (Some("landscape"), Some(12.0), true));
    assert!(s.get("pixels").is_none(), "internal helper value is not stored");
    assert_eq!(it.tags(a), vec!["high-res".to_owned(), "landscape".to_owned(), "user-tag".to_owned()]);
}

#[test]
fn t_psample_07_provider_handles_an_asset_without_dimensions_and_small_portraits() {
    // Arrange — 크기 정보 없는 자산과 작은 세로 이미지
    let it = Integ::new();
    it.rt.rt.install(&sample_dir(), ID, "1.0.0", declared(), EN).unwrap();
    let (unknown, small) = (it.rt.add_asset("x.jpg"), it.rt.add_asset("y.jpg"));
    set_size(&it, small, 100, 200);

    // Act
    it.edit(unknown, "u");
    it.jobs.feed_tick();
    let first = it.run(1);
    it.edit(small, "u");
    it.jobs.feed_tick();
    let second = it.run(1);

    // Assert — 크기를 모르면 방향 태그 없이 요약만, 작은 세로 이미지는 high-res 없음
    assert!(matches!(first[0], Ok(JobOutcome::Done)) && matches!(second[0], Ok(JobOutcome::Done)));
    let s = kv(&it, Some(unknown), "summary").unwrap();
    assert!(s["orientation"].is_null() && s["width"].is_null() && s["megapixels"].is_null());
    assert_eq!(it.tags(unknown), vec!["u".to_owned()]);
    assert_eq!(it.tags(small), vec!["portrait".to_owned(), "u".to_owned()]);
}
