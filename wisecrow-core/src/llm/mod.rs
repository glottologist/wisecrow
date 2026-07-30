pub mod anthropic;
pub mod openai;
pub mod prompts;

use crate::config::Config;
use crate::errors::WisecrowError;
use async_trait::async_trait;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, WisecrowError>;
    fn name(&self) -> &str;
}

/// Parses JSON from an LLM response, tolerating a leading/trailing markdown code
/// fence (```` ```json ```` … ```` ``` ````) that models often wrap output in.
/// `context` names the expected shape and appears in the error on failure.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmError`] if the trimmed body is not valid JSON for `T`.
pub fn parse_fenced_json<T>(response: &str, context: &str) -> Result<T, WisecrowError>
where
    T: serde::de::DeserializeOwned,
{
    let trimmed = response.trim();
    let json_str = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };
    serde_json::from_str(json_str)
        .map_err(|e| WisecrowError::LlmError(format!("Failed to parse {context}: {e}")))
}

/// Creates an LLM provider based on configuration.
///
/// # Errors
///
/// Returns an error if the provider is not configured or unsupported.
pub fn create_provider(config: &Config) -> Result<Box<dyn LlmProvider>, WisecrowError> {
    let provider_name = config.llm_provider.as_deref().ok_or_else(|| {
        WisecrowError::ConfigurationError("llm_provider not configured".to_owned())
    })?;
    let api_key = config.llm_api_key.as_ref().ok_or_else(|| {
        WisecrowError::ConfigurationError("llm_api_key not configured".to_owned())
    })?;

    match provider_name {
        "anthropic" => {
            let model = config.llm_model_or(anthropic::DEFAULT_MODEL).to_owned();
            Ok(Box::new(anthropic::AnthropicProvider::new(
                api_key.expose(),
                model,
            )))
        }
        "openai" => {
            let model = config.llm_model_or(openai::DEFAULT_MODEL).to_owned();
            Ok(Box::new(openai::OpenAiProvider::new(
                api_key.expose(),
                model,
            )))
        }
        other => Err(WisecrowError::ConfigurationError(format!(
            "Unsupported LLM provider: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize, PartialEq, Eq, Debug)]
    struct Sample {
        value: i32,
    }

    #[test]
    fn parses_plain_and_fenced_and_reports_context() {
        assert_eq!(
            parse_fenced_json::<Sample>(r#"{"value":1}"#, "sample").unwrap(),
            Sample { value: 1 }
        );
        assert_eq!(
            parse_fenced_json::<Sample>("```json\n{\"value\":2}\n```", "sample").unwrap(),
            Sample { value: 2 }
        );
        assert_eq!(
            parse_fenced_json::<Sample>("```\n{\"value\":3}\n```", "sample").unwrap(),
            Sample { value: 3 }
        );
        let err = parse_fenced_json::<Sample>("not json", "sample").unwrap_err();
        assert!(matches!(err, WisecrowError::LlmError(m) if m.contains("sample")));
    }
}
