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
    /// `max_tokens` here means the answer was cut mid-sentence. Every caller
    /// asks for JSON, so a cut answer reaches the parser as a syntax error
    /// that names a line and column in text nobody kept -- see
    /// [`AnthropicResponse::into_text`].
    #[serde(default)]
    stop_reason: Option<String>,
}

impl AnthropicResponse {
    /// Joins the text-bearing blocks, refusing an answer the model did not
    /// finish.
    fn into_text(self, max_tokens: u32) -> Result<String, WisecrowError> {
        if self.stop_reason.as_deref() == Some("max_tokens") {
            return Err(WisecrowError::LlmError(format!(
                "Anthropic stopped at the max_tokens ceiling of {max_tokens}: the answer is \
                 truncated, not malformed. Ask for less in one call or raise the budget."
            )));
        }
        let text: String = self
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

        parsed.into_text(max_tokens)
    }

    fn name(&self) -> &str {
        "anthropic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(json: &str) -> AnthropicResponse {
        serde_json::from_str(json).expect("the fixture is a well-formed Anthropic response")
    }

    /// Seeding a CEFR level asked for fifteen rules inside a 4096-token budget
    /// and the answer landed 34 tokens under it, so some levels were cut. The
    /// cut arrived as `EOF while parsing a string at line 228 column 112`,
    /// which reads like a model that cannot write JSON rather than a budget
    /// that is too small.
    #[test]
    fn a_truncated_answer_names_the_ceiling_rather_than_the_json() {
        let err = response(
            r#"{"stop_reason":"max_tokens","content":[{"type":"text","text":"[{\"title\": \"Unfinis"}]}"#,
        )
        .into_text(4096)
        .expect_err("an answer the model did not finish is not an answer");
        assert!(
            matches!(&err, WisecrowError::LlmError(m) if m.contains("4096") && m.contains("truncated")),
            "got {err:?}"
        );
    }

    #[test]
    fn a_finished_answer_is_its_text_blocks_joined() {
        let text = response(
            r#"{"stop_reason":"end_turn","content":[{"type":"thinking"},{"type":"text","text":"[1,"},{"type":"text","text":"2]"}]}"#,
        )
        .into_text(4096)
        .expect("a finished answer is returned whole");
        assert_eq!(text, "[1,2]");
    }

    #[test]
    fn an_answer_with_no_text_is_refused() {
        let err = response(r#"{"stop_reason":"end_turn","content":[]}"#)
            .into_text(4096)
            .expect_err("no text is nothing to parse");
        assert!(
            matches!(&err, WisecrowError::LlmError(m) if m.contains("Empty")),
            "got {err:?}"
        );
    }
}
