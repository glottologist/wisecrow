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
        "anthropic" => Ok(Box::new(anthropic::AnthropicProvider::new(
            api_key.expose().to_owned(),
        ))),
        "openai" => Ok(Box::new(openai::OpenAiProvider::new(
            api_key.expose().to_owned(),
        ))),
        other => Err(WisecrowError::ConfigurationError(format!(
            "Unsupported LLM provider: {other}"
        ))),
    }
}
