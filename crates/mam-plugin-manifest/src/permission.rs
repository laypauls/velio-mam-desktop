use std::fmt;
use std::str::FromStr;

use serde::{Serialize, Serializer};

use crate::error::ManifestError;

/// 알려진 권한 6종. 그 외 문자열은 `UnknownPermission`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Permission {
    AssetReadMetadata,
    AssetReadDerived,
    KvRead,
    KvWrite,
    TagWrite,
    ExportWrite,
}

impl Permission {
    pub const ALL: [Permission; 6] = [
        Permission::AssetReadMetadata,
        Permission::AssetReadDerived,
        Permission::KvRead,
        Permission::KvWrite,
        Permission::TagWrite,
        Permission::ExportWrite,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Permission::AssetReadMetadata => "asset.read.metadata",
            Permission::AssetReadDerived => "asset.read.derived",
            Permission::KvRead => "kv.read",
            Permission::KvWrite => "kv.write",
            Permission::TagWrite => "tag.write",
            Permission::ExportWrite => "export.write",
        }
    }
}

impl FromStr for Permission {
    type Err = ManifestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Permission::ALL
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| ManifestError::UnknownPermission {
                value: s.to_owned(),
            })
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Permission {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}
