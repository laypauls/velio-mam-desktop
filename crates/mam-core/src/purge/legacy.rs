//! 기존 보관 데이터 1회 정리 — 폴더 제거가 위치만 detach 하던 때(D9) 남은 자산의 연결 데이터.
//!
//! 대상: `status = 'active'` 이고 **활성 위치가 하나도 없으며**, 가진 위치 행이 모두 **현재 어떤 관리 폴더 범위에도 들지 않는** 자산.
//! 마지막 조건은 "관리 폴더 밖으로 이동"(`protect` 의 `detach_moved_out`)처럼 폴더는 남아 있는 채 위치만 detach 된 자산을
//! 잘못 지우지 않기 위한 것이다 (그 자산의 옛 위치는 아직 등록된 폴더 안이다).
//! 원본 삭제는 `deleted`, 내용 변경은 `superseded` 라 대상이 아니다.

use rusqlite::{Connection, TransactionBehavior};

use super::linked::purge_assets;

/// `after` 보다 큰 asset_id 중 "활성인데 활성 위치가 없는" 후보를 오름차순으로 최대 `limit` 개 (읽기 전용, 키셋 페이지).
/// 관리 폴더 범위 판정은 쓰기 단계에서 다시 한다.
pub fn legacy_candidates(conn: &Connection, after: i64, limit: usize) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT a.asset_id FROM media_assets a
         WHERE a.asset_id > ?1 AND a.status = 'active'
           AND NOT EXISTS (SELECT 1 FROM file_locations l WHERE l.asset_id = a.asset_id AND l.detached_at IS NULL)
         ORDER BY a.asset_id LIMIT ?2",
    )?;
    let rows = stmt.query_map([after, limit as i64], |r| r.get(0))?;
    rows.collect()
}

fn covered(folders: &[(i64, String)], volume_id: i64, key: &str) -> bool {
    folders.iter().any(|(v, root)| {
        *v == volume_id
            && (root.is_empty()
                || key == root
                || key.strip_prefix(root.as_str()).is_some_and(|r| r.starts_with('/')))
    })
}

/// 후보 중 조건을 다시 확인한 자산의 연결 데이터와 (detach 된) 위치 행을 지운다. 지운 자산 id 와 지운 임베딩의 모델 키를 돌려준다.
/// 위치 행이 아예 없는 자산은 근거가 없어 건드리지 않는다. 한 트랜잭션, 멱등.
pub fn purge_legacy_batch(
    conn: &mut Connection,
    now: i64,
    candidates: &[i64],
) -> rusqlite::Result<(Vec<i64>, Vec<super::EmbeddingKey>)> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let folders: Vec<(i64, String)> = {
        let mut stmt = tx.prepare("SELECT volume_id, path_key FROM managed_folders")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut targets = Vec::new();
    for &asset_id in candidates {
        let still: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM media_assets a WHERE a.asset_id = ?1 AND a.status = 'active'
               AND NOT EXISTS (SELECT 1 FROM file_locations l WHERE l.asset_id = a.asset_id AND l.detached_at IS NULL))",
            [asset_id],
            |r| r.get(0),
        )?;
        if !still {
            continue;
        }
        let rows: Vec<(i64, String)> = {
            let mut stmt =
                tx.prepare_cached("SELECT volume_id, path_key FROM file_locations WHERE asset_id = ?1")?;
            let it = stmt.query_map([asset_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            it.collect::<Result<_, _>>()?
        };
        if !rows.is_empty() && !rows.iter().any(|(v, k)| covered(&folders, *v, k)) {
            targets.push(asset_id);
        }
    }
    let keys = purge_assets(&tx, now, &targets)?;
    {
        let mut del = tx.prepare_cached("DELETE FROM file_locations WHERE asset_id = ?1")?;
        for id in &targets {
            del.execute([id])?;
        }
    }
    tx.commit()?;
    Ok((targets, keys))
}
