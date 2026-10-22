//! Notes on the word forms of a language's book translations.
//!
//! One note per language, learner language and normalised form, shared by
//! every book. A form the model will not describe after two attempts is
//! recorded as failed, so it is never asked about again and the reader
//! shows its note as pending.

use serde::Deserialize;

use crate::errors::WisecrowError;
use crate::llm::cache::{CachedLlm, CachedLlmError};
use crate::llm::parse_fenced_json;
use crate::llm::prompts::book_gloss_prompt;

/// Forms sent in one call.
pub const GLOSS_BATCH: usize = 50;
/// Answer budget for one batch of fifty notes.
pub const GLOSS_MAX_TOKENS: u32 = 8_192;
const MAX_MEANING_CHARS: usize = 80;
const MAX_LEMMA_CHARS: usize = 64;
const MAX_CLASS_CHARS: usize = 24;
const MAX_NOTE_CHARS: usize = 200;

/// One validated note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordNote {
    pub form: String,
    pub meaning: String,
    pub lemma: String,
    pub class: String,
    pub note: String,
}

/// An answer read against the forms it was asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteAnswer {
    pub notes: Vec<WordNote>,
    /// Forms with no usable note, and why.
    pub rejected: Vec<(String, String)>,
}

/// One batch to note.
#[derive(Debug, Clone, Copy)]
pub struct GlossRequest<'a> {
    /// Each form with one sentence that uses it.
    pub forms: &'a [(&'a str, &'a str)],
    pub language_name: &'a str,
    pub native_name: &'a str,
}

/// What a batch yielded after at most two calls.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlossOutcome {
    pub notes: Vec<WordNote>,
    /// Forms still without a usable note after the second call.
    pub failed: Vec<String>,
}

#[derive(Deserialize)]
struct RawAnswer {
    notes: Vec<RawNote>,
}

#[derive(Deserialize)]
struct RawNote {
    form: String,
    meaning: String,
    lemma: String,
    class: String,
    note: String,
}

/// Reads an answer for exactly `forms`.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmError`] when the answer is not the JSON
/// shape asked for; a missing or invalid note is a rejection, not an error.
pub fn parse_notes(response: &str, forms: &[&str]) -> Result<NoteAnswer, WisecrowError> {
    let raw: RawAnswer = parse_fenced_json(response, "book word notes")?;
    let mut answer = NoteAnswer {
        notes: Vec::new(),
        rejected: Vec::new(),
    };
    for form in forms {
        let Some(entry) = raw.notes.iter().find(|entry| entry.form == *form) else {
            answer
                .rejected
                .push(((*form).to_owned(), "missing".to_owned()));
            continue;
        };
        match validated(entry) {
            Ok(note) => answer.notes.push(note),
            Err(reason) => answer.rejected.push(((*form).to_owned(), reason)),
        }
    }
    Ok(answer)
}

fn validated(entry: &RawNote) -> Result<WordNote, String> {
    let field = |name: &str, value: &str, limit: usize| -> Result<String, String> {
        let value = value.trim();
        if value.is_empty() || value.chars().count() > limit {
            return Err(format!("its {name} is empty or too long"));
        }
        Ok(value.to_owned())
    };
    Ok(WordNote {
        form: entry.form.clone(), // clone: the note owns its form
        meaning: field("meaning", &entry.meaning, MAX_MEANING_CHARS)?,
        lemma: field("lemma", &entry.lemma, MAX_LEMMA_CHARS)?,
        class: field("class", &entry.class, MAX_CLASS_CHARS)?,
        note: field("note", &entry.note, MAX_NOTE_CHARS)?,
    })
}

/// Notes one batch, asking once more about the forms the first answer did
/// not describe.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmUnavailable`], a configuration error or a
/// database error from the cache.
pub async fn gloss_batch(
    model: &CachedLlm<'_>,
    request: &GlossRequest<'_>,
) -> Result<GlossOutcome, WisecrowError> {
    let mut outcome = GlossOutcome::default();
    let retry: Vec<(&str, &str)>;
    let reasons: String;
    match attempt(model, request, request.forms, None).await? {
        Ok(answer) => {
            outcome.notes = answer.notes;
            retry = request
                .forms
                .iter()
                .copied()
                .filter(|(form, _)| {
                    answer
                        .rejected
                        .iter()
                        .any(|(rejected, _)| rejected.as_str() == *form)
                })
                .collect();
            reasons = answer
                .rejected
                .iter()
                .map(|(form, reason)| format!("{form}: {reason}"))
                .collect::<Vec<_>>()
                .join("; ");
        }
        Err(reason) => {
            retry = request.forms.to_vec();
            reasons = reason;
        }
    }
    if retry.is_empty() {
        return Ok(outcome);
    }
    match attempt(model, request, &retry, Some(&reasons)).await? {
        Ok(answer) => {
            outcome.notes.extend(answer.notes);
            outcome.failed = answer.rejected.into_iter().map(|(form, _)| form).collect();
        }
        Err(_) => {
            outcome.failed = retry.iter().map(|(form, _)| (*form).to_owned()).collect();
        }
    }
    Ok(outcome)
}

/// One call for `forms`: the answer, or why it could not be read at all.
async fn attempt(
    model: &CachedLlm<'_>,
    request: &GlossRequest<'_>,
    forms: &[(&str, &str)],
    rejection: Option<&str>,
) -> Result<Result<NoteAnswer, String>, WisecrowError> {
    let prompt = book_gloss_prompt(forms, request.language_name, request.native_name, rejection)?;
    let names: Vec<&str> = forms.iter().map(|(form, _)| *form).collect();
    match model
        .generate(
            &prompt,
            GLOSS_MAX_TOKENS,
            |response| parse_notes(response, &names),
            |answer| answer.rejected.is_empty(),
        )
        .await
    {
        Ok(answer) => Ok(Ok(answer)),
        Err(CachedLlmError::Response(error))
        | Err(CachedLlmError::Request(error @ WisecrowError::LlmError(_))) => {
            Ok(Err(error.to_string()))
        }
        Err(CachedLlmError::Request(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::translate::tests::Scripted;

    fn note(form: &str) -> serde_json::Value {
        serde_json::json!({
            "form": form,
            "meaning": format!("meaning of {form}"),
            "lemma": form,
            "class": "noun",
            "note": "Singular."
        })
    }

    fn answer(forms: &[&str]) -> Result<String, WisecrowError> {
        let notes: Vec<serde_json::Value> = forms.iter().map(|form| note(form)).collect();
        Ok(serde_json::json!({ "notes": notes }).to_string())
    }

    fn request<'a>(forms: &'a [(&'a str, &'a str)]) -> GlossRequest<'a> {
        GlossRequest {
            forms,
            language_name: "Scottish Gaelic",
            native_name: "English",
        }
    }

    #[test]
    fn an_answer_is_read_for_the_batch_forms_only() {
        let parsed = parse_notes(
            &answer(&["cat", "dog", "fish"]).expect("json"),
            &["cat", "dog"],
        )
        .expect("json parses");

        let forms: Vec<&str> = parsed.notes.iter().map(|note| note.form.as_str()).collect();
        assert_eq!(forms, ["cat", "dog"], "a form not asked about is ignored");
        assert!(parsed.rejected.is_empty());
    }

    #[test]
    fn a_missing_or_empty_note_is_rejected_with_its_reason() {
        let response = serde_json::json!({
            "notes": [note("cat"), {
                "form": "dog", "meaning": " ", "lemma": "dog", "class": "noun", "note": "x"
            }]
        })
        .to_string();

        let parsed = parse_notes(&response, &["cat", "dog", "fish"]).expect("json parses");

        assert_eq!(parsed.notes.len(), 1);
        assert_eq!(
            parsed.rejected,
            vec![
                (
                    "dog".to_owned(),
                    "its meaning is empty or too long".to_owned()
                ),
                ("fish".to_owned(), "missing".to_owned()),
            ]
        );
    }

    #[test]
    fn an_answer_that_is_not_json_is_an_error() {
        assert!(parse_notes("no notes today", &["cat"]).is_err());
    }

    #[tokio::test]
    async fn rejected_forms_are_asked_again_and_then_fail() {
        let provider = Scripted::new(vec![answer(&["cat"]), answer(&[])]);
        let model = CachedLlm::new(&provider, None);
        let forms = [("cat", "The cat sat."), ("dog", "The dog ran.")];

        let outcome = gloss_batch(&model, &request(&forms))
            .await
            .expect("no outage");

        assert_eq!(outcome.notes.len(), 1);
        assert_eq!(outcome.failed, ["dog"]);
        let prompts = provider.prompts();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[1].contains("dog: missing"));
        assert!(
            !prompts[1].contains(r#""form":"cat""#),
            "only the rejected forms are asked again"
        );
    }

    #[tokio::test]
    async fn a_complete_answer_needs_one_call() {
        let provider = Scripted::new(vec![answer(&["cat", "dog"])]);
        let model = CachedLlm::new(&provider, None);
        let forms = [("cat", "The cat sat."), ("dog", "The dog ran.")];

        let outcome = gloss_batch(&model, &request(&forms))
            .await
            .expect("no outage");

        assert_eq!(outcome.notes.len(), 2);
        assert!(outcome.failed.is_empty());
        assert_eq!(provider.prompts().len(), 1);
    }

    #[tokio::test]
    async fn an_unreadable_answer_twice_fails_every_form() {
        let provider = Scripted::new(vec![Ok("nonsense".into()), Ok("still nonsense".into())]);
        let model = CachedLlm::new(&provider, None);
        let forms = [("cat", "The cat sat.")];

        let outcome = gloss_batch(&model, &request(&forms))
            .await
            .expect("no outage");

        assert!(outcome.notes.is_empty());
        assert_eq!(outcome.failed, ["cat"]);
    }

    #[tokio::test]
    async fn an_outage_is_returned() {
        let provider = Scripted::new(vec![Err(WisecrowError::LlmUnavailable("limit".into()))]);
        let model = CachedLlm::new(&provider, None);
        let forms = [("cat", "The cat sat.")];

        let error = gloss_batch(&model, &request(&forms))
            .await
            .expect_err("an outage");

        assert!(matches!(error, WisecrowError::LlmUnavailable(_)));
    }
}
