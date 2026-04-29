use crate::errors::WisecrowError;
use crate::llm::LlmProvider;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

pub(crate) fn hash_sentence(sentence: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(sentence.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Returns a cached gloss for `(sentence, lang_code)` or generates one via the
/// LLM provider, persisting the result for future lookups.
///
/// # Errors
///
/// Returns an error if the database query fails or the LLM provider returns an error.
pub async fn generate_or_lookup(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    sentence: &str,
    lang_code: &str,
    lang_name: &str,
) -> Result<String, WisecrowError> {
    generate_or_lookup_with_refresh(pool, provider, sentence, lang_code, lang_name, false).await
}

/// Like `generate_or_lookup` but with explicit cache control. When `refresh` is
/// true, any cached value is discarded and the LLM is re-prompted; the new
/// gloss replaces the cached one.
///
/// # Errors
///
/// Returns an error if the database query fails or the LLM provider returns an error.
pub async fn generate_or_lookup_with_refresh(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    sentence: &str,
    lang_code: &str,
    lang_name: &str,
    refresh: bool,
) -> Result<String, WisecrowError> {
    let hash = hash_sentence(sentence);
    if !refresh {
        if let Some(cached) = sqlx::query_scalar::<_, String>(
            "SELECT gloss_text FROM glosses WHERE sentence_hash = $1 AND lang_code = $2",
        )
        .bind(&hash)
        .bind(lang_code)
        .fetch_optional(pool)
        .await?
        {
            return Ok(cached);
        }
    }

    let prompt = crate::llm::prompts::gloss_prompt(sentence, lang_name);
    let gloss = provider.generate(&prompt, 1024).await?;

    sqlx::query(
        "INSERT INTO glosses (sentence_hash, lang_code, gloss_text) VALUES ($1, $2, $3)
         ON CONFLICT (sentence_hash, lang_code) DO UPDATE SET
           gloss_text = EXCLUDED.gloss_text,
           created_at = CURRENT_TIMESTAMP",
    )
    .bind(&hash)
    .bind(lang_code)
    .bind(&gloss)
    .execute(pool)
    .await?;

    Ok(gloss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_64_hex_chars() {
        let h = hash_sentence("hello");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(hash_sentence("x"), hash_sentence("x"));
    }

    #[test]
    fn hash_differs_for_different_inputs() {
        assert_ne!(hash_sentence("a"), hash_sentence("b"));
    }
}
