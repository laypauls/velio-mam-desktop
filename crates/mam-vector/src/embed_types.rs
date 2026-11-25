//! 원시 임베딩 저장소의 공개 타입 — 벡터 종류·출처, 모델 키, 저장할 벡터, 읽기 보고. `embed_store` 가 다시 내보낸다.

use crate::types::EmbeddingId;

/// 벡터 종류. 종류마다 모델·인덱스가 따로다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorKind {
    /// CLIP 이미지/프레임 (512-d)
    Visual,
    /// 텍스트 의미 — e5 (384-d)
    Semantic,
}

impl VectorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Visual => "visual",
            Self::Semantic => "semantic",
        }
    }
}

/// 벡터의 출처.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorSource {
    Image,
    Frame,
    Text,
}

impl VectorSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Frame => "frame",
            Self::Text => "text",
        }
    }
}

/// 서로 섞이면 안 되는 단위: 종류 + 모델 ID + 버전 (+ 차원).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelKey {
    pub kind: VectorKind,
    pub model_id: String,
    pub model_version: String,
    pub dimension: usize,
}

impl ModelKey {
    /// 디렉터리 이름 — 파일 시스템에 안전한 문자만 남긴다.
    pub fn dir_name(&self) -> String {
        let safe = |s: &str| -> String {
            s.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        };
        format!(
            "{}__v{}__d{}",
            safe(&self.model_id),
            safe(&self.model_version),
            self.dimension
        )
    }
}

/// 저장할 벡터 한 건.
pub struct NewVector<'a> {
    pub segment_id: Option<i64>,
    pub source: VectorSource,
    pub frame_index: Option<i64>,
    pub vector: &'a [f32],
}

/// 활성 행을 raw 파일에서 읽은 결과 보고.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReadReport {
    /// 콜백에 넘긴 레코드 수
    pub read: u64,
    /// raw 파일에서 읽을 수 없어 건너뛴 `embedding_id` (파일 손상·잘림)
    pub skipped: Vec<EmbeddingId>,
}
