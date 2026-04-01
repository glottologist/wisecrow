use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::LlmProvider;
use crate::errors::WisecrowError;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
}

impl AnthropicProvider {
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<Message<'a>>,
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

#[derive(Deserialize)]
struct ContentBlock {
    text: String,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, WisecrowError> {
        let request = AnthropicRequest {
            model: DEFAULT_MODEL,
            max_tokens,
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

        let parsed: AnthropicResponse = response.json().await.map_err(|e| {
            WisecrowError::LlmError(format!("Failed to parse Anthropic response: {e}"))
        })?;

        parsed
            .content
            .into_iter()
            .next()
            .map(|block| block.text)
            .ok_or_else(|| WisecrowError::LlmError("Empty response from Anthropic".to_owned()))
    }

    fn name(&self) -> &str {
        "anthropic"
    }
}
