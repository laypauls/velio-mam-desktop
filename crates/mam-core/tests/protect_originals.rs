//! P2-9 원본 삭제·인증 시험 (T-prot-06, 07, 09; 이동은 protect_move.rs). 임시 디렉터리 픽스처만 사용한다.
//! 분류: Happy / Business rule / External
//!
//! 실제 휴지통을 쓰는 시험(`*_real_recycle_bin`)은 `mam-test-*.bin` 이름의 작은 파일을 사용자의 휴지통에 남긴다.

mod common;
#[path = "common/protect_env.rs"]
mod protect_env;

use std::fs;
use std::path::Path;

use common::{FakeVolumes, Snapshot, active_locations, asset_state, count, outbox};
use mam_core::protect::{MoveEnv, OriginalsEnv, ProtectError, delete_originals, move_originals};
use mam_core::{Blake3Hasher, MetadataOnly, set_volume_online};
use protect_env::{Fx, PASSWORD};

macro_rules! env {
    ($fx:expr, $trash:expr) => {
        OriginalsEnv {
            guard: &$fx.guard,
            password_set: true,
            clock: &$fx.w.clock,
            trash: $trash,
        }
    };
}

// --- T-prot-06 삭제 ---------------------------------------------------------

#[test]
fn t_prot_06_delete_with_recording_trash_marks_asset_deleted_and_emits_delete_event() {
    // Arrange
    let mut fx = Fx::new();
    let target = fx.add("media/mam-test-a.bin", b"aaaa");
    let keep = fx.add("keep/mam-test-b.bin", b"bbbb");
    fx.authenticate();
    let rev0 = asset_state(&fx.w.db.conn, target).revision;
    let bystander = Snapshot::take(&fx.vol.path("keep"));
    let outbox_before = outbox(&fx.w.db.conn).len();

    // Act
    let r = delete_originals(&mut fx.w.db.conn, &env!(fx, &fx.trash), &[target, target]).unwrap();

    // Assert
    assert_eq!((r.succeeded, r.failed.len()), (1, 0)); // 중복 ID 는 한 번만
    assert_eq!(fx.trash.calls().len(), 1);
    let st = asset_state(&fx.w.db.conn, target);
    assert_eq!((st.status.as_str(), st.revision), ("deleted", rev0 + 1));
    assert_eq!(
        outbox(&fx.w.db.conn).last().unwrap(),
        &(target, rev0 + 1, "delete".to_string())
    );
    assert_eq!(outbox(&fx.w.db.conn).len(), outbox_before + 1);
    let locs = active_locations(&fx.w.db.conn);
    assert_eq!(locs.len(), 1);
    assert_eq!(locs[0].asset_id, keep);
    assert_eq!(asset_state(&fx.w.db.conn, keep).status, "active");
    assert_eq!(Snapshot::take(&fx.vol.path("keep")), bystander);
}

/// 실제 휴지통을 쓰는 시험은 사용자 휴지통에 파일을 남기므로 명시적으로 켤 때만 실행한다
/// (`MAM_TEST_REAL_RECYCLE_BIN=1`). 기본 실행·CI 에서는 사유를 출력하고 건너뛴다.
#[cfg(windows)]
fn real_recycle_bin_enabled(test: &str) -> bool {
    let on = std::env::var("MAM_TEST_REAL_RECYCLE_BIN").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("SKIP {test}: set MAM_TEST_REAL_RECYCLE_BIN=1 to use the real Recycle Bin");
    }
    on
}

#[cfg(windows)]
#[test]
fn t_prot_06b_delete_to_real_recycle_bin_removes_file_from_directory() {
    if !real_recycle_bin_enabled("t_prot_06b") {
        return;
    }
    // Arrange
    let mut fx = Fx::new();
    let target = fx.add("media/mam-test-recycle-1.bin", b"x");
    fx.authenticate();
    let path = fx.vol.path("media/mam-test-recycle-1.bin");

    // Act
    let bin = mam_core::protect::RecycleBin;
    let r = delete_originals(&mut fx.w.db.conn, &env!(fx, &bin), &[target]).unwrap();

    // Assert
    assert_eq!(r.succeeded, 1, "{:?}", r.failed);
    assert!(!path.exists());
    assert_eq!(asset_state(&fx.w.db.conn, target).status, "deleted");
    assert_eq!(
        count(
            &fx.w.db.conn,
            "SELECT count(*) FROM file_locations WHERE detached_at IS NULL"
        ),
        0
    );
}

// --- T-prot-07 인증 없음 -----------------------------------------------------

#[test]
fn t_prot_07_without_authentication_nothing_is_touched() {
    // Arrange
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-c.bin", b"cccc");
    let dest = fx.vol.path("photos");
    fs::create_dir_all(&dest).unwrap();
    let vols = FakeVolumes(vec![fx.vol.info.clone()]);
    let before = Snapshot::take(fx.vol.dir.path());
    let revision = asset_state(&fx.w.db.conn, id).revision;

    // Act + Assert: 미인증 / 만료 / 비밀번호 미설정
    let base = env!(fx, &fx.trash);
    let e = delete_originals(&mut fx.w.db.conn, &base, &[id]).unwrap_err();
    assert_eq!(e.code(), "auth_required");
    let menv = MoveEnv {
        base: env!(fx, &fx.trash),
        volumes: &vols,
        identity: &MetadataOnly,
        hasher: &Blake3Hasher,
        excluded_dirs: &[],
    };
    let e = move_originals(&mut fx.w.db.conn, &menv, &[id], &dest).unwrap_err();
    assert_eq!(e.code(), "auth_required");

    fx.authenticate();
    fx.guard_clock.advance(5 * 60_000);
    let e = delete_originals(&mut fx.w.db.conn, &env!(fx, &fx.trash), &[id]).unwrap_err();
    assert_eq!(e.code(), "auth_required");

    let mut no_pw = env!(fx, &fx.trash);
    no_pw.password_set = false;
    let e = delete_originals(&mut fx.w.db.conn, &no_pw, &[id]).unwrap_err();
    assert_eq!(e.code(), "password_not_set");

    assert!(fx.trash.calls().is_empty());
    assert_eq!(Snapshot::take(fx.vol.dir.path()), before);
    assert_eq!(asset_state(&fx.w.db.conn, id).revision, revision);
    assert_eq!(asset_state(&fx.w.db.conn, id).status, "active");
}

#[test]
fn t_prot_07b_empty_selection_and_bad_destination_are_invalid_input() {
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-d.bin", b"d");
    fx.authenticate();
    let vols = FakeVolumes(vec![fx.vol.info.clone()]);

    let e = delete_originals(&mut fx.w.db.conn, &env!(fx, &fx.trash), &[]).unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    let menv = MoveEnv {
        base: env!(fx, &fx.trash),
        volumes: &vols,
        identity: &MetadataOnly,
        hasher: &Blake3Hasher,
        excluded_dirs: &[],
    };
    let missing_dir = fx.vol.path("nope");
    let e = move_originals(&mut fx.w.db.conn, &menv, &[id], &missing_dir).unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    let e = move_originals(&mut fx.w.db.conn, &menv, &[id], Path::new("relative")).unwrap_err();
    assert_eq!(e.code(), "invalid_input");
}

// --- T-prot-09 일부 실패 -------------------------------------------------------

#[test]
fn t_prot_09_partial_failure_is_reported_per_asset_and_others_succeed() {
    // Arrange
    let mut fx = Fx::new();
    let ok1 = fx.add("media/mam-test-1.bin", b"1");
    let bad = fx.add("media/mam-test-2.bin", b"2");
    let ok2 = fx.add("media/mam-test-3.bin", b"3");
    let gone = fx.add("media/mam-test-4.bin", b"4");
    fs::remove_file(fx.vol.path("media/mam-test-4.bin")).unwrap();
    fx.trash.fail_for("mam-test-2.bin");
    fx.authenticate();

    // Act
    let r = delete_originals(
        &mut fx.w.db.conn,
        &env!(fx, &fx.trash),
        &[ok1, bad, ok2, gone, 9999],
    )
    .unwrap();

    // Assert
    assert_eq!(r.succeeded, 2);
    let failed: Vec<i64> = r.failed.iter().map(|f| f.asset_id).collect();
    assert_eq!(failed, vec![bad, gone, 9999]);
    assert!(r.failed[1].message.contains("missing"));
    assert_eq!(asset_state(&fx.w.db.conn, bad).status, "active");
    assert_eq!(asset_state(&fx.w.db.conn, gone).status, "active"); // 파일 없음 != 삭제 승인
    assert_eq!(asset_state(&fx.w.db.conn, ok1).status, "deleted");
    assert_eq!(asset_state(&fx.w.db.conn, ok2).status, "deleted");
    assert!(fx.vol.path("media/mam-test-2.bin").exists());
}

#[test]
fn t_prot_09b_offline_volume_is_a_per_asset_failure_and_data_is_kept() {
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-off.bin", b"off");
    set_volume_online(&fx.w.db.conn, &fx.w.clock, fx.vol.id, false).unwrap();
    fx.authenticate();

    let r = delete_originals(&mut fx.w.db.conn, &env!(fx, &fx.trash), &[id]).unwrap();

    assert_eq!((r.succeeded, r.failed.len()), (0, 1));
    assert!(r.failed[0].message.contains("offline"));
    assert!(fx.trash.calls().is_empty());
    assert_eq!(asset_state(&fx.w.db.conn, id).status, "active");
}

#[test]
fn t_prot_09c_file_changed_since_scan_is_refused() {
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-chg.bin", b"before");
    fs::write(fx.vol.path("media/mam-test-chg.bin"), b"changed content!").unwrap();
    fx.authenticate();

    let r = delete_originals(&mut fx.w.db.conn, &env!(fx, &fx.trash), &[id]).unwrap();

    assert_eq!(r.failed.len(), 1);
    assert!(fx.vol.path("media/mam-test-chg.bin").exists());
}

#[cfg(windows)]
#[test]
fn t_prot_09d_locked_file_with_real_recycle_bin_fails_while_others_succeed() {
    use std::os::windows::fs::OpenOptionsExt;
    if !real_recycle_bin_enabled("t_prot_09d") {
        return;
    }

    // Arrange: 삭제 공유 없이 열어 둔 파일 = 삭제 불가
    let mut fx = Fx::new();
    let ok = fx.add("media/mam-test-free.bin", b"f");
    let locked = fx.add("media/mam-test-locked.bin", b"l");
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1 | 2) // FILE_SHARE_READ | FILE_SHARE_WRITE (FILE_SHARE_DELETE 없음)
        .open(fx.vol.path("media/mam-test-locked.bin"))
        .unwrap();
    fx.authenticate();

    // Act
    let bin = mam_core::protect::RecycleBin;
    let r = delete_originals(&mut fx.w.db.conn, &env!(fx, &bin), &[locked, ok]).unwrap();
    drop(held);

    // Assert
    assert_eq!(r.succeeded, 1, "{:?}", r.failed);
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].asset_id, locked);
    assert!(fx.vol.path("media/mam-test-locked.bin").exists());
    assert!(!fx.vol.path("media/mam-test-free.bin").exists());
    assert_eq!(asset_state(&fx.w.db.conn, locked).status, "active");
    assert_eq!(asset_state(&fx.w.db.conn, ok).status, "deleted");
}

#[test]
fn t_prot_09e_error_codes_map_to_ui_contract() {
    let cases: [(ProtectError, &str); 11] = [
        (ProtectError::InvalidInput(String::new()), "invalid_input"),
        (ProtectError::PasswordNotSet, "password_not_set"),
        (ProtectError::PasswordAlreadySet, "password_already_set"),
        (ProtectError::PasswordIncorrect, "password_incorrect"),
        (ProtectError::AuthRequired, "auth_required"),
        (ProtectError::NotFound(String::new()), "not_found"),
        (ProtectError::Offline(String::new()), "offline"),
        (ProtectError::Io(String::new()), "io"),
        (ProtectError::Internal(String::new()), "internal"),
        (ProtectError::BackupFailed(String::new()), "io"),
        (
            ProtectError::TooManyAttempts { retry_after_ms: 1 },
            "too_many_attempts",
        ),
    ];
    for (e, code) in cases {
        assert_eq!(e.code(), code);
    }
    let json = serde_json::to_string(&mam_core::protect::OriginalsResult {
        succeeded: 1,
        failed: vec![mam_core::protect::AssetFailure {
            asset_id: 7,
            message: "m".into(),
        }],
    })
    .unwrap();
    assert_eq!(
        json,
        r#"{"succeeded":1,"failed":[{"assetId":7,"message":"m"}]}"#
    );
    let _ = PASSWORD;
}
