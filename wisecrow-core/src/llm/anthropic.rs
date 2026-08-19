use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::LlmProvider;
use crate::errors::WisecrowError;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Default Anthropic model when `WISECROW__LLM_MODEL` is unset.
pub const DEFAULT_MODEL: &str = "claude-sonnet-5";

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        // Bound to IPv4: the production host's IPv6 path to api.anthropic.com
        // drops large transfers (measured 2026-08-09: an identical long
        // generation returned 0 bytes over v6 and completed over v4), so
        // responses longer than a few seconds died with "error decoding
        // response body" whenever the connection came up over v6.
        let client = Client::builder()
            .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
            .build()
            .unwrap_or_default();
        Self {
            client,
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    thinking: Thinking,
    messages: Vec<Message<'a>>,
}

/// Every prompt here asks for a fixed JSON shape, so thinking buys nothing —
/// and on `claude-sonnet-5` it is on by default and draws from the same
/// `max_tokens` budget as the answer. Measured on one 25-phrase prompt:
/// 517–2048 thinking tokens across six identical calls, truncating the JSON
/// mid-entry in two of them. Disabling it holds output at ~670 tokens.
#[derive(Serialize)]
struct Thinking {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

/// One block of the response's `content` array. Newer models prepend
/// blocks without a `text` field (thinking output), so the field is
/// optional and callers take the text-bearing blocks only.
#[derive(Deserialize)]
struct ContentBlock {
    #[serde(default)]
    text: Option<String>,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, WisecrowError> {
        let request = AnthropicRequest {
            model: &self.model,
            max_tokens,
            thinking: Thinking { kind: "disabled" },
            messages: vec![Message {
                role: "user",
                content: prompt,
            }],
        };

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| WisecrowError::LlmError(format!("Anthropic request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(WisecrowError::LlmError(format!(
                "Anthropic API error {status}: {body}"
            )));
        }

        // `{e:?}` rather than `{e}`: reqwest's Display for a decode failure is
        // the bare "error decoding response body", with the serde detail that
        // names the offending field only in the source chain.
        let parsed: AnthropicResponse = response.json().await.map_err(|e| {
            WisecrowError::LlmError(format!("Failed to parse Anthropic response: {e:?}"))
        })?;

        let text: String = parsed
            .content
            .into_iter()
            .filter_map(|block| block.text)
            .collect();
        if text.is_empty() {
            return Err(WisecrowError::LlmError(
                "Empty response from Anthropic".to_owned(),
            ));
        }
        Ok(text)
    }

    fn name(&self) -> &str {
        "anthropic"
    }
}
