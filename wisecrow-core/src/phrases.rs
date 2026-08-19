//! Frequent multi-word phrase extraction.
//!
//! Words are ranked by corpus occurrence, but the units a speaker actually
//! reaches for — *tha mi a' dol*, *ciamar a tha* — are multi-word. This
//! module counts 2–5-token n-grams across the corpus's distinct foreign
//! surfaces and stages the frequent ones for LLM translation and promotion
//! into ordinary translation rows.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::errors::WisecrowError;

pub const MIN_TOKENS: usize = 2;
pub const MAX_TOKENS: usize = 5;
pub const MIN_SENTENCES: i32 = 5;

/// Folds one surface's tokens into the running n-gram counts. Each surface
/// contributes at most one count per n-gram, so callers page surfaces
/// through without holding them all.
pub fn count_ngrams_into(counts: &mut HashMap<Vec<String>, i32>, tokens: &[String]) {
    let mut seen: std::collections::HashSet<&[String]> = std::collections::HashSet::new();
    for len in MIN_TOKENS..=MAX_TOKENS.min(tokens.len()) {
        for window in tokens.windows(len) {
            if seen.insert(window) {
                *counts.entry(window.to_vec()).or_insert(0) += 1;
            }
        }
    }
}

/// Extracts frequent n-grams for a language into `phrases`.
///
/// Pages through DISTINCT normalised foreign surfaces (keyset on the
/// normalised text) so corpora of millions of rows never load at once,
/// tokenising each page with the language's tokenizer and folding into the
/// running counts. Surfaces from promoted phrase rows are excluded — the
/// app's own output is not corpus evidence. Upserts n-grams seen in at
/// least [`MIN_SENTENCES`] surfaces.
///
/// # Errors
///
/// Returns an error if the language is unknown, has no tokeniser, or a
/// query fails.
pub async fn extract_phrases(
    pool: &PgPool,
    lang_code: &str,
    page_size: i64,
) -> Result<usize, WisecrowError> {
    let tokenizer = crate::preview::tokenize::for_language(lang_code)?;

    let statement = format!(
        "SELECT DISTINCT lower(btrim(t.to_phrase, '{trim}')) AS surface
         FROM translations t
         JOIN languages tl ON tl.id = t.to_language_id
         WHERE tl.code = $1
           AND NOT EXISTS (SELECT 1 FROM phrase_translations pt
                           WHERE pt.translation_id = t.id)
           AND lower(btrim(t.to_phrase, '{trim}')) > $2
         ORDER BY surface
         LIMIT $3",
        trim = crate::frequency::MATCH_TRIM_SQL
    );

    let mut counts: HashMap<Vec<String>, i32> = HashMap::new();
    let mut cursor = String::new();
    loop {
        let surfaces: Vec<(String,)> = sqlx::query_as(&statement)
            .bind(lang_code)
            .bind(&cursor)
            .bind(page_size)
            .fetch_all(pool)
            .await?;
        let Some((last,)) = surfaces.last() else {
            break;
        };
        cursor = last.clone(); // clone: keyset cursor for the next page
        for (surface,) in &surfaces {
            let tokens = tokenizer.tokenize(surface);
            count_ngrams_into(&mut counts, &tokens);
        }
    }

    let mut qualifying = 0usize;
    for (tokens, sentence_count) in counts {
        if sentence_count < MIN_SENTENCES {
            continue;
        }
        let phrase = tokens.join(" ");
        let token_count = i32::try_from(tokens.len()).unwrap_or(i32::MAX);
        sqlx::query(
            "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
             SELECT id, $2, $3, $4 FROM languages WHERE code = $1
             ON CONFLICT (language_id, phrase)
               DO UPDATE SET sentence_count = EXCLUDED.sentence_count",
        )
        .bind(lang_code)
        .bind(&phrase)
        .bind(token_count)
        .bind(sentence_count)
        .execute(pool)
        .await?;
        qualifying = qualifying.saturating_add(1);
    }
    Ok(qualifying)
}

/// Whether translation re-glosses phrases that already have a native side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    Yes,
    No,
}

#[derive(serde::Deserialize)]
struct PhraseTranslationResponse {
    translations: Vec<PhraseTranslationEntry>,
}

#[derive(serde::Deserialize)]
struct PhraseTranslationEntry {
    phrase: String,
    translation: String,
}

const TRANSLATE_BATCH: usize = 25;

/// Translates staged phrases with the LLM and promotes them into
/// `translations`.
///
/// Normal mode selects phrases with no `phrase_translations` row for the
/// native language, so re-runs are idempotent; [`Refresh::Yes`] selects
/// phrases *with* a row and updates the linked translation in place.
/// Responses are matched back to the requested phrases: hallucinated
/// phrases, empty translations and duplicates are dropped, and an omitted
/// phrase simply stays untranslated.
///
/// # Errors
///
/// Returns an error if a provider call fails, a response cannot be parsed,
/// or a write fails.
pub async fn translate_phrases(
    pool: &PgPool,
    provider: &dyn crate::llm::LlmProvider,
    lang_code: &str,
    native_code: &str,
    limit: u32,
    refresh: Refresh,
) -> Result<usize, WisecrowError> {
    let lang_name = crate::cli::SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(code, _)| *code == lang_code)
        .map(|(_, name)| *name)
        .ok_or_else(|| WisecrowError::InvalidInput(format!("Unknown language: {lang_code}")))?;
    let native_name = crate::cli::SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(code, _)| *code == native_code)
        .map(|(_, name)| *name)
        .ok_or_else(|| WisecrowError::InvalidInput(format!("Unknown language: {native_code}")))?;

    let membership = match refresh {
        Refresh::No => "NOT EXISTS",
        Refresh::Yes => "EXISTS",
    };
    let statement = format!(
        "SELECT p.id, p.phrase, p.sentence_count
         FROM phrases p
         WHERE p.language_id = (SELECT id FROM languages WHERE code = $1)
           AND {membership} (SELECT 1 FROM phrase_translations pt
                             WHERE pt.phrase_id = p.id
                               AND pt.native_language_id =
                                   (SELECT id FROM languages WHERE code = $2))
         ORDER BY p.sentence_count DESC
         LIMIT $3"
    );
    let pending: Vec<(i32, String, i32)> = sqlx::query_as(&statement)
        .bind(lang_code)
        .bind(native_code)
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await?;

    let mut written = 0usize;
    for batch in pending.chunks(TRANSLATE_BATCH) {
        let requested: Vec<String> = batch.iter().map(|(_, phrase, _)| phrase.clone()).collect();
        let prompt =
            crate::llm::prompts::phrase_translation_prompt(&requested, lang_name, native_name);
        // A provider or parse failure costs this batch alone. Selection keys on
        // the absence of a `phrase_translations` row, so the next run picks the
        // skipped phrases up; aborting instead would discard the batches still
        // queued behind a single malformed response.
        let parsed: PhraseTranslationResponse =
            match provider.generate(&prompt, 2048).await.and_then(|response| {
                crate::llm::parse_fenced_json(&response, "phrase-translation JSON")
            }) {
                Ok(parsed) => parsed,
                Err(e) => {
                    tracing::warn!(
                        "Skipping batch of {} {lang_code} phrases: {e}",
                        requested.len()
                    );
                    continue;
                }
            };

        for (phrase, translation) in pair_with_requested(&requested, parsed) {
            let Some((phrase_id, _, sentence_count)) = batch.iter().find(|(_, p, _)| *p == phrase)
            else {
                continue;
            };
            promote(
                pool,
                *phrase_id,
                &phrase,
                &translation,
                *sentence_count,
                lang_code,
                native_code,
            )
            .await?;
            written = written.saturating_add(1);
        }
    }
    Ok(written)
}

/// Accepts only requested phrases with non-empty translations; the first
/// answer wins where the model repeats itself.
fn pair_with_requested(
    requested: &[String],
    parsed: PhraseTranslationResponse,
) -> Vec<(String, String)> {
    let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
    parsed
        .translations
        .into_iter()
        .filter(|entry| !entry.translation.trim().is_empty())
        .filter_map(|entry| {
            let offered = entry.phrase.trim().to_lowercase();
            requested
                .iter()
                .find(|asked| **asked == offered)
                .filter(|asked| taken.insert((*asked).clone()))
                .map(|asked| (asked.clone(), entry.translation.trim().to_owned()))
        })
        .collect()
}

/// Writes the native side and keeps the linked `translations` row in step,
/// inside one transaction so a refresh can never leave the pair split.
async fn promote(
    pool: &PgPool,
    phrase_id: i32,
    phrase: &str,
    translation: &str,
    sentence_count: i32,
    lang_code: &str,
    native_code: &str,
) -> Result<(), WisecrowError> {
    let mut tx = pool.begin().await?;

    let (link_id, existing): (i32, Option<i32>) = sqlx::query_as(
        "INSERT INTO phrase_translations (phrase_id, native_language_id, translation)
         SELECT $1, id, $2 FROM languages WHERE code = $3
         ON CONFLICT (phrase_id, native_language_id)
           DO UPDATE SET translation = EXCLUDED.translation,
                         translated_at = CURRENT_TIMESTAMP
         RETURNING id, translation_id",
    )
    .bind(phrase_id)
    .bind(translation)
    .bind(native_code)
    .fetch_one(&mut *tx)
    .await?;

    if let Some(translation_id) = existing {
        sqlx::query(
            "UPDATE translations SET from_phrase = $1, corpus_frequency = $2 WHERE id = $3",
        )
        .bind(translation)
        .bind(sentence_count)
        .bind(translation_id)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO translations
                 (from_language_id, from_phrase, to_language_id, to_phrase, corpus_frequency)
             SELECT nl.id, $1, fl.id, $2, $3
             FROM languages nl, languages fl
             WHERE nl.code = $4 AND fl.code = $5
             ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
               DO UPDATE SET corpus_frequency = EXCLUDED.corpus_frequency",
        )
        .bind(translation)
        .bind(phrase)
        .bind(sentence_count)
        .bind(native_code)
        .bind(lang_code)
        .execute(&mut *tx)
        .await?;
        let (translation_id,): (i32,) = sqlx::query_as(
            "SELECT t.id FROM translations t
             WHERE t.from_phrase = $1 AND t.to_phrase = $2
               AND t.from_language_id = (SELECT id FROM languages WHERE code = $3)
               AND t.to_language_id = (SELECT id FROM languages WHERE code = $4)",
        )
        .bind(translation)
        .bind(phrase)
        .bind(native_code)
        .bind(lang_code)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE phrase_translations SET translation_id = $1 WHERE id = $2")
            .bind(translation_id)
            .bind(link_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn count_all(surfaces: &[Vec<String>]) -> HashMap<Vec<String>, i32> {
        let mut counts = HashMap::new();
        for tokens in surfaces {
            count_ngrams_into(&mut counts, tokens);
        }
        counts
    }

    #[test]
    fn counts_bigram_across_surfaces() {
        let surfaces = vec![s(&["tha", "mi", "sgìth"]), s(&["tha", "mi", "toilichte"])];
        assert_eq!(count_all(&surfaces).get(&s(&["tha", "mi"])), Some(&2));
    }

    #[test]
    fn repeated_ngram_in_one_surface_counts_once() {
        assert_eq!(
            count_all(&[s(&["a", "b", "a", "b"])]).get(&s(&["a", "b"])),
            Some(&1)
        );
    }

    #[test]
    fn two_token_surfaces_contribute() {
        assert_eq!(
            count_all(&[s(&["tha", "mi"])]).get(&s(&["tha", "mi"])),
            Some(&1)
        );
    }

    #[test]
    fn ngram_sizes_stay_within_bounds() {
        let counts = count_all(&[s(&["a", "b", "c", "d", "e", "f"])]);
        assert!(counts
            .keys()
            .all(|k| (MIN_TOKENS..=MAX_TOKENS).contains(&k.len())));
    }

    #[test]
    fn counting_is_incremental_across_calls() {
        let mut counts = HashMap::new();
        count_ngrams_into(&mut counts, &s(&["tha", "mi", "sgìth"]));
        count_ngrams_into(&mut counts, &s(&["tha", "mi", "toilichte"]));
        assert_eq!(counts.get(&s(&["tha", "mi"])), Some(&2));
    }
}
