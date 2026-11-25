//! 관리 폴더 이전(relocate) 트랜잭션 — 폴더 경로가 바뀌었을 때(폴더 이동·다른 드라이브로 복사) 같은 자산에 다시 연결한다.
//!
//! 위치 행의 경로(`volume_id`/`rel_path`/`path_key`)만 새 루트로 옮기고 `file_identity` 는 비운다(다음 스캔이 채운다).
//! `presence` 는 건드리지 않는다 — 실제 파일 유무는 뒤이은 스캔이 판정하고, 끝나면 못 본 파일을 `missing` 으로 표시한다.
//! 원본 파일은 읽지도 쓰지도 않는다. 모든 변경과 자산 `revision`/색인 이벤트는 한 트랜잭션이다.

use mam_db::Clock;
use rusqlite::{Connection, TransactionBehavior, params};

use super::IngestError;
use super::folders::{check_overlap, is_inside, load_folder};
use crate::jobs::JobKind;
use crate::keyrange::{join_key, key_range};
use crate::registry::register_volume;
use crate::volume::VolumeInfo;

/// 한 트랜잭션 안에서 한 번에 옮기는 위치 수 (메모리 상한; 트랜잭션은 하나로 유지)
const BATCH: i64 = 5_000;

/// 새 경로 아래 남은 활성 위치 존재 검사 / 옛 경로 아래 위치 한 묶음 조회 — 둘 다 `uq_file_locations_active_path` 범위 조회여야 한다 (T-relocate-10).
const ORPHAN_SQL: &str = "SELECT EXISTS(SELECT 1 FROM file_locations
          WHERE volume_id = ?1 AND detached_at IS NULL AND path_key >= ?2 AND path_key < ?3)";
const BATCH_SQL: &str = "SELECT location_id, asset_id, rel_path, path_key FROM file_locations
                 WHERE volume_id = ?1 AND detached_at IS NULL AND path_key >= ?2 AND path_key < ?3
                 LIMIT ?4";

/// 배치 작업용 임시 테이블 (트랜잭션 안에서 만들고 끝에 지운다 — 롤백되면 함께 사라진다)
const RELOC_TEMP_SQL: &str = "CREATE TEMP TABLE IF NOT EXISTS reloc_batch (
           location_id INTEGER PRIMARY KEY, asset_id INTEGER NOT NULL, new_rel TEXT NOT NULL, new_key TEXT NOT NULL);
         DELETE FROM reloc_batch;";

/// 이전 대상: 새 루트 (파일시스템 검증을 마친 값).
#[derive(Debug, Clone)]
pub struct RelocateTarget {
    pub info: VolumeInfo,
    /// 볼륨 기준 상대 루트 (`\` 구분, 볼륨 루트면 '')
    pub rel_root: String,
    pub path_key: String,
    pub display_path: String,
}

/// 폴더의 위치 행을 새 루트로 옮기고 스캔을 예약한다. 옮긴 위치 수를 돌려준다. 실패하면 아무것도 바뀌지 않는다.
pub fn relocate_folder_tx(
    conn: &mut Connection,
    clock: &dyn Clock,
    folder_id: i64,
    target: &RelocateTarget,
) -> Result<u64, IngestError> {
    // 인덱스 여럿을 흩어 갱신하므로 페이지 캐시를 이 트랜잭션 동안만 키운다 (쓰기 연결은 공유 — 성공·실패 모두 복원).
    // 측정(100k): 기본 캐시 3.2 s -> 256 MiB 2.0 s
    let prev_cache: Option<i64> = conn.query_row("PRAGMA cache_size", [], |r| r.get(0)).ok();
    let _ = conn.execute_batch(&format!("PRAGMA cache_size = {RELOCATE_CACHE_KIB_NEG};"));
    let result = relocate_inner(conn, clock, folder_id, target);
    if let Some(prev) = prev_cache {
        let _ = conn.execute_batch(&format!("PRAGMA cache_size = {prev};"));
    }
    result
}

/// SQLite `cache_size` 음수 = KiB 단위 (256 MiB)
const RELOCATE_CACHE_KIB_NEG: i64 = -262_144;

fn relocate_inner(
    conn: &mut Connection,
    clock: &dyn Clock,
    folder_id: i64,
    target: &RelocateTarget,
) -> Result<u64, IngestError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let now = clock.now_ms();
    let old = load_folder(&tx, folder_id)?;
    if !old.enabled {
        return Err(IngestError::NotFound);
    }
    let running: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE kind = 'scan' AND folder_id = ?1 AND status = 'running')",
        [folder_id],
        |r| r.get(0),
    )?;
    if running {
        return Err(IngestError::ScanRunning);
    }
    let new_volume = register_volume(&tx, clock, &target.info)?;
    if new_volume == old.volume_id {
        if target.path_key == old.path_key {
            return Err(IngestError::SamePath);
        }
        if is_inside(&target.path_key, &old.path_key) {
            return Err(IngestError::NestedFolder {
                inside_existing: true,
            });
        }
        if is_inside(&old.path_key, &target.path_key) {
            return Err(IngestError::NestedFolder {
                inside_existing: false,
            });
        }
    }
    check_overlap(&tx, new_volume, &target.path_key, Some(folder_id))?;
    // 새 경로 아래에 다른 출처의 활성 위치가 남아 있으면 (유일 인덱스 충돌·자산 뒤섞임 방지) 거부한다
    let (new_lo, new_hi) = key_range(&target.path_key);
    let orphans: bool = tx.query_row(ORPHAN_SQL, params![new_volume, new_lo, new_hi], |r| {
        r.get(0)
    })?;
    if orphans {
        tracing::warn!(folder_id, "relocation target already has active locations");
        return Err(IngestError::NestedFolder {
            inside_existing: false,
        });
    }

    tx.execute(
        "UPDATE managed_folders SET volume_id = ?2, rel_root = ?3, path_key = ?4, display_path = ?5
         WHERE folder_id = ?1",
        params![
            folder_id,
            new_volume,
            target.rel_root,
            target.path_key,
            target.display_path
        ],
    )?;
    // 멈춰 둔 스캔의 이어하기 지점(cursor)은 옛 트리 기준이다 — 새 루트부터 다시 훑도록 취소한다
    tx.execute(
        "UPDATE scan_runs SET status = 'cancelled', finished_at = ?2
         WHERE folder_id = ?1 AND status IN ('running', 'paused')",
        params![folder_id, now],
    )?;

    let (old_lo, old_hi) = key_range(&old.path_key);
    // 배치마다 (위치, 새 경로) 를 임시 테이블에 모은 뒤 집합 단위로 반영한다 (행마다 문장 재해석·왕복 제거).
    // 활성 위치는 자산당 하나(`uq_file_locations_active_asset`)이므로 자산당 revision +1·이벤트 1건이 행별 처리와 같다.
    tx.execute_batch(RELOC_TEMP_SQL)?;
    let mut moved = 0u64;
    loop {
        let rows: Vec<(i64, i64, String, String)> = {
            let mut stmt = tx.prepare_cached(BATCH_SQL)?;
            stmt.query_map(params![old.volume_id, old_lo, old_hi, BATCH], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<Result<_, _>>()?
        };
        if rows.is_empty() {
            break;
        }
        tx.execute("DELETE FROM reloc_batch", [])?;
        {
            let mut ins = tx.prepare_cached(
                "INSERT INTO reloc_batch (location_id, asset_id, new_rel, new_key) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (location_id, asset_id, rel_path, key) in &rows {
                let new_key = rebase_key(key, &old.path_key, &target.path_key);
                let new_rel =
                    rebase_rel(rel_path, &old.rel_root, &target.rel_root).unwrap_or_else(|| {
                        tracing::warn!(
                            location_id,
                            "rel_path does not start with the folder root; rebuilt from key"
                        );
                        let suffix = key_suffix(key, &old.path_key).replace('/', "\\");
                        join_rel(&target.rel_root, &suffix)
                    });
                ins.execute(params![location_id, asset_id, new_rel, new_key])?;
            }
        }
        tx.execute(
            "UPDATE file_locations SET volume_id = ?1, rel_path = b.new_rel, path_key = b.new_key, file_identity = NULL
             FROM reloc_batch b WHERE file_locations.location_id = b.location_id",
            params![new_volume],
        )?;
        tx.execute(
            "UPDATE media_assets SET revision = revision + 1, updated_at = ?1
             WHERE asset_id IN (SELECT asset_id FROM reloc_batch)",
            params![now],
        )?;
        // `reconcile::store::emit_outbox` 와 같은 행: upsert, 스캔 우선순위, 갱신 후 revision
        tx.execute(
            "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
             SELECT a.asset_id, a.revision, 'upsert', ?1, ?2
             FROM reloc_batch b JOIN media_assets a ON a.asset_id = b.asset_id ORDER BY b.location_id",
            params![crate::reconcile::SCAN_OUTBOX_PRIORITY, now],
        )?;
        moved += rows.len() as u64;
    }
    tx.execute_batch("DROP TABLE reloc_batch")?;

    // 이전 직후 새 위치를 스캔한다: 실패 기록은 정리하고, 대기 중인 스캔은 즉시 실행 가능하게 하며, 없으면 만든다
    tx.execute(
        "DELETE FROM jobs WHERE kind = 'scan' AND folder_id = ?1 AND status = 'failed'",
        [folder_id],
    )?;
    tx.execute(
        "UPDATE jobs SET not_before = 0, attempts = 0
         WHERE kind = 'scan' AND folder_id = ?1 AND status = 'pending'",
        [folder_id],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO jobs (kind, folder_id, priority, created_at, updated_at)
         VALUES ('scan', ?1, ?2, ?3, ?3)",
        params![folder_id, JobKind::Scan.default_priority(), now],
    )?;
    tx.commit()?;
    Ok(moved)
}

/// `old_key`(옛 루트 하위 키)의 루트 이후 부분 (`/` 제외).
fn key_suffix<'a>(old_key: &'a str, old_root: &str) -> &'a str {
    let skip = old_root.len() + usize::from(!old_root.is_empty());
    old_key.get(skip..).unwrap_or("")
}

/// 옛 루트 하위 `path_key` 를 새 루트 하위로 옮긴다 (NFC·소문자 키는 접미부를 그대로 보존).
pub(crate) fn rebase_key(old_key: &str, old_root: &str, new_root: &str) -> String {
    join_key(new_root, key_suffix(old_key, old_root))
}

fn join_rel(root: &str, rest: &str) -> String {
    match (root.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_string(),
        (false, true) => root.to_string(),
        (false, false) => format!("{root}\\{rest}"),
    }
}

fn segments(s: &str) -> Vec<&str> {
    s.split(['\\', '/']).filter(|p| !p.is_empty()).collect()
}

/// 옛 `rel_path` 의 루트 부분을 새 루트로 바꾼다 (대소문자·원래 유니코드 형태 보존, 구분자는 `\`).
/// 옛 루트로 시작하지 않으면(대소문자 무시) `None`.
pub(crate) fn rebase_rel(old_rel: &str, old_rel_root: &str, new_rel_root: &str) -> Option<String> {
    let root = segments(old_rel_root);
    let parts = segments(old_rel);
    if parts.len() <= root.len()
        || !root
            .iter()
            .zip(&parts)
            .all(|(a, b)| a.to_lowercase() == b.to_lowercase())
    {
        return None;
    }
    Some(join_rel(new_rel_root, &parts[root.len()..].join("\\")))
}

#[cfg(test)]
mod tests {
    //! 분류: Boundary / Validation / Business rule (인덱스 사용)
    use super::*;
    use unicode_normalization::UnicodeNormalization;

    #[test]
    fn t_relocate_10_batch_and_orphan_queries_use_the_active_path_index() {
        // Arrange: 마이그레이션이 적용된 실제 DB
        let dir = tempfile::TempDir::new().unwrap();
        let mut conn = mam_db::open(&dir.path().join("t.sqlite")).unwrap();
        struct Fixed;
        impl Clock for Fixed {
            fn now_ms(&self) -> i64 {
                0
            }
        }
        mam_db::migrate(&mut conn, &dir.path().join("bk"), &Fixed).unwrap();
        for (name, sql) in [("batch", BATCH_SQL), ("orphan", ORPHAN_SQL)] {
            // Act
            let n = if name == "batch" { 4 } else { 3 };
            let params: Vec<rusqlite::types::Value> = vec![
                1.into(),
                "a/".to_string().into(),
                "a0".to_string().into(),
                5000.into(),
            ];
            let plan: Vec<String> = {
                let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
                stmt.query_map(rusqlite::params_from_iter(params.iter().take(n)), |r| {
                    r.get::<_, String>(3)
                })
                .unwrap()
                .map(Result::unwrap)
                .collect()
            };
            // Assert
            let text = plan.join(" / ");
            assert!(
                text.contains("uq_file_locations_active_path"),
                "{name}: {text}"
            );
            assert!(
                !text.contains("idx_file_locations_volume"),
                "{name}: {text}"
            );
            assert!(!text.contains("SCAN file_locations"), "{name}: {text}");
            eprintln!("EXPLAIN {name}: {text}");
        }
    }

    #[test]
    fn t_relocate_15_page_cache_is_restored_after_a_failed_relocation() {
        // Arrange: 기본 캐시 크기를 가진 연결, 존재하지 않는 폴더
        let dir = tempfile::TempDir::new().unwrap();
        let mut conn = mam_db::open(&dir.path().join("t.sqlite")).unwrap();
        struct Fixed;
        impl Clock for Fixed {
            fn now_ms(&self) -> i64 {
                0
            }
        }
        mam_db::migrate(&mut conn, &dir.path().join("bk"), &Fixed).unwrap();
        let cache = |c: &Connection| {
            c.query_row("PRAGMA cache_size", [], |r| r.get::<_, i64>(0))
                .unwrap()
        };
        let before = cache(&conn);
        let target = RelocateTarget {
            info: VolumeInfo {
                key: "k".into(),
                kind: crate::volume::VolumeKind::Fixed,
                label: None,
                mount_point: "C:\\".into(),
            },
            rel_root: String::new(),
            path_key: "x".into(),
            display_path: "C:\\x".into(),
        };
        // Act
        let err = relocate_folder_tx(&mut conn, &Fixed, 9_999, &target).unwrap_err();
        // Assert
        assert!(matches!(err, IngestError::NotFound), "{err:?}");
        assert_eq!(cache(&conn), before);
    }

    #[test]
    fn t_relocate_11a_rebase_key_handles_empty_roots() {
        assert_eq!(rebase_key("a/b/c.jpg", "a/b", "x"), "x/c.jpg");
        assert_eq!(rebase_key("a/b/c.jpg", "", "x/y"), "x/y/a/b/c.jpg");
        assert_eq!(rebase_key("a/b/c.jpg", "a/b", ""), "c.jpg");
        assert_eq!(rebase_key("c.jpg", "", ""), "c.jpg");
    }

    #[test]
    fn t_relocate_11b_rebase_rel_keeps_case_and_unicode_form() {
        // Arrange: NFD 로 저장된 한글 파일명과 대소문자가 섞인 이름
        let nfd: String = "사진.JPG".nfd().collect();
        let old = format!("Photos\\2024\\{nfd}");
        // Act
        let moved = rebase_rel(&old, "Photos", "Backup\\Pics").unwrap();
        // Assert: 접미부는 그대로, 루트만 교체
        assert_eq!(moved, format!("Backup\\Pics\\2024\\{nfd}"));
        assert_eq!(
            mam_db::path_key(&moved).unwrap(),
            rebase_key(&mam_db::path_key(&old).unwrap(), "photos", "backup/pics")
        );
    }

    #[test]
    fn t_relocate_11c_rebase_rel_mixed_separators_and_empty_roots() {
        assert_eq!(
            rebase_rel("a/b\\c.jpg", "A", "z").as_deref(),
            Some("z\\b\\c.jpg")
        );
        assert_eq!(
            rebase_rel("a\\c.jpg", "", "z").as_deref(),
            Some("z\\a\\c.jpg")
        );
        assert_eq!(rebase_rel("a\\c.jpg", "a", "").as_deref(), Some("c.jpg"));
    }

    #[test]
    fn t_relocate_11d_rebase_rel_prefix_mismatch_or_no_tail_is_none() {
        assert_eq!(rebase_rel("other\\c.jpg", "a", "z"), None);
        assert_eq!(
            rebase_rel("a", "a", "z"),
            None,
            "the root itself has no file part"
        );
    }
}
