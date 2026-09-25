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
    match serde_json::from_str(json_str) {
        Ok(parsed) => Ok(parsed),
        Err(original) => escape_raw_control_characters(json_str)
            .and_then(|repaired| serde_json::from_str(&repaired).ok())
            .ok_or_else(|| {
                WisecrowError::LlmError(format!("Failed to parse {context}: {original}"))
            }),
    }
}

/// Escapes control characters sitting raw inside a JSON string literal.
///
/// A model writing prose occasionally presses a literal newline into a string
/// instead of `\n`. RFC 8259 forbids it and `serde_json` rejects the whole
/// document, so one stray byte costs every rule in the answer -- seeding Welsh
/// C1 lost fifteen that way. The repair is deliberately narrow: only
/// `U+0000`-`U+001F`, only inside a string, and only after a straight parse has
/// already failed, so a well-formed answer is never rewritten and a genuinely
/// broken one still surfaces its original error.
///
/// Returns `None` when there was nothing to repair, which spares the caller a
/// second parse that would fail the same way.
fn escape_raw_control_characters(json: &str) -> Option<String> {
    let mut out = String::with_capacity(json.len());
    let mut in_string = false;
    let mut after_backslash = false;
    let mut repaired = false;

    for ch in json.chars() {
        if after_backslash {
            out.push(ch);
            after_backslash = false;
            continue;
        }
        match ch {
            '\\' if in_string => {
                after_backslash = true;
                out.push(ch);
            }
            '"' => {
                in_string = !in_string;
                out.push(ch);
            }
            // Outside a string these same bytes are legal whitespace between
            // tokens, so the guard carries its weight.
            c if in_string && c < '\u{20}' => {
                repaired = true;
                match c {
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    other => {
                        let code = other as u32;
                        out.push_str("\\u00");
                        out.push(char::from_digit(code >> 4, 16).unwrap_or('0'));
                        out.push(char::from_digit(code & 0xF, 16).unwrap_or('0'));
                    }
                }
            }
            c => out.push(c),
        }
    }

    repaired.then_some(out)
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

    #[derive(serde::Deserialize, PartialEq, Eq, Debug)]
    struct Prose {
        explanation: String,
    }

    /// Seeding Welsh C1 failed with `control character (\u0000-\u001F) found
    /// while parsing a string at line 53 column 0`, which cost the level all
    /// fifteen of its rules. The answer was complete -- the `max_tokens`
    /// refusal did not fire -- so the only fault was an unescaped newline.
    #[rstest::rstest]
    #[case("a line\nand another", "a newline, which is what production hit")]
    #[case("a tab\there", "a tab")]
    #[case("a carriage\rreturn", "a carriage return")]
    #[case("a bell\u{7}rings", "a control character with no short escape")]
    fn a_raw_control_character_inside_a_string_is_repaired_rather_than_fatal(
        #[case] raw: &str,
        #[case] why: &str,
    ) {
        let malformed = format!(r#"{{"explanation":"{raw}"}}"#);
        match parse_fenced_json::<Prose>(&malformed, "prose") {
            Ok(parsed) => assert_eq!(parsed.explanation, raw, "{why}"),
            Err(e) => panic!("{why}: {e:?}"),
        }
    }

    #[test]
    fn a_control_character_outside_a_string_was_always_legal_and_is_left_alone() {
        assert_eq!(
            parse_fenced_json::<Sample>("{\n\t\"value\"\t:\t4\n}", "sample").unwrap(),
            Sample { value: 4 }
        );
        assert!(escape_raw_control_characters("{\n\t\"value\": 4\n}").is_none());
    }

    #[test]
    fn an_already_escaped_newline_is_not_escaped_twice() {
        let parsed = parse_fenced_json::<Prose>(r#"{"explanation":"one\ntwo"}"#, "prose").unwrap();
        assert_eq!(parsed.explanation, "one\ntwo");
        assert!(escape_raw_control_characters(r#"{"explanation":"one\ntwo"}"#).is_none());
    }

    /// A backslash immediately before the closing quote once tempted a simpler
    /// implementation into believing the string had not ended.
    #[test]
    fn a_trailing_escaped_backslash_does_not_swallow_the_closing_quote() {
        let parsed =
            parse_fenced_json::<Prose>(r#"{"explanation":"ends with \\"}"#, "prose").unwrap();
        assert_eq!(parsed.explanation, r"ends with \");
        assert!(escape_raw_control_characters(r#"{"explanation":"ends with \\"}"#).is_none());
    }

    /// The repair must not turn an unparseable answer into a silent success,
    /// and the error it reports has to be the one that describes the real
    /// fault rather than whatever the rewritten text failed on.
    #[test]
    fn an_answer_that_is_broken_some_other_way_still_reports_its_original_error() {
        // Both faults at once: a raw newline, which the repair does fix, and a
        // dangling key, which it cannot. The repaired text fails on the latter,
        // so reporting the original error is the only way the message still
        // names the control character.
        let err = parse_fenced_json::<Prose>("{\"explanation\":\"a\nb\", \"dangling\"}", "prose")
            .unwrap_err();
        let WisecrowError::LlmError(message) = err else {
            panic!("expected an LlmError");
        };
        assert!(message.contains("prose"), "got {message}");
        assert!(
            message.contains("control character"),
            "the original error names the real fault, got {message}"
        );
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
