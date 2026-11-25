//! `contributes`·`activationEvents` 타입, 검증, 상호 참조.

use std::collections::HashSet;

use crate::error::{ManifestError, required};
use crate::ids::{CommandId, ProviderId, TitleKey};
use crate::limits::{
    MAX_ACTIVATION_EVENTS, MAX_COMMANDS, MAX_MENU_ITEMS, MAX_PROVIDERS_PER_KIND,
    MAX_PROVIDERS_TOTAL,
};
use crate::raw::{RawContributes, RawMenus, RawProviders};
use crate::when::WhenExpr;

pub use crate::contrib_types::*;

fn too_many(path: &str, max: usize) -> ManifestError {
    ManifestError::TooMany {
        path: path.to_owned(),
        max,
    }
}

pub(crate) fn build_contributes(raw: Option<RawContributes>) -> Result<Contributes, ManifestError> {
    let raw = raw.unwrap_or_default();
    let commands = build_commands(raw.commands.unwrap_or_default())?;
    let menus = build_menus(raw.menus, &commands)?;
    let providers = build_providers(raw.providers)?;
    Ok(Contributes {
        commands,
        menus,
        providers,
    })
}

fn build_commands(raw: Vec<crate::raw::RawCommand>) -> Result<Vec<Command>, ManifestError> {
    if raw.len() > MAX_COMMANDS {
        return Err(too_many("contributes.commands", MAX_COMMANDS));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for (i, c) in raw.into_iter().enumerate() {
        let base = format!("contributes.commands[{i}]");
        let id_path = format!("{base}.id");
        let id = CommandId::parse(&required(c.id, &id_path)?, &id_path)?;
        if !seen.insert(id.as_str().to_owned()) {
            return Err(ManifestError::DuplicateId {
                path: id_path,
                id: id.to_string(),
            });
        }
        let title_path = format!("{base}.title");
        let title = TitleKey::parse(&required(c.title, &title_path)?, &title_path)?;
        out.push(Command { id, title });
    }
    Ok(out)
}

fn build_menus(raw: Option<RawMenus>, commands: &[Command]) -> Result<Menus, ManifestError> {
    let items = raw.and_then(|m| m.asset_context).unwrap_or_default();
    if items.len() > MAX_MENU_ITEMS {
        return Err(too_many("contributes.menus.assetContext", MAX_MENU_ITEMS));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        let base = format!("contributes.menus.assetContext[{i}]");
        let cmd_path = format!("{base}.command");
        let command = CommandId::parse(&required(item.command, &cmd_path)?, &cmd_path)?;
        if !commands.iter().any(|c| c.id == command) {
            return Err(ManifestError::UnknownCommandRef {
                path: cmd_path,
                command: command.to_string(),
            });
        }
        if !seen.insert(command.as_str().to_owned()) {
            return Err(ManifestError::DuplicateId {
                path: cmd_path,
                id: command.to_string(),
            });
        }
        let when = item
            .when
            .map(|src| {
                WhenExpr::parse(&src).map_err(|error| ManifestError::InvalidWhen {
                    path: format!("{base}.when"),
                    error,
                })
            })
            .transpose()?;
        out.push(MenuItem { command, when });
    }
    Ok(Menus { asset_context: out })
}

type Common = (ProviderId, TitleKey, Vec<AssetKind>);

fn build_common(
    id: Option<String>,
    title: Option<String>,
    kinds: Option<Vec<String>>,
    base: &str,
    seen: &mut HashSet<String>,
) -> Result<Common, ManifestError> {
    let id_path = format!("{base}.id");
    let id = ProviderId::parse(&required(id, &id_path)?, &id_path)?;
    if !seen.insert(id.as_str().to_owned()) {
        return Err(ManifestError::DuplicateId {
            path: id_path,
            id: id.to_string(),
        });
    }
    let title_path = format!("{base}.title");
    let title = TitleKey::parse(&required(title, &title_path)?, &title_path)?;
    let mut asset_kinds = Vec::new();
    for (j, k) in kinds.unwrap_or_default().iter().enumerate() {
        let invalid = || ManifestError::InvalidAssetKind {
            path: format!("{base}.assetKinds[{j}]"),
            value: k.clone(),
        };
        let kind = AssetKind::parse(k).ok_or_else(invalid)?;
        if asset_kinds.contains(&kind) {
            return Err(invalid());
        }
        asset_kinds.push(kind);
    }
    Ok((id, title, asset_kinds))
}

/// 내보내기 확장자 허용 목록 — 데이터 형식만 (실행·스크립트·바로가기 형식, html/svg 제외)
pub const EXPORT_EXTENSIONS: &[&str] = &["csv", "tsv", "json", "jsonl", "txt", "md", "xml", "xmp", "srt", "vtt", "yaml", "yml"];

/// 소문자 정확 일치만 허용한다
pub fn export_extension_allowed(s: &str) -> bool {
    EXPORT_EXTENSIONS.contains(&s)
}

fn extension_ok(s: &str) -> bool {
    export_extension_allowed(s)
}

fn build_providers(raw: Option<RawProviders>) -> Result<Providers, ManifestError> {
    let raw = raw.unwrap_or(RawProviders {
        metadata: None,
        analysis: None,
        export: None,
    });
    let (metadata, analysis, export) = (
        raw.metadata.unwrap_or_default(),
        raw.analysis.unwrap_or_default(),
        raw.export.unwrap_or_default(),
    );
    for (name, len) in [
        ("metadata", metadata.len()),
        ("analysis", analysis.len()),
        ("export", export.len()),
    ] {
        if len > MAX_PROVIDERS_PER_KIND {
            return Err(too_many(
                &format!("contributes.providers.{name}"),
                MAX_PROVIDERS_PER_KIND,
            ));
        }
    }
    if metadata.len() + analysis.len() + export.len() > MAX_PROVIDERS_TOTAL {
        return Err(too_many("contributes.providers", MAX_PROVIDERS_TOTAL));
    }
    let mut seen = HashSet::new();
    let mut plain = |name: &str, list: Vec<crate::raw::RawProvider>| {
        list.into_iter()
            .enumerate()
            .map(|(i, p)| {
                let base = format!("contributes.providers.{name}[{i}]");
                let (id, title, asset_kinds) =
                    build_common(p.id, p.title, p.asset_kinds, &base, &mut seen)?;
                Ok(Provider {
                    id,
                    title,
                    asset_kinds,
                })
            })
            .collect::<Result<Vec<_>, ManifestError>>()
    };
    let metadata = plain("metadata", metadata)?;
    let analysis = plain("analysis", analysis)?;
    let mut exports = Vec::with_capacity(export.len());
    for (i, p) in export.into_iter().enumerate() {
        let base = format!("contributes.providers.export[{i}]");
        let (id, title, asset_kinds) =
            build_common(p.id, p.title, p.asset_kinds, &base, &mut seen)?;
        let ext_path = format!("{base}.fileExtension");
        let file_extension = required(p.file_extension, &ext_path)?;
        if !extension_ok(&file_extension) {
            return Err(ManifestError::InvalidFileExtension {
                path: ext_path,
                value: file_extension,
            });
        }
        exports.push(ExportProvider {
            id,
            title,
            asset_kinds,
            file_extension,
        });
    }
    Ok(Providers {
        metadata,
        analysis,
        export: exports,
    })
}

pub(crate) fn build_activation(
    raw: Option<Vec<String>>,
    c: &Contributes,
) -> Result<Vec<ActivationEvent>, ManifestError> {
    let raw = raw.unwrap_or_default();
    if raw.len() > MAX_ACTIVATION_EVENTS {
        return Err(too_many("activationEvents", MAX_ACTIVATION_EVENTS));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(raw.len());
    for (i, ev) in raw.into_iter().enumerate() {
        let path = format!("activationEvents[{i}]");
        let invalid = || ManifestError::InvalidActivationEvent { value: ev.clone() };
        let parsed = if let Some(id) = ev.strip_prefix("onCommand:") {
            let id = CommandId::parse(id, &path).map_err(|_| invalid())?;
            if !c.commands.iter().any(|x| x.id == id) {
                return Err(ManifestError::UnknownCommandRef {
                    path,
                    command: id.to_string(),
                });
            }
            ActivationEvent::OnCommand(id)
        } else if let Some(id) = ev.strip_prefix("onProvider:") {
            let id = ProviderId::parse(id, &path).map_err(|_| invalid())?;
            let p = &c.providers;
            let declared = p.metadata.iter().chain(&p.analysis).any(|x| x.id == id)
                || p.export.iter().any(|x| x.id == id);
            if !declared {
                return Err(ManifestError::UnknownProviderRef {
                    path,
                    provider: id.to_string(),
                });
            }
            ActivationEvent::OnProvider(id)
        } else {
            return Err(invalid());
        };
        if !seen.insert(ev.clone()) {
            return Err(ManifestError::DuplicateId { path, id: ev });
        }
        out.push(parsed);
    }
    Ok(out)
}
