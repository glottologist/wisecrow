use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::LlmProvider;
use crate::errors::WisecrowError;

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";

/// Default OpenAI model when `WISECROW__LLM_MODEL` is unset.
pub const DEFAULT_MODEL: &str = "gpt-4o";

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAiProvider {
    #[must_use]
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
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
struct OpenAiRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<OpenAiMessage<'a>>,
}

#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    /// `length` is OpenAI's spelling of "cut at the ceiling". Without it a
    /// truncated answer reaches the caller's JSON parser as a syntax error.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, WisecrowError> {
        let request = OpenAiRequest {
            model: &self.model,
            max_tokens,
            messages: vec![OpenAiMessage {
                role: "user",
                content: prompt,
            }],
        };

        let response = self
            .client
            .post(OPENAI_API_URL)
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| WisecrowError::LlmError(format!("OpenAI request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(WisecrowError::LlmError(format!(
                "OpenAI API error {status}: {body}"
            )));
        }

        let parsed: OpenAiResponse = response.json().await.map_err(|e| {
            WisecrowError::LlmError(format!("Failed to parse OpenAI response: {e}"))
        })?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| WisecrowError::LlmError("Empty response from OpenAI".to_owned()))?;
        if choice.finish_reason.as_deref() == Some("length") {
            return Err(WisecrowError::LlmError(format!(
                "OpenAI stopped at the max_tokens ceiling of {max_tokens}: the answer is \
                 truncated, not malformed. Ask for less in one call or raise the budget."
            )));
        }
        Ok(choice.message.content)
    }

    fn name(&self) -> &str {
        "openai"
    }
}
