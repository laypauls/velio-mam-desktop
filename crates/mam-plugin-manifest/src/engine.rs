use semver::{BuildMetadata, Prerelease, Version};

use crate::error::EngineError;
use crate::manifest::Manifest;

/// 앱 버전의 pre-release·build 를 떼고 비교한다(그러지 않으면 `0.2.0-beta.1` 이 `>=0.2.0` 에 안 맞는다).
pub fn check_engine(m: &Manifest, app: &Version) -> Result<(), EngineError> {
    let mut v = app.clone();
    v.pre = Prerelease::EMPTY;
    v.build = BuildMetadata::EMPTY;
    if m.engines.mam.matches(&v) {
        Ok(())
    } else {
        Err(EngineError::Incompatible {
            required: m.engines.mam.to_string(),
            app: app.to_string(),
        })
    }
}
