//! Gemini `generateContent` — 키는 `x-goog-api-key` 헤더로만 보낸다 (URL 쿼리에 넣으면 URL 로그로 샌다).

use serde_json::{Value, json};

use super::adapter::{Adapter, Reply, b64, json_request, no_text, parse_json, tokens};
use super::error::ExternalError;
use super::payload::PreparedImage;
use super::transport::HttpRequest;
use super::types::Provider;

pub(crate) struct Gemini;

impl Adapter for Gemini {
    fn provider(&self) -> Provider {
        Provider::Gemini
    }

    fn build(
        &self,
        key: &str,
        model: &str,
        prompt: &str,
        images: &[PreparedImage],
        max_tokens: u32,
    ) -> HttpRequest {
        let mut parts = vec![json!({"text": prompt})];
        for img in images {
            parts.push(json!({"inline_data": {"mime_type": img.mime, "data": b64(&img.bytes)}}));
        }
        let body = json!({
            "contents": [{"role": "user", "parts": parts}],
            "generationConfig": {
                "responseMimeType": "application/json",
                "maxOutputTokens": max_tokens,
            },
        });
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent"
        );
        json_request(url, vec![("x-goog-api-key", key.to_string())], &body)
    }

    fn parse(&self, body: &[u8]) -> Result<Reply, ExternalError> {
        let v = parse_json(body)?;
        let parts = v["candidates"][0]["content"]["parts"]
            .as_array()
            .ok_or_else(no_text)?;
        let text: String = parts.iter().filter_map(|p| p["text"].as_str()).collect();
        if text.is_empty() {
            return Err(no_text());
        }
        let usage: Option<&Value> = v.get("usageMetadata");
        Ok(Reply {
            text,
            input_tokens: tokens(usage.and_then(|u| u.get("promptTokenCount"))),
            output_tokens: tokens(usage.and_then(|u| u.get("candidatesTokenCount"))),
        })
    }
}
