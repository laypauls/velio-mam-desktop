//! 감시 이벤트 1건(경로 하나)의 반영 — 파일은 단건 reconcile, 폴더는 그 하위만 순회, 사라진 경로는 missing.
//! 이벤트 종류는 믿지 않고 디바운스 이후의 **실제 파일시스템 상태**로 판단한다 (이름 변경 = 옛 경로 사라짐 + 새 경로 생김).

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use mam_db::Priority;
use rusqlite::params;

use super::IngestError;
use super::errors::record_error;
use super::folders::{FolderRow, load_folder};
use super::guarded::{EntryKind, UNRESPONSIVE_CODE, UNRESPONSIVE_MESSAGE};
use super::io_guard::Guarded;
use super::walk::{is_hidden_name, is_reserved_dir_name};
use super::work::{DirRec, ScanCounts, Task, Walk, WalkEnd, Work, join_rel};
use crate::keyrange::{join_key, key_range};
use crate::reconcile::{Reconciler, ScanSession};
use crate::scan::ScanEnv;
use crate::volume::strip_verbatim;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathAction {
    /// 관리 범위 밖·숨김·제외·미디어 아님·폴더 비활성/오프라인
    Ignored,
    /// 파일 1건을 반영했다
    Reconciled,
    /// 사라진 경로 하위의 위치 n 건을 missing 으로 표시했다
    Missing(usize),
    /// 폴더 하위를 순회해 반영했다 (`apply_path_settled` 는 파일을 대기열에 넘겼다)
    Subtree,
    /// `apply_path_settled`: 하위 파일을 대기열에 다 넣지 못했다(폴더 행 상한 등) — 호출자가 폴더 재스캔 1회로 바꿔야 한다
    SubtreeOverflow,
    /// 읽지 못해 scan_errors 에 기록했다
    Failed,
}

fn parts(p: &Path) -> Vec<String> {
    PathBuf::from(strip_verbatim(&p.to_string_lossy()))
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// `root` 아래 `abs` 의 이름 구성요소 (원래 표기). 루트 자신·밖이면 `None`.
fn rel_parts(root: &Path, abs: &Path) -> Option<Vec<String>> {
    let (r, a) = (parts(root), parts(abs));
    if a.len() <= r.len() || a[..r.len()] != r[..] {
        return None;
    }
    let abs = PathBuf::from(strip_verbatim(&abs.to_string_lossy()));
    let names: Vec<String> = abs
        .components()
        .skip(r.len())
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    (names.len() == a.len() - r.len()).then_some(names)
}

fn key_of(names: &[String]) -> String {
    mam_db::path_key(&names.join("/")).unwrap_or_default()
}

fn dir_chain(names: &[String]) -> Vec<DirRec> {
    (1..=names.len())
        .map(|n| DirRec {
            key: key_of(&names[..n]),
            parent_key: key_of(&names[..n - 1]),
            name: names[n - 1].clone(),
            rel: names[..n].join("/"),
        })
        .collect()
}

fn single_walk<'a>(
    env: &'a ScanEnv,
    folder: &'a FolderRow,
    now: i64,
    cancel: &'a AtomicBool,
    stop: &'a dyn Fn() -> bool,
) -> Result<Walk<'a>, IngestError> {
    let volume_id = folder.volume_id;
    let session = env.write(Priority::Low, move |c| {
        Ok(ScanSession::begin_at(c, now, volume_id)?)
    })?;
    Ok(Walk::new(
        env,
        folder,
        Arc::new(Mutex::new(session)),
        now,
        None,
        ScanCounts::default(),
        cancel,
        stop,
    ))
}

/// 경로 하나를 현재 파일시스템 상태에 맞춰 반영한다 (폴더면 그 하위의 파일도 바로 반영한다).
pub fn apply_path(env: &ScanEnv, folder_id: i64, abs: &Path) -> Result<PathAction, IngestError> {
    apply_path_with(env, folder_id, abs, None)
}

/// [`apply_path`] 의 감시(변경 대기열) 경로용 — 폴더를 순회하면 새로 찾은 하위 폴더는 계속 따라가되 **파일은 바로 반영하지 않고**
/// 변경 대기열에 `path` 행(`source` 보존)으로 넘겨 안정화 게이트를 거치게 한다. 대기열에 다 못 넣으면 `SubtreeOverflow`.
pub fn apply_path_settled(env: &ScanEnv, folder_id: i64, abs: &Path, source: &str) -> Result<PathAction, IngestError> {
    apply_path_with(env, folder_id, abs, Some(source))
}

fn apply_path_with(
    env: &ScanEnv,
    folder_id: i64,
    abs: &Path,
    defer_source: Option<&str>,
) -> Result<PathAction, IngestError> {
    let folder = match env.pool.read(|c| Ok(load_folder(c, folder_id))) {
        Ok(Ok(f)) => f,
        Ok(Err(IngestError::NotFound)) => return Ok(PathAction::Ignored),
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(e.into()),
    };
    if !folder.enabled || !folder.online {
        return Ok(PathAction::Ignored);
    }
    let root = folder.root_path();
    let Some(names) = rel_parts(&root, abs) else {
        return Ok(PathAction::Ignored);
    };
    let rules = env.rules_for(&folder);
    if names.iter().any(|n| is_hidden_name(n))
        || rules.is_under_excluded(abs)
        || names.iter().any(|n| is_reserved_dir_name(n))
    {
        return Ok(PathAction::Ignored);
    }
    let rel = names.join("/");
    let now = env.clock.now_ms();
    let never = AtomicBool::new(false);
    let no_stop = || false;
    let record = |code: &str, message: &str| -> Result<PathAction, IngestError> {
        let (code, message, rel) = (code.to_string(), message.to_string(), rel.clone());
        env.write(Priority::Low, move |c| {
            record_error(c, folder_id, &rel, &code, &message, now)
        })?;
        Ok(PathAction::Failed)
    };
    // 응답 없음 — 이번에는 건너뛴다. 오류로 남기고 `Failed` 를 돌려 호출자가 나중에 다시 보게 한다 (missing 으로 만들지 않는다)
    let unresponsive = || record(UNRESPONSIVE_CODE, UNRESPONSIVE_MESSAGE);

    // 스캔(`list_dir`)이 건너뛰는 숨김·시스템 폴더 아래의 감시 이벤트도 무시한다 — 조상 속성 조회도 제한 시간을 건다
    match env.hidden_ancestor_guarded(&folder, &names) {
        Guarded::Done(true) => return Ok(PathAction::Ignored),
        Guarded::Done(false) => {}
        Guarded::TimedOut => return unresponsive(),
    }

    // 루트 자체를 읽을 수 없으면 볼륨이 빠진 것일 수 있다 — 아무것도 바꾸지 않는다
    match env.root_reachable_guarded(&folder) {
        Guarded::Done(Ok(())) => {}
        Guarded::Done(Err(_)) => return Ok(PathAction::Ignored),
        Guarded::TimedOut => return unresponsive(),
    }

    let kind = match env.entry_kind_guarded(&folder, abs) {
        Guarded::Done(Ok(k)) => k,
        Guarded::Done(Err(f)) if f.not_found => {
            return gone_if_root_reachable(env, &folder, &names, now, &unresponsive, &record);
        }
        Guarded::Done(Err(f)) => return record(f.code, &f.message),
        Guarded::TimedOut => return unresponsive(),
    };
    if kind == EntryKind::Symlink {
        return Ok(PathAction::Ignored);
    }
    let parent = &names[..names.len() - 1];
    if kind == EntryKind::File {
        let media = match env.classify_guarded(&folder, abs) {
            Guarded::Done(Ok(Some(m))) => m,
            Guarded::Done(Ok(None)) => return Ok(PathAction::Ignored),
            Guarded::Done(Err(f)) => return record(f.code, &f.message),
            Guarded::TimedOut => return unresponsive(),
        };
        if !folder.media_types.contains(media) {
            // 대상 유형 밖 — 등록·변경 반영 없이, 이미 있는 위치만 "봤다" 로 표시한다 (되살아난 파일은 present 로)
            let vol_rel = if folder.rel_root.is_empty() {
                rel.replace('/', "\\")
            } else {
                format!("{}\\{}", folder.rel_root, rel.replace('/', "\\"))
            };
            let mut walk = single_walk(env, &folder, now, &never, &no_stop)?;
            walk.apply_once(Vec::new(), Vec::new(), vec![vol_rel])?;
            return Ok(PathAction::Ignored);
        }
        let file_meta = match env.stat_guarded(&folder, abs) {
            Guarded::Done(Ok(m)) => m,
            Guarded::Done(Err(f)) if f.not_found => {
                return gone_if_root_reachable(env, &folder, &names, now, &unresponsive, &record);
            }
            Guarded::Done(Err(f)) => return record(f.code, &f.message),
            Guarded::TimedOut => return unresponsive(),
        };
        let vol_rel = if folder.rel_root.is_empty() {
            rel.replace('/', "\\")
        } else {
            format!("{}\\{}", folder.rel_root, rel.replace('/', "\\"))
        };
        let work = Work {
            rel_vol: vol_rel,
            rel_root: rel.clone(),
            abs: abs.to_path_buf(),
            media,
            meta: file_meta,
        };
        let mut walk = single_walk(env, &folder, now, &never, &no_stop)?;
        walk.apply_once(vec![work], dir_chain(parent), Vec::new())?;
        return Ok(PathAction::Reconciled);
    }
    if kind == EntryKind::Dir {
        let mut walk = single_walk(env, &folder, now, &never, &no_stop)?;
        walk.apply_once(Vec::new(), dir_chain(parent), Vec::new())?;
        walk.defer_source = defer_source.map(str::to_owned);
        let name = names.last().cloned().unwrap_or_default();
        let task = Task {
            abs: abs.to_path_buf(),
            key: key_of(&names),
            rel: join_rel(&parent.join("/"), &name),
            name,
            parent_key: key_of(parent),
        };
        if matches!(walk.walk(task, None)?, WalkEnd::Unresponsive) {
            return unresponsive();
        }
        if walk.deferred_overflow {
            return Ok(PathAction::SubtreeOverflow);
        }
        return Ok(PathAction::Subtree);
    }
    Ok(PathAction::Ignored)
}

/// 사라진 경로: 하위 위치를 missing 으로, 트리 행을 삭제한다.
fn mark_gone(
    env: &ScanEnv,
    folder: &FolderRow,
    names: &[String],
    now: i64,
) -> Result<PathAction, IngestError> {
    let rel_key = key_of(names);
    let vol_key = join_key(&folder.path_key, &rel_key);
    let (folder_id, volume_id) = (folder.folder_id, folder.volume_id);
    let clock = Arc::clone(&env.clock);
    let n = env.write(Priority::Low, move |c| {
        let reconciler = Reconciler {
            clock: &*clock,
            hasher: &crate::hash::Blake3Hasher,
            cancel: &AtomicBool::new(false),
        };
        let n = reconciler.mark_missing_under(c, volume_id, &vol_key)?;
        let (lo, hi) = key_range(&rel_key);
        c.execute(
            "DELETE FROM folder_dirs WHERE folder_id = ?1
             AND (dir_key = ?2 OR (dir_key >= ?3 AND dir_key < ?4))",
            params![folder_id, rel_key, lo, hi],
        )?;
        let _ = now;
        Ok(n)
    })?;
    Ok(PathAction::Missing(n))
}

/// 경로가 없다고 나온 뒤 `mark_gone` 직전에 루트가 아직 닿는지 다시 확인한다 — 앞선 루트 확인과 이 조회 사이에 공유가 끊기면
/// 경로 조회가 NotFound 로 보일 수 있다 (연결 끊김은 삭제가 아니다). 루트에 닿지 않거나 응답이 없으면 아무것도 바꾸지 않고
/// `Failed` 로 돌려 호출자가 나중에 다시 보게 한다.
fn gone_if_root_reachable(
    env: &ScanEnv,
    folder: &FolderRow,
    names: &[String],
    now: i64,
    unresponsive: &dyn Fn() -> Result<PathAction, IngestError>,
    record: &dyn Fn(&str, &str) -> Result<PathAction, IngestError>,
) -> Result<PathAction, IngestError> {
    match env.root_reachable_guarded(folder) {
        Guarded::Done(Ok(())) => mark_gone(env, folder, names, now),
        Guarded::Done(Err(f)) => record(f.code, &f.message),
        Guarded::TimedOut => unresponsive(),
    }
}
