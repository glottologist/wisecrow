//! Scoring and selection for sentence cards.
//!
//! The word deck can only teach from rows that *are* one word, and most of a
//! corpus is not. Irish holds 1,629 single-word rows in 4,328,254; the other
//! 99.96% are sentences that no path reaches. This module scores those rows so
//! they can be ordered, and selects among them so that a learner is shown a
//! sentence they can almost entirely read.
//!
//! It does not replace the word deck, it feeds from it. `unlearned` decides
//! *which* word comes next; [`sentence_for_word`] finds a sentence that teaches
//! that word and whose every other token the learner already knows. That is the
//! i+1 condition, and it is why the two decks compose rather than compete.

use crate::errors::WisecrowError;
use sqlx::PgPool;

/// Rows fetched per page when scoring, matching the paging the frequency
/// derivation uses over the same table.
const SCORE_PAGE_SIZE: i64 = 5000;

/// The longest sentence worth serving as a card.
///
/// Not a linguistic bound but a pedagogical one: a learner meeting one new word
/// needs the rest of the sentence to be holdable in the head at once. Subtitle
/// corpora also lengthen sharply at the tail, where the rows are transcription
/// runs rather than sentences.
const MAX_SENTENCE_TOKENS: usize = 12;

/// The shortest sentence worth serving as a card.
///
/// The score is the *minimum* count over a row's tokens, so a further token can
/// only lower it, and ordering by score descending is therefore ordering by
/// shortness. Measured on the scored Gaelic rows, the highest score at each
/// length falls monotonically — 20,134 at two tokens, 8,308 at three, 6,283 at
/// four, 509 at twelve — so whatever the corpus holds, the head of the ordering
/// is whatever is shortest.
///
/// Below four tokens a subtitle corpus offers fragments rather than sentences.
/// The Gaelic head was `Tha a?`, `Tha an` and `Tha e ...`, which carry no
/// context the word deck does not already teach, and whose native partners were
/// `Have You Say?` and `Die!`. Four is where the same corpus begins giving whole
/// clauses — `An e sin e?`, `Tha mi an seo.` — and reaching them costs 19,085 of
/// 64,741 scored Gaelic rows.
const MIN_SENTENCE_TOKENS: usize = 4;

/// The least share of a sentence's tokens that must be distinct.
///
/// Requiring merely two distinct tokens admits `Chan eil, chan eil, chan eil.`,
/// which is six tokens of two words. Repetition is not context however long the
/// row runs. A half admits `An e sin e?`, where one word of four recurs once,
/// and declines 258 further Gaelic rows.
const MIN_DISTINCT_SHARE: (usize, usize) = (1, 2);

/// A sentence card: the pair, and how approachable the sentence is.
#[derive(Debug, PartialEq, Eq)]
pub struct SentenceCard {
    pub translation_id: i32,
    pub native_phrase: String,
    pub foreign_phrase: String,
    pub score: i32,
}

/// Scores every unscored multi-token row for `lang_code`, returning how many
/// were written.
///
/// The score is the corpus count of the sentence's rarest token, because a
/// sentence is as hard as its hardest word. A sentence holding a token the
/// corpus counts have never seen is left unscored rather than given a floor
/// value: an unrecognised token is usually corruption, and this is the one place
/// that can decline to serve it without a rule enumerating the shapes.
///
/// # Errors
///
/// Returns an error if the language is unknown, has no tokeniser, or a query
/// fails.
pub async fn score_sentences(pool: &PgPool, lang_code: &str) -> Result<usize, WisecrowError> {
    if !crate::lang::is_valid_code(lang_code) {
        return Err(WisecrowError::InvalidInput(format!(
            "Invalid language code: {lang_code}"
        )));
    }
    let tokenizer = crate::preview::tokenize::for_language(lang_code)?;
    let counts = crate::frequency::FrequencyUpdater::derive_counts(pool, lang_code).await?;

    let mut scored = 0usize;
    let mut after_id = 0i32;
    loop {
        let rows: Vec<(i32, String)> = sqlx::query_as(
            "SELECT t.id, t.to_phrase
               FROM translations t
               JOIN languages tl ON tl.id = t.to_language_id
              WHERE tl.code = $1 AND t.id > $2 AND t.sentence_score IS NULL
              ORDER BY t.id
              LIMIT $3",
        )
        .bind(lang_code)
        .bind(after_id)
        .bind(SCORE_PAGE_SIZE)
        .fetch_all(pool)
        .await?;

        let Some((last_id, _)) = rows.last() else {
            return Ok(scored);
        };
        after_id = *last_id;

        let batch: Vec<serde_json::Value> = rows
            .iter()
            .filter_map(|(id, phrase)| {
                let tokens = tokenizer.tokenize(phrase);
                let score = score_for(&tokens, &counts)?;
                let normalised: Vec<String> = tokens
                    .iter()
                    .map(|t| crate::lang::normalise_for_match(t))
                    .collect();
                Some(serde_json::json!({
                    "id": id,
                    "tokens": normalised,
                    "score": score,
                }))
            })
            .collect();

        if !batch.is_empty() {
            let written = batch.len();
            // One statement per page rather than one per row. Written row by row
            // first, this managed some eighty updates a minute against the
            // production table — eighteen hours for Gaelic alone — because every
            // write was a separate round trip to a rotational disk already
            // carrying an ingest. `jsonb_to_recordset` rather than `unnest`
            // because the payload holds an array per row, which `unnest` cannot
            // unpack into columns.
            sqlx::query(
                "UPDATE translations
                    SET sentence_tokens = t.tokens, sentence_score = t.score
                   FROM jsonb_to_recordset($1::jsonb) AS t(id int, tokens text[], score int)
                  WHERE translations.id = t.id",
            )
            .bind(serde_json::Value::Array(batch))
            .execute(pool)
            .await?;
            scored = scored.saturating_add(written);
        }
    }
}

/// The score for a tokenised phrase, or `None` if it is not a servable sentence.
///
/// Declines a phrase shorter than [`MIN_SENTENCE_TOKENS`] or longer than
/// [`MAX_SENTENCE_TOKENS`], one repeating too few distinct words to meet
/// [`MIN_DISTINCT_SHARE`], and one holding a token the corpus has never counted.
///
/// Every one of those is a bound on what is *scored* rather than on how the
/// scored rows are ordered, and that is deliberate. Because the score falls with
/// length, no ordering key available to the selection query can prefer a whole
/// clause over a fragment that outscores it; the fragment has to leave the
/// scored set instead. Declining a row here leaves it unscored, which the
/// selection query already excludes.
fn score_for(tokens: &[String], counts: &std::collections::HashMap<String, i32>) -> Option<i32> {
    if tokens.len() < MIN_SENTENCE_TOKENS || tokens.len() > MAX_SENTENCE_TOKENS {
        return None;
    }
    // A sentence scores as its rarest word, so one made largely of the commonest
    // word scores highest of all: "Tha, tha." and "Tha Tha." took the first four
    // places of the Gaelic ordering, and requiring two distinct tokens only
    // moved the problem to "Chan eil, chan eil, chan eil." Repetition is not
    // context.
    let distinct: std::collections::HashSet<String> = tokens
        .iter()
        .map(|t| crate::lang::normalise_for_match(t))
        .collect();
    let (numerator, denominator) = MIN_DISTINCT_SHARE;
    if distinct.len() * denominator < tokens.len() * numerator {
        return None;
    }
    tokens
        .iter()
        .map(|token| {
            counts
                .get(&crate::lang::normalise_for_match(token))
                .copied()
        })
        .try_fold(i32::MAX, |rarest, count| Some(rarest.min(count?)))
}

/// Finds the most approachable sentence that teaches `word` and whose every
/// other token this user already has a card for.
///
/// The i+1 condition, expressed as two array operators a GIN index answers
/// directly: the sentence must hold the target word, and its token set must be a
/// subset of what the learner knows plus that word. Returns `None` when the
/// corpus holds no such sentence, which is expected and is not an error — early
/// on, a learner knows too little for any sentence to qualify.
///
/// # Errors
///
/// Returns an error if the query fails.
pub async fn sentence_for_word(
    pool: &PgPool,
    user_id: i32,
    native_lang: &str,
    foreign_lang: &str,
    word: &str,
) -> Result<Option<SentenceCard>, WisecrowError> {
    let normalised = crate::lang::normalise_for_match(word);
    let statement = format!(
        "WITH known AS (
           SELECT array_agg(DISTINCT lower(btrim(t.to_phrase, '{trim}'))) AS words
             FROM cards c
             JOIN translations t ON t.id = c.translation_id
             JOIN languages tl ON tl.id = t.to_language_id
            WHERE tl.code = $2 AND c.user_id = $4
         )
         SELECT t.id, t.from_phrase, t.to_phrase, t.sentence_score
           FROM translations t
           JOIN languages fl ON t.from_language_id = fl.id
           JOIN languages tl ON t.to_language_id = tl.id
           LEFT JOIN cards c ON c.translation_id = t.id AND c.user_id = $4
           CROSS JOIN known
          WHERE fl.code = $1 AND tl.code = $2 AND c.id IS NULL
            AND t.sentence_score IS NOT NULL
            AND t.sentence_tokens @> ARRAY[$3]
            AND t.sentence_tokens <@ (coalesce(known.words, ARRAY[]::text[]) || ARRAY[$3])
          ORDER BY t.sentence_score DESC, cardinality(t.sentence_tokens), t.id
          LIMIT 1",
        trim = crate::frequency::MATCH_TRIM_SQL
    );

    let row: Option<(i32, String, String, i32)> = sqlx::query_as(&statement)
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(&normalised)
        .bind(user_id)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(
        |(translation_id, native_phrase, foreign_phrase, score)| SentenceCard {
            translation_id,
            native_phrase,
            foreign_phrase,
            score,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::collections::HashMap;

    fn counts(pairs: &[(&str, i32)]) -> HashMap<String, i32> {
        pairs.iter().map(|(w, c)| ((*w).to_owned(), *c)).collect()
    }

    fn tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    /// The four commonest Gaelic words and their measured corpus counts, which
    /// is the shortest servable sentence and so the basis of most cases here.
    fn gaelic() -> HashMap<String, i32> {
        counts(&[("tha", 37048), ("an", 20134), ("sin", 6284), ("seo", 5730)])
    }

    #[rstest]
    #[case(
        &["tha", "an", "sin", "seo"],
        Some(5730),
        "the rarest word governs, because a sentence is as hard as its hardest word"
    )]
    #[case(
        &["Tha.", "An", "Sin?", "Seo!"],
        Some(5730),
        "tokens are normalised before lookup, as ranking normalised them"
    )]
    #[case(
        &["tha", "an", "sin", "bthey"],
        None,
        "a token the corpus never counted is usually corruption, and declines the row"
    )]
    fn a_sentence_scores_as_its_rarest_word(
        #[case] words: &[&str],
        #[case] expected: Option<i32>,
        #[case] why: &str,
    ) {
        assert_eq!(score_for(&tokens(words), &gaelic()), expected, "{why}");
    }

    #[rstest]
    #[case(&["tha"], "one token is not a sentence; the word deck already teaches it")]
    #[case(&["tha", "an"], "two is the fragment `Tha a?`, whose partner was `Have You Say?`")]
    #[case(&["tha", "an", "sin"], "three is `Tha e ...`, a fragment still")]
    fn a_phrase_too_short_to_carry_context_is_declined(#[case] words: &[&str], #[case] why: &str) {
        assert_eq!(score_for(&tokens(words), &gaelic()), None, "{why}");
    }

    #[rstest]
    #[case(
        &["chan", "eil", "chan", "eil", "chan", "eil"],
        None,
        "two words in six is repetition, which a bare two-distinct test admitted"
    )]
    #[case(
        &["chan", "eil", "chan", "eil"],
        Some(1672),
        "half distinct is the bound itself, and servable"
    )]
    #[case(
        &["chan", "eil", "chan", "tha"],
        Some(1672),
        "as is a sentence that merely repeats one of its words once"
    )]
    fn a_sentence_repeating_too_few_words_is_declined(
        #[case] words: &[&str],
        #[case] expected: Option<i32>,
        #[case] why: &str,
    ) {
        let vocabulary = counts(&[("chan", 8801), ("eil", 1672), ("tha", 37048)]);
        assert_eq!(score_for(&tokens(words), &vocabulary), expected, "{why}");
    }

    /// Wholly distinct words, so the length bound is what decides these cases
    /// and neither the fragment floor nor the repetition guard above it.
    fn distinct_words(len: usize) -> Vec<String> {
        (0..len).map(|i| format!("w{i}")).collect()
    }

    fn distinct_counts(len: usize) -> HashMap<String, i32> {
        (0..len)
            .map(|i| (format!("w{i}"), 100 + i32::try_from(i).unwrap_or(0)))
            .collect()
    }

    #[test]
    fn a_sentence_past_the_length_bound_is_declined() {
        let over = MAX_SENTENCE_TOKENS + 1;
        assert_eq!(
            score_for(&distinct_words(over), &distinct_counts(over)),
            None,
            "a learner meeting one new word must be able to hold the rest at once"
        );
        assert_eq!(
            score_for(
                &distinct_words(MAX_SENTENCE_TOKENS),
                &distinct_counts(MAX_SENTENCE_TOKENS)
            ),
            Some(100),
            "the bound itself is servable"
        );
    }
}
