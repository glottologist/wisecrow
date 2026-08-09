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

/// One word and its translation, as [`prompts::unknown_words_prompt`] asks for
/// them.
///
/// Lives beside the prompt rather than beside either caller, because a response
/// shape and the prompt that specifies it are one contract: `preview
/// --gloss-unknowns` and [`crate::glossing`] both read this, and a second copy
/// would drift the moment the prompt changed.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct GlossEntry {
    pub word: String,
    pub translation: String,
}

/// The object [`prompts::unknown_words_prompt`] asks the model to return.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct GlossResponse {
    pub glosses: Vec<GlossEntry>,
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
    // Blank counts as absent. The deployment renders this from a vault variable
    // with `| default('')`, so an unset key arrives as an empty string rather
    // than as nothing at all; without this check the provider is built happily
    // and the first call fails with `401 x-api-key header is required`, which
    // says nothing about the configuration that caused it.
    let api_key = config
        .llm_api_key
        .as_ref()
        .filter(|key| !key.expose().trim().is_empty())
        .ok_or_else(|| {
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

    /// The deployment renders the key from a vault variable with `| default('')`,
    /// so an unset key reaches the process as an empty string. Production hit
    /// this: the provider was built and the first call returned
    /// `401 x-api-key header is required`, naming nothing that would lead anyone
    /// to the vault.
    #[rstest::rstest]
    #[case(Some(""), "unset in the vault renders as empty")]
    #[case(Some("   "), "whitespace is no more a key than nothing is")]
    #[case(None, "genuinely absent")]
    fn a_blank_api_key_is_reported_as_unconfigured(#[case] key: Option<&str>, #[case] why: &str) {
        match create_provider(&config_with_key(key)) {
            Err(WisecrowError::ConfigurationError(m)) => {
                assert!(m.contains("llm_api_key"), "{why}: got {m}");
            }
            Err(other) => panic!("{why}: expected a configuration error, got {other:?}"),
            Ok(provider) => panic!("{why}: built a provider named {}", provider.name()),
        }
    }

    #[test]
    fn a_real_key_still_builds_a_provider() {
        match create_provider(&config_with_key(Some("sk-ant-not-a-real-key"))) {
            Ok(provider) => assert_eq!(provider.name(), "anthropic"),
            Err(e) => panic!("a key that is present should build a provider, got {e:?}"),
        }
    }

    fn config_with_key(key: Option<&str>) -> Config {
        Config {
            db_url: None,
            db_address: None,
            db_name: None,
            db_user: None,
            db_password: None,
            image_provider: None,
            unsplash_api_key: None,
            pexels_api_key: None,
            pixabay_api_key: None,
            cereproc_email: None,
            cereproc_password: None,
            cereproc_welsh_voice: None,
            llm_provider: Some("anthropic".to_owned()),
            llm_api_key: key.map(|k| crate::config::SecureString::from(k.to_owned())),
            llm_model: None,
            remote_url: None,
            remote_api_key: None,
            sync_api_key: None,
        }
    }
}
