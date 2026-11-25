//! 내보내기 (B7) — 대기열을 거치지 않고 직접 호출한다. 검증은 모두 provider 호출 **전에** 끝낸다(겹침 거부 포함).
//!
//! 순서: 자산 수(1..=5,000, 중복 제거) -> provider 종류 `Export` + `export.write` -> 대상 경로(`export_fs`) -> 호출
//! `{ assetIds, kind: "export" }` -> 출력 `{text}` 또는 `{dataBase64}` 정확히 하나(디코드 ≤ 3 MiB) -> 겹침 재확인 + 임시 파일 원자 쓰기.

use std::io;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mam_db::PoolError;
use serde_json::{Value, json};

use super::core::Core;
use super::export_fs::{check_dest, write_atomic};
use crate::api::{ExportResult, Permission, ProviderKind, RunError};

/// 한 번에 내보낼 수 있는 자산 수 (워커 메시지 예산 10,000 의 절반 — 명세 E3)
pub const MAX_EXPORT_ASSETS: usize = 5_000;
/// 디코드한 출력 상한 — 워커 한 줄(4 MiB)에 base64 + JSON 봉투로 들어가야 한다. 정확히 3 MiB 는 base64 만 4 MiB 라 봉투(약 60 바이트)
/// 때문에 줄 한도를 넘어 `too_large` 로 실패했다(T-pjob-24 회귀) — 4 KiB 여유를 둔다
pub const MAX_EXPORT_BYTES: usize = 3 * 1024 * 1024 - 4 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// 호출자 입력 위반 — 고정 문구라 그대로 응답해도 된다 (경로를 싣지 않는다)
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
    #[error(transparent)]
    Run(#[from] RunError),
    /// provider 가 계약(`{text}` | `{dataBase64}`, ≤ 3 MiB)에 맞지 않는 값을 돌려줬다 -> `plugin_error`
    #[error("provider output invalid: {0}")]
    BadOutput(&'static str),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Db(#[from] PoolError),
    /// 관리 폴더 목록을 읽지 못했다 (겹침을 확인할 수 없으면 쓰지 않는다)
    #[error("cannot read managed folders: {0}")]
    Folders(String),
}

pub(super) fn export(
    core: &Core,
    plugin: &str,
    provider: &str,
    mut asset_ids: Vec<i64>,
    dest: &str,
) -> Result<ExportResult, ExportError> {
    // 정렬·중복 제거 전에 원본 길이를 먼저 막는다 (거대한 입력을 정렬하지 않는다)
    if asset_ids.len() > MAX_EXPORT_ASSETS.saturating_mul(4) {
        return Err(ExportError::InvalidInput(
            "assetIds must have 1..=5000 entries",
        ));
    }
    asset_ids.sort_unstable();
    asset_ids.dedup();
    if asset_ids.is_empty() || asset_ids.len() > MAX_EXPORT_ASSETS {
        return Err(ExportError::InvalidInput(
            "assetIds must have 1..=5000 entries",
        ));
    }
    let spec = core.invoker.provider(plugin, provider)?;
    if spec.kind != ProviderKind::Export {
        return Err(ExportError::InvalidInput(
            "provider is not an export provider",
        ));
    }
    if !core.invoker.allows(plugin, Permission::ExportWrite) {
        return Err(RunError::PermissionDenied(Permission::ExportWrite).into());
    }
    let ext = spec.file_extension.ok_or(ExportError::InvalidInput(
        "provider declares no file extension",
    ))?;
    let dest = check_dest(core, dest, &ext)?;
    let params = json!({ "assetIds": asset_ids, "kind": "export" });
    let out = core
        .invoker
        .invoke_provider(plugin, provider, asset_ids, params)?;
    let bytes = decode_output(out)?;
    write_atomic(core, &dest, &bytes)?;
    Ok(ExportResult {
        bytes_written: bytes.len() as u64,
    })
}

/// `{ text: string }` 또는 `{ dataBase64: string }` 정확히 하나 -> 바이트 (≤ `MAX_EXPORT_BYTES`)
pub(super) fn decode_output(out: Value) -> Result<Vec<u8>, ExportError> {
    let Value::Object(mut map) = out else {
        return Err(ExportError::BadOutput("output must be an object"));
    };
    if map.len() != 1 {
        return Err(ExportError::BadOutput(
            "output must have exactly one of text or dataBase64",
        ));
    }
    let bytes = if let Some(Value::String(text)) = map.remove("text") {
        text.into_bytes()
    } else if let Some(Value::String(b64)) = map.remove("dataBase64") {
        if b64.len() > MAX_EXPORT_BYTES.div_ceil(3) * 4 {
            return Err(ExportError::BadOutput("output too large"));
        }
        STANDARD
            .decode(b64.as_bytes())
            .map_err(|_| ExportError::BadOutput("dataBase64 is not valid base64"))?
    } else {
        return Err(ExportError::BadOutput(
            "output must have a string text or dataBase64",
        ));
    };
    if bytes.len() > MAX_EXPORT_BYTES {
        return Err(ExportError::BadOutput("output too large"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 분류: Happy / Boundary(3 MiB 경계) / Validation(둘 다·둘 다 아님·잘못된 base64)

    fn bad(v: Value) -> bool {
        matches!(decode_output(v), Err(ExportError::BadOutput(_)))
    }

    #[test]
    fn t_pjob_export_01_decode_text_and_base64() {
        assert_eq!(
            decode_output(json!({ "text": "héllo" })).unwrap(),
            "héllo".as_bytes()
        );
        assert_eq!(
            decode_output(json!({ "dataBase64": "AAEC/w==" })).unwrap(),
            vec![0, 1, 2, 255]
        );
        assert_eq!(
            decode_output(json!({ "text": "" })).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn t_pjob_export_02_rejects_wrong_shapes() {
        assert!(bad(json!(null)));
        assert!(bad(json!("text")));
        assert!(bad(json!({})));
        assert!(bad(json!({ "text": "a", "dataBase64": "AA==" })));
        assert!(bad(json!({ "text": 1 })));
        assert!(bad(json!({ "other": "a" })));
        assert!(bad(json!({ "dataBase64": "!!!" })));
    }

    #[test]
    fn t_pjob_export_03_size_boundary() {
        let ok = vec![b'a'; MAX_EXPORT_BYTES];
        assert_eq!(
            decode_output(json!({ "dataBase64": STANDARD.encode(&ok) }))
                .unwrap()
                .len(),
            MAX_EXPORT_BYTES
        );
        assert_eq!(
            decode_output(json!({ "text": "a".repeat(MAX_EXPORT_BYTES) }))
                .unwrap()
                .len(),
            MAX_EXPORT_BYTES
        );
        let over = vec![b'a'; MAX_EXPORT_BYTES + 1];
        assert!(bad(json!({ "dataBase64": STANDARD.encode(&over) })));
        assert!(bad(json!({ "text": "a".repeat(MAX_EXPORT_BYTES + 1) })));
    }
}
