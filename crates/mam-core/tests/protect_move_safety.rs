//! P2-9 원본 삭제·이동 안전 보강 시험 (T-prot-14 ~ 19): 이동 대상 제외, 덮어쓰기 금지, 고정 오류 메시지,
//! 1회용 인증 소모 시점, DB 실패 보고. 임시 디렉터리 픽스처만 사용한다.
//! 분류: Business rule / Concurrency(확인 이후 생성된 대상) / Validation / External

mod common;
#[path = "common/protect_env.rs"]
mod protect_env;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use common::{FakeVolumes, Snapshot, active_locations, asset_state};
use mam_core::protect::{MoveEnv, OriginalsEnv, OriginalsResult, delete_originals, move_originals};
use mam_core::{
    Blake3Hasher, ContentHasher, CoreError, HashedFile, MetadataOnly, VolumeKind, set_volume_online,
};
use protect_env::{Fx, PASSWORD, shared_hash};

macro_rules! base {
    ($fx:expr) => {
        OriginalsEnv {
            guard: &$fx.guard,
            password_set: true,
            clock: &$fx.w.clock,
            trash: &$fx.trash,
        }
    };
}

macro_rules! menv {
    ($fx:expr, $vols:expr, $hasher:expr, $excluded:expr) => {
        MoveEnv {
            base: base!($fx),
            volumes: $vols,
            identity: &MetadataOnly,
            hasher: $hasher,
            excluded_dirs: $excluded,
        }
    };
}

/// 메시지가 고정 코드이고 경로 구분자·픽스처 경로를 담지 않는다
fn assert_fixed_messages(r: &OriginalsResult, fx: &Fx) {
    let temp = fx.vol.dir.path().to_string_lossy().to_string();
    for f in &r.failed {
        assert!(
            f.message
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'),
            "not a code: {}",
            f.message
        );
        assert!(!f.message.contains(['\\', '/', ':']));
        assert!(!f.message.contains(&temp));
    }
}

// --- T-prot-14 이동 대상 제외 ---------------------------------------------------

#[test]
fn t_prot_14_destination_inside_mamdata_or_backup_dir_is_refused_and_nothing_moves() {
    // Arrange
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-x.bin", b"orig");
    let data = fx.vol.path("MAMData");
    let backup = fx.vol.path("my-backups");
    fs::create_dir_all(data.join("cache/sub")).unwrap();
    fs::create_dir_all(&backup).unwrap();
    fx.manage(&fx.vol, "");
    fx.authenticate();
    let vols = FakeVolumes(vec![fx.vol.info.clone()]);
    let excluded = vec![data.clone(), backup.clone()];
    let before = Snapshot::take(fx.vol.dir.path());
    let rev0 = asset_state(&fx.w.db.conn, id).revision;

    // Act + Assert: 같은 경로, 하위 폴더, 대소문자만 다른 경로 모두 거부
    let upper = PathBuf::from(data.join("cache").to_string_lossy().to_uppercase());
    for dest in [data.clone(), data.join("cache/sub"), upper, backup.clone()] {
        let me = menv!(fx, &vols, &Blake3Hasher, &excluded);
        let e = move_originals(&mut fx.w.db.conn, &me, &[id], &dest).unwrap_err();
        assert_eq!(e.code(), "invalid_input", "{}", dest.display());
    }
    assert_eq!(Snapshot::take(fx.vol.dir.path()), before);
    assert_eq!(asset_state(&fx.w.db.conn, id).revision, rev0);

    // 제외 폴더 밖(형제 이름이 접두어만 같은 폴더)은 허용
    let sibling = fx.vol.path("MAMData-other");
    fs::create_dir_all(&sibling).unwrap();
    let me = menv!(fx, &vols, &Blake3Hasher, &excluded);
    let ok = move_originals(&mut fx.w.db.conn, &me, &[id], &sibling).unwrap();
    assert_eq!(ok.result.succeeded, 1);
}

// --- T-prot-15 덮어쓰기 금지 (확인 이후 생성된 대상) -----------------------------

/// 임시 복사본을 검증하는 시점(= 존재 확인 이후, 최종 이동 이전)에 같은 이름의 파일을 만든다
struct CreatesTargetDuringVerify {
    target: PathBuf,
}

impl ContentHasher for CreatesTargetDuringVerify {
    fn hash_file(&self, path: &Path, cancel: &AtomicBool) -> Result<HashedFile, CoreError> {
        if path.extension().is_some_and(|e| e == "tmp") {
            fs::write(&self.target, b"someone else's file").unwrap();
        }
        Blake3Hasher.hash_file(path, cancel)
    }
}

#[test]
fn t_prot_15_target_created_after_the_check_is_never_replaced_cross_volume() {
    // Arrange
    let mut fx = Fx::new();
    let vol2 = fx.w.volume("vol-b", VolumeKind::Fixed);
    let id = fx.add("media/mam-test-r.bin", b"mine");
    fx.manage(&vol2, "");
    let dest = vol2.path("in");
    fs::create_dir_all(&dest).unwrap();
    fx.authenticate();
    let rev0 = asset_state(&fx.w.db.conn, id).revision;
    let vols = FakeVolumes(vec![fx.vol.info.clone(), vol2.info.clone()]);
    let hasher = CreatesTargetDuringVerify {
        target: dest.join("mam-test-r.bin"),
    };

    // Act
    let me = menv!(fx, &vols, &hasher, &[]);
    let report = move_originals(&mut fx.w.db.conn, &me, &[id], &dest).unwrap();

    // Assert
    assert_eq!(report.result.succeeded, 0);
    assert_eq!(report.result.failed[0].message, "destination_exists");
    assert_eq!(
        fs::read(dest.join("mam-test-r.bin")).unwrap(),
        b"someone else's file"
    );
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 1); // 임시 복사본 정리됨
    assert!(fx.vol.path("media/mam-test-r.bin").exists()); // 원본 유지
    assert!(fx.trash.calls().is_empty());
    assert_eq!(asset_state(&fx.w.db.conn, id).revision, rev0);
    assert_eq!(active_locations(&fx.w.db.conn).len(), 1);
}

// --- T-prot-16 고정 오류 메시지 --------------------------------------------------

#[test]
fn t_prot_16_failure_messages_are_fixed_codes_without_paths() {
    // Arrange
    let mut fx = Fx::new();
    let gone = fx.add("media/mam-test-g.bin", b"g");
    fs::remove_file(fx.vol.path("media/mam-test-g.bin")).unwrap();
    let changed = fx.add("media/mam-test-c.bin", b"before");
    fs::write(fx.vol.path("media/mam-test-c.bin"), b"after-longer").unwrap();
    let trashfail = fx.add("media/mam-test-t.bin", b"t");
    fx.trash.fail_for("mam-test-t.bin");
    let off = fx.add("media/mam-test-o.bin", b"o");
    fx.authenticate();

    // Act: 삭제 - 파일 없음 / 변경됨 / 휴지통 실패 / 없는 자산
    let r = delete_originals(
        &mut fx.w.db.conn,
        &base!(fx),
        &[gone, changed, trashfail, 9999],
    )
    .unwrap();

    // Assert
    let msgs: Vec<&str> = r.failed.iter().map(|f| f.message.as_str()).collect();
    assert_eq!(
        msgs,
        ["file_missing", "changed_since_scan", "io", "not_found"]
    );
    assert_fixed_messages(&r, &fx);

    // Act: 오프라인 볼륨
    set_volume_online(&fx.w.db.conn, &fx.w.clock, fx.vol.id, false).unwrap();
    let r = delete_originals(&mut fx.w.db.conn, &base!(fx), &[off]).unwrap();
    assert_eq!(r.failed[0].message, "offline");
    assert_fixed_messages(&r, &fx);
}

#[test]
fn t_prot_16b_move_failure_messages_are_fixed_codes() {
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-n.bin", b"mine");
    fx.manage(&fx.vol, "photos");
    let dest = fx.vol.path("photos");
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("mam-test-n.bin"), b"other").unwrap();
    fx.authenticate();
    let vols = FakeVolumes(vec![fx.vol.info.clone()]);

    let me = menv!(fx, &vols, &Blake3Hasher, &[]);
    let collide = move_originals(&mut fx.w.db.conn, &me, &[id], &dest).unwrap();
    let same = move_originals(&mut fx.w.db.conn, &me, &[id], &fx.vol.path("media")).unwrap();

    assert_eq!(collide.result.failed[0].message, "destination_exists");
    assert_eq!(same.result.failed[0].message, "already_in_destination");
    assert_fixed_messages(&collide.result, &fx);
    assert_fixed_messages(&same.result, &fx);
}

// --- T-prot-17 1회용 인증은 입력 검증 뒤에 소모된다 --------------------------------

#[test]
fn t_prot_17_single_use_auth_survives_invalid_input_and_is_consumed_by_a_valid_call() {
    // Arrange: 면제 0분 = 1회용
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-s.bin", b"s");
    let data = fx.vol.path("MAMData");
    fs::create_dir_all(&data).unwrap();
    let excluded = vec![data.clone()];
    let vols = FakeVolumes(vec![fx.vol.info.clone()]);
    fx.guard
        .verify_password(Some(shared_hash()), PASSWORD, 0)
        .unwrap();

    // Act + Assert: 잘못된 입력들은 인증을 소모하지 않는다
    let e = delete_originals(&mut fx.w.db.conn, &base!(fx), &[]).unwrap_err();
    assert_eq!(e.code(), "invalid_input");
    let me = menv!(fx, &vols, &Blake3Hasher, &excluded);
    for bad in [fx.vol.path("nope"), data.clone()] {
        let e = move_originals(&mut fx.w.db.conn, &me, &[id], &bad).unwrap_err();
        assert_eq!(e.code(), "invalid_input");
    }
    let e = move_originals(&mut fx.w.db.conn, &me, &[], &fx.vol.path("media")).unwrap_err();
    assert_eq!(e.code(), "invalid_input");

    // 올바른 호출 한 번은 성공하고, 그 뒤에는 다시 인증이 필요하다
    let r = delete_originals(&mut fx.w.db.conn, &base!(fx), &[id]).unwrap();
    assert_eq!(r.succeeded, 1);
    let e = delete_originals(&mut fx.w.db.conn, &base!(fx), &[id]).unwrap_err();
    assert_eq!(e.code(), "auth_required");
}

#[test]
fn t_prot_17b_unauthenticated_caller_gets_auth_required_before_input_validation() {
    let mut fx = Fx::new();
    let e = delete_originals(&mut fx.w.db.conn, &base!(fx), &[]).unwrap_err();
    assert_eq!(e.code(), "auth_required");
}

// --- T-prot-18 DB 실패 보고 -----------------------------------------------------

const FAIL_UPDATES: &str = "CREATE TRIGGER boom_loc BEFORE UPDATE ON file_locations
    BEGIN SELECT RAISE(ABORT, 'boom'); END;
    CREATE TRIGGER boom_asset BEFORE UPDATE ON media_assets
    BEGIN SELECT RAISE(ABORT, 'boom'); END;";

#[test]
fn t_prot_18_cross_volume_move_with_db_failure_reports_moved_db_pending() {
    // Arrange
    let mut fx = Fx::new();
    let vol2 = fx.w.volume("vol-b", VolumeKind::Fixed);
    let id = fx.add("media/mam-test-p.bin", b"pending");
    fx.manage(&vol2, "");
    let dest = vol2.path("in");
    fs::create_dir_all(&dest).unwrap();
    fx.authenticate();
    fx.w.db.conn.execute_batch(FAIL_UPDATES).unwrap();
    let vols = FakeVolumes(vec![fx.vol.info.clone(), vol2.info.clone()]);

    // Act
    let me = menv!(fx, &vols, &Blake3Hasher, &[]);
    let report = move_originals(&mut fx.w.db.conn, &me, &[id], &dest).unwrap();

    // Assert: 원본은 휴지통으로 갔고 복사본이 새 위치에 남아 있다 - 다음 스캔이 다시 연결한다
    assert_eq!(report.result.failed[0].message, "moved_db_pending");
    assert_fixed_messages(&report.result, &fx);
    assert_eq!(fx.trash.calls().len(), 1);
    assert_eq!(fs::read(dest.join("mam-test-p.bin")).unwrap(), b"pending");
}

#[test]
fn t_prot_18b_same_volume_move_with_db_failure_rolls_the_file_back() {
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-q.bin", b"back");
    fx.manage(&fx.vol, "photos");
    let dest = fx.vol.path("photos");
    fs::create_dir_all(&dest).unwrap();
    fx.authenticate();
    fx.w.db.conn.execute_batch(FAIL_UPDATES).unwrap();
    let vols = FakeVolumes(vec![fx.vol.info.clone()]);

    let me = menv!(fx, &vols, &Blake3Hasher, &[]);
    let report = move_originals(&mut fx.w.db.conn, &me, &[id], &dest).unwrap();

    assert_eq!(report.result.failed.len(), 1);
    assert_ne!(report.result.failed[0].message, "moved_db_pending");
    assert_fixed_messages(&report.result, &fx);
    assert_eq!(
        fs::read(fx.vol.path("media/mam-test-q.bin")).unwrap(),
        b"back"
    );
    assert!(!dest.join("mam-test-q.bin").exists());
}

#[test]
fn t_prot_18c_delete_with_db_failure_after_trash_reports_moved_db_pending() {
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-d.bin", b"d");
    fx.authenticate();
    fx.w.db.conn.execute_batch(FAIL_UPDATES).unwrap();

    let r = delete_originals(&mut fx.w.db.conn, &base!(fx), &[id]).unwrap();

    assert_eq!(r.failed[0].message, "moved_db_pending");
    assert_fixed_messages(&r, &fx);
}
