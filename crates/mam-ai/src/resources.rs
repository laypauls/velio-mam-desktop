//! 사용자 PC 사양별 동시성·스레드 기본값 (§13 저사양 8GB PC 결정 - P1-6 측정 근거는 작업 문서 "§13 결론").
//!
//! 8GB/16GB 는 실기가 아니라 개발 PC(6C/12T, 32GB)에서 스레드·동시성을 제한해 모사한 측정이다.

use crate::local::embed::EmbedConfig;
use crate::local::stt::SttConfig;

/// 로컬 AI 작업의 리소스 한도.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceConfig {
    /// 임베딩 ONNX 연산 스레드 수.
    pub embed_threads: usize,
    /// 임베딩 배치 크기 (이미지는 배치 효과가 거의 없고, 텍스트는 배치 8 에서 처리량이 1.3~2배).
    pub embed_batch: usize,
    /// whisper 스레드 수.
    pub stt_threads: usize,
    /// 동시에 실행하는 STT 작업 수 (측정상 1개 x 다중 스레드가 메모리 대비 효율이 낫다).
    pub stt_concurrency: usize,
}

impl ResourceConfig {
    /// 물리 RAM(GB)으로 고른다. 8GB 이하는 임베딩 2스레드·STT 2스레드, 그 이상은 4스레드.
    pub fn for_ram_gb(ram_gb: u64) -> Self {
        if ram_gb <= 8 {
            Self {
                embed_threads: 2,
                embed_batch: 4,
                stt_threads: 2,
                stt_concurrency: 1,
            }
        } else {
            Self {
                embed_threads: 4,
                embed_batch: 8,
                stt_threads: 4,
                stt_concurrency: 1,
            }
        }
    }

    pub fn embed_config(&self) -> EmbedConfig {
        EmbedConfig {
            threads: self.embed_threads,
            batch_size: self.embed_batch,
        }
    }

    pub fn stt_config(&self) -> SttConfig {
        SttConfig {
            threads: self.stt_threads,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_ai_16_low_spec_uses_fewer_threads_and_single_stt() {
        // Boundary: 8GB 경계와 0 입력(알 수 없음)은 저사양 쪽으로 처리
        let low = ResourceConfig::for_ram_gb(8);
        let zero = ResourceConfig::for_ram_gb(0);
        let mid = ResourceConfig::for_ram_gb(16);
        assert_eq!(low, zero);
        assert!(low.embed_threads < mid.embed_threads && low.stt_threads < mid.stt_threads);
        assert_eq!((low.stt_concurrency, mid.stt_concurrency), (1, 1));
        assert_eq!(low.embed_config().threads, 2);
        assert_eq!(mid.stt_config().threads, 4);
    }
}
