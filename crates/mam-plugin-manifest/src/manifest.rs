//! `Manifest` 타입과 `parse_manifest`.

use std::collections::BTreeSet;

use semver::{Version, VersionReq};
use serde::Serialize;
use serde_json::Value;

use crate::SUPPORTED_API_VERSIONS;
use crate::contributions::{ActivationEvent, Contributes, build_activation, build_contributes};
use crate::error::{ManifestError, required};
use crate::ids::{PluginId, TitleKey, check_len};
use crate::limits::{MAX_MANIFEST_BYTES, MAX_VERSION_LEN};
use crate::main_path::MainPath;
use crate::permission::Permission;
use crate::raw::RawManifest;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub api_version: u32,
    pub id: PluginId,
    pub version: Version,
    pub engines: Engines,
    pub main: MainPath,
    pub display_name: TitleKey,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<TitleKey>,
    pub permissions: BTreeSet<Permission>,
    pub activation_events: Vec<ActivationEvent>,
    pub contributes: Contributes,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Engines {
    pub mam: VersionReq,
}

fn json_err(e: serde_json::Error) -> ManifestError {
    ManifestError::Json {
        line: e.line(),
        column: e.column(),
        message: e.to_string(),
    }
}

/// 첫 오류를 반환한다. 순서: 크기 → BOM → JSON → apiVersion → 원시 구조(알 수 없는 필드) → 값 검증.
pub fn parse_manifest(json: &str) -> Result<Manifest, ManifestError> {
    if json.len() > MAX_MANIFEST_BYTES {
        return Err(ManifestError::TooLarge {
            len: json.len(),
            max: MAX_MANIFEST_BYTES,
        });
    }
    let s = json.strip_prefix('\u{feff}').unwrap_or(json);
    let value: Value = serde_json::from_str(s).map_err(json_err)?;
    let Some(obj) = value.as_object() else {
        return Err(ManifestError::Json {
            line: 1,
            column: 1,
            message: "root must be a JSON object".to_owned(),
        });
    };
    let api_version = match obj.get("apiVersion") {
        None | Some(Value::Null) => {
            return Err(ManifestError::MissingField {
                path: "apiVersion".to_owned(),
            });
        }
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| SUPPORTED_API_VERSIONS.contains(n))
            .ok_or_else(|| ManifestError::UnsupportedApiVersion {
                found: v.to_string(),
            })?,
    };
    let raw: RawManifest = serde_json::from_str(s).map_err(json_err)?;
    build(raw, api_version)
}

fn build(raw: RawManifest, api_version: u32) -> Result<Manifest, ManifestError> {
    let id = PluginId::parse(&required(raw.id, "id")?, "id")?;

    let version_str = required(raw.version, "version")?;
    check_len(&version_str, "version", MAX_VERSION_LEN)?;
    let version = Version::parse(&version_str).map_err(|_| ManifestError::InvalidVersion {
        value: version_str.clone(),
    })?;

    let mam = required(required(raw.engines, "engines")?.mam, "engines.mam")?;
    check_len(&mam, "engines.mam", MAX_VERSION_LEN)?;
    let mam = VersionReq::parse(&mam)
        .map_err(|_| ManifestError::InvalidVersionReq { value: mam.clone() })?;

    let main = MainPath::parse_manifest(&required(raw.main, "main")?)?;
    let display_name = TitleKey::parse(&required(raw.display_name, "displayName")?, "displayName")?;
    let description = raw
        .description
        .map(|d| TitleKey::parse(&d, "description"))
        .transpose()?;

    let mut permissions = BTreeSet::new();
    for s in raw.permissions.unwrap_or_default() {
        let p: Permission = s.parse()?;
        if !permissions.insert(p) {
            return Err(ManifestError::DuplicatePermission { value: s });
        }
    }

    let contributes = build_contributes(raw.contributes)?;
    let activation_events = build_activation(raw.activation_events, &contributes)?;
    if !contributes.providers.export.is_empty() && !permissions.contains(&Permission::ExportWrite)
    {
        return Err(ManifestError::MissingPermissionFor {
            path: "contributes.providers.export".to_owned(),
            permission: Permission::ExportWrite,
        });
    }

    Ok(Manifest {
        api_version,
        id,
        version,
        engines: Engines { mam },
        main,
        display_name,
        description,
        permissions,
        activation_events,
        contributes,
    })
}
