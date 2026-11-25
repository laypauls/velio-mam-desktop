//! AI 설정 문서의 SQLite 저장소 + 메모리 캐시. 작업 코어는 `snapshot()` 을 자주 부르므로 캐시에서 읽는다.
//!
//! 키 암호문(`dpapi1:...`)은 `ai_key_<provider>` 행에 둔다 — 평문 키가 아니다 (PA1).

use std::sync::{Arc, Mutex, RwLock};

use mam_ai::external::Provider;
use mam_core::SharedClock;
use mam_db::{DbPool, Priority};
use rusqlite::params;

use super::AiFailure;
use super::settings::{AiSettings, StoredAi};

const DOC_KEY: &str = "ai_settings";
const KEY_PREFIX: &str = "ai_key_";

pub struct AiStore {
    pool: Arc<DbPool>,
    clock: SharedClock,
    doc: RwLock<StoredAi>,
    /// 문서 쓰기를 직렬화한다 (읽기-수정-쓰기)
    write: Mutex<()>,
}

fn key_row(p: Provider) -> String {
    format!("{KEY_PREFIX}{}", p.as_str())
}

impl AiStore {
    /// 저장된 문서를 읽는다. 손상된 문서는 기본값으로 대체한다(경고만, 내용은 싣지 않는다).
    pub fn load(pool: Arc<DbPool>, clock: SharedClock) -> Result<Arc<Self>, AiFailure> {
        let raw: Option<String> = pool.read(|c| {
            Ok(c.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                [DOC_KEY],
                |r| r.get(0),
            )
            .ok())
        })?;
        let doc = match raw.map(|s| serde_json::from_str::<StoredAi>(&s)) {
            Some(Ok(d)) => d,
            Some(Err(_)) => {
                tracing::warn!("stored AI settings are unreadable; using defaults");
                StoredAi::default()
            }
            None => StoredAi::default(),
        };
        Ok(Arc::new(Self {
            pool,
            clock,
            doc: RwLock::new(doc),
            write: Mutex::new(()),
        }))
    }

    pub fn now(&self) -> i64 {
        self.clock.now_ms()
    }

    pub fn settings(&self) -> AiSettings {
        self.doc
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .settings
            .clone()
    }

    pub fn stored(&self) -> StoredAi {
        self.doc.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// 문서를 수정해 저장한다. 저장에 성공한 뒤에만 캐시를 바꾼다. 수정 결과와 새 문서를 돌려준다.
    pub fn update<R>(
        &self,
        f: impl FnOnce(&mut StoredAi) -> Result<R, AiFailure>,
    ) -> Result<(R, StoredAi), AiFailure> {
        let _serial = self.write.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = self.stored();
        let out = f(&mut next)?;
        let json = serde_json::to_string(&next).map_err(|e| AiFailure::internal(&e))?;
        self.pool.write(Priority::High, move |c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![DOC_KEY, json],
            )?;
            Ok(())
        })?;
        *self.doc.write().unwrap_or_else(|e| e.into_inner()) = next.clone();
        Ok((out, next))
    }

    /// 키 암호문을 저장하거나(`Some`) 지운다(`None`).
    pub fn set_key_blob(&self, p: Provider, blob: Option<String>) -> Result<(), AiFailure> {
        let row = key_row(p);
        self.pool.write(Priority::High, move |c| {
            match blob {
                Some(b) => c.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![row, serde_json::Value::String(b).to_string()],
                )?,
                None => c.execute("DELETE FROM settings WHERE key = ?1", [row])?,
            };
            Ok(())
        })?;
        Ok(())
    }

    /// 저장된 키 암호문 전부 (시작 시 한 번).
    pub fn key_blobs(&self) -> Result<Vec<(Provider, String)>, AiFailure> {
        let rows: Vec<(String, String)> = self.pool.read(|c| {
            let mut stmt = c.prepare(
                "SELECT key, value FROM settings WHERE key LIKE 'ai\\_key\\_%' ESCAPE '\\'",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })?;
        Ok(Provider::ALL
            .into_iter()
            .filter_map(|p| {
                let row = key_row(p);
                let (_, v) = rows.iter().find(|(k, _)| *k == row)?;
                let blob = serde_json::from_str::<String>(v).ok()?;
                Some((p, blob))
            })
            .collect())
    }
}
