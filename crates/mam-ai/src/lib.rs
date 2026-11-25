//! MAM Desktop 로컬 AI 런타임 (P1-6) — 모델 매니페스트·다운로드, ONNX 임베딩, whisper.cpp STT.
//!
//! - [`models`] : 모델 매니페스트(이름·URL·크기·SHA-256·라이선스), 이어받기 다운로드, 저장소
//! - [`local::embed`] : CLIP 이미지/다국어 텍스트 공동 임베딩(512차원), multilingual-e5 텍스트 임베딩(384차원)
//! - [`local::stt`] : whisper.cpp 사이드카 실행(타임아웃·강제 종료), 세그먼트 파싱
//! - [`resources`] : 사용자 PC 사양별 스레드·동시성 기본값
//!
//! - [`external`] : 외부 AI(OpenAI·Gemini·Claude) 어댑터 — 동의 게이트, 축소 이미지 페이로드, 재시도, 사용량 (P3-2)
//! - [`keys`] : API 키 보관 — 메모리(zeroize) + 선택적 Windows DPAPI 영속화 (P3-2)
//!
//! 원칙: 네트워크는 모델 최초 다운로드와 (동의 후) 외부 AI 분석에만 쓴다. 원본 미디어는 읽기 전용으로만 열고,
//! 산출물(임시 WAV 등)은 호출자가 넘기는 MAM 데이터 디렉터리 안에만 쓴다. 원본 음성을 외부로 보내지 않는다.

pub mod error;
pub mod external;
pub mod keys;
pub mod local;
pub mod models;
pub mod resources;

pub use error::{AiError, Result};
