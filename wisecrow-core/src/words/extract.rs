//! Bounded extraction of candidate words from corpus sentences.
//!
//! Counting happens in PostgreSQL temporary tables on one detached
//! connection, so no whole-vocabulary map is built in memory and a cancelled
//! run leaves nothing behind: the connection, and its temporary tables with
//! it, are dropped rather than returned to the pool.

use std::collections::BTreeMap;

use sqlx::{Connection, PgConnection, PgPool};

use super::{store, ExtractionOptions, ExtractionSummary};
use crate::errors::WisecrowError;
use crate::preview::tokenize::Tokenizer;

/// Evidence rows fetched per page in production.
const PAGE_SIZE: i64 = 1000;

#[derive(Debug, serde::Serialize)]
struct TokenEvidence<'a> {
    word: String,
    n: i64,
    source_id: i32,
    native_sentence: &'a str,
    foreign_sentence: &'a str,
}

/// Counts words across the pair's evidence sentences and publishes the most
/// frequent as candidates with up to three source examples each.
///
/// # Errors
///
/// Returns an error when either code is unsupported or the codes are equal,
/// the pair has no evidence rows, the foreign language has no tokeniser, or
/// any database step fails.
pub async fn extract_words(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
    options: &ExtractionOptions,
) -> Result<ExtractionSummary, WisecrowError> {
    extract_words_paged(pool, native_lang, foreign_lang, options, PAGE_SIZE).await
}

/// [`extract_words`] with an explicit page size, so a test can force page
/// boundaries on a handful of rows.
async fn extract_words_paged(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
    options: &ExtractionOptions,
    page_size: i64,
) -> Result<ExtractionSummary, WisecrowError> {
    let pair = resolve_pair(pool, native_lang, foreign_lang).await?;
    let tokenizer = crate::preview::tokenize::for_language(foreign_lang)?;
    let mut connection = pool.acquire().await?.detach();
    let result = scan_and_publish(
        &mut connection,
        &pair,
        tokenizer.as_ref(),
        foreign_lang,
        options,
        page_size,
    )
    .await;
    match (result, connection.close().await) {
        (Ok(summary), Ok(())) => Ok(summary),
        (Ok(_), Err(close)) => Err(close.into()),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(close)) => {
            tracing::warn!(%close, "extraction connection did not close cleanly");
            Err(error)
        }
    }
}

/// Language IDs of a validated, distinct, supported pair.
pub(crate) struct LanguagePair {
    pub(crate) native_id: i32,
    pub(crate) foreign_id: i32,
}

pub(crate) async fn resolve_pair(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
) -> Result<LanguagePair, WisecrowError> {
    if !crate::cli::is_supported_language(native_lang)
        || !crate::cli::is_supported_language(foreign_lang)
        || native_lang == foreign_lang
    {
        return Err(WisecrowError::InvalidInput(format!(
            "Unsupported or identical language pair {native_lang}/{foreign_lang}"
        )));
    }
    let ids: Vec<(String, i32)> =
        sqlx::query_as("SELECT code, id FROM languages WHERE code = $1 OR code = $2")
            .bind(native_lang)
            .bind(foreign_lang)
            .fetch_all(pool)
            .await?;
    let id_of = |code: &str| {
        ids.iter()
            .find(|(candidate, _)| candidate == code)
            .map(|(_, id)| *id)
            .ok_or_else(|| WisecrowError::InvalidInput(format!("Language {code} has no rows")))
    };
    Ok(LanguagePair {
        native_id: id_of(native_lang)?,
        foreign_id: id_of(foreign_lang)?,
    })
}

async fn scan_and_publish(
    connection: &mut PgConnection,
    pair: &LanguagePair,
    tokenizer: &dyn Tokenizer,
    foreign_lang: &str,
    options: &ExtractionOptions,
    page_size: i64,
) -> Result<ExtractionSummary, WisecrowError> {
    // The scan reads rows up to the pair's current highest ID: a cutoff that
    // makes the run finite, not protection against concurrent edits.
    let upper: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(id) FROM corpus_evidence_translations
         WHERE from_language_id = $1 AND to_language_id = $2",
    )
    .bind(pair.native_id)
    .bind(pair.foreign_id)
    .fetch_one(&mut *connection)
    .await?;
    let Some(upper) = upper else {
        return Err(WisecrowError::InvalidInput(
            "No corpus evidence for this language pair".into(),
        ));
    };
    sqlx::query("CREATE TEMP TABLE word_count_stage (word TEXT PRIMARY KEY, n BIGINT NOT NULL)")
        .execute(&mut *connection)
        .await?;
    sqlx::query(
        "CREATE TEMP TABLE word_example_stage (
             word TEXT NOT NULL, source_id INTEGER NOT NULL,
             native_sentence TEXT NOT NULL, foreign_sentence TEXT NOT NULL,
             PRIMARY KEY (word, source_id)
         )",
    )
    .execute(&mut *connection)
    .await?;

    let mut scanned_rows = 0u64;
    let mut cursor = 0i32;
    loop {
        let rows: Vec<(i32, String, String)> = sqlx::query_as(
            "SELECT id, from_phrase, to_phrase FROM corpus_evidence_translations
             WHERE from_language_id = $1 AND to_language_id = $2 AND id > $3 AND id <= $4
             ORDER BY id LIMIT $5",
        )
        .bind(pair.native_id)
        .bind(pair.foreign_id)
        .bind(cursor)
        .bind(upper)
        .bind(page_size)
        .fetch_all(&mut *connection)
        .await?;
        let Some((last_id, _, _)) = rows.last() else {
            break;
        };
        cursor = *last_id;
        scanned_rows = u64::try_from(rows.len())
            .ok()
            .and_then(|count| scanned_rows.checked_add(count))
            .ok_or_else(|| WisecrowError::InvalidInput("Scanned row count overflow".into()))?;
        let evidence = page_evidence(&rows, tokenizer, foreign_lang)?;
        let page = serde_json::to_string(&evidence).map_err(|error| {
            WisecrowError::InvalidInput(format!("Cannot encode extraction page: {error}"))
        })?;
        merge_page(connection, &page).await?;
    }

    let candidates = store::publish_candidates(connection, pair, upper, options).await?;
    Ok(ExtractionSummary {
        scanned_rows,
        candidates,
    })
}

/// Tokenises one page. Repeated tokens within one sentence add to the count
/// but yield one example row for that source.
fn page_evidence<'a>(
    rows: &'a [(i32, String, String)],
    tokenizer: &dyn Tokenizer,
    foreign_lang: &str,
) -> Result<Vec<TokenEvidence<'a>>, WisecrowError> {
    let mut evidence = Vec::new();
    for (id, native, text) in rows {
        let mut counts = BTreeMap::<String, i64>::new();
        for token in tokenizer.tokenize(text) {
            let word = crate::lang::normalise_for_match(&token);
            if crate::lang::is_meaningful_text(&word, crate::lang::MAX_WORD_CHARS)
                && crate::lang::is_plausible_script(&word, foreign_lang)
            {
                let count = counts.entry(word).or_default();
                *count = count
                    .checked_add(1)
                    .ok_or_else(|| WisecrowError::InvalidInput("Word count overflow".into()))?;
            }
        }
        evidence.extend(counts.into_iter().map(|(word, n)| TokenEvidence {
            word,
            n,
            source_id: *id,
            native_sentence: native,
            foreign_sentence: text,
        }));
    }
    Ok(evidence)
}

/// Merges one encoded page into the staging tables and trims each word to its
/// three lowest-ID examples, in a short transaction so a failure leaves the
/// stage at a page boundary.
async fn merge_page(connection: &mut PgConnection, page: &str) -> Result<(), WisecrowError> {
    let mut transaction = connection.begin().await?;
    sqlx::query(
        "INSERT INTO word_count_stage (word, n)
         SELECT word, sum(n)::BIGINT FROM jsonb_to_recordset($1::jsonb)
             AS e(word TEXT, n BIGINT, source_id INTEGER,
                  native_sentence TEXT, foreign_sentence TEXT)
         GROUP BY word
         ON CONFLICT (word) DO UPDATE SET n = word_count_stage.n + EXCLUDED.n",
    )
    .bind(page)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO word_example_stage (word, source_id, native_sentence, foreign_sentence)
         SELECT word, source_id, native_sentence, foreign_sentence
         FROM jsonb_to_recordset($1::jsonb)
             AS e(word TEXT, n BIGINT, source_id INTEGER,
                  native_sentence TEXT, foreign_sentence TEXT)
         WHERE char_length(native_sentence) BETWEEN 1 AND 200
           AND char_length(foreign_sentence) BETWEEN 1 AND 200
         ON CONFLICT DO NOTHING",
    )
    .bind(page)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "DELETE FROM word_example_stage s USING (
             SELECT word, source_id,
                    row_number() OVER (PARTITION BY word ORDER BY source_id) AS position
             FROM word_example_stage
         ) excess
         WHERE s.word = excess.word AND s.source_id = excess.source_id AND excess.position > 3",
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::tokenize::WhitespaceTokenizer;

    #[test]
    fn repeated_tokens_count_once_per_source_example() -> Result<(), WisecrowError> {
        let rows = vec![
            (
                7,
                "my house house".to_owned(),
                "mo thaigh taigh taigh.".to_owned(),
            ),
            (8, "...".to_owned(), "42 ... \u{200b}".to_owned()),
        ];
        let evidence = page_evidence(&rows, &WhitespaceTokenizer, "gd")?;
        let taigh: Vec<_> = evidence.iter().filter(|e| e.word == "taigh").collect();
        assert_eq!(taigh.len(), 1, "one example row per source");
        assert_eq!(taigh[0].n, 2, "both occurrences counted");
        assert_eq!(taigh[0].source_id, 7);
        assert_eq!(taigh[0].foreign_sentence, "mo thaigh taigh taigh.");
        assert!(
            evidence.iter().all(|e| e.source_id != 8),
            "digits, punctuation and invisible tokens are not words"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn page_boundaries_do_not_change_counts() -> Result<(), Box<dyn std::error::Error>> {
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
        for (native, foreign) in [
            ("the big house", "an taigh mòr"),
            ("the small house", "an taigh beag"),
            ("a warm house", "taigh blàth"),
            ("a cold house", "taigh fuar"),
            ("my house", "mo thaigh taigh"),
        ] {
            sqlx::query(
                "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
                 SELECT n.id, f.id, $1, $2 FROM languages n, languages f
                 WHERE n.code = 'en' AND f.code = 'gd'",
            )
            .bind(native)
            .bind(foreign)
            .execute(&pool)
            .await?;
        }
        let options = ExtractionOptions::new(500, 5)?;
        let summary = extract_words_paged(&pool, "en", "gd", &options, 2).await?;
        assert_eq!(
            summary,
            ExtractionSummary {
                scanned_rows: 5,
                candidates: 1
            }
        );
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT c.occurrence_count, count(e.ordinal)
             FROM word_candidates c
             JOIN word_candidate_examples e ON e.candidate_id = c.id
             WHERE c.word = 'taigh'
             GROUP BY c.id, c.occurrence_count",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(counts, (5, 3));
        Ok(())
    }
}
