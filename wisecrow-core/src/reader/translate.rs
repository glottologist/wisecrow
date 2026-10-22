//! Translates a book one numbered run of sentences at a time.
//!
//! A run is the unit of payment and of resumption: it is one model call,
//! cached by its prompt, and its answers are committed with the cursor that
//! passes it. The answer must hold exactly one translation per sentence, in
//! order, because the dual view pairs them by number.

use serde::Deserialize;

use crate::errors::WisecrowError;
use crate::llm::cache::{CachedLlm, CachedLlmError};
use crate::llm::parse_fenced_json;
use crate::llm::prompts::book_translation_prompt;

/// Most sentences one run holds.
pub const RUN_MAX_SENTENCES: usize = 60;
/// Most characters of English one run holds; a longer sentence is a run of
/// its own.
pub const RUN_MAX_CHARS: usize = 6_000;
/// Answer budget for one run: sixty short sentences and their JSON fit well
/// inside it.
pub const TRANSLATION_MAX_TOKENS: u32 = 8_192;
/// Longest translation of one sentence accepted, in characters.
const MAX_TRANSLATION_CHARS: usize = 2_000;

/// One run to translate.
#[derive(Debug, Clone, Copy)]
pub struct RunRequest<'a> {
    pub sentences: &'a [&'a str],
    pub language_name: &'a str,
    pub cefr_level: &'a str,
}

/// What became of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunOutcome {
    /// One translation per sentence, in order.
    Translated(Vec<String>),
    /// Two answers were refused; the reason is the second refusal.
    Failed(String),
}

#[derive(Deserialize)]
struct TranslationAnswer {
    translations: Vec<NumberedTranslation>,
}

#[derive(Deserialize)]
struct NumberedTranslation {
    n: usize,
    text: String,
}

/// How many of `texts`, taken from the start, make the next run.
#[must_use]
pub fn run_length(texts: &[&str]) -> usize {
    let mut characters = 0usize;
    let mut count = 0usize;
    for text in texts.iter().take(RUN_MAX_SENTENCES) {
        let length = text.chars().count();
        if count > 0 && characters.saturating_add(length) > RUN_MAX_CHARS {
            break;
        }
        characters = characters.saturating_add(length);
        count = count.saturating_add(1);
    }
    count
}

/// Reads an answer, refusing one that does not number exactly `expected`
/// non-empty translations in order.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmError`] naming what was wrong.
pub fn parse_translations(response: &str, expected: usize) -> Result<Vec<String>, WisecrowError> {
    let answer: TranslationAnswer = parse_fenced_json(response, "book translations")?;
    let got = answer.translations.len();
    if got != expected {
        return Err(WisecrowError::LlmError(format!(
            "returned {got} translations for {expected} sentences"
        )));
    }
    answer
        .translations
        .into_iter()
        .enumerate()
        .map(|(index, entry)| {
            let number = index.saturating_add(1);
            let text = entry.text.trim();
            if entry.n != number {
                return Err(WisecrowError::LlmError(format!(
                    "entry {number} is numbered {}",
                    entry.n
                )));
            }
            if text.is_empty() || text.chars().count() > MAX_TRANSLATION_CHARS {
                return Err(WisecrowError::LlmError(format!(
                    "translation {number} is empty or longer than {MAX_TRANSLATION_CHARS} characters"
                )));
            }
            Ok(text.to_owned())
        })
        .collect()
}

/// Translates one run, asking once more with the rejection quoted when the
/// first answer is refused.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmUnavailable`], a configuration error or a
/// database error from the cache; a refused answer is an outcome, not an
/// error.
pub async fn translate_run(
    model: &CachedLlm<'_>,
    request: &RunRequest<'_>,
) -> Result<RunOutcome, WisecrowError> {
    let first = attempt(model, request, None).await?;
    let rejection = match first {
        Ok(translations) => return Ok(RunOutcome::Translated(translations)),
        Err(rejection) => rejection,
    };
    Ok(match attempt(model, request, Some(&rejection)).await? {
        Ok(translations) => RunOutcome::Translated(translations),
        Err(rejection) => RunOutcome::Failed(rejection),
    })
}

/// One call: the translations, or the reason the answer was refused.
async fn attempt(
    model: &CachedLlm<'_>,
    request: &RunRequest<'_>,
    rejection: Option<&str>,
) -> Result<Result<Vec<String>, String>, WisecrowError> {
    let prompt = book_translation_prompt(
        request.sentences,
        request.language_name,
        request.cefr_level,
        rejection,
    );
    let expected = request.sentences.len();
    match model
        .generate(
            &prompt,
            TRANSLATION_MAX_TOKENS,
            |response| parse_translations(response, expected),
            |_| true,
        )
        .await
    {
        Ok(translations) => Ok(Ok(translations)),
        Err(CachedLlmError::Response(error))
        | Err(CachedLlmError::Request(error @ WisecrowError::LlmError(_))) => {
            Ok(Err(refusal(&error)))
        }
        Err(CachedLlmError::Request(error)) => Err(error),
    }
}

/// The rejection as the retry prompt quotes it, without the error's prefix.
fn refusal(error: &WisecrowError) -> String {
    match error {
        WisecrowError::LlmError(reason) => reason.clone(), // clone: the retry prompt owns its quote
        other => other.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::llm::LlmProvider;

    /// A provider that answers from a script and keeps every prompt.
    pub(crate) struct Scripted {
        answers: Mutex<VecDeque<Result<String, WisecrowError>>>,
        pub(crate) prompts: Mutex<Vec<String>>,
    }

    impl Scripted {
        pub(crate) fn new(answers: Vec<Result<String, WisecrowError>>) -> Self {
            Self {
                answers: Mutex::new(answers.into()),
                prompts: Mutex::new(Vec::new()),
            }
        }

        pub(crate) fn prompts(&self) -> Vec<String> {
            self.prompts.lock().expect("prompt log").clone() // clone: the test reads a snapshot
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for Scripted {
        async fn generate(&self, prompt: &str, _: u32) -> Result<String, WisecrowError> {
            self.prompts
                .lock()
                .expect("prompt log")
                .push(prompt.to_owned());
            self.answers
                .lock()
                .expect("script")
                .pop_front()
                .unwrap_or_else(|| Err(WisecrowError::LlmError("script exhausted".into())))
        }

        fn name(&self) -> &str {
            "scripted"
        }
    }

    fn answer(texts: &[&str]) -> Result<String, WisecrowError> {
        let entries: Vec<serde_json::Value> = texts
            .iter()
            .enumerate()
            .map(|(index, text)| serde_json::json!({ "n": index + 1, "text": text }))
            .collect();
        Ok(serde_json::json!({ "translations": entries }).to_string())
    }

    fn request<'a>(sentences: &'a [&'a str]) -> RunRequest<'a> {
        RunRequest {
            sentences,
            language_name: "Spanish",
            cefr_level: "A2",
        }
    }

    #[test]
    fn a_run_stops_at_sixty_sentences() {
        let texts = vec!["Short."; 100];

        assert_eq!(run_length(&texts), RUN_MAX_SENTENCES);
    }

    #[test]
    fn a_run_stops_before_six_thousand_characters() {
        let long = "x".repeat(2_500);
        let texts = vec![long.as_str(); 3];

        assert_eq!(run_length(&texts), 2);
    }

    #[test]
    fn an_overlong_sentence_is_a_run_of_its_own() {
        let long = "x".repeat(7_000);

        assert_eq!(run_length(&[long.as_str(), "Next."]), 1);
        assert_eq!(run_length(&[]), 0);
    }

    #[test]
    fn an_answer_must_number_one_translation_per_sentence() {
        let parsed = parse_translations(&answer(&["Uno.", "Dos."]).expect("json"), 2);
        assert_eq!(parsed.expect("valid"), ["Uno.", "Dos."]);

        let short = parse_translations(&answer(&["Uno."]).expect("json"), 2)
            .expect_err("a missing sentence is refused");
        assert!(
            short
                .to_string()
                .contains("returned 1 translations for 2 sentences"),
            "{short}"
        );

        let shuffled = r#"{"translations":[{"n":2,"text":"Dos."},{"n":1,"text":"Uno."}]}"#;
        assert!(
            parse_translations(shuffled, 2).is_err(),
            "numbers must run in order"
        );

        let blank = r#"{"translations":[{"n":1,"text":"  "}]}"#;
        assert!(
            parse_translations(blank, 1).is_err(),
            "an empty translation is refused"
        );
    }

    #[tokio::test]
    async fn a_mismatch_is_retried_once_with_the_rejection_quoted() {
        let provider = Scripted::new(vec![answer(&["Uno."]), answer(&["Uno.", "Dos."])]);
        let model = CachedLlm::new(&provider, None);

        let outcome = translate_run(&model, &request(&["One.", "Two."]))
            .await
            .expect("no outage");

        assert_eq!(
            outcome,
            RunOutcome::Translated(vec!["Uno.".into(), "Dos.".into()])
        );
        let prompts = provider.prompts();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[1].contains("returned 1 translations for 2 sentences"));
    }

    #[tokio::test]
    async fn a_second_rejection_fails_the_run_without_an_error() {
        let provider = Scripted::new(vec![answer(&["Uno."]), Ok("not json".into())]);
        let model = CachedLlm::new(&provider, None);

        let outcome = translate_run(&model, &request(&["One.", "Two."]))
            .await
            .expect("a rejected run is an outcome, not an error");

        assert!(matches!(outcome, RunOutcome::Failed(_)), "{outcome:?}");
    }

    #[tokio::test]
    async fn a_request_fault_is_retried_like_a_rejection() {
        let provider = Scripted::new(vec![
            Err(WisecrowError::LlmError("stopped at max_tokens".into())),
            answer(&["Uno."]),
        ]);
        let model = CachedLlm::new(&provider, None);

        let outcome = translate_run(&model, &request(&["One."]))
            .await
            .expect("no outage");

        assert_eq!(outcome, RunOutcome::Translated(vec!["Uno.".into()]));
    }

    #[tokio::test]
    async fn an_outage_stops_the_run() {
        let provider = Scripted::new(vec![Err(WisecrowError::LlmUnavailable("limit".into()))]);
        let model = CachedLlm::new(&provider, None);

        let error = translate_run(&model, &request(&["One."]))
            .await
            .expect_err("an outage is returned");

        assert!(matches!(error, WisecrowError::LlmUnavailable(_)));
        assert_eq!(provider.prompts().len(), 1, "an outage is not retried");
    }
}
