//! P6 — 설치·점검·업그레이드·제거·부팅 정리. 모두 `PluginRuntime` 의 메서드로 구현한다(상태는 `runtime/`).
//! 원본 폴더는 읽기만 한다. 쓰기는 `plugins/`(MAMData) 안으로 한정하고 삭제는 `fsx::remove_confined` 를 거친다.

pub(crate) mod boot;
mod fsx;
mod inspect;
pub(crate) mod package;
mod stage;
pub(crate) mod store;
mod uninstall;
mod upgrade;
