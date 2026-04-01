use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::LlmProvider;
use crate::errors::WisecrowError;

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_MODEL: &str = "gpt-4o";

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
}

impl OpenAiProvider {
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
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
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, WisecrowError> {
        let request = OpenAiRequest {
            model: DEFAULT_MODEL,
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

        parsed
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .ok_or_else(|| WisecrowError::LlmError("Empty response from OpenAI".to_owned()))
    }

    fn name(&self) -> &str {
        "openai"
    }
}
