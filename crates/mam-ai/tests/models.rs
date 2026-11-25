//! T-ai-01~04, 12, 13 - 모델 매니페스트·다운로드·검증.
//! 분류: Happy / External / Business rule / Boundary / Validation. 네트워크는 로컬 스텁 서버만 쓴다.

mod common;

use std::sync::Arc;

use common::{StubServer, sha256_hex};
use mam_ai::AiError;
use mam_ai::models::{DisabledReason, FeatureState, Manifest, ModelStatus, ModelStore, SpaceProbe};

fn manifest_for(url: &str, body: &[u8], sha: &str) -> Manifest {
    let json = format!(
        r#"{{"version":1,"models":[{{"id":"m1","kind":"text_embedding","display_name":"t","license":"MIT",
        "license_url":"https://x","source_url":"https://x","dimension":4,
        "files":[{{"path":"sub/model.bin","url":"{url}","size":{},"sha256":"{sha}"}}]}}]}}"#,
        body.len()
    );
    Manifest::parse(&json).expect("manifest")
}

fn payload(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 % 251) as u8).collect()
}

fn no_progress() -> impl FnMut(&str, u64, u64) {
    |_, _, _| {}
}

#[test]
fn t_ai_01_builtin_manifest_parses_and_pins_licenses() {
    // Arrange / Act
    let m = Manifest::builtin().expect("builtin manifest must be valid");
    // Assert: 상업 이용 가능 라이선스만, 모든 파일에 SHA-256 과 https URL
    assert!(!m.models.is_empty());
    for model in &m.models {
        assert!(
            ["MIT", "Apache-2.0"].contains(&model.license.as_str()),
            "{} has license {}",
            model.id,
            model.license
        );
        assert!(model.total_size() > 0);
    }
}

#[test]
fn t_ai_01_download_verifies_sha256_and_moves_atomically() {
    // Arrange
    let body = payload(700_000);
    let server = StubServer::start(body.clone());
    let manifest = manifest_for(&server.url("/file"), &body, &sha256_hex(&body));
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path(), manifest);
    assert_eq!(store.status("m1").unwrap(), ModelStatus::NotDownloaded);

    // Act
    let mut seen = 0u64;
    let paths = store.ensure("m1", &mut |_, got, total| {
        assert_eq!(total, 700_000);
        seen = got;
    });

    // Assert
    let paths = paths.expect("download");
    assert_eq!(seen, 700_000);
    let file = paths.file("sub/model.bin").unwrap();
    assert_eq!(std::fs::read(file).unwrap(), body);
    assert!(!file.with_extension("bin.part").exists(), "no .part left");
    assert!(matches!(store.status("m1").unwrap(), ModelStatus::Ready(_)));
    assert!(store.verify("m1").unwrap().is_empty());

    // 이미 있으면 네트워크를 쓰지 않는다 (멱등)
    let before = server.request_count();
    store.ensure("m1", &mut no_progress()).unwrap();
    assert_eq!(server.request_count(), before);
}

#[test]
fn t_ai_02_hash_mismatch_deletes_part_and_fails() {
    // Arrange: 매니페스트 해시가 실제 내용과 다르다
    let body = payload(10_000);
    let server = StubServer::start(body.clone());
    let manifest = manifest_for(&server.url("/file"), &body, &"0".repeat(64));
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path(), manifest);

    // Act
    let err = store.ensure("m1", &mut no_progress()).unwrap_err();

    // Assert
    assert!(matches!(err, AiError::HashMismatch { .. }), "{err}");
    let model_dir = dir.path().join("m1").join("sub");
    assert!(!model_dir.join("model.bin").exists());
    assert!(!model_dir.join("model.bin.part").exists());
    assert_eq!(store.status("m1").unwrap(), ModelStatus::NotDownloaded);
}

#[test]
fn t_ai_02_http_404_is_an_error_and_leaves_nothing() {
    let body = payload(1_000);
    let server = StubServer::start(body.clone());
    server.set(|b| b.status = Some(404));
    let manifest = manifest_for(&server.url("/file"), &body, &sha256_hex(&body));
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path(), manifest);

    let err = store.ensure("m1", &mut no_progress()).unwrap_err();

    assert!(matches!(err, AiError::Http { status: 404, .. }), "{err}");
    assert_eq!(store.status("m1").unwrap(), ModelStatus::NotDownloaded);
    // prepare 는 오류를 전파하지 않고 비활성 상태로 바꾼다
    assert!(matches!(
        store.prepare("m1", &mut no_progress()),
        FeatureState::Disabled(DisabledReason::DownloadFailed(_))
    ));
}

#[test]
fn t_ai_02_interrupted_download_resumes_with_range() {
    // Arrange: 300,000 바이트만 보내고 끊는 서버
    let body = payload(900_000);
    let server = StubServer::start(body.clone());
    server.set(|b| b.cut_after = Some(300_000));
    let manifest = manifest_for(&server.url("/file"), &body, &sha256_hex(&body));
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path(), manifest);

    // Act 1: 중단
    let err = store.ensure("m1", &mut no_progress()).unwrap_err();
    assert!(matches!(err, AiError::Interrupted { .. }), "{err}");
    let part = dir.path().join("m1").join("sub").join("model.bin.part");
    let kept = std::fs::metadata(&part).expect("part kept").len();
    assert!(kept > 0 && kept < 900_000, "kept {kept}");
    assert!(!dir.path().join("m1/sub/model.bin").exists());

    // Act 2: 서버 복구 후 재시도 -> 이어받기
    server.set(|b| b.cut_after = None);
    let paths = store.ensure("m1", &mut no_progress()).expect("resume");

    // Assert
    assert_eq!(
        std::fs::read(paths.file("sub/model.bin").unwrap()).unwrap(),
        body
    );
    let last = server.requests.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last, Some(format!("bytes={kept}-")));
}

#[test]
fn t_ai_13_server_ignoring_range_restarts_from_zero() {
    // Arrange: 앞부분 부분 파일이 있고, 서버는 Range 를 무시한다
    let body = payload(200_000);
    let server = StubServer::start(body.clone());
    server.set(|b| b.ignore_range = true);
    let manifest = manifest_for(&server.url("/file"), &body, &sha256_hex(&body));
    let dir = tempfile::tempdir().unwrap();
    let part_dir = dir.path().join("m1").join("sub");
    std::fs::create_dir_all(&part_dir).unwrap();
    std::fs::write(part_dir.join("model.bin.part"), &body[..50_000]).unwrap();
    let store = ModelStore::new(dir.path(), manifest);

    // Act
    let paths = store.ensure("m1", &mut no_progress()).expect("download");

    // Assert: 200 전체를 받아 처음부터 덮어썼다
    assert_eq!(
        std::fs::read(paths.file("sub/model.bin").unwrap()).unwrap(),
        body
    );
}

#[test]
fn t_ai_13_corrupt_part_is_detected_by_hash_and_discarded() {
    // Arrange: 부분 파일 앞부분이 손상됨 (같은 길이의 다른 바이트)
    let body = payload(100_000);
    let server = StubServer::start(body.clone());
    let manifest = manifest_for(&server.url("/file"), &body, &sha256_hex(&body));
    let dir = tempfile::tempdir().unwrap();
    let part_dir = dir.path().join("m1").join("sub");
    std::fs::create_dir_all(&part_dir).unwrap();
    std::fs::write(part_dir.join("model.bin.part"), vec![0xAAu8; 40_000]).unwrap();
    let store = ModelStore::new(dir.path(), manifest);

    // Act 1: 이어받기 결과의 전체 해시가 달라 실패하고 부분 파일을 버린다
    let err = store.ensure("m1", &mut no_progress()).unwrap_err();
    assert!(matches!(err, AiError::HashMismatch { .. }), "{err}");
    assert!(!part_dir.join("model.bin.part").exists());

    // Act 2: 재시도는 처음부터 받아 성공
    store.ensure("m1", &mut no_progress()).expect("clean retry");
    assert!(store.verify("m1").unwrap().is_empty());
}

#[test]
fn t_ai_03_offline_disables_only_that_feature() {
    // Arrange: 모델 A 는 이미 받아 둠, 모델 B 는 닫힌 포트(오프라인)
    let body_a = payload(5_000);
    let server = StubServer::start(body_a.clone());
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    }; // 리스너를 닫아 연결 거부
    let json = format!(
        r#"{{"version":1,"models":[
        {{"id":"a","kind":"stt","display_name":"a","license":"MIT","license_url":"https://x","source_url":"https://x",
          "files":[{{"path":"a.bin","url":"{}","size":{},"sha256":"{}"}}]}},
        {{"id":"b","kind":"stt","display_name":"b","license":"MIT","license_url":"https://x","source_url":"https://x",
          "files":[{{"path":"b.bin","url":"http://{closed}/b","size":10,"sha256":"{}"}}]}}]}}"#,
        server.url("/file"),
        body_a.len(),
        sha256_hex(&body_a),
        "1".repeat(64)
    );
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path(), Manifest::parse(&json).unwrap());
    store.ensure("a", &mut no_progress()).unwrap();
    drop(server); // 이후 네트워크 없음

    // Act
    let b = store.prepare("b", &mut no_progress());
    let a = store.prepare("a", &mut no_progress());

    // Assert: B 만 비활성(오프라인), A 는 네트워크 없이 사용 가능
    assert_eq!(b, FeatureState::Disabled(DisabledReason::Offline));
    assert!(matches!(a, FeatureState::Ready(_)));
    assert!(matches!(
        store.ensure("b", &mut no_progress()),
        Err(AiError::Offline { .. })
    ));
}

struct FixedSpace(u64);
impl SpaceProbe for FixedSpace {
    fn available(&self, _dir: &std::path::Path) -> Option<u64> {
        Some(self.0)
    }
}

#[test]
fn t_ai_04_insufficient_space_fails_before_any_download() {
    // Arrange: 여유 100 바이트
    let body = payload(50_000);
    let server = StubServer::start(body.clone());
    let manifest = manifest_for(&server.url("/file"), &body, &sha256_hex(&body));
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path(), manifest).with_space_probe(Arc::new(FixedSpace(100)));

    // Act
    let err = store.ensure("m1", &mut no_progress()).unwrap_err();

    // Assert: 요청 자체를 보내지 않았다
    assert!(
        matches!(
            err,
            AiError::InsufficientSpace {
                needed: 50_000,
                available: 100,
                ..
            }
        ),
        "{err}"
    );
    assert_eq!(server.request_count(), 0);
    assert!(matches!(
        store.prepare("m1", &mut no_progress()),
        FeatureState::Disabled(DisabledReason::DiskFull { .. })
    ));
}

#[test]
fn t_ai_04_write_failure_returns_io_error_and_no_final_file() {
    // Arrange: `.part` 자리에 폴더가 있어 쓰기를 열 수 없다 (쓰기 실패 모사)
    let body = payload(20_000);
    let server = StubServer::start(body.clone());
    let manifest = manifest_for(&server.url("/file"), &body, &sha256_hex(&body));
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("m1/sub/model.bin.part")).unwrap();
    let store = ModelStore::new(dir.path(), manifest);

    // Act
    let err = store.ensure("m1", &mut no_progress()).unwrap_err();

    // Assert
    assert!(matches!(err, AiError::Io { .. }), "{err}");
    assert!(!dir.path().join("m1/sub/model.bin").exists());
}

#[test]
fn t_ai_12_manifest_rejects_unsafe_entries() {
    let file = |path: &str, sha: &str, url: &str| {
        format!(
            r#"{{"version":1,"models":[{{"id":"m","kind":"stt","display_name":"x","license":"MIT","license_url":"https://x",
            "source_url":"https://x","files":[{{"path":"{path}","url":"{url}","size":1,"sha256":"{sha}"}}]}}]}}"#
        )
    };
    let ok_sha = "a".repeat(64);
    // 경로 탈출 / 절대 경로 / 역슬래시 / 짧은 해시 / 대문자 해시 / 평문 http 외부 URL
    for json in [
        file("../evil.bin", &ok_sha, "https://x/y"),
        file("/abs.bin", &ok_sha, "https://x/y"),
        file("a\\\\b.bin", &ok_sha, "https://x/y"),
        file("ok.bin", "abc", "https://x/y"),
        file("ok.bin", &"A".repeat(64), "https://x/y"),
        file("ok.bin", &ok_sha, "http://example.com/y"),
    ] {
        assert!(
            matches!(Manifest::parse(&json), Err(AiError::Manifest(_))),
            "should reject: {json}"
        );
    }
    assert!(Manifest::parse(&file("ok.bin", &ok_sha, "https://x/y")).is_ok());
    assert!(matches!(
        Manifest::parse("{not json"),
        Err(AiError::Manifest(_))
    ));
    let m = Manifest::parse(&file("ok.bin", &ok_sha, "https://x/y")).unwrap();
    assert!(matches!(m.get("nope"), Err(AiError::UnknownModel(_))));
}
