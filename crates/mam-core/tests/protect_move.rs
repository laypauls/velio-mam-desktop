//! P2-9 원본 이동 시험 (T-prot-08*). 임시 디렉터리 픽스처만 사용한다.
//! 분류: Happy / Business rule / External

mod common;
#[path = "common/protect_env.rs"]
mod protect_env;

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use common::{FakeVolumes, Snapshot, active_locations, asset_state, count, outbox};
use mam_core::protect::{MoveEnv, OriginalsEnv, move_originals};
use mam_core::{Blake3Hasher, ContentHasher, CoreError, HashedFile, MetadataOnly, VolumeKind};
use protect_env::Fx;

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

// --- T-prot-08 이동 ---------------------------------------------------------

macro_rules! move_env {
    ($fx:expr, $vols:expr, $hasher:expr) => {
        MoveEnv {
            base: env!($fx, &$fx.trash),
            volumes: $vols,
            identity: &MetadataOnly,
            hasher: $hasher,
            excluded_dirs: &[],
        }
    };
}

#[test]
fn t_prot_08_same_volume_move_relinks_asset_and_keeps_tags() {
    // Arrange
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-m.bin", b"move-me");
    let keep = fx.add("keep/mam-test-k.bin", b"keep-me");
    fx.add_tag(id, "holiday");
    fx.manage(&fx.vol, "photos");
    let dest = fx.vol.path("photos/sub");
    fs::create_dir_all(&dest).unwrap();
    fx.authenticate();
    let rev0 = asset_state(&fx.w.db.conn, id).revision;
    let keep_rev0 = asset_state(&fx.w.db.conn, keep).revision;
    let vols = FakeVolumes(vec![fx.vol.info.clone()]);
    let bystander = Snapshot::take(&fx.vol.path("keep"));

    // Act
    let me = move_env!(fx, &vols, &Blake3Hasher);
    let report = move_originals(&mut fx.w.db.conn, &me, &[id], &dest).unwrap();

    // Assert
    assert_eq!(
        (report.result.succeeded, report.result.failed.len()),
        (1, 0)
    );
    assert!(report.detached_asset_ids.is_empty());
    assert!(!fx.vol.path("media/mam-test-m.bin").exists());
    assert_eq!(fs::read(dest.join("mam-test-m.bin")).unwrap(), b"move-me");
    assert!(fx.trash.calls().is_empty()); // 같은 볼륨은 휴지통 불필요
    let loc = active_locations(&fx.w.db.conn)
        .into_iter()
        .find(|l| l.asset_id == id)
        .unwrap();
    assert_eq!(loc.rel_path.replace('\\', "/"), "photos/sub/mam-test-m.bin");
    assert_eq!(asset_state(&fx.w.db.conn, id).status, "active");
    assert_eq!(asset_state(&fx.w.db.conn, id).revision, rev0 + 1);
    assert_eq!(
        outbox(&fx.w.db.conn).last().unwrap(),
        &(id, rev0 + 1, "upsert".to_string())
    );
    assert_eq!(
        count(
            &fx.w.db.conn,
            &format!("SELECT count(*) FROM asset_tags WHERE asset_id = {id}")
        ),
        1
    );
    assert_eq!(asset_state(&fx.w.db.conn, keep).revision, keep_rev0);
    assert_eq!(Snapshot::take(&fx.vol.path("keep")), bystander);
}

#[test]
fn t_prot_08b_cross_volume_move_copies_verifies_then_trashes_source() {
    // Arrange
    let mut fx = Fx::new();
    let vol2 = fx.w.volume("vol-b", VolumeKind::Removable);
    let id = fx.add("media/mam-test-x.bin", b"cross-volume-content");
    fx.manage(&vol2, "");
    fs::create_dir_all(vol2.path("in")).unwrap();
    let dest = vol2.path("incoming");
    fs::create_dir_all(&dest).unwrap();
    fx.authenticate();
    let rev0 = asset_state(&fx.w.db.conn, id).revision;
    let vols = FakeVolumes(vec![fx.vol.info.clone(), vol2.info.clone()]);

    // Act
    let me = move_env!(fx, &vols, &Blake3Hasher);
    let report = move_originals(&mut fx.w.db.conn, &me, &[id], &dest).unwrap();

    // Assert
    assert_eq!(report.result.succeeded, 1, "{:?}", report.result.failed);
    assert_eq!(fx.trash.calls(), vec![fx.vol.path("media/mam-test-x.bin")]);
    assert_eq!(
        fs::read(dest.join("mam-test-x.bin")).unwrap(),
        b"cross-volume-content"
    );
    assert!(!dest.join(".mam-move-1.tmp").exists());
    let loc = active_locations(&fx.w.db.conn).pop().unwrap();
    assert_eq!((loc.asset_id, loc.volume_id), (id, vol2.id));
    assert_eq!(asset_state(&fx.w.db.conn, id).revision, rev0 + 1);
}

#[test]
fn t_prot_08c_destination_outside_managed_folders_detaches_and_flags() {
    // Arrange
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-o.bin", b"outside");
    fx.add_tag(id, "kept");
    fx.manage(&fx.vol, "photos");
    let dest = fx.vol.path("elsewhere");
    fs::create_dir_all(&dest).unwrap();
    fx.authenticate();
    let rev0 = asset_state(&fx.w.db.conn, id).revision;
    let vols = FakeVolumes(vec![fx.vol.info.clone()]);

    // Act
    let me = move_env!(fx, &vols, &Blake3Hasher);
    let report = move_originals(&mut fx.w.db.conn, &me, &[id], &dest).unwrap();

    // Assert
    assert_eq!(report.result.succeeded, 1);
    assert_eq!(report.detached_asset_ids, vec![id]);
    assert!(dest.join("mam-test-o.bin").exists());
    assert!(active_locations(&fx.w.db.conn).is_empty());
    let st = asset_state(&fx.w.db.conn, id);
    assert_eq!((st.status.as_str(), st.revision), ("active", rev0 + 1));
    assert_eq!(count(&fx.w.db.conn, "SELECT count(*) FROM asset_tags"), 1);
}

/// 복사본(`.tmp`)의 해시만 틀리게 보고하는 해시 계산기
struct CorruptCopy;

impl ContentHasher for CorruptCopy {
    fn hash_file(&self, path: &Path, cancel: &AtomicBool) -> Result<HashedFile, CoreError> {
        let mut h = Blake3Hasher.hash_file(path, cancel)?;
        if path.extension().is_some_and(|e| e == "tmp") {
            let mut bytes = *h.hash.as_bytes();
            bytes[0] ^= 1;
            h.hash = mam_core::ContentHash::from_bytes(bytes);
        }
        Ok(h)
    }
}

#[test]
fn t_prot_08d_copy_verification_failure_keeps_original_and_cleans_up() {
    let mut fx = Fx::new();
    let vol2 = fx.w.volume("vol-b", VolumeKind::Fixed);
    let id = fx.add("media/mam-test-v.bin", b"verify-me");
    fx.manage(&vol2, "");
    fs::create_dir_all(vol2.path("in")).unwrap();
    fx.authenticate();
    let rev0 = asset_state(&fx.w.db.conn, id).revision;
    let vols = FakeVolumes(vec![fx.vol.info.clone(), vol2.info.clone()]);
    let src_before = Snapshot::take(&fx.vol.path("media"));

    let me = move_env!(fx, &vols, &CorruptCopy);
    let report = move_originals(&mut fx.w.db.conn, &me, &[id], &vol2.path("in")).unwrap();

    assert_eq!(report.result.succeeded, 0);
    assert_eq!(report.result.failed.len(), 1);
    assert_eq!(report.result.failed[0].asset_id, id);
    assert!(fx.trash.calls().is_empty());
    assert_eq!(Snapshot::take(&fx.vol.path("media")), src_before);
    assert_eq!(fs::read_dir(vol2.path("in")).unwrap().count(), 0); // 임시·최종 복사본 없음
    assert_eq!(asset_state(&fx.w.db.conn, id).revision, rev0);
    assert_eq!(active_locations(&fx.w.db.conn).len(), 1);
}

#[test]
fn t_prot_08e_trash_failure_after_copy_removes_the_copy() {
    let mut fx = Fx::new();
    let vol2 = fx.w.volume("vol-b", VolumeKind::Fixed);
    let id = fx.add("media/mam-test-t.bin", b"trash-fails");
    fx.manage(&vol2, "");
    fs::create_dir_all(vol2.path("in")).unwrap();
    fx.trash.fail_for("mam-test-t.bin");
    fx.authenticate();
    let rev0 = asset_state(&fx.w.db.conn, id).revision;
    let vols = FakeVolumes(vec![fx.vol.info.clone(), vol2.info.clone()]);

    let me = move_env!(fx, &vols, &Blake3Hasher);
    let report = move_originals(&mut fx.w.db.conn, &me, &[id], &vol2.path("in")).unwrap();

    assert_eq!(report.result.failed.len(), 1);
    assert!(fx.vol.path("media/mam-test-t.bin").exists());
    assert_eq!(fs::read_dir(vol2.path("in")).unwrap().count(), 0);
    assert_eq!(asset_state(&fx.w.db.conn, id).revision, rev0);
}

#[test]
fn t_prot_08f_name_collision_and_same_folder_are_per_asset_failures_without_overwrite() {
    let mut fx = Fx::new();
    let id = fx.add("media/mam-test-n.bin", b"mine");
    fx.manage(&fx.vol, "photos");
    let dest = fx.vol.path("photos");
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("mam-test-n.bin"), b"someone else's file").unwrap();
    fx.authenticate();
    let rev0 = asset_state(&fx.w.db.conn, id).revision;
    let vols = FakeVolumes(vec![fx.vol.info.clone()]);
    let dest_before = Snapshot::take(&dest);

    let me = move_env!(fx, &vols, &Blake3Hasher);
    let collide = move_originals(&mut fx.w.db.conn, &me, &[id], &dest).unwrap();
    let same = move_originals(&mut fx.w.db.conn, &me, &[id], &fx.vol.path("media")).unwrap();

    assert_eq!(collide.result.failed.len(), 1);
    assert_eq!(same.result.failed.len(), 1);
    assert_eq!(Snapshot::take(&dest), dest_before);
    assert!(fx.vol.path("media/mam-test-n.bin").exists());
    assert_eq!(asset_state(&fx.w.db.conn, id).revision, rev0);
}
