//! Transactional writes for candidates and promotions.

use sqlx::{PgConnection, PgPool};

use super::extract::LanguagePair;
use super::ExtractionOptions;
use crate::errors::WisecrowError;
use crate::glossing::ValidatedPresentation;

/// Publishes the staged counts as candidates in one transaction: the top
/// `limit` words at or above the occurrence threshold with at least two
/// example sources are upserted, their examples replaced, the ranks of any
/// owned generated rows refreshed, and unselected candidates that were never
/// accepted removed. Returns the number of candidates written.
///
/// # Errors
///
/// Returns an error when any statement fails or the row count overflows.
pub(crate) async fn publish_candidates(
    connection: &mut PgConnection,
    pair: &LanguagePair,
    upper: i32,
    options: &ExtractionOptions,
) -> Result<u64, WisecrowError> {
    let mut transaction = sqlx::Connection::begin(connection).await?;
    sqlx::query(
        "CREATE TEMP TABLE selected_words_stage ON COMMIT DROP AS
         SELECT s.word, s.n FROM word_count_stage s
         WHERE s.n >= $1
           AND (SELECT count(*) FROM word_example_stage e WHERE e.word = s.word) >= 2
         ORDER BY s.n DESC, s.word
         LIMIT $2",
    )
    .bind(i64::from(options.min_occurrences))
    .bind(i64::from(options.limit))
    .execute(&mut *transaction)
    .await?;
    // A candidate the new selection no longer contains is stale: its count
    // describes an earlier scan, and left in place it would queue ahead of
    // genuine words for promotion. Accepted candidates stay because their
    // promotions and cards refer to them.
    sqlx::query(
        "DELETE FROM word_candidates c
         WHERE c.native_language_id = $1 AND c.foreign_language_id = $2
           AND c.status <> 'accepted'
           AND NOT EXISTS (SELECT 1 FROM selected_words_stage s WHERE s.word = c.word)",
    )
    .bind(pair.native_id)
    .bind(pair.foreign_id)
    .execute(&mut *transaction)
    .await?;
    let upserted = sqlx::query(
        "INSERT INTO word_candidates
             (native_language_id, foreign_language_id, word, surface, occurrence_count,
              source_upper_id)
         SELECT $1, $2, word, word, n, $3 FROM selected_words_stage
         ON CONFLICT (native_language_id, foreign_language_id, word) DO UPDATE
         SET occurrence_count = EXCLUDED.occurrence_count,
             source_upper_id = EXCLUDED.source_upper_id,
             surface = EXCLUDED.surface,
             revision = word_candidates.revision + 1",
    )
    .bind(pair.native_id)
    .bind(pair.foreign_id)
    .bind(upper)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    sqlx::query(
        "DELETE FROM word_candidate_examples e
         USING word_candidates c, selected_words_stage s
         WHERE e.candidate_id = c.id AND c.word = s.word
           AND c.native_language_id = $1 AND c.foreign_language_id = $2",
    )
    .bind(pair.native_id)
    .bind(pair.foreign_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO word_candidate_examples
             (candidate_id, ordinal, source_translation_id, native_sentence, foreign_sentence)
         SELECT c.id, e.ordinal::SMALLINT, e.source_id, e.native_sentence, e.foreign_sentence
         FROM (
             SELECT word, source_id, native_sentence, foreign_sentence,
                    row_number() OVER (PARTITION BY word ORDER BY source_id) AS ordinal
             FROM word_example_stage
         ) e
         JOIN selected_words_stage s ON s.word = e.word
         JOIN word_candidates c ON c.word = e.word
              AND c.native_language_id = $1 AND c.foreign_language_id = $2
         WHERE e.ordinal <= 3",
    )
    .bind(pair.native_id)
    .bind(pair.foreign_id)
    .execute(&mut *transaction)
    .await?;
    // A generated learning row ranks by the evidence count that produced it;
    // a reused corpus row keeps its own import counter untouched.
    sqlx::query(
        "UPDATE translations t
         SET frequency = LEAST(c.occurrence_count, 2147483647)::INTEGER,
             corpus_frequency = LEAST(c.occurrence_count, 2147483647)::INTEGER
         FROM word_candidates c
         JOIN word_promotions p ON p.candidate_id = c.id AND p.owns_translation
         JOIN selected_words_stage s ON s.word = c.word
         WHERE t.id = p.translation_id
           AND c.native_language_id = $1 AND c.foreign_language_id = $2",
    )
    .bind(pair.native_id)
    .bind(pair.foreign_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(upserted)
}

/// One candidate as promotion reads it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct StoredCandidate {
    pub(crate) id: i64,
    pub(crate) word: String,
    pub(crate) surface: String,
    pub(crate) revision: i64,
    pub(crate) occurrence_count: i64,
    pub(crate) native_language_id: i32,
    pub(crate) foreign_language_id: i32,
}

/// Result of one promotion transaction.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PromotionOutcome {
    /// The candidate now teaches the linked translation row.
    Accepted(i32),
    /// The model judged the word unteachable; any existing link is kept.
    Rejected,
    /// The candidate was re-extracted or removed after the model was asked.
    Stale,
}

/// Rank stored on a generated row: the evidence count, capped to the column.
fn rank_of(candidate: &StoredCandidate) -> Result<i32, WisecrowError> {
    let capped = candidate.occurrence_count.min(i64::from(i32::MAX));
    i32::try_from(capped)
        .ok()
        .filter(|rank| *rank > 0)
        .ok_or_else(|| {
            WisecrowError::InvalidInput(format!(
                "Candidate {} has an invalid occurrence count {}",
                candidate.id, candidate.occurrence_count
            ))
        })
}

/// Links `candidate` to the translation row that teaches it and stores the
/// validated presentation, all in one transaction under the candidate's row
/// lock. An existing corpus row spelling the word is reused; otherwise a
/// generated row is created and owned. IDs never change on refresh.
///
/// # Errors
///
/// Returns [`WisecrowError::Conflict`] when refreshing an owned row's meaning
/// would collide with another row's unique pair, and any database error.
pub(crate) async fn promote_candidate(
    pool: &PgPool,
    candidate: &StoredCandidate,
    presentation: &ValidatedPresentation,
    native_lang: &str,
    foreign_lang: &str,
) -> Result<PromotionOutcome, WisecrowError> {
    let mut transaction = pool.begin().await?;
    let revision: Option<i64> =
        sqlx::query_scalar("SELECT revision FROM word_candidates WHERE id = $1 FOR UPDATE")
            .bind(candidate.id)
            .fetch_optional(&mut *transaction)
            .await?;
    if revision != Some(candidate.revision) {
        transaction.rollback().await?;
        return Ok(PromotionOutcome::Stale);
    }
    let link: Option<(Option<i32>, bool)> = sqlx::query_as(
        "SELECT translation_id, owns_translation FROM word_promotions WHERE candidate_id = $1",
    )
    .bind(candidate.id)
    .fetch_optional(&mut *transaction)
    .await?;

    let (outcome, status) = if presentation.teachable {
        let translation_id = match link {
            Some((Some(id), owns)) => {
                if owns {
                    refresh_owned_row(&mut transaction, candidate, presentation, id).await?;
                }
                id
            }
            _ => link_translation(&mut transaction, candidate, presentation).await?,
        };
        (PromotionOutcome::Accepted(translation_id), "accepted")
    } else {
        (PromotionOutcome::Rejected, "rejected")
    };

    crate::glossing::store_in_transaction(
        &mut transaction,
        native_lang,
        foreign_lang,
        std::slice::from_ref(presentation),
    )
    .await?;
    sqlx::query(
        "UPDATE word_candidates
         SET status = $2, enriched_revision = revision, presentation_version = $3
         WHERE id = $1",
    )
    .bind(candidate.id)
    .bind(status)
    .bind(crate::presentation::CURRENT_PRESENTATION_VERSION)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(outcome)
}

/// Updates the meaning and ranks of a row this promotion generated, keeping
/// its ID. A meaning that would duplicate another row's unique pair is a
/// conflict for the operator, not a merge.
async fn refresh_owned_row(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    candidate: &StoredCandidate,
    presentation: &ValidatedPresentation,
    translation_id: i32,
) -> Result<(), WisecrowError> {
    let colliding: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM translations
         WHERE from_language_id = $1 AND to_language_id = $2
           AND from_phrase = $3 AND to_phrase = $4 AND id <> $5",
    )
    .bind(candidate.native_language_id)
    .bind(candidate.foreign_language_id)
    .bind(&presentation.translation)
    .bind(&candidate.word)
    .bind(translation_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(other) = colliding {
        return Err(WisecrowError::Conflict(format!(
            "Refreshing generated row {translation_id} for `{}` would duplicate row {other}",
            candidate.word
        )));
    }
    let rank = rank_of(candidate)?;
    sqlx::query(
        "UPDATE translations SET from_phrase = $2, frequency = $3, corpus_frequency = $3
         WHERE id = $1",
    )
    .bind(translation_id)
    .bind(&presentation.translation)
    .bind(rank)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// Chooses or creates the row that teaches the candidate and records the link.
async fn link_translation(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    candidate: &StoredCandidate,
    presentation: &ValidatedPresentation,
) -> Result<i32, WisecrowError> {
    let representative: Option<i32> =
        sqlx::query_scalar(&crate::vocabulary::representative_statement())
            .bind(candidate.native_language_id)
            .bind(candidate.foreign_language_id)
            .bind(&candidate.word)
            .fetch_optional(&mut **transaction)
            .await?;
    let (translation_id, owns) = match representative {
        Some(id) => (id, false),
        None => insert_generated_row(transaction, candidate, presentation).await?,
    };
    sqlx::query(
        "INSERT INTO word_promotions (candidate_id, translation_id, owns_translation)
         VALUES ($1, $2, $3)
         ON CONFLICT (candidate_id) DO UPDATE
         SET translation_id = EXCLUDED.translation_id,
             owns_translation = EXCLUDED.owns_translation",
    )
    .bind(candidate.id)
    .bind(translation_id)
    .bind(owns)
    .execute(&mut **transaction)
    .await?;
    Ok(translation_id)
}

/// Inserts the generated learning row. If another transaction created the
/// same pair meanwhile, that row is reused and not owned.
async fn insert_generated_row(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    candidate: &StoredCandidate,
    presentation: &ValidatedPresentation,
) -> Result<(i32, bool), WisecrowError> {
    let rank = rank_of(candidate)?;
    let inserted: Option<i32> = sqlx::query_scalar(
        "INSERT INTO translations
             (from_language_id, to_language_id, from_phrase, to_phrase, frequency,
              corpus_frequency)
         VALUES ($1, $2, $3, $4, $5, $5)
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase) DO NOTHING
         RETURNING id",
    )
    .bind(candidate.native_language_id)
    .bind(candidate.foreign_language_id)
    .bind(&presentation.translation)
    .bind(&candidate.word)
    .bind(rank)
    .fetch_optional(&mut **transaction)
    .await?;
    if let Some(id) = inserted {
        return Ok((id, true));
    }
    let existing: i32 = sqlx::query_scalar(
        "SELECT id FROM translations
         WHERE from_language_id = $1 AND to_language_id = $2
           AND from_phrase = $3 AND to_phrase = $4",
    )
    .bind(candidate.native_language_id)
    .bind(candidate.foreign_language_id)
    .bind(&presentation.translation)
    .bind(&candidate.word)
    .fetch_one(&mut **transaction)
    .await?;
    Ok((existing, false))
}

/// The stored presentation for `word` when it is current and complete, for
/// repairing a link without asking the model again.
pub(crate) async fn current_presentation(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
    word: &str,
) -> Result<Option<ValidatedPresentation>, WisecrowError> {
    let row: Option<(String, String, bool, Option<String>)> = sqlx::query_as(
        "SELECT display_form, translation, teachable, image_query
         FROM word_glosses
         WHERE lang_code = $1 AND native_lang = $2 AND word = $3
           AND presentation_version >= $4
           AND display_form IS NOT NULL",
    )
    .bind(foreign_lang)
    .bind(native_lang)
    .bind(word)
    .bind(crate::presentation::CURRENT_PRESENTATION_VERSION)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(display_form, translation, teachable, image_query)| ValidatedPresentation {
            word: word.to_owned(),
            display_form,
            translation,
            teachable,
            image_query,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_is_the_capped_positive_count() -> Result<(), WisecrowError> {
        let mut candidate = StoredCandidate {
            id: 1,
            word: "taigh".into(),
            surface: "taigh".into(),
            revision: 1,
            occurrence_count: i64::from(i32::MAX) + 10,
            native_language_id: 1,
            foreign_language_id: 2,
        };
        assert_eq!(rank_of(&candidate)?, i32::MAX);
        candidate.occurrence_count = 5;
        assert_eq!(rank_of(&candidate)?, 5);
        candidate.occurrence_count = 0;
        assert!(rank_of(&candidate).is_err());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn stale_revision_writes_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned()
        });
        let pool = PgPool::connect(&url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        sqlx::query("TRUNCATE translations, languages, word_glosses CASCADE")
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('gd', 'Gaelic')",
        )
        .execute(&pool)
        .await?;
        let (native_id, foreign_id): (i32, i32) = sqlx::query_as(
            "SELECT n.id, f.id FROM languages n, languages f WHERE n.code = 'en' AND f.code = 'gd'",
        )
        .fetch_one(&pool)
        .await?;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO word_candidates
                 (native_language_id, foreign_language_id, word, surface, occurrence_count,
                  source_upper_id)
             VALUES ($1, $2, 'stale', 'stale', 5, 1) RETURNING id",
        )
        .bind(native_id)
        .bind(foreign_id)
        .fetch_one(&pool)
        .await?;
        // The model answered for revision 1; extraction has since moved on.
        sqlx::query("UPDATE word_candidates SET revision = 2 WHERE id = $1")
            .bind(id)
            .execute(&pool)
            .await?;
        let candidate = StoredCandidate {
            id,
            word: "stale".into(),
            surface: "stale".into(),
            revision: 1,
            occurrence_count: 5,
            native_language_id: native_id,
            foreign_language_id: foreign_id,
        };
        let presentation = ValidatedPresentation {
            word: "stale".into(),
            display_form: "stale".into(),
            translation: "old".into(),
            teachable: true,
            image_query: None,
        };
        let outcome = promote_candidate(&pool, &candidate, &presentation, "en", "gd").await?;
        assert_eq!(outcome, PromotionOutcome::Stale);
        let written: Vec<i64> = sqlx::query_scalar(
            "SELECT count(*) FROM word_glosses WHERE word = 'stale'
             UNION ALL SELECT count(*) FROM word_promotions WHERE candidate_id = $1
             UNION ALL SELECT count(*) FROM translations WHERE to_phrase = 'stale'",
        )
        .bind(id)
        .fetch_all(&pool)
        .await?;
        assert_eq!(written, [0, 0, 0]);
        let status: String = sqlx::query_scalar("SELECT status FROM word_candidates WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
        assert_eq!(status, "pending");
        Ok(())
    }
}
