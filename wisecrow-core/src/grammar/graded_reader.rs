use crate::errors::WisecrowError;
use crate::llm::LlmProvider;
use sqlx::PgPool;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct GlossaryEntry {
    pub word: String,
    pub translation: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct GradedReader {
    pub passage: String,
    pub glossary: Vec<GlossaryEntry>,
}

impl GradedReader {
    /// Renders the reader as a Markdown document with passage and glossary.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::from("# Graded Reader\n\n");
        out.push_str(&self.passage);
        out.push_str("\n\n## Glossary\n\n");
        for entry in &self.glossary {
            out.push_str(&format!("- **{}** — {}\n", entry.word, entry.translation));
        }
        out
    }

    /// Renders the reader as a self-contained HTML document, escaping user content.
    #[must_use]
    pub fn to_html(&self) -> String {
        let escape = |s: &str| {
            s.replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
        };
        let mut out = String::from(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Graded Reader</title></head><body>\n",
        );
        out.push_str("<h1>Graded Reader</h1>\n<p>");
        out.push_str(&escape(&self.passage));
        out.push_str("</p>\n<h2>Glossary</h2>\n<ul>\n");
        for entry in &self.glossary {
            out.push_str(&format!(
                "<li><strong>{}</strong> — {}</li>\n",
                escape(&entry.word),
                escape(&entry.translation),
            ));
        }
        out.push_str("</ul>\n</body></html>\n");
        out
    }
}

/// Parses an LLM JSON response (with or without code fences) into a `GradedReader`.
///
/// # Errors
///
/// Returns `WisecrowError::LlmError` if the JSON cannot be parsed.
pub fn parse_response(response: &str) -> Result<GradedReader, WisecrowError> {
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
        .map_err(|e| WisecrowError::LlmError(format!("Failed to parse graded reader: {e}")))
}

pub struct GradedReaderRequest<'a> {
    pub native_lang: &'a str,
    pub foreign_lang: &'a str,
    pub foreign_lang_name: &'a str,
    pub user_id: i32,
    pub cefr: &'a str,
    pub seed_states: &'a [i16],
    pub seed_min_stability: Option<f32>,
    pub seed_limit: u32,
    pub length_words: u32,
}

/// Generates a graded reader passage from learned vocabulary.
///
/// # Errors
///
/// Returns an error if no learned vocabulary exists for the filters, the LLM
/// provider call fails, or the response cannot be parsed as the expected JSON.
pub async fn generate(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    request: &GradedReaderRequest<'_>,
) -> Result<GradedReader, WisecrowError> {
    let entries = crate::vocabulary::VocabularyQuery::learned(
        pool,
        request.native_lang,
        request.foreign_lang,
        request.user_id,
        request.seed_states,
        request.seed_min_stability,
        request.seed_limit,
    )
    .await?;
    if entries.is_empty() {
        return Err(WisecrowError::InvalidInput(
            "No learned vocabulary found for the given filters".to_owned(),
        ));
    }
    let pairs: Vec<(&str, &str)> = entries
        .iter()
        .map(|e| (e.to_phrase.as_str(), e.from_phrase.as_str()))
        .collect();
    let prompt = crate::llm::prompts::graded_reader_prompt(
        &pairs,
        request.cefr,
        request.foreign_lang_name,
        request.length_words,
    );
    let response = provider.generate(&prompt, 4096).await?;
    parse_response(&response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parse_happy_path() {
        let r = parse_response(
            r#"{"passage":"Hola","glossary":[{"word":"hola","translation":"hello"}]}"#,
        )
        .expect("parse failed");
        assert_eq!(r.passage, "Hola");
        assert_eq!(r.glossary.len(), 1);
    }

    #[test]
    fn parse_with_code_fence() {
        let r = parse_response(
            r#"```json
{"passage":"x","glossary":[]}
```"#,
        )
        .expect("parse failed");
        assert_eq!(r.passage, "x");
    }

    #[test]
    fn parse_with_plain_code_fence() {
        let r = parse_response(
            r#"```
{"passage":"y","glossary":[]}
```"#,
        )
        .expect("parse failed");
        assert_eq!(r.passage, "y");
    }

    #[test]
    fn parse_malformed_returns_error() {
        let result = parse_response("not json");
        assert!(matches!(result, Err(WisecrowError::LlmError(_))));
    }

    fn sample_reader() -> GradedReader {
        GradedReader {
            passage: "Hola amigo.".to_owned(),
            glossary: vec![GlossaryEntry {
                word: "amigo".to_owned(),
                translation: "friend".to_owned(),
            }],
        }
    }

    #[test]
    fn to_markdown_includes_passage_and_glossary() {
        let md = sample_reader().to_markdown();
        assert!(md.contains("Hola amigo."));
        assert!(md.contains("amigo"));
        assert!(md.contains("friend"));
        assert!(md.contains("# "));
    }

    #[test]
    fn to_html_is_self_contained_document_and_escapes_content() {
        let r = GradedReader {
            passage: "Hola <amigo>.".to_owned(),
            glossary: vec![GlossaryEntry {
                word: "amigo".to_owned(),
                translation: "friend".to_owned(),
            }],
        };
        let html = r.to_html();
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("<html"));
        assert!(html.contains("</html>"));
        assert!(
            html.contains("Hola &lt;amigo&gt;."),
            "html must escape user content"
        );
        assert!(html.contains("amigo"));
        assert!(html.contains("friend"));
    }

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_input(s in ".*") {
            let _ = parse_response(&s);
        }
    }
}
