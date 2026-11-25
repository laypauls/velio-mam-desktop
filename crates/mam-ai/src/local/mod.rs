//! 로컬 추론 - 임베딩([`embed`])과 음성 인식([`stt`]). 둘 다 네트워크를 쓰지 않는다.

pub mod embed;
pub(crate) mod runtime;
pub mod stt;
