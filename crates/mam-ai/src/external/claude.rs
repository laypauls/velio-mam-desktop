//! Anthropic Messages API — `x-api-key` + `anthropic-version`, 이미지는 base64 source 블록.

use serde_json::{Value, json};

use super::adapter::{Adapter, Reply, b64, json_request, no_text, parse_json, tokens};
use super::error::ExternalError;
use super::payload::PreparedImage;
use super::transport::HttpRequest;
use super::types::Provider;

pub(crate) const URL: &str = "https://api.anthropic.com/v1/messages";
pub(crate) const API_VERSION: &str = "2023-06-01";

pub(crate) struct Claude;

impl Adapter for Claude {
    fn provider(&self) -> Provider {
        Provider::Claude
    }

    fn build(
        &self,
        key: &str,
        model: &str,
        prompt: &str,
        images: &[PreparedImage],
        max_tokens: u32,
    ) -> HttpRequest {
        let mut content: Vec<Value> = images
            .iter()
            .map(|img| {
                json!({"type": "image", "source": {"type": "base64", "media_type": img.mime, "data": b64(&img.bytes)}})
            })
            .collect();
        content.push(json!({"type": "text", "text": prompt}));
        let body = json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [{"role": "user", "content": content}],
        });
        json_request(
            URL.into(),
            vec![
                ("x-api-key", key.to_string()),
                ("anthropic-version", API_VERSION.into()),
            ],
            &body,
        )
    }

    fn parse(&self, body: &[u8]) -> Result<Reply, ExternalError> {
        let v = parse_json(body)?;
        let blocks = v["content"].as_array().ok_or_else(no_text)?;
        let text: String = blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        if text.is_empty() {
            return Err(no_text());
        }
        let usage: Option<&Value> = v.get("usage");
        Ok(Reply {
            text,
            input_tokens: tokens(usage.and_then(|u| u.get("input_tokens"))),
            output_tokens: tokens(usage.and_then(|u| u.get("output_tokens"))),
        })
    }
}
