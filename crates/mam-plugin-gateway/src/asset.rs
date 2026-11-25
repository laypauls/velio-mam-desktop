//! `asset.get`(`asset.read.metadata`)·`asset.getDerived`(`asset.read.derived`).
//! 범위 검사를 DB 접근보다 먼저 한다. 경로·볼륨·해시·사용자 작성 필드는 돌려주지 않고, 원본 파일은 열지 않는다.

use std::io::Read;
use std::sync::atomic::Ordering;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mam_core::assets::{AssetDetail, detail};
use mam_media::thumbnail::{ThumbSize, thumbnail_path};
use mam_plugin_host::CallCx;
use serde_json::{Value, json};

use crate::errors::GwError;
use crate::gateway::Gateway;
use crate::params::{AssetParams, DerivedParams, asset_id, parse};

impl Gateway {
    fn load_detail(&self, cx: &CallCx<'_>, id: i64) -> Result<AssetDetail, GwError> {
        self.pool
            .read_until(self.read_deadline(cx), move |c| Ok(detail(c, id)?))?
            .ok_or(GwError::NotFound)
    }

    pub(crate) fn asset_get(&self, cx: &CallCx<'_>, params: Value) -> Result<Value, GwError> {
        let id = asset_id(&parse::<AssetParams>(params)?.asset_id)?;
        Self::in_scope(cx, id)?;
        let d = self.load_detail(cx, id)?;
        Ok(json!({
            "assetId": id,
            "mediaType": d.media_type,
            "status": d.status,
            "availability": d.availability,
            "presence": d.presence,
            "fileName": d.file_name,
            "capturedAt": d.captured_at,
            "durationMs": d.duration_ms,
            "width": d.width,
            "height": d.height,
        }))
    }

    pub(crate) fn asset_get_derived(
        &self,
        cx: &CallCx<'_>,
        params: Value,
    ) -> Result<Value, GwError> {
        let p = parse::<DerivedParams>(params)?;
        let id = asset_id(&p.asset_id)?;
        if p.kind != "thumbnail" {
            return Err(GwError::BadRequest("invalid derived kind"));
        }
        let (size, size_name) = match p.size.as_deref() {
            None | Some("small") => (ThumbSize::Small, "small"),
            Some("large") => (ThumbSize::Large, "large"),
            Some(_) => return Err(GwError::BadRequest("invalid params")),
        };
        Self::in_scope(cx, id)?;
        let d = self.load_detail(cx, id)?;
        if d.status != "active" {
            return Err(GwError::NotFound);
        }
        if d.media_type == "audio" {
            return Err(GwError::Unavailable); // 음성은 썸네일이 없다
        }
        // 썸네일은 MAM 캐시의 로컬 파일이다 — 원본은 열지 않고, 오프라인 자산도 돌려준다. 경로는 정수 id 로만 만든다(탈출 없음)
        let path = thumbnail_path(&self.cfg.cache_root, id, size);
        let len = match std::fs::metadata(&path) {
            Ok(m) if m.is_file() => m.len(),
            Ok(_) => return Err(GwError::Unavailable),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(GwError::Unavailable),
            Err(e) => return Err(GwError::internal(e)),
        };
        if len > self.cfg.derived_max_bytes {
            return Err(GwError::TooLarge("derived data too large"));
        }
        let used = cx.scope.derived_bytes.load(Ordering::Acquire);
        if used
            .checked_add(len)
            .is_none_or(|n| n > self.cfg.derived_budget_bytes)
        {
            return Err(GwError::Quota("derived data budget exceeded"));
        }
        let mut bytes = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(GwError::Unavailable),
            Err(e) => return Err(GwError::internal(e)),
        };
        // 검사 뒤 파일이 커졌어도 상한을 넘겨 읽지 않는다
        file.take(self.cfg.derived_max_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(GwError::internal)?;
        if bytes.len() as u64 > self.cfg.derived_max_bytes {
            return Err(GwError::TooLarge("derived data too large"));
        }
        cx.scope
            .derived_bytes
            .fetch_add(bytes.len() as u64, Ordering::AcqRel);
        Ok(json!({
            "kind": "thumbnail",
            "size": size_name,
            "mime": "image/webp",
            "byteLength": bytes.len(),
            "dataBase64": STANDARD.encode(&bytes),
        }))
    }
}
