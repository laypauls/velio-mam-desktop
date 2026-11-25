//! 복원·복구 후처리 — 현재 설정 덮어쓰기(PR1), 동의 규칙, 시퀀스 상한(H1), 파생 인덱스 재구축 요청(H3·H4).
//! 모두 멱등이다 (표지 적용이 중단돼 다시 실행돼도 같은 결과).

use std::collections::BTreeMap;
use std::path::Path;

use mam_core::storage::DataPaths;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

use super::{ConsentState, KeptState};

const PASSWORD_KEY: &str = "password_hash";
const EXEMPT_KEY: &str = "auth_exempt_minutes";
const AI_DOC_KEY: &str = "ai_settings";

fn raw_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

fn put_setting(conn: &Connection, key: &str, value: &Option<String>) -> rusqlite::Result<()> {
    match value {
        Some(v) => conn
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, v],
            )
            .map(drop),
        None => conn
            .execute("DELETE FROM settings WHERE key = ?1", [key])
            .map(drop),
    }
}

fn consent_in(doc: &Value) -> Option<bool> {
    doc.pointer("/settings/consent/given")
        .and_then(Value::as_bool)
}

/// 복원 요청 시점의 현재 값을 읽는다 (보호 설정·동의·시퀀스).
pub fn read_kept(conn: &Connection) -> rusqlite::Result<KeptState> {
    let consent = raw_setting(conn, AI_DOC_KEY)?
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .map(|doc| ConsentState {
            given: consent_in(&doc).unwrap_or(false),
            at: doc.pointer("/settings/consent/at").and_then(Value::as_i64),
        })
        .unwrap_or(ConsentState {
            given: false,
            at: None,
        });
    let mut stmt = conn.prepare("SELECT name, seq FROM sqlite_sequence")?;
    let sequences = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<BTreeMap<_, _>>>()?;
    Ok(KeptState {
        password_hash: raw_setting(conn, PASSWORD_KEY)?,
        auth_exempt_minutes: raw_setting(conn, EXEMPT_KEY)?,
        consent,
        sequences,
        ai_keys: read_ai_keys(conn)?,
        plugins: super::plugins::read_plugins(conn)?,
    })
}

/// 외부 AI 동의의 결과 값 = 현재 && 백업 (철회 쪽을 따른다). 백업 문서가 없으면 아무것도 하지 않는다(기본값 = 미동의).
fn merge_consent(conn: &Connection, current: ConsentState) -> rusqlite::Result<()> {
    let Some(raw) = raw_setting(conn, AI_DOC_KEY)? else {
        return Ok(());
    };
    let Ok(mut doc) = serde_json::from_str::<Value>(&raw) else {
        return Ok(()); // 읽을 수 없는 문서는 앱이 기본값(미동의)으로 대체한다
    };
    if consent_in(&doc) == Some(true)
        && !current.given
        && let Some(slot) = doc.pointer_mut("/settings/consent")
    {
        *slot = json!({"given": false, "at": current.at});
        conn.execute(
            "UPDATE settings SET value = ?2 WHERE key = ?1",
            params![AI_DOC_KEY, doc.to_string()],
        )?;
    }
    Ok(())
}

/// 동의를 철회 상태로 되돌린다 (자동 복구 — 현재 값을 읽을 수 없어 재동의를 요구한다). 동의 상태였으면 `true`.
pub fn withdraw_consent(conn: &Connection) -> rusqlite::Result<bool> {
    let was_given = raw_setting(conn, AI_DOC_KEY)?
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|d| consent_in(&d))
        == Some(true);
    if was_given {
        merge_consent(
            conn,
            ConsentState {
                given: false,
                at: None,
            },
        )?;
    }
    Ok(was_given)
}

/// `sqlite_sequence` 를 `floors` 이상으로 올린다 (낮추지 않는다) — 삭제·복원 전에 발급된 ID 의 재사용을 막는다.
pub fn raise_sequences(conn: &Connection, floors: &BTreeMap<String, i64>) -> rusqlite::Result<()> {
    for (name, floor) in floors {
        let updated = conn.execute(
            "UPDATE sqlite_sequence SET seq = ?1 WHERE name = ?2 AND seq < ?1",
            params![floor, name],
        )?;
        if updated == 0 {
            let exists: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = ?1)",
                [name],
                |r| r.get(0),
            )?;
            if !exists {
                conn.execute(
                    "INSERT INTO sqlite_sequence (name, seq) VALUES (?1, ?2)",
                    params![name, floor],
                )?;
            }
        }
    }
    Ok(())
}

/// 사용자 복원의 후처리: 복원된 DB 위에 현재 값을 덮어쓰고(한 트랜잭션) 파생 인덱스 재구축을 요청한다.
pub fn finish_user_restore(paths: &DataPaths, kept: &KeptState) -> Result<(), String> {
    let conn = mam_db::open(&paths.db()).map_err(|e| e.to_string())?;
    let apply = || -> rusqlite::Result<()> {
        let tx = conn.unchecked_transaction()?;
        put_setting(&tx, PASSWORD_KEY, &kept.password_hash)?;
        put_setting(&tx, EXEMPT_KEY, &kept.auth_exempt_minutes)?;
        replace_ai_keys(&tx, &kept.ai_keys)?;
        merge_consent(&tx, kept.consent)?;
        super::plugins::merge_plugins(&tx, &kept.plugins)?;
        raise_sequences(&tx, &kept.sequences)?;
        tx.commit()?;
        // 설정 병합이 커밋된 뒤 — 배치마다 따로 커밋하며, 실패하면 표지를 남겨 다음 기동에서 이어서 지운다 (멱등)
        super::plugins::purge_uninstalled_data(&conn, &kept.plugins).map(drop)
    };
    apply().map_err(|e| e.to_string())?;
    conn.close().map_err(|(_, e)| e.to_string())?;
    request_derived_rebuild(paths).map_err(|e| e.to_string())
}

/// Tantivy·벡터 인덱스의 **전체 재구축을 명시적으로 요청**한다 (H3·H4). 문서 수 허용 오차(`needs_rebuild`)로는 내용이
/// 달라진 것을 못 잡는다. Tantivy: 인덱스 옆 표지(`mam_search::sync::request_rebuild`) — 다음 워커 열기에서 재구축하고
/// 끝난 뒤 지운다. 벡터: 모델별 활성 세대 포인터(`gen/ACTIVE`)를 지워 다음 `open_or_rebuild` 가 원시 임베딩(`raw.f16`)과
/// 복원된 DB 메타로 새 세대를 만들게 한다 (옛 세대 디렉터리는 그 재구축이 정리한다).
pub fn request_derived_rebuild(paths: &DataPaths) -> std::io::Result<()> {
    mam_search::sync::request_rebuild(&paths.tantivy())?;
    let vectors = paths.root().join("indexes").join("vectors");
    for kind in subdirs(&vectors) {
        for model in subdirs(&kind) {
            match std::fs::remove_file(model.join("gen").join("ACTIVE")) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
    }
    Ok(())
}

fn subdirs(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

/// 캐시(썸네일·프레임 스트립)에 남은 파일 이름에서 가장 큰 `asset_id` — 자동 복구(H1)가 `media_assets` 시퀀스 하한으로 쓴다.
/// 파일 이름은 `<asset_id>.webp`, `<asset_id>_strip.webp`.
pub fn max_cache_asset_id(cache_root: &Path) -> i64 {
    let mut max = 0i64;
    let mut stack = vec![cache_root.join("thumbnails"), cache_root.join("previews")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(id) = path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.split(['.', '_']).next())
                .and_then(|s| s.parse::<i64>().ok())
            {
                max = max.max(id);
            }
        }
    }
    max
}

/// 빈 DB 로 시작할 때: 옛 자산 ID 로 만든 파생 캐시는 모두 고아다 — 새 자산이 같은 ID 를 받으면 엉뚱한 썸네일이 붙으므로 지운다.
pub fn clear_asset_caches(paths: &DataPaths) -> std::io::Result<()> {
    for dir in [paths.thumbnails(), paths.previews()] {
        match std::fs::remove_dir_all(&dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        std::fs::create_dir_all(&dir)?;
    }
    Ok(())
}

const AI_KEY_LIKE: &str = r"ai\_key\_%";

/// `ai_key_<provider>` 행 원문(암호문 그대로). 로그에 남기지 않는다.
fn read_ai_keys(conn: &Connection) -> rusqlite::Result<BTreeMap<String, String>> {
    let mut stmt = conn.prepare(r"SELECT key, value FROM settings WHERE key LIKE ?1 ESCAPE '\'")?;
    stmt.query_map([AI_KEY_LIKE], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect()
}

/// 복원된 DB 의 키 행을 모두 지우고 현재 키 행으로 바꾼다 (백업 뒤에 지운 키는 되살아나지 않고, 있는 키는 유지된다).
fn replace_ai_keys(conn: &Connection, keys: &BTreeMap<String, String>) -> rusqlite::Result<()> {
    conn.execute(
        r"DELETE FROM settings WHERE key LIKE ?1 ESCAPE '\'",
        [AI_KEY_LIKE],
    )?;
    for (key, value) in keys {
        put_setting(conn, key, &Some(value.clone()))?;
    }
    Ok(())
}
