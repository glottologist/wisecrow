use crate::errors::WisecrowError;
use crate::llm::LlmProvider;
use sqlx::PgPool;

#[derive(Debug, PartialEq, Eq)]
pub enum Status {
    Known,
    Learning,
    New,
    Unknown,
}

#[derive(Debug)]
pub struct AnnotatedToken {
    pub token: String,
    pub frequency: Option<i32>,
    pub status: Status,
    pub llm_translation: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct LlmGlossEntry {
    word: String,
    translation: String,
}

#[derive(Debug, serde::Deserialize)]
struct LlmGlossResponse {
    glosses: Vec<LlmGlossEntry>,
}

/// Asks the LLM to translate the unknown tokens (status `Unknown`) into the
/// native language and writes results back into the matching `AnnotatedToken`
/// entries via `llm_translation`.
///
/// # Errors
///
/// Returns an error if the LLM call fails or the response cannot be parsed.
pub async fn enrich_unknowns_with_llm(
    annotated: &mut [AnnotatedToken],
    provider: &dyn LlmProvider,
    foreign_lang_name: &str,
    native_lang_name: &str,
) -> Result<(), WisecrowError> {
    let unknowns: Vec<String> = annotated
        .iter()
        .filter(|a| matches!(a.status, Status::Unknown))
        .map(|a| a.token.clone()) // clone: collected for prompt input
        .collect();
    if unknowns.is_empty() {
        return Ok(());
    }
    let prompt =
        crate::llm::prompts::unknown_words_prompt(&unknowns, foreign_lang_name, native_lang_name);
    let response = provider.generate(&prompt, 2048).await?;

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
    let parsed: LlmGlossResponse = serde_json::from_str(json_str)
        .map_err(|e| WisecrowError::LlmError(format!("Failed to parse unknown-words JSON: {e}")))?;

    let lookup: std::collections::HashMap<String, String> = parsed
        .glosses
        .into_iter()
        .map(|e| (e.word, e.translation))
        .collect();

    for entry in annotated
        .iter_mut()
        .filter(|a| matches!(a.status, Status::Unknown))
    {
        if let Some(translation) = lookup.get(&entry.token) {
            entry.llm_translation = Some(translation.clone()); // clone: HashMap value
        }
    }
    Ok(())
}

/// Annotates each token with its frequency from the corpus and SRS state for
/// the given user. Tokens not present in the corpus get `Status::Unknown`;
/// tokens present but without a card for this user get `Status::New`.
///
/// # Errors
///
/// Returns an error if the database query fails.
pub async fn annotate_tokens(
    pool: &PgPool,
    foreign_lang: &str,
    user_id: i32,
    tokens: &[String],
) -> Result<Vec<AnnotatedToken>, WisecrowError> {
    let rows = sqlx::query_as::<_, (String, Option<i32>, Option<i16>)>(
        "WITH input(token) AS (SELECT unnest($1::text[]))
         SELECT input.token, t.frequency, c.state
         FROM input
         LEFT JOIN translations t
           ON t.to_phrase = input.token
          AND t.to_language_id = (SELECT id FROM languages WHERE code = $2)
         LEFT JOIN cards c
           ON c.translation_id = t.id AND c.user_id = $3",
    )
    .bind(tokens)
    .bind(foreign_lang)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(token, freq, state)| {
            let status = match (freq, state) {
                (None, _) => Status::Unknown,
                (Some(_), Some(2)) => Status::Known,
                (Some(_), Some(1)) => Status::Learning,
                (Some(_), Some(0) | None) => Status::New,
                (Some(_), Some(_)) => Status::Learning,
            };
            AnnotatedToken {
                token,
                frequency: freq,
                status,
                llm_translation: None,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct StubProvider {
        response: String,
        captured_prompt: Mutex<Option<String>>,
    }

    #[async_trait]
    impl LlmProvider for StubProvider {
        async fn generate(&self, prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
            *self.captured_prompt.lock().expect("lock") = Some(prompt.to_owned());
            Ok(self.response.clone()) // clone: response stored for repeat calls
        }
        fn name(&self) -> &str {
            "stub"
        }
    }

    #[tokio::test]
    async fn enrich_unknowns_writes_translations() {
        let mut annotated = vec![
            AnnotatedToken {
                token: "casa".to_owned(),
                frequency: Some(100),
                status: Status::Known,
                llm_translation: None,
            },
            AnnotatedToken {
                token: "desconocido".to_owned(),
                frequency: None,
                status: Status::Unknown,
                llm_translation: None,
            },
        ];
        let provider = StubProvider {
            response: r#"{"glosses":[{"word":"desconocido","translation":"unknown"}]}"#.to_owned(),
            captured_prompt: Mutex::new(None),
        };
        enrich_unknowns_with_llm(&mut annotated, &provider, "Spanish", "English")
            .await
            .expect("enrich failed");
        assert_eq!(annotated[0].llm_translation, None, "known unchanged");
        assert_eq!(
            annotated[1].llm_translation.as_deref(),
            Some("unknown"),
            "unknown gets translation"
        );
    }

    #[tokio::test]
    async fn enrich_unknowns_no_op_when_no_unknowns() {
        let mut annotated = vec![AnnotatedToken {
            token: "casa".to_owned(),
            frequency: Some(100),
            status: Status::Known,
            llm_translation: None,
        }];
        let provider = StubProvider {
            response: "should not be called".to_owned(),
            captured_prompt: Mutex::new(None),
        };
        enrich_unknowns_with_llm(&mut annotated, &provider, "Spanish", "English")
            .await
            .expect("no-op enrich failed");
        assert!(provider.captured_prompt.lock().expect("lock").is_none());
    }

    #[tokio::test]
    async fn enrich_unknowns_handles_code_fence_response() {
        let mut annotated = vec![AnnotatedToken {
            token: "x".to_owned(),
            frequency: None,
            status: Status::Unknown,
            llm_translation: None,
        }];
        let provider = StubProvider {
            response: "```json\n{\"glosses\":[{\"word\":\"x\",\"translation\":\"y\"}]}\n```"
                .to_owned(),
            captured_prompt: Mutex::new(None),
        };
        enrich_unknowns_with_llm(&mut annotated, &provider, "F", "N")
            .await
            .expect("fence handling");
        assert_eq!(annotated[0].llm_translation.as_deref(), Some("y"));
    }
}
