//! 표시 이름·제목 번역 — `[현재 로케일, "en"]` 순으로 찾고, 없으면 대체값(플러그인 id·명령 id·`None`).

use mam_plugin_manifest::{LocaleBundle, Manifest, TitleKey};

use crate::api::{CommandInfo, PluginProviderInfo, ProviderKind};

pub(crate) fn tr(bundle: &LocaleBundle, key: &TitleKey, locale: &str, fallback: &str) -> String {
    mam_plugin_manifest::clean_text(bundle.resolve(key, &[locale, "en"], fallback))
}

/// 번역이 없거나 빈 문자열이면 `None`
pub(crate) fn tr_opt(bundle: &LocaleBundle, key: &TitleKey, locale: &str) -> Option<String> {
    let text = mam_plugin_manifest::clean_text(bundle.resolve(key, &[locale, "en"], ""));
    (!text.trim().is_empty()).then_some(text)
}

pub(crate) fn display_name(m: &Manifest, b: &LocaleBundle, locale: &str) -> String {
    tr(b, &m.display_name, locale, m.id.as_str())
}

pub(crate) fn description(m: &Manifest, b: &LocaleBundle, locale: &str) -> Option<String> {
    m.description.as_ref().and_then(|d| tr_opt(b, d, locale))
}

pub(crate) fn commands_info(m: &Manifest, b: &LocaleBundle, locale: &str) -> Vec<CommandInfo> {
    m.contributes
        .commands
        .iter()
        .map(|c| CommandInfo { id: c.id.to_string(), title: tr(b, &c.title, locale, c.id.as_str()) })
        .collect()
}

/// metadata -> analysis -> export 순서 (manifest 선언 순서 유지)
pub(crate) fn providers_info(m: &Manifest, b: &LocaleBundle, locale: &str) -> Vec<PluginProviderInfo> {
    let p = &m.contributes.providers;
    let plain = |kind: ProviderKind, list: &[mam_plugin_manifest::Provider]| {
        list.iter()
            .map(|x| PluginProviderInfo {
                id: x.id.to_string(),
                title: tr(b, &x.title, locale, x.id.as_str()),
                kind,
                asset_kinds: x.asset_kinds.clone(),
                file_extension: None,
            })
            .collect::<Vec<_>>()
    };
    let mut out = plain(ProviderKind::Metadata, &p.metadata);
    out.extend(plain(ProviderKind::Analysis, &p.analysis));
    out.extend(p.export.iter().map(|x| PluginProviderInfo {
        id: x.id.to_string(),
        title: tr(b, &x.title, locale, x.id.as_str()),
        kind: ProviderKind::Export,
        asset_kinds: x.asset_kinds.clone(),
        file_extension: Some(x.file_extension.clone()),
    }));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mam_plugin_manifest::{parse_locale_file, parse_manifest};

    // 분류: Validation(서식·제어 문자) / Boundary(빈 번역·대체값)

    fn manifest() -> Manifest {
        let json = r#"{"apiVersion":1,"id":"com.example.l10n","version":"0.1.0","engines":{"mam":">=0.1.0"},"main":"main.js","displayName":"name","description":"blank"}"#;
        parse_manifest(json).unwrap()
    }

    fn bundle(v: &serde_json::Value) -> LocaleBundle {
        let mut b = LocaleBundle::default();
        b.insert("en", parse_locale_file(&v.to_string()).unwrap()).unwrap();
        b
    }

    #[test]
    fn t_pinst_l10n_01_display_strings_drop_bidi_overrides_and_control_chars() {
        let m = manifest();
        let b = bundle(&serde_json::json!({ "name": "Safe\u{202E}gpj.exe\u{200B}\u{2028}Name", "blank": "\u{202E}\u{200B}" }));
        assert_eq!(display_name(&m, &b, "en"), "Safegpj.exe Name");
        assert_eq!(description(&m, &b, "en"), None, "only invisible chars means no translation");
        // 번역이 없으면 id 로 대체
        assert_eq!(display_name(&m, &LocaleBundle::default(), "en"), "com.example.l10n");
    }
}

