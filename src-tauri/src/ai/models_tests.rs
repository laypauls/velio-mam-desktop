// Categories: Happy (목록·다운로드 완료) / Validation (알 수 없는 모델) / External (해시 불일치·오프라인) / Concurrency (중복 시작)
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::Instant;

use mam_ai::models::{Manifest, ModelStore, sha256_file};

use super::super::test_support::CollectSink;
use super::*;

/// 본문을 한 번씩 돌려주는 로컬 스텁 서버 (실제 네트워크 없음).
fn serve(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
        }
    });
    format!("http://127.0.0.1:{port}/model.bin")
}

fn manifest(url: &str, size: usize, sha: &str) -> Manifest {
    Manifest::parse(&format!(
        r#"{{"version":1,"models":[{{"id":"multilingual-e5-small","kind":"text_embedding","display_name":"e5","license":"MIT",
        "license_url":"https://x","source_url":"https://x","dimension":384,
        "files":[{{"path":"model.onnx","url":"{url}","size":{size},"sha256":"{sha}"}}]}}]}}"#
    ))
    .unwrap()
}

fn svc(dir: &Path, m: Manifest) -> (Arc<ModelService>, Arc<CollectSink>) {
    let sink = Arc::new(CollectSink::default());
    (
        ModelService::new(
            Arc::new(ModelStore::new(dir.join("models"), m)),
            sink.clone(),
        ),
        sink,
    )
}

fn wait_until(svc: &ModelService, want: ModelState) -> LocalModel {
    let t = Instant::now();
    loop {
        let m = svc.list().remove(0);
        if m.state == want {
            return m;
        }
        assert!(
            t.elapsed() < Duration::from_secs(15),
            "timeout waiting for {want:?}: {m:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn t_p3w_mdl_01_builtin_manifest_lists_all_models_as_missing() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let (svc, _) = svc(dir.path(), Manifest::builtin().unwrap());
    // Act
    let list = svc.list();
    // Assert
    let purposes: Vec<&str> = list.iter().map(|m| m.purpose.as_str()).collect();
    assert_eq!(
        purposes,
        [
            "clip-image",
            "clip-text",
            "e5",
            "whisper-base",
            "whisper-small"
        ]
    );
    assert!(
        list.iter()
            .all(|m| m.state == ModelState::Missing && m.size_bytes > 0)
    );
    assert!(!svc.any_embedding_ready());
    assert!(matches!(
        svc.start_download("nope"),
        Err(AiFailure::NotFound)
    ));
}

#[test]
fn t_p3w_mdl_02_download_verifies_hash_reports_progress_and_becomes_ready() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let body = vec![7u8; 600_000];
    let probe = dir.path().join("probe");
    std::fs::write(&probe, &body).unwrap();
    let sha = sha256_file(&probe).unwrap();
    let url = serve(body.clone());
    let (svc, sink) = svc(dir.path(), manifest(&url, body.len(), &sha));
    // Act
    svc.start_download("multilingual-e5-small").unwrap();
    svc.start_download("multilingual-e5-small").unwrap(); // 중복 시작은 무시
    let m = wait_until(&svc, ModelState::Ready);
    // Assert
    assert_eq!(m.progress, None);
    assert!(svc.any_embedding_ready());
    assert_eq!(
        std::fs::read(dir.path().join("models/multilingual-e5-small/model.onnx")).unwrap(),
        body
    );
    let states: Vec<String> = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(e, _)| e == MODELS_PROGRESS_EVENT)
        .map(|(_, p)| p["state"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(states.first().map(String::as_str), Some("downloading"));
    assert_eq!(states.last().map(String::as_str), Some("ready"));
    assert_eq!(states.iter().filter(|s| *s == "ready").count(), 1);
    // 준비된 모델은 다시 받지 않는다
    svc.start_download("multilingual-e5-small").unwrap();
}

#[test]
fn t_p3w_mdl_03_hash_mismatch_fails_and_leaves_no_model_file() {
    let dir = tempfile::tempdir().unwrap();
    let body = vec![1u8; 4096];
    let url = serve(body.clone());
    let wrong = "0".repeat(64);
    let (svc, sink) = svc(dir.path(), manifest(&url, body.len(), &wrong));
    svc.start_download("multilingual-e5-small").unwrap();
    let m = wait_until(&svc, ModelState::Failed);
    assert_eq!(m.error_code.as_deref(), Some("hash_mismatch"));
    assert!(
        !dir.path()
            .join("models/multilingual-e5-small/model.onnx")
            .exists()
    );
    assert!(!svc.any_embedding_ready());
    let last = sink.0.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.1["state"], "failed");
    // 실패 뒤 다시 시도할 수 있다
    svc.start_download("multilingual-e5-small").unwrap();
    wait_until(&svc, ModelState::Failed);
}

#[test]
fn t_p3w_mdl_04_unreachable_server_is_offline_code() {
    let dir = tempfile::tempdir().unwrap();
    let (svc, _) = svc(
        dir.path(),
        manifest("http://127.0.0.1:1/model.bin", 10, &"a".repeat(64)),
    );
    svc.start_download("multilingual-e5-small").unwrap();
    let m = wait_until(&svc, ModelState::Failed);
    assert_eq!(m.error_code.as_deref(), Some("offline"));
}
