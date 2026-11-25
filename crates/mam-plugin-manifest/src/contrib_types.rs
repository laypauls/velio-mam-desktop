//! `contributes`·`activationEvents` 타입(검증·구축은 `contributions.rs`).

use serde::{Serialize, Serializer};

use crate::ids::{CommandId, ProviderId, TitleKey};
use crate::when::WhenExpr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Image,
    Video,
    Audio,
}

impl AssetKind {
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "image" => Some(Self::Image),
            "video" => Some(Self::Video),
            "audio" => Some(Self::Audio),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationEvent {
    OnCommand(CommandId),
    OnProvider(ProviderId),
}

impl Serialize for ActivationEvent {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::OnCommand(id) => s.serialize_str(&format!("onCommand:{id}")),
            Self::OnProvider(id) => s.serialize_str(&format!("onProvider:{id}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contributes {
    pub commands: Vec<Command>,
    pub menus: Menus,
    pub providers: Providers,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Command {
    pub id: CommandId,
    pub title: TitleKey,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Menus {
    pub asset_context: Vec<MenuItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MenuItem {
    pub command: CommandId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<WhenExpr>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Providers {
    pub metadata: Vec<Provider>,
    pub analysis: Vec<Provider>,
    pub export: Vec<ExportProvider>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: ProviderId,
    pub title: TitleKey,
    pub asset_kinds: Vec<AssetKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProvider {
    pub id: ProviderId,
    pub title: TitleKey,
    pub asset_kinds: Vec<AssetKind>,
    pub file_extension: String,
}
