//! 모델 매니페스트 — 이름·URL·크기·SHA-256·라이선스를 코드에 고정한다 (요구사항 §5 미정 -> D5).
//!
//! 내장 매니페스트는 `manifest.json` 이며, URL 은 저장소의 **커밋 해시로 고정**한 주소다(브랜치 이동으로
//! 파일이 바뀌어도 SHA-256 검증이 실패하지 않게).

use serde::Deserialize;

use crate::error::{AiError, Result};

/// 모델 용도.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    /// 이미지 -> 공동 임베딩 공간 (CLIP 비전 인코더).
    ImageEmbedding,
    /// 텍스트 -> 이미지와 같은 공간 (다국어 CLIP 텍스트 인코더).
    ClipTextEmbedding,
    /// 텍스트 의미 임베딩 (AI 설명·STT 검색).
    TextEmbedding,
    /// 음성 인식 (ggml 가중치).
    Stt,
}

/// 모델을 이루는 파일 하나.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModelFile {
    /// 모델 폴더 안의 상대 경로 (`/` 구분, `..` 금지).
    pub path: String,
    pub url: String,
    pub size: u64,
    /// 소문자 16진 SHA-256 (64자).
    pub sha256: String,
}

/// 모델 한 개 (여러 파일).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ModelSpec {
    pub id: String,
    pub kind: ModelKind,
    pub display_name: String,
    /// 상업 이용이 가능한 SPDX 식별자 (MIT, Apache-2.0).
    pub license: String,
    pub license_url: String,
    pub source_url: String,
    /// 임베딩 차원 (임베딩 모델만).
    #[serde(default)]
    pub dimension: Option<usize>,
    pub files: Vec<ModelFile>,
}

impl ModelSpec {
    /// 모델 전체 다운로드 크기 (바이트).
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub models: Vec<ModelSpec>,
}

const BUILTIN: &str = include_str!("manifest.json");

impl Manifest {
    /// 앱에 내장된 매니페스트.
    pub fn builtin() -> Result<Self> {
        Self::parse(BUILTIN)
    }

    /// JSON 을 읽고 규칙을 검증한다 (경로 탈출, 해시 형식, 중복 ID).
    pub fn parse(json: &str) -> Result<Self> {
        let m: Manifest =
            serde_json::from_str(json).map_err(|e| AiError::Manifest(e.to_string()))?;
        m.validate()?;
        Ok(m)
    }

    pub fn get(&self, id: &str) -> Result<&ModelSpec> {
        self.models
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| AiError::UnknownModel(id.to_string()))
    }

    fn validate(&self) -> Result<()> {
        let bad = |s: String| Err(AiError::Manifest(s));
        let mut seen = std::collections::HashSet::new();
        for m in &self.models {
            if !seen.insert(m.id.as_str()) {
                return bad(format!("duplicate model id {}", m.id));
            }
            if m.id.is_empty() || m.id.contains(['/', '\\', '.']) {
                return bad(format!("model id {:?} must be a plain name", m.id));
            }
            if m.files.is_empty() {
                return bad(format!("model {} has no files", m.id));
            }
            for f in &m.files {
                if !is_safe_relative(&f.path) {
                    return bad(format!("unsafe file path {:?} in {}", f.path, m.id));
                }
                let hex_ok = f
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
                if f.sha256.len() != 64 || !hex_ok {
                    return bad(format!(
                        "sha256 of {}/{} must be 64 lowercase hex",
                        m.id, f.path
                    ));
                }
                // 로컬 스텁 서버(시험)만 평문 HTTP 를 허용한다.
                if !f.url.starts_with("https://") && !f.url.starts_with("http://127.0.0.1") {
                    return bad(format!("url of {}/{} must be https", m.id, f.path));
                }
            }
        }
        Ok(())
    }
}

/// 절대 경로·`..`·드라이브 문자·역슬래시를 거부한다.
fn is_safe_relative(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains(['\\', ':'])
        && p.split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}
