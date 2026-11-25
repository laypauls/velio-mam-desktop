//! `asset.get`·`asset.getDerived` (T-pgw-07 ~ 16) — 실제 SQLite, 실제 WebP 썸네일(`mam_media`), 임시 폴더 원본.
//!
//! 분류: Happy / Business rule(경로 미유출, 범위) / Boundary(상태·크기·예산) / External(파일 없음) / Validation

mod common;

use common::*;
use mam_plugin_host::InvokeScope;
use serde_json::{Value, json};

const LEAKS: [&str; 7] = ["SecretLibrary", "SecretVolumeLabel", "SecretMount", "vol-x", "secretdir", "deep", "Z:"];

fn setup() -> (Fx, mam_plugin_manifest::PluginId) {
    let fx = Fx::new();
    let p = fx.full_plugin("com.example.assets");
    (fx, p)
}

#[test]
fn t_pgw_07_asset_get_returns_all_fields_for_image_video_audio() {
    // Arrange
    let (fx, p) = setup();
    let img = fx.add_asset("image", "secretdir/photo.jpg");
    let vid = fx.add_asset("video", "secretdir/clip.mp4");
    let aud = fx.add_asset("audio", "secretdir/song.wav");
    fx.exec(&format!(
        "INSERT INTO media_metadata (asset_id, width, height, duration_ms, captured_at) VALUES ({img}, 4000, 3000, NULL, 1700000000000), ({vid}, 1920, 1080, 5000, NULL), ({aud}, NULL, NULL, 61000, NULL)"
    ));
    // Act
    let (i, v, a) = (
        fx.call(&p, &[img, vid, aud], "asset.get", json!({"assetId": img})).unwrap(),
        fx.call(&p, &[img, vid, aud], "asset.get", json!({"assetId": vid})).unwrap(),
        fx.call(&p, &[img, vid, aud], "asset.get", json!({"assetId": aud})).unwrap(),
    );
    // Assert: 모든 키가 항상 존재하고 nullable 은 null
    assert_eq!(i, json!({"assetId": img, "mediaType": "image", "status": "active", "availability": "online", "presence": "present", "fileName": "photo.jpg", "capturedAt": 1_700_000_000_000i64, "durationMs": null, "width": 4000, "height": 3000}));
    assert_eq!(v, json!({"assetId": vid, "mediaType": "video", "status": "active", "availability": "online", "presence": "present", "fileName": "clip.mp4", "capturedAt": null, "durationMs": 5000, "width": 1920, "height": 1080}));
    assert_eq!(a["mediaType"], "audio");
    assert_eq!((a["durationMs"].clone(), a["width"].clone(), a["height"].clone()), (json!(61000), Value::Null, Value::Null));
    assert_eq!(a.as_object().unwrap().len(), 10);
}

#[test]
fn t_pgw_08_no_path_volume_or_library_names_leak_in_any_result() {
    let (fx, p) = setup();
    let id = fx.add_asset("image", "secretdir/deep/photo.jpg");
    fx.thumbs(id);
    let out = [
        fx.call(&p, &[id], "asset.get", json!({"assetId": id})).unwrap().to_string(),
        // 썸네일 본문(base64)은 제외하고 메타 부분만 본다
        {
            let mut d = fx.call(&p, &[id], "asset.getDerived", json!({"assetId": id, "kind": "thumbnail"})).unwrap();
            d["dataBase64"] = json!("");
            d.to_string()
        },
    ];
    for s in out {
        for leak in LEAKS {
            assert!(!s.contains(leak), "{leak} in {s}");
        }
    }
}

#[test]
fn t_pgw_09_offline_missing_and_inactive_states_are_data_not_errors() {
    let (fx, p) = setup();
    let id = fx.add_asset("image", "a/x.jpg");
    fx.exec("UPDATE volumes SET availability = 'offline'");
    fx.exec(&format!("UPDATE file_locations SET presence = 'missing' WHERE asset_id = {id}"));
    let r = fx.call(&p, &[id], "asset.get", json!({"assetId": id})).unwrap();
    assert_eq!((r["availability"].clone(), r["presence"].clone()), (json!("offline"), json!("missing")));
    for status in ["deleted", "superseded"] {
        fx.exec(&format!("UPDATE media_assets SET status = '{status}' WHERE asset_id = {id}"));
        assert_eq!(fx.call(&p, &[id], "asset.get", json!({"assetId": id})).unwrap()["status"], status);
    }
}

#[test]
fn t_pgw_10_unknown_and_out_of_scope_assets_are_indistinguishable_and_skip_the_db() {
    // Arrange: 범위 안 자산을 읽으려 하면 SQL 오류가 나도록 표를 망가뜨린다 -> DB 에 닿았는지 구분된다
    let (fx, p) = setup();
    let (inside, outside) = (fx.add_asset("image", "a/in.jpg"), fx.add_asset("image", "a/out.jpg"));
    fx.exec("ALTER TABLE media_metadata RENAME TO media_metadata_gone");
    // Act
    let out_of_scope = fx.call(&p, &[inside], "asset.get", json!({"assetId": outside})).unwrap_err();
    let nonexistent = fx.call(&p, &[inside], "asset.get", json!({"assetId": 999_999})).unwrap_err();
    let in_scope = fx.call(&p, &[inside], "asset.get", json!({"assetId": inside})).unwrap_err();
    // Assert
    assert_eq!(out_of_scope, nonexistent, "코드·메시지 동일");
    assert_eq!((out_of_scope.code.as_str(), out_of_scope.message.as_str()), ("not_found", "asset not found"));
    assert_eq!((in_scope.code.as_str(), in_scope.message.as_str()), ("internal", "host function failed"), "범위 안은 DB 에 닿아 실패한다");
    // 같은 범위 검사가 getDerived·tag.add·kv(자산 키)에도 DB 전에 적용된다
    for (m, params) in [
        ("asset.getDerived", json!({"assetId": outside, "kind": "thumbnail"})),
        ("tag.add", json!({"assetId": outside, "tags": ["x"]})),
        ("kv.get", json!({"assetId": outside, "key": "k"})),
        ("kv.set", json!({"assetId": outside, "key": "k", "value": 1})),
        ("kv.delete", json!({"assetId": outside, "key": "k"})),
        ("kv.list", json!({"assetId": outside})),
    ] {
        assert_eq!(fx.call(&p, &[inside], m, params).unwrap_err(), out_of_scope, "{m}");
    }
}

#[test]
fn t_pgw_11_file_name_is_sanitized_like_the_ai_transport_layer() {
    let (fx, p) = setup();
    let long: String = "가".repeat(300);
    let cases = [("a/b\\c.jpg", "c.jpg"), ("x/na\u{7}me\u{1b}.jpg", "name.jpg"), ("d/line\nbreak\r.png", "linebreak.png")];
    for (n, (rel, want)) in cases.into_iter().enumerate() {
        let id = fx.add_asset("image", &format!("ok/f{n}.jpg"));
        fx.exec(&format!("UPDATE file_locations SET rel_path = '{}' WHERE asset_id = {id}", rel.replace('\'', "''")));
        let r = fx.call(&p, &[id], "asset.get", json!({"assetId": id})).unwrap();
        assert_eq!(r["fileName"], want, "{rel:?}");
    }
    let id = fx.add_asset("image", "ok/g.jpg");
    fx.exec(&format!("UPDATE file_locations SET rel_path = 'd/{long}' WHERE asset_id = {id}"));
    let r = fx.call(&p, &[id], "asset.get", json!({"assetId": id})).unwrap();
    assert_eq!(r["fileName"].as_str().unwrap().chars().count(), 200);
    // 활성 위치가 없으면 fileName 은 null
    fx.exec(&format!("UPDATE file_locations SET detached_at = 5 WHERE asset_id = {id}"));
    let r = fx.call(&p, &[id], "asset.get", json!({"assetId": id})).unwrap();
    assert_eq!((r["fileName"].clone(), r["presence"].clone(), r["availability"].clone()), (Value::Null, Value::Null, json!("offline")));
}

#[test]
fn t_pgw_12_get_derived_returns_real_webp_for_small_large_and_default_size() {
    use base64::Engine;
    let (fx, p) = setup();
    let id = fx.add_asset("image", "a/x.jpg");
    fx.thumbs(id);
    let decode = |r: &Value| base64::engine::general_purpose::STANDARD.decode(r["dataBase64"].as_str().unwrap()).unwrap();
    let default = fx.call(&p, &[id], "asset.getDerived", json!({"assetId": id, "kind": "thumbnail"})).unwrap();
    let small = fx.call(&p, &[id], "asset.getDerived", json!({"assetId": id, "kind": "thumbnail", "size": "small"})).unwrap();
    let large = fx.call(&p, &[id], "asset.getDerived", json!({"assetId": id, "kind": "thumbnail", "size": "large"})).unwrap();
    assert_eq!((default["size"].clone(), small["size"].clone(), large["size"].clone()), (json!("small"), json!("small"), json!("large")));
    assert_eq!(default, small);
    for r in [&small, &large] {
        assert_eq!((r["kind"].clone(), r["mime"].clone()), (json!("thumbnail"), json!("image/webp")));
        let bytes = decode(r);
        assert_eq!(r["byteLength"], bytes.len());
        assert_eq!((&bytes[..4], &bytes[8..12]), (&b"RIFF"[..], &b"WEBP"[..]));
    }
    let edge = |r: &Value| image::load_from_memory(&decode(r)).map(|i| i.width().max(i.height())).unwrap();
    assert_eq!((edge(&small), edge(&large)), (320, 1024));
}

#[test]
fn t_pgw_13_audio_or_missing_thumbnail_is_unavailable_without_size_fallback() {
    let (fx, p) = setup();
    let (aud, img) = (fx.add_asset("audio", "a/s.wav"), fx.add_asset("image", "a/x.jpg"));
    fx.thumbs(img);
    let ask = |id: i64, size: &str| fx.call(&p, &[aud, img], "asset.getDerived", json!({"assetId": id, "kind": "thumbnail", "size": size}));
    let e = ask(aud, "small").unwrap_err();
    assert_eq!((e.code.as_str(), e.message.as_str()), ("unavailable", "derived data is not available"));
    // large 만 지우면 small 이 있어도 large 요청은 unavailable (폴백 없음)
    let large = mam_media::thumbnail::thumbnail_path(&fx.cache, img, mam_media::thumbnail::ThumbSize::Large);
    std::fs::remove_file(large).unwrap();
    assert_eq!(Fx::err_code(ask(img, "large")), "unavailable");
    assert!(ask(img, "small").is_ok());
    // 썸네일 경로가 폴더여도 unavailable
    let small = mam_media::thumbnail::thumbnail_path(&fx.cache, img, mam_media::thumbnail::ThumbSize::Small);
    std::fs::remove_file(&small).unwrap();
    std::fs::create_dir_all(&small).unwrap();
    assert_eq!(Fx::err_code(ask(img, "small")), "unavailable");
}

#[test]
fn t_pgw_14_size_cap_and_per_call_byte_budget_are_enforced() {
    // Arrange: 파일 1 MiB + 1 바이트 / 정확히 1 MiB, 예산 100,000 바이트
    let fx = Fx::with_cfg(|c| c.derived_budget_bytes = 100_000);
    let p = fx.full_plugin("com.example.big");
    let (big, small) = (fx.add_asset("image", "a/big.jpg"), fx.add_asset("image", "a/s.jpg"));
    let path = mam_media::thumbnail::thumbnail_path(&fx.cache, big, mam_media::thumbnail::ThumbSize::Small);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, vec![0u8; (1 << 20) + 1]).unwrap();
    let exactly = mam_media::thumbnail::thumbnail_path(&fx.cache, big, mam_media::thumbnail::ThumbSize::Large);
    std::fs::create_dir_all(exactly.parent().unwrap()).unwrap();
    std::fs::write(&exactly, vec![0u8; 1 << 20]).unwrap();
    fx.thumbs(small);
    let tsz = std::fs::metadata(mam_media::thumbnail::thumbnail_path(&fx.cache, small, mam_media::thumbnail::ThumbSize::Small)).unwrap().len();
    // Act / Assert: 1 MiB 초과 -> too_large, 정확히 1 MiB 는 크기 상한 통과(예산 초과로 quota)
    let scope = InvokeScope::new(vec![big, small]).unwrap().with_deadline(soon());
    let e = fx.call_scope(&p, &scope, "asset.getDerived", json!({"assetId": big, "kind": "thumbnail"})).unwrap_err();
    assert_eq!((e.code.as_str(), e.message.as_str()), ("too_large", "derived data too large"));
    let e = fx.call_scope(&p, &scope, "asset.getDerived", json!({"assetId": big, "kind": "thumbnail", "size": "large"})).unwrap_err();
    assert_eq!(e.code, "quota_exceeded", "1 MiB 는 크기 상한 이내지만 호출당 예산(100,000)을 넘는다");
    // 예산: 같은 scope 에서 반복하면 누적되어 어느 시점부터 quota_exceeded
    let mut ok = 0;
    let code = loop {
        match fx.call_scope(&p, &scope, "asset.getDerived", json!({"assetId": small, "kind": "thumbnail"})) {
            Ok(_) => ok += 1,
            Err(e) => break e.code,
        }
        assert!(ok < 1000);
    };
    assert_eq!(code, "quota_exceeded");
    assert_eq!(ok, (100_000 / tsz) as i32, "예산 안에서만 통과한다");
    // 새 호출(새 scope)은 예산이 새로 시작한다
    assert!(fx.call(&p, &[small], "asset.getDerived", json!({"assetId": small, "kind": "thumbnail"})).is_ok());
}

#[test]
fn t_pgw_15_inactive_and_out_of_scope_are_not_found_and_unknown_kind_is_bad_request() {
    let (fx, p) = setup();
    let (id, other) = (fx.add_asset("image", "a/x.jpg"), fx.add_asset("image", "a/y.jpg"));
    fx.thumbs(id);
    fx.thumbs(other);
    let ask = |assets: &[i64], params: Value| fx.call(&p, assets, "asset.getDerived", params);
    assert_eq!(Fx::err_code(ask(&[id], json!({"assetId": other, "kind": "thumbnail"}))), "not_found");
    fx.exec(&format!("UPDATE media_assets SET status = 'deleted' WHERE asset_id = {id}"));
    assert_eq!(Fx::err_code(ask(&[id], json!({"assetId": id, "kind": "thumbnail"}))), "not_found");
    fx.exec(&format!("UPDATE media_assets SET status = 'superseded' WHERE asset_id = {id}"));
    assert_eq!(Fx::err_code(ask(&[id], json!({"assetId": id, "kind": "thumbnail"}))), "not_found");
    for params in [
        json!({"assetId": other, "kind": "original"}),
        json!({"assetId": other, "kind": "../../x"}),
        json!({"assetId": other, "kind": "thumbnail", "size": "huge"}),
        json!({"assetId": other, "kind": "thumbnail", "size": 1}),
        json!({"assetId": other}),
        json!({"assetId": other, "kind": "thumbnail", "path": "x"}),
    ] {
        assert_eq!(Fx::err_code(ask(&[other], params.clone())), "bad_request", "{params}");
    }
}

#[test]
fn t_pgw_16_originals_are_never_modified_by_any_call() {
    // Arrange
    let (fx, p) = setup();
    let id = fx.add_asset("image", "a/x.jpg");
    fx.thumbs(id);
    let before = snapshot(&fx.lib);
    assert!(!before.is_empty());
    // Act: 모든 메서드를 성공·실패 경로로 호출
    for (m, params) in [
        ("asset.get", json!({"assetId": id})),
        ("asset.getDerived", json!({"assetId": id, "kind": "thumbnail", "size": "large"})),
        ("kv.set", json!({"assetId": id, "key": "k", "value": [1, 2]})),
        ("kv.get", json!({"assetId": id, "key": "k"})),
        ("kv.list", json!({"assetId": id})),
        ("kv.delete", json!({"assetId": id, "key": "k"})),
        ("tag.add", json!({"assetId": id, "tags": ["t"]})),
        ("asset.get", json!({"assetId": 99})),
        ("asset.getDerived", json!({"assetId": id, "kind": "original"})),
    ] {
        let _ = fx.call(&p, &[id], m, params);
    }
    // Assert: 내용·수정 시각 불변
    assert_eq!(snapshot(&fx.lib), before);
}
