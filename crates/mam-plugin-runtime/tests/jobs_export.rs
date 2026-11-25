//! P5 내보내기 (T-pjob-22 / 25) — provider 출력이 정확한 바이트로 원자적으로 쓰이고, 쓰기 실패는 부분 파일 없이 `io`.
//!
//! 분류: Happy(text·base64 정확한 바이트, 기존 파일 원자 교체, 자산 정렬·중복 제거) / Boundary(빈 출력) / External(쓰기 실패 -> Io) /
//! Business rule(원본 미디어 내용·수정 시각 불변, 임시 파일 `.mamtmp` 잔류 없음).
//! 실제 SQLite·실제 워커·임시 폴더. 게이트 ON: 플러그인 활성 + `export.write` 허용.

mod common_jobs;

use std::fs::{self, File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use common_jobs::{EXPORTER, Env, exporter, snapshot};
use mam_plugin_runtime::jobs::ExportError;
use serde_json::json;

fn world() -> (Env, Vec<i64>) {
    let env = Env::new();
    env.install(exporter());
    let assets = vec![
        env.add_asset("image", "a.jpg"),
        env.add_asset("image", "b.jpg"),
    ];
    (env, assets)
}

fn leftovers(dir: &std::path::Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".mamtmp"))
        .collect()
}

#[test]
fn t_pjob_22_text_output_is_written_byte_for_byte_replacing_an_existing_file_atomically() {
    // Arrange: 기존 파일이 있고, provider 호출이 끝난 직후(쓰기 전)에는 아직 옛 내용이어야 한다
    let (env, assets) = world();
    let (dest, before) = (env.out_dir().join("report.csv"), snapshot(&env.lib));
    fs::write(&dest, b"old contents").unwrap();
    let text = "héllo,\r\n한글\n\"q\"\u{0}end";
    env.invoker.script(vec![json!({"out": {"text": text}})]);
    let seen = Arc::new(Mutex::new(None));
    let (d, s) = (dest.clone(), Arc::clone(&seen));
    env.invoker
        .set_after(move |_| *s.lock().unwrap() = Some(fs::read(&d).unwrap()));

    // Act
    let result = env.export(EXPORTER, "exp.step", &assets, &dest).unwrap();

    // Assert: 정확한 UTF-8 바이트(CRLF·NUL 변환 없음), 쓰기 전에는 옛 내용, 임시 파일 없음
    assert_eq!(fs::read(&dest).unwrap(), text.as_bytes());
    assert_eq!(result.bytes_written, text.len() as u64);
    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some(&b"old contents"[..]),
        "dest is replaced only by the final rename"
    );
    assert!(leftovers(&env.out_dir()).is_empty());
    assert_eq!(
        snapshot(&env.lib),
        before,
        "original media: content and mtime unchanged"
    );
}

#[test]
fn t_pjob_22_base64_output_is_decoded_to_exact_bytes_and_empty_output_creates_an_empty_file() {
    // Arrange
    let (env, assets) = world();
    let bytes: Vec<u8> = vec![0, 1, 2, 255, 13, 10, 26, 0, 0x80, 0xFE];
    let (dest, empty) = (
        env.out_dir().join("bin.csv"),
        env.out_dir().join("empty.csv"),
    );
    env.invoker.script(vec![
        json!({"out": {"dataBase64": STANDARD.encode(&bytes)}}),
        json!({"out": {"text": ""}}),
    ]);

    // Act
    let bin = env.export(EXPORTER, "exp.step", &assets, &dest).unwrap();
    let none = env.export(EXPORTER, "exp.step", &assets, &empty).unwrap();

    // Assert
    assert_eq!(
        (fs::read(&dest).unwrap(), bin.bytes_written),
        (bytes.clone(), bytes.len() as u64)
    );
    assert_eq!(
        (fs::read(&empty).unwrap().len(), none.bytes_written),
        (0, 0)
    );
    assert!(leftovers(&env.out_dir()).is_empty());
}

#[test]
fn t_pjob_22_provider_receives_sorted_deduped_asset_ids_and_kind_export() {
    // Arrange
    let (env, assets) = world();
    let dest = env.out_dir().join("ids.csv");
    let ids = vec![assets[1], assets[0], assets[1], assets[0]];

    // Act
    env.export(EXPORTER, "exp.echo", &ids, &dest).unwrap();

    // Assert
    assert_eq!(
        fs::read_to_string(&dest).unwrap(),
        format!("{},{}|export", assets[0], assets[1])
    );
}

#[test]
fn t_pjob_25_parent_folder_removed_during_the_provider_call_fails_with_io_and_leaves_nothing() {
    // Arrange: provider 호출이 끝난 직후 대상 폴더가 사라진다
    let (env, assets) = world();
    let out = env.out_dir().join("vanish");
    fs::create_dir(&out).unwrap();
    let dest = out.join("report.csv");
    env.invoker.script(vec![json!({"out": {"text": "data"}})]);
    let o = out.clone();
    env.invoker
        .set_after(move |_| fs::remove_dir_all(&o).unwrap());

    // Act
    let err = env
        .export(EXPORTER, "exp.step", &assets, &dest)
        .unwrap_err();

    // Assert
    assert!(matches!(err, ExportError::Io(_)), "{err:?}");
    assert!(!dest.exists() && !out.exists());
}

#[test]
fn t_pjob_25_rename_failure_keeps_the_old_file_intact_and_removes_the_temp_file() {
    // Arrange: 기존 대상 파일을 공유 없이 열어 둔다 -> 마지막 rename 이 실패한다
    let (env, assets) = world();
    let dest = env.out_dir().join("held.csv");
    fs::write(&dest, b"old contents").unwrap();
    env.invoker
        .script(vec![json!({"out": {"text": "new contents"}})]);
    let held: Arc<Mutex<Option<File>>> = Arc::default();
    let (d, h) = (dest.clone(), Arc::clone(&held));
    env.invoker.set_after(move |_| {
        *h.lock().unwrap() = Some(
            OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&d)
                .unwrap(),
        )
    });

    // Act
    let err = env
        .export(EXPORTER, "exp.step", &assets, &dest)
        .unwrap_err();
    held.lock().unwrap().take();

    // Assert: io 오류, 옛 파일 그대로, 임시 파일 없음 (부분 파일 없음)
    assert!(matches!(err, ExportError::Io(_)), "{err:?}");
    assert_eq!(fs::read(&dest).unwrap(), b"old contents");
    assert!(leftovers(&env.out_dir()).is_empty());
    assert_eq!(fs::read_dir(env.out_dir()).unwrap().count(), 1);

    // 같은 대상에 다시 내보내면 성공한다 (실패가 상태를 남기지 않는다)
    env.invoker.clear_hooks();
    env.invoker
        .script(vec![json!({"out": {"text": "new contents"}})]);
    env.export(EXPORTER, "exp.step", &assets, &dest).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"new contents");
}
