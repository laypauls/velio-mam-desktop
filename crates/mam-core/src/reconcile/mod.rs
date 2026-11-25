//! reconcile — 폴더 스캔 결과(파일 1건)와 DB 를 비교해 자산 상태를 판정·반영한다.
//!
//! 판정 (결정 기록 D1/D4/D5/D6, 작업 문서 `20261028-000833-p1-4-asset-identity.md`) — 재연결 우선순위:
//! R1 같은 경로 / R2 같은 볼륨의 NTFS 파일 ID / R3 폴더 재등록 / 그 밖의 해시 재연결은 전체 해시가 있는 자산만 대상.
//! 폴더 자체의 이전은 여기가 아니라 `scan::folder_relocate` 가 위치 행을 옮겨 처리한다.
//! - 같은 경로, 크기·수정 시각 동일 -> 변화 없음 (`missing` 이었다면 `present` 복귀)
//! - 같은 경로, 크기 또는 수정 시각 변화 -> 전체 해시가 있는 자산은 해시 재계산: 같으면 같은 자산(터치), 다르면 새 자산 + 구자산 `superseded`.
//!   전체 해시가 없는 자산(크기가 유일해 생략한 경우 포함)은 같은 경로에서 크기·수정 시각이 바뀌면 비교할 근거가 없으므로 새 자산 + 구자산 `superseded`
//!   (새 파일의 앞뒤 4K 체크섬 `edge_sig` 를 스캔 중에 계산해 함께 기록). 파일 ID 만 바뀌고 크기·수정 시각이 같으면 메타만 갱신
//! - 새 경로, 같은 볼륨·같은 파일 ID·같은 크기/수정 시각 -> 기존 자산 재연결 (외부 이동·이름 변경)
//! - 새 경로, 크기가 같은 `missing` 후보가 있으면 전체 해시 비교 -> 같으면 재연결 (볼륨 간 이동, 파일 ID 없는 볼륨).
//!   후보는 전체 해시가 있는(`done`) 자산뿐이다 — 보조 경로이며 보장 대상이 아니다
//! - 그 외 -> 새 자산 (후보가 없으면 해시는 `pending`, 전체 해시는 `hash` 작업이 같은 크기·같은 `edge_sig` 상대가 있을 때만 계산:
//!   [`Reconciler::hash_pending_asset`], `processing::hash_job`)
//! - 볼륨 스캔 완료([`Reconciler::finish_scan`]) 시 못 본 활성 위치 -> `missing` (볼륨이 online 일 때만)
//!
//! 모든 자산 변경은 `revision` 증가와 `search_outbox` 이벤트를 **같은 트랜잭션**에 기록한다.
//! 해시 I/O 는 쓰기 트랜잭션 밖에서 수행한다.

mod apply;
mod batch;
mod store;

pub use batch::{BatchItem, BatchOutcome, touch_excluded_paths, touch_under};

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use mam_db::Clock;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::error::CoreError;
use crate::hash::ContentHasher;
use crate::identity::FileMeta;
use store::AssetChange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
    Audio,
}

impl MediaType {
    /// `media_assets.media_type` CHECK 값
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
}

/// 스캔에서 본 파일 1건. `rel_path` 는 볼륨 마운트 지점 기준 상대 경로, `abs_path` 는 해시를 읽을 실제 경로.
#[derive(Debug, Clone)]
pub struct ScanEntry<'a> {
    pub rel_path: &'a str,
    pub abs_path: &'a std::path::Path,
    pub media_type: MediaType,
    pub meta: FileMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveBy {
    /// 같은 볼륨의 파일 ID + 크기 + 수정 시각
    FileId,
    /// 전체 내용 해시
    Hash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Created {
        asset_id: i64,
    },
    Unchanged {
        asset_id: i64,
    },
    /// `missing` 이던 위치가 같은 경로에 다시 나타남
    Reappeared {
        asset_id: i64,
    },
    Moved {
        asset_id: i64,
        by: MoveBy,
    },
    /// 수정 시각/크기만 바뀌고 해시는 같음
    Touched {
        asset_id: i64,
    },
    ContentChanged {
        old_asset_id: i64,
        new_asset_id: i64,
    },
    /// 해시 미계산(pending) 자산의 메타만 갱신 (크기·수정 시각은 같고 파일 ID 만 바뀐 경우)
    PendingUpdated {
        asset_id: i64,
    },
    /// 폴더 제거로 detach 된 위치가 같은 경로·크기·수정 시각으로 다시 나타나 기존 자산에 재연결 (P2-3 D9)
    Reattached {
        asset_id: i64,
    },
    /// 해시 도중 변경이 감지돼 결과를 폐기 — 데이터 변경 없음, 다음 스캔에서 재시도 (D5)
    HashDeferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingHash {
    Done,
    Deferred,
}

#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    pub location_id: i64,
    pub asset_id: i64,
    pub hash: [u8; 32],
}

/// 볼륨 1회 스캔의 문맥. 시작 시 `missing` 후보를 한 번 적재한다 (신규 파일마다 쿼리하지 않기 위해).
///
/// `scan_at` 은 직전 스캔보다 커야 한다 (위치의 `last_seen_at < scan_at` 로 "이번에 못 본" 파일을 가린다).
#[derive(Debug)]
pub struct ScanSession {
    pub(crate) volume_id: i64,
    pub(crate) scan_at: i64,
    pub(crate) is_network: bool,
    is_online: bool,
    candidates: HashMap<i64, Vec<Candidate>>,
}

impl ScanSession {
    pub fn begin(conn: &Connection, clock: &dyn Clock, volume_id: i64) -> Result<Self, CoreError> {
        Self::begin_at(conn, clock.now_ms(), volume_id)
    }

    /// `scan_at` 을 지정해 시작한다 — 중단한 스캔을 이어갈 때 원래 시작 시각을 쓴다
    /// (앞 구간에서 이미 본 위치가 "못 본" 것으로 바뀌지 않게).
    pub fn begin_at(conn: &Connection, scan_at: i64, volume_id: i64) -> Result<Self, CoreError> {
        let (kind, availability): (String, String) = conn
            .query_row(
                "SELECT kind, availability FROM volumes WHERE volume_id = ?1",
                [volume_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(CoreError::VolumeNotFound(volume_id))?;
        let mut stmt = conn.prepare(
            "SELECT l.location_id, l.asset_id, a.file_size, a.content_hash
             FROM file_locations l JOIN media_assets a ON a.asset_id = l.asset_id
             WHERE l.detached_at IS NULL AND l.presence = 'missing'
               AND a.status = 'active' AND a.hash_state = 'done'",
        )?;
        let mut candidates: HashMap<i64, Vec<Candidate>> = HashMap::new();
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        for row in rows {
            let (size, location_id, asset_id, blob) = row?;
            if let Ok(hash) = <[u8; 32]>::try_from(blob) {
                candidates.entry(size).or_default().push(Candidate {
                    location_id,
                    asset_id,
                    hash,
                });
            }
        }
        Ok(Self {
            volume_id,
            scan_at,
            is_network: kind == "network",
            is_online: availability == "online",
            candidates,
        })
    }

    pub fn scan_at(&self) -> i64 {
        self.scan_at
    }

    pub(crate) fn candidates_for(&self, size: u64) -> &[Candidate] {
        i64::try_from(size)
            .ok()
            .and_then(|s| self.candidates.get(&s))
            .map_or(&[], Vec::as_slice)
    }

    fn consume(&mut self, location_id: i64) {
        for list in self.candidates.values_mut() {
            list.retain(|c| c.location_id != location_id);
        }
    }
}

/// 시계·해시 계산기·취소 플래그를 묶은 reconcile 실행기.
pub struct Reconciler<'a> {
    pub clock: &'a dyn Clock,
    pub hasher: &'a dyn ContentHasher,
    pub cancel: &'a AtomicBool,
}

impl Reconciler<'_> {
    /// 스캔 결과 1건을 반영한다. 업무 규칙 위반(`CaseCollision`)이나 오류가 나면 DB 는 변하지 않는다.
    pub fn reconcile(
        &self,
        conn: &mut Connection,
        session: &mut ScanSession,
        entry: &ScanEntry<'_>,
    ) -> Result<Outcome, CoreError> {
        let mut hashed = None;
        let mut edge = None;
        loop {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            match apply::apply(
                &tx,
                self.clock.now_ms(),
                session,
                entry,
                hashed.as_ref(),
                edge.as_ref(),
            )? {
                apply::Step::Done {
                    outcome,
                    consumed_candidate,
                } => {
                    tx.commit()?;
                    if let Some(location_id) = consumed_candidate {
                        session.consume(location_id);
                    }
                    return Ok(outcome);
                }
                apply::Step::NeedHash if hashed.is_some() => {
                    return Err(CoreError::Internal("hash requested twice".into()));
                }
                apply::Step::NeedEdge if edge.is_some() => {
                    return Err(CoreError::Internal("edge signature requested twice".into()));
                }
                apply::Step::NeedHash => {
                    drop(tx); // 해시 I/O 동안 쓰기 잠금을 잡지 않는다
                    match self.hasher.hash_file(entry.abs_path, self.cancel) {
                        Ok(h)
                            if h.size == entry.meta.size
                                && h.modified_ms == entry.meta.modified_ms =>
                        {
                            hashed = Some(h);
                        }
                        // 스캔 이후 또는 해시 도중 파일이 바뀜 (D5): 결과 폐기, 다음 스캔에서 재시도
                        Ok(_) | Err(CoreError::ChangedDuringHash(_)) => {
                            self.mark_seen_deferred(conn, session, entry)?;
                            return Ok(Outcome::HashDeferred);
                        }
                        Err(e) => return Err(e),
                    }
                }
                apply::Step::NeedEdge => {
                    drop(tx);
                    match self.hasher.edge_signature(entry.abs_path, self.cancel) {
                        Ok(e)
                            if e.size == entry.meta.size
                                && e.modified_ms == entry.meta.modified_ms =>
                        {
                            edge = Some(e);
                        }
                        Ok(_) | Err(CoreError::ChangedDuringHash(_)) => {
                            self.mark_seen_deferred(conn, session, entry)?;
                            return Ok(Outcome::HashDeferred);
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }

    fn mark_seen_deferred(
        &self,
        conn: &Connection,
        session: &ScanSession,
        entry: &ScanEntry<'_>,
    ) -> Result<(), CoreError> {
        let key = mam_db::path_key(entry.rel_path)?;
        store::mark_seen_at_path(conn, session.volume_id, &key, session.scan_at)
    }

    /// 볼륨 스캔이 **끝까지 완료됐을 때** 호출한다. 이번 스캔에서 못 본 활성 위치를 `missing` 으로 표시하고
    /// 개수를 돌려준다. 볼륨이 offline 이면 연결 끊김일 수 있으므로 아무것도 바꾸지 않는다 (0).
    pub fn finish_scan(
        &self,
        conn: &mut Connection,
        session: &ScanSession,
    ) -> Result<usize, CoreError> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // 세션 시작 이후 볼륨 상태가 바뀌었을 수 있으므로 DB 의 현재 값도 확인한다
        let online: String = tx.query_row(
            "SELECT availability FROM volumes WHERE volume_id = ?1",
            [session.volume_id],
            |r| r.get(0),
        )?;
        if !session.is_online || online != "online" {
            return Ok(0);
        }
        let unseen: Vec<(i64, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT location_id, asset_id FROM file_locations
                 WHERE volume_id = ?1 AND detached_at IS NULL AND presence = 'present' AND last_seen_at < ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![session.volume_id, session.scan_at], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            rows.collect::<Result<_, _>>()?
        };
        let now = self.clock.now_ms();
        for (location_id, asset_id) in &unseen {
            tx.execute(
                "UPDATE file_locations SET presence = 'missing' WHERE location_id = ?1",
                [location_id],
            )?;
            store::update_asset(&tx, now, *asset_id, AssetChange::default())?;
        }
        tx.commit()?;
        Ok(unseen.len())
    }

    /// 해시가 `pending` 인 자산의 해시를 계산해 저장한다 (P2 작업 대기열이 호출).
    ///
    /// 파일이 DB 의 크기·수정 시각과 다르거나 해시 도중 바뀌면 결과를 폐기하고 `pending` 으로 둔다.
    pub fn hash_pending_asset(
        &self,
        conn: &mut Connection,
        asset_id: i64,
        abs_path: &std::path::Path,
    ) -> Result<PendingHash, CoreError> {
        // 삭제 표시된 자산(폴더 제거 뒤 늦게 끝난 해시 작업)에는 해시·revision·색인 이벤트를 남기지 않는다
        if !crate::purge::asset_is_active(conn, asset_id)? {
            return Ok(PendingHash::Deferred);
        }
        let asset = store::asset(conn, asset_id)?;
        if asset.hash.is_some() {
            return Ok(PendingHash::Done);
        }
        let hashed = match self.hasher.hash_file(abs_path, self.cancel) {
            Ok(h) => h,
            Err(CoreError::ChangedDuringHash(_)) => return Ok(PendingHash::Deferred),
            Err(e) => return Err(e),
        };
        if i64::try_from(hashed.size) != Ok(asset.size) || hashed.modified_ms != asset.modified_at {
            return Ok(PendingHash::Deferred);
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = self.commit_pending_hash(&tx, asset_id, &hashed)?;
        tx.commit()?;
        Ok(result)
    }

    /// 계산된 해시를 `pending` 자산에 기록한다 (호출자의 트랜잭션 안, 커밋은 호출자). 해시 I/O 를 쓰기 잠금 밖에서 한
    /// 뒤 반영하는 P2-3 해시 실행기가 쓴다. 이미 해시가 있으면 `Done` (멱등), 크기·수정 시각이 DB 와 다르면 `Deferred`.
    pub fn commit_pending_hash(
        &self,
        conn: &Connection,
        asset_id: i64,
        hashed: &crate::hash::HashedFile,
    ) -> Result<PendingHash, CoreError> {
        // 삭제 표시된 자산(폴더 제거 뒤 늦게 끝난 해시 작업)에는 해시·revision·색인 이벤트를 남기지 않는다
        if !crate::purge::asset_is_active(conn, asset_id)? {
            return Ok(PendingHash::Deferred);
        }
        let asset = store::asset(conn, asset_id)?;
        if asset.hash.is_some() {
            return Ok(PendingHash::Done);
        }
        if i64::try_from(hashed.size) != Ok(asset.size) || hashed.modified_ms != asset.modified_at {
            return Ok(PendingHash::Deferred);
        }
        store::update_asset(
            conn,
            self.clock.now_ms(),
            asset_id,
            AssetChange {
                hash: Some(hashed.hash.as_bytes()),
                ..Default::default()
            },
        )?;
        Ok(PendingHash::Done)
    }
}

/// 스캔이 남기는 outbox 이벤트 우선순위 (집합 단위 갱신 경로가 `emit_outbox` 와 같은 값을 쓴다)
pub(crate) const SCAN_OUTBOX_PRIORITY: i64 = store::SCAN_PRIORITY;

/// 자산의 `revision` 을 올리고 색인 이벤트를 남긴다 (메타데이터 변경 등 자산 컬럼 밖의 변경). 호출자의 트랜잭션 안에서만.
pub(crate) fn bump_revision(conn: &Connection, now: i64, asset_id: i64) -> Result<i64, CoreError> {
    store::update_asset(conn, now, asset_id, AssetChange::default())
}

/// 자산 상태(`superseded`/`deleted` 등)를 바꾼다 (병합 등 P2-3 내부용). 호출자의 트랜잭션 안에서만.
pub(crate) fn set_asset_status(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    status: &str,
) -> Result<i64, CoreError> {
    store::update_asset(
        conn,
        now,
        asset_id,
        AssetChange {
            status: Some(status),
            ..Default::default()
        },
    )
}

/// 자산의 크기·수정 시각을 바꾼다 (병합 등 P2-3 내부용). 호출자의 트랜잭션 안에서만.
pub(crate) fn set_asset_file_state(
    conn: &Connection,
    now: i64,
    asset_id: i64,
    size: i64,
    modified_at: i64,
) -> Result<i64, CoreError> {
    store::update_asset(
        conn,
        now,
        asset_id,
        AssetChange {
            size: Some(size),
            modified_at: Some(modified_at),
            ..Default::default()
        },
    )
}
