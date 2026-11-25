//! 부팅 — 설정의 `plugin_installed:*` 행마다 `plugins/<id>/<version>/` 를 읽어 항목을 만든다. 절대 실패하지 않는다:
//! 패키지가 없거나 손상됐으면 Broken, 앱 버전이 안 맞으면 Incompatible 로 목록에 남긴다. 그 뒤 크래시 잔여물을 정리한다.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use mam_db::plugin_settings::{ENABLED, GRANTED, INSTALLED, load_prefixed, parse_enabled};
use semver::Version;

use super::fsx::remove_confined;
use super::package::read_package;
use super::stage::{STAGING_DIR, TRASH_DIR};
use super::store::{known_perms, parse_installed};
use crate::api::PluginId;
use crate::runtime::{Entry, PluginRuntime};

const BOOT_DB_WAIT: Duration = Duration::from_secs(30);

pub(crate) fn load_all(rt: &PluginRuntime) {
    let rows = rt.deps.pool.read_until(Instant::now() + BOOT_DB_WAIT, |c| {
        Ok((
            load_prefixed(c, INSTALLED)?,
            load_prefixed(c, ENABLED)?,
            load_prefixed(c, GRANTED)?,
        ))
    });
    let (installed, enabled, granted) = match rows {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "plugin settings unreadable at boot; no plugins loaded");
            return;
        }
    };
    let mut current: BTreeMap<String, Option<String>> = BTreeMap::new();
    for (id, raw) in &installed {
        let entry = load_one(
            rt,
            id,
            raw,
            enabled.get(id).map(String::as_str),
            granted.get(id).map(String::as_str),
        );
        current.insert(id.clone(), parse_installed(raw).map(|(v, _)| v));
        rt.put_entry(entry);
    }
    if let Err(e) = rt
        .gateway
        .grants()
        .refresh_all(&rt.deps.pool, Instant::now() + BOOT_DB_WAIT)
    {
        tracing::warn!(error = %e, "grant refresh failed at boot; affected plugins stay closed");
    }
    cleanup(&rt.deps.plugins_dir, &current);
}

fn load_one(
    rt: &PluginRuntime,
    id: &str,
    raw: &str,
    enabled_raw: Option<&str>,
    granted_raw: Option<&str>,
) -> Entry {
    let (version, installed_at) = parse_installed(raw).unwrap_or_default();
    let broken = || Entry::broken(id, &version, installed_at);
    // 활성 값이 없거나 손상됐으면 꺼진 것으로 본다
    let enabled = enabled_raw.is_some_and(parse_enabled);
    let (Ok(pid), Ok(ver)) = (PluginId::try_from(id), Version::parse(&version)) else {
        tracing::warn!("plugin row with invalid id or version ignored as broken");
        return broken();
    };
    let dir = rt.deps.plugins_dir.join(pid.as_str()).join(ver.to_string());
    match read_package(&dir, None) {
        Ok(pkg) if pkg.manifest.id == pid && pkg.manifest.version == ver => {
            rt.build_entry(pkg, enabled, known_perms(granted_raw), installed_at)
        }
        Ok(_) => {
            tracing::warn!(
                plugin = id,
                "installed package does not match its settings row"
            );
            broken()
        }
        Err(e) => {
            tracing::warn!(plugin = %mam_plugin_manifest::clean_text(id), error = %mam_plugin_manifest::clean_text(&e.to_string()), "plugin package unreadable; listed as broken");
            broken()
        }
    }
}

/// `.staging`·`.trash`, 설정 행이 없는 `<id>/` 폴더, 현재 버전이 아닌 옛 버전 폴더를 지운다 — 모두 `plugins/` 안으로 한정
fn cleanup(root: &Path, current: &BTreeMap<String, Option<String>>) {
    if !root.is_dir() {
        return;
    }
    let warn =
        |what: &str, e: std::io::Error| tracing::warn!(error = %e, "plugin cleanup failed: {what}");
    for name in [STAGING_DIR, TRASH_DIR] {
        if let Err(e) = remove_confined(root, &root.join(name)) {
            warn(name, e);
        }
    }
    let Ok(children) = fs::read_dir(root) else {
        return;
    };
    for child in children.flatten() {
        let path = child.path();
        let name = child.file_name();
        let Some(name) = name.to_str() else {
            let _ = remove_confined(root, &path);
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        match current.get(name) {
            None => {
                if let Err(e) = remove_confined(root, &path) {
                    warn("orphan folder", e);
                }
            }
            Some(Some(version)) if path.is_dir() => remove_old_versions(root, &path, version),
            Some(_) => {}
        }
    }
}

fn remove_old_versions(root: &Path, id_dir: &Path, keep: &str) {
    let Ok(versions) = fs::read_dir(id_dir) else {
        return;
    };
    for v in versions.flatten() {
        if v.file_name().to_str() != Some(keep) {
            let _ = remove_confined(root, &v.path());
        }
    }
}
