//! OpenAI Chat Completions — Bearer 인증, 이미지는 data URL, JSON 출력 모드.

use serde_json::{Value, json};

use super::adapter::{Adapter, Reply, b64, json_request, no_text, parse_json, tokens};
use super::error::ExternalError;
use super::payload::PreparedImage;
use super::transport::HttpRequest;
use super::types::Provider;

pub(crate) const URL: &str = "https://api.openai.com/v1/chat/completions";

pub(crate) struct OpenAi;

impl Adapter for OpenAi {
    fn provider(&self) -> Provider {
        Provider::OpenAi
    }

    fn build(
        &self,
        key: &str,
        model: &str,
        prompt: &str,
        images: &[PreparedImage],
        max_tokens: u32,
    ) -> HttpRequest {
        let mut content = vec![json!({"type": "text", "text": prompt})];
        for img in images {
            content.push(json!({
                "type": "image_url",
                "image_url": {"url": format!("data:{};base64,{}", img.mime, b64(&img.bytes))}
            }));
        }
        let body = json!({
            "model": model,
            "messages": [{"role": "user", "content": content}],
            "response_format": {"type": "json_object"},
            "max_completion_tokens": max_tokens,
        });
        json_request(
            URL.into(),
            vec![("authorization", format!("Bearer {key}"))],
            &body,
        )
    }

    fn parse(&self, body: &[u8]) -> Result<Reply, ExternalError> {
        let v = parse_json(body)?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(no_text)?;
        let usage: Option<&Value> = v.get("usage");
        Ok(Reply {
            text: text.to_string(),
            input_tokens: tokens(usage.and_then(|u| u.get("prompt_tokens"))),
            output_tokens: tokens(usage.and_then(|u| u.get("completion_tokens"))),
        })
    }
}
