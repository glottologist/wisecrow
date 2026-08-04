//! Translations for deck words whose corpus pairing nothing corroborates.
//!
//! A deck teaches a word by showing the native phrase the corpus stored beside
//! it. Where the corpus stored exactly one, there is nothing to choose between
//! and no ordering rule can help — measured on Gaelic, 200 of the deck's 315
//! words are in that position, which is why `Die! → An!` heads the Irish deck.
//!
//! The corpus is not short of evidence, only of *single-word* evidence:
//! `dìreach` is chosen from one row and appears in 4,007. Rather than mine those
//! sentences for an alignment this codebase has no way to compute, the words
//! with no corroboration are translated once and cached, and the deck prefers
//! that translation over an uncorroborated corpus partner.
//!
//! Corroboration is `agreement`, the count of rows sharing a normalised pairing,
//! which [`crate::vocabulary::VocabularyRepository::unlearned`] already computes
//! to choose between candidates. An agreement of 1 means the corpus said this
//! once and never again.

use crate::errors::WisecrowError;
use crate::llm::LlmProvider;
use sqlx::PgPool;

/// Words sent to the model in one request. The prompt asks for one to three
/// words back per entry, so a batch this size stays well inside the 2048-token
/// budget while keeping the number of round trips down.
const GLOSS_BATCH: usize = 50;

/// Returns the deck words for `foreign_lang` whose chosen pairing occurs exactly
/// once and which have no gloss yet, most frequent first.
///
/// This deliberately mirrors `unlearned`: the same `corpus_frequency > 1`
/// filter, the same normalisation, the same `agreement` window and the same
/// choice of best row per word. A word is only worth glossing if it is the row
/// the deck would actually serve that lacks corroboration.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn uncorroborated_words(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
    limit: u32,
) -> Result<Vec<String>, WisecrowError> {
    let statement = format!(
        "SELECT best.norm_to
           FROM (
             SELECT DISTINCT ON (norm_to) norm_to, frequency, agreement
             FROM (
               SELECT t.id,
                      t.corpus_frequency AS frequency,
                      lower(btrim(t.to_phrase, '{trim}')) AS norm_to,
                      count(*) OVER (
                        PARTITION BY lower(btrim(t.to_phrase, '{trim}')),
                                     lower(btrim(t.from_phrase, '{trim}'))
                      ) AS agreement,
                      t.to_phrase,
                      t.from_phrase
               FROM translations t
               JOIN languages fl ON t.from_language_id = fl.id
               JOIN languages tl ON t.to_language_id = tl.id
               WHERE fl.code = $1 AND tl.code = $2
                 AND t.corpus_frequency > 1
                 AND LENGTH(t.from_phrase) BETWEEN 2 AND 200
                 AND LENGTH(t.to_phrase) BETWEEN 2 AND 200
             ) scored
             ORDER BY norm_to,
                      frequency DESC,
                      agreement DESC,
                      LENGTH(to_phrase),
                      LENGTH(from_phrase),
                      id
           ) best
           LEFT JOIN word_glosses g
             ON g.lang_code = $2 AND g.native_lang = $1 AND g.word = best.norm_to
          WHERE best.agreement = 1 AND g.id IS NULL
          ORDER BY best.frequency DESC
          LIMIT $3",
        trim = crate::frequency::MATCH_TRIM_SQL
    );

    let words: Vec<(String,)> = sqlx::query_as(&statement)
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await?;

    Ok(words.into_iter().map(|(word,)| word).collect())
}

/// Translates `words` in batches and stores the results, returning how many were
/// written.
///
/// A batch whose response omits a word simply leaves it unglossed; it will be
/// selected again next run rather than stored wrong. The whole batch is not
/// failed for one missing entry, because the model returning nine of ten is
/// commonplace and the tenth costs one more request rather than a lost run.
///
/// # Errors
///
/// Returns an error if the provider call fails, a response cannot be parsed, or
/// a write fails.
pub async fn gloss_words(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    words: &[String],
    native_lang: &str,
    foreign_lang: &str,
    native_lang_name: &str,
    foreign_lang_name: &str,
) -> Result<usize, WisecrowError> {
    let mut written = 0usize;
    for batch in words.chunks(GLOSS_BATCH) {
        let prompt =
            crate::llm::prompts::unknown_words_prompt(batch, foreign_lang_name, native_lang_name);
        let response = provider.generate(&prompt, 2048).await?;
        let parsed: crate::llm::GlossResponse =
            crate::llm::parse_fenced_json(&response, "word-gloss JSON")?;

        for (word, translation) in pair_with_requested(batch, parsed) {
            sqlx::query(
                "INSERT INTO word_glosses (lang_code, word, native_lang, translation)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (lang_code, word, native_lang) DO UPDATE
                   SET translation = EXCLUDED.translation,
                       created_at = CURRENT_TIMESTAMP",
            )
            .bind(foreign_lang)
            .bind(&word)
            .bind(native_lang)
            .bind(&translation)
            .execute(pool)
            .await?;
            written = written.saturating_add(1);
        }
    }
    Ok(written)
}

/// Keeps only the returned glosses that answer a word actually asked about, and
/// restores the spelling that was asked.
///
/// Models echo the word back with its own capitalisation and punctuation, and a
/// gloss stored under a spelling the deck does not use would never be found: the
/// deck joins on the normalised form. Matching is therefore on the normalised
/// form and the requested spelling is what gets stored. Entries answering
/// nothing that was asked are dropped rather than stored under a key no deck
/// will look up.
fn pair_with_requested(
    requested: &[String],
    parsed: crate::llm::GlossResponse,
) -> Vec<(String, String)> {
    parsed
        .glosses
        .into_iter()
        .filter(|entry| !entry.translation.trim().is_empty())
        .filter_map(|entry| {
            let normalised = crate::lang::normalise_for_match(&entry.word);
            requested
                .iter()
                .find(|asked| **asked == normalised)
                .map(|asked| (asked.clone(), entry.translation.trim().to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{GlossEntry, GlossResponse};
    use rstest::rstest;

    fn response(pairs: &[(&str, &str)]) -> GlossResponse {
        GlossResponse {
            glosses: pairs
                .iter()
                .map(|(word, translation)| GlossEntry {
                    word: (*word).to_owned(),
                    translation: (*translation).to_owned(),
                })
                .collect(),
        }
    }

    #[rstest]
    #[case(&["dìreach"], &[("dìreach", "exactly")], &[("dìreach", "exactly")], "the plain case")]
    #[case(
        &["dìreach"],
        &[("Dìreach.", "exactly")],
        &[("dìreach", "exactly")],
        "the model echoes its own capitalisation and punctuation back"
    )]
    #[case(
        &["agus"],
        &[("beannachd", "blessing")],
        &[],
        "an answer to a word nobody asked about is dropped rather than stored"
    )]
    #[case(
        &["agus"],
        &[("agus", "   ")],
        &[],
        "an empty translation is worse than none, because the deck would show it"
    )]
    #[case(
        &["agus", "tha"],
        &[("agus", "and")],
        &[("agus", "and")],
        "a short response glosses what it answered and leaves the rest for next run"
    )]
    fn responses_are_paired_with_the_words_that_were_asked(
        #[case] requested: &[&str],
        #[case] returned: &[(&str, &str)],
        #[case] expected: &[(&str, &str)],
        #[case] why: &str,
    ) {
        let asked: Vec<String> = requested.iter().map(|w| (*w).to_owned()).collect();
        let paired = pair_with_requested(&asked, response(returned));
        let expected: Vec<(String, String)> = expected
            .iter()
            .map(|(w, t)| ((*w).to_owned(), (*t).to_owned()))
            .collect();
        assert_eq!(paired, expected, "{why}");
    }
}
