//! API 키 보관 (PL-A1). session = 프로세스 메모리(zeroize), persist = DPAPI blob(base64) 을 호출자가 settings 에 저장.
//! 키 값은 `with_key` 로 빌려줄 뿐 반환하지 않고, `Debug`/오류에 나타나지 않는다.

mod dpapi;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, MutexGuard};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::external::Provider;

/// 영속 blob 형식 식별 접두어 (DPAPI 출력의 base64).
const BLOB_PREFIX: &str = "dpapi1:";
const MAX_KEY_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("invalid key: {0}")]
    Invalid(&'static str),
    #[error("key protection failed: {0}")]
    Crypto(&'static str),
    #[error("persisted key blob is corrupted")]
    BadBlob,
    #[error("key persistence is not supported on this platform")]
    Unsupported,
}

/// UI `KeyStorage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyStorage {
    Persisted,
    Session,
    None,
}

struct Entry {
    secret: Zeroizing<String>,
    /// persist 로 저장한 경우의 blob (복호 불가능한 암호문이라 평문 키 아님).
    blob: Option<String>,
}

#[derive(Default)]
pub struct KeyVault {
    entries: Mutex<HashMap<Provider, Entry>>,
}

impl fmt::Debug for KeyVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let g = self.lock();
        let mut list: Vec<(&str, KeyStorage)> =
            g.iter().map(|(p, e)| (p.as_str(), storage_of(e))).collect();
        list.sort_by_key(|(p, _)| *p);
        f.debug_struct("KeyVault")
            .field("providers", &list)
            .finish_non_exhaustive()
    }
}

fn storage_of(e: &Entry) -> KeyStorage {
    if e.blob.is_some() {
        KeyStorage::Persisted
    } else {
        KeyStorage::Session
    }
}

impl KeyVault {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Provider, Entry>> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 키를 설정한다. `persist` 면 DPAPI 로 보호한 blob 도 만든다 — 보호에 실패하면 키를 저장하지 않고 오류를 돌려준다.
    pub fn set(
        &self,
        provider: Provider,
        key: &str,
        persist: bool,
    ) -> Result<KeyStorage, KeyError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(KeyError::Invalid("empty key"));
        }
        if key.len() > MAX_KEY_BYTES || key.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(KeyError::Invalid("malformed key"));
        }
        let blob = if persist {
            let cipher = dpapi::protect(key.as_bytes())?;
            Some(format!("{BLOB_PREFIX}{}", B64.encode(cipher)))
        } else {
            None
        };
        let entry = Entry {
            secret: Zeroizing::new(key.to_string()),
            blob,
        };
        let storage = storage_of(&entry);
        self.lock().insert(provider, entry);
        Ok(storage)
    }

    pub fn clear(&self, provider: Provider) {
        self.lock().remove(&provider);
    }

    pub fn storage(&self, provider: Provider) -> KeyStorage {
        self.lock()
            .get(&provider)
            .map_or(KeyStorage::None, storage_of)
    }

    /// 키를 빌려 쓴다 (반환 불가). 키가 없으면 `None`. `f` 안에서 다시 이 vault 를 호출하지 말 것.
    pub fn with_key<R>(&self, provider: Provider, f: impl FnOnce(&str) -> R) -> Option<R> {
        self.lock().get(&provider).map(|e| f(e.secret.as_str()))
    }

    /// settings 에 저장할 persist blob (base64 문자열). persist 가 아니면 `None`.
    pub fn export_persisted(&self, provider: Provider) -> Option<String> {
        self.lock().get(&provider).and_then(|e| e.blob.clone())
    }

    /// 저장된 blob 들을 복호해 session 메모리에 올린다. 공급자별로 결과를 돌려주며 한 건의 실패가 다른 건을 막지 않는다.
    pub fn load_persisted<'a>(
        &self,
        blobs: impl IntoIterator<Item = (Provider, &'a str)>,
    ) -> Vec<(Provider, Result<(), KeyError>)> {
        blobs
            .into_iter()
            .map(|(p, blob)| (p, self.load_one(p, blob)))
            .collect()
    }

    fn load_one(&self, provider: Provider, blob: &str) -> Result<(), KeyError> {
        let b64 = blob.strip_prefix(BLOB_PREFIX).ok_or(KeyError::BadBlob)?;
        let cipher = B64.decode(b64).map_err(|_| KeyError::BadBlob)?;
        let plain = dpapi::unprotect(&cipher)?;
        let secret = std::str::from_utf8(&plain).map_err(|_| KeyError::BadBlob)?;
        self.lock().insert(
            provider,
            Entry {
                secret: Zeroizing::new(secret.to_string()),
                blob: Some(blob.to_string()),
            },
        );
        Ok(())
    }
}
