//! One page of a phrase mine. The cursor and the counts commit together, so a
//! second claim of the same page rolls back.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::phrases::count_ngrams_into;

use super::ranking::evidence_checksum;

/// Surfaces read in one unit invocation. A mine is one long step, like one archive.
const MINE_PAGE: i64 = 5_000;

/// One attempt to claim the next page of foreign surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinePage {
    /// False when the generation or cursor changed before the claim landed.
    pub advanced: bool,
    /// True when this claim read no surfaces.
    pub empty: bool,
    /// Generation the claim used.
    pub generation: i64,
    /// Cursor the claim started from, before this page.
    pub cursor_before: String,
}

/// Opens a mine at generation 1 when none exists, then claims its next page.
///
/// # Errors
///
/// Returns an error when `foreign` is not a language code, has no tokeniser,
/// or a query fails.
pub async fn commit_mine_page(
    pool: &PgPool,
    foreign: &str,
    page_size: i64,
) -> Result<MinePage, WisecrowError> {
    let checksum = evidence_checksum(pool, foreign).await?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO pipeline_phrase_mines (foreign_code, evidence_checksum)
         VALUES ($1, $2)
         ON CONFLICT (foreign_code) DO NOTHING",
    )
    .bind(foreign)
    .bind(checksum.as_slice())
    .execute(&mut *tx)
    .await?;
    let (generation, cursor): (i64, String) = sqlx::query_as(
        "SELECT generation, surface_cursor
         FROM pipeline_phrase_mines
         WHERE foreign_code = $1",
    )
    .bind(foreign)
    .fetch_one(&mut *tx)
    .await?;
    let page = claim_page(&mut tx, foreign, generation, &cursor, page_size).await?;
    settle(tx, page).await
}

/// Claims the page at `cursor` only while that generation and cursor still match.
///
/// A mismatch rolls the transaction back, including any counts this attempt added.
///
/// # Errors
///
/// Returns an error when `foreign` has no tokeniser or a query fails.
pub async fn commit_mine_page_at(
    pool: &PgPool,
    foreign: &str,
    generation: i64,
    cursor: &str,
    page_size: i64,
) -> Result<MinePage, WisecrowError> {
    let mut tx = pool.begin().await?;
    let page = claim_page(&mut tx, foreign, generation, cursor, page_size).await?;
    settle(tx, page).await
}

async fn settle(
    tx: sqlx::Transaction<'_, sqlx::Postgres>,
    page: MinePage,
) -> Result<MinePage, WisecrowError> {
    if page.advanced {
        tx.commit().await?;
    } else {
        tx.rollback().await?;
    }
    Ok(page)
}

async fn claim_page(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    foreign: &str,
    generation: i64,
    cursor: &str,
    page_size: i64,
) -> Result<MinePage, WisecrowError> {
    let tokenizer = crate::preview::tokenize::for_language(foreign)?;
    let statement = format!(
        "SELECT DISTINCT lower(btrim(t.to_phrase, '{trim}')) AS surface
         FROM corpus_evidence_translations t
         JOIN languages tl ON tl.id = t.to_language_id
         WHERE tl.code = $1
           AND lower(btrim(t.to_phrase, '{trim}')) > $2
         ORDER BY surface
         LIMIT $3",
        trim = crate::frequency::MATCH_TRIM_SQL
    );
    let surfaces: Vec<(String,)> = sqlx::query_as(&statement)
        .bind(foreign)
        .bind(cursor)
        .bind(page_size)
        .fetch_all(&mut **tx)
        .await?;

    let mut counts: HashMap<Vec<String>, i32> = HashMap::new();
    for (surface,) in &surfaces {
        let tokens = tokenizer.tokenize(surface);
        count_ngrams_into(&mut counts, &tokens);
    }
    for (tokens, sentence_count) in &counts {
        let phrase = tokens.join(" ");
        let token_count = i32::try_from(tokens.len()).map_err(|_| {
            WisecrowError::InvalidInput("phrase token count does not fit".to_owned())
        })?;
        sqlx::query(
            "INSERT INTO pipeline_phrase_counts
                 (foreign_code, phrase, token_count, sentence_count)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (foreign_code, phrase)
             DO UPDATE SET sentence_count =
                 pipeline_phrase_counts.sentence_count + EXCLUDED.sentence_count",
        )
        .bind(foreign)
        .bind(&phrase)
        .bind(token_count)
        .bind(sentence_count)
        .execute(&mut **tx)
        .await?;
    }

    let cursor_after = match surfaces.last() {
        Some((last,)) => last.clone(), // clone: keyset cursor stored on the mine
        None => cursor.to_owned(),
    };
    let updated = sqlx::query(
        "UPDATE pipeline_phrase_mines
            SET surface_cursor = $4, updated_at = CURRENT_TIMESTAMP
          WHERE foreign_code = $1 AND generation = $2 AND surface_cursor = $3",
    )
    .bind(foreign)
    .bind(generation)
    .bind(cursor)
    .bind(&cursor_after)
    .execute(&mut **tx)
    .await?;
    Ok(MinePage {
        advanced: updated.rows_affected() > 0,
        empty: surfaces.is_empty(),
        generation,
        cursor_before: cursor.to_owned(),
    })
}

/// `true` when every archive `archive_step` would read is already recorded.
///
/// # Errors
///
/// Returns an error when `foreign` is not a language code or a lookup fails.
pub async fn archives_settled(
    pool: &PgPool,
    native: &str,
    foreign: &str,
) -> Result<bool, WisecrowError> {
    settled(pool, native, foreign, None).await
}

/// Mines `foreign` when its archives are recorded and its rank stamp matches.
///
/// One call reads pages until the cursor is exhausted, then publishes or
/// abandons. It does not hold the pair lock.
///
/// # Errors
///
/// Returns an error when a language code, tokeniser, or query fails.
pub async fn mine_pair(pool: &PgPool, native: &str, foreign: &str) -> Result<bool, WisecrowError> {
    mine_selected(pool, native, foreign, None).await
}

/// Mines the first pair that has an unfinished mine or a phrase-stamp mismatch.
///
/// # Errors
///
/// Returns an error when a language code, tokeniser, or query fails.
pub async fn run_mine_if_due(
    pool: &PgPool,
    pairs: &[super::config::PairSpec],
) -> Result<bool, WisecrowError> {
    for pair in pairs {
        if !mine_is_due(pool, &pair.foreign).await? {
            continue;
        }
        return mine_selected(pool, &pair.native, &pair.foreign, pair.corpora.as_deref()).await;
    }
    Ok(false)
}

/// Publishes the mine when its checksum still matches, otherwise drops it.
///
/// # Errors
///
/// Returns an error when the checksum or a write fails.
pub async fn finish_or_abandon(pool: &PgPool, foreign: &str) -> Result<bool, WisecrowError> {
    let live = evidence_checksum(pool, foreign).await?;
    let mut tx = pool.begin().await?;
    let row: Option<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT generation, evidence_checksum
         FROM pipeline_phrase_mines
         WHERE foreign_code = $1",
    )
    .bind(foreign)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((generation, stored)) = row else {
        tx.rollback().await?;
        return Ok(false);
    };
    if stored.as_slice() != live.as_slice() {
        return abandon(tx, foreign, generation, &live).await;
    }
    publish(tx, foreign, generation, &stored).await
}

async fn abandon(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    foreign: &str,
    generation: i64,
    live: &[u8; 32],
) -> Result<bool, WisecrowError> {
    let bumped = sqlx::query(
        "UPDATE pipeline_phrase_mines
            SET generation = generation + 1,
                surface_cursor = '',
                evidence_checksum = $3,
                updated_at = CURRENT_TIMESTAMP
          WHERE foreign_code = $1 AND generation = $2",
    )
    .bind(foreign)
    .bind(generation)
    .bind(live.as_slice())
    .execute(&mut *tx)
    .await?;
    if bumped.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    sqlx::query("DELETE FROM pipeline_phrase_counts WHERE foreign_code = $1")
        .bind(foreign)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

async fn publish(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    foreign: &str,
    generation: i64,
    stored: &[u8],
) -> Result<bool, WisecrowError> {
    sqlx::query(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT lang.id, counts.phrase, counts.token_count, counts.sentence_count
         FROM pipeline_phrase_counts AS counts
         JOIN languages AS lang ON lang.code = counts.foreign_code
         WHERE counts.foreign_code = $1
           AND counts.sentence_count >= $2
         ON CONFLICT (language_id, phrase)
           DO UPDATE SET sentence_count = EXCLUDED.sentence_count",
    )
    .bind(foreign)
    .bind(crate::phrases::MIN_SENTENCES)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE translations AS existing
            SET corpus_frequency = counts.sentence_count
           FROM pipeline_phrase_counts AS counts
           JOIN phrases AS p
             ON p.phrase = counts.phrase
            AND p.language_id = (SELECT id FROM languages WHERE code = counts.foreign_code)
           JOIN phrase_translations AS link ON link.phrase_id = p.id
          WHERE counts.foreign_code = $1
            AND counts.sentence_count >= $2
            AND existing.id = link.translation_id",
    )
    .bind(foreign)
    .bind(crate::phrases::MIN_SENTENCES)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM pipeline_phrase_counts WHERE foreign_code = $1")
        .bind(foreign)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO pipeline_phrase_stamps (foreign_code, evidence_checksum)
         VALUES ($1, $2)
         ON CONFLICT (foreign_code) DO UPDATE
            SET evidence_checksum = EXCLUDED.evidence_checksum,
                mined_at = CURRENT_TIMESTAMP",
    )
    .bind(foreign)
    .bind(stored)
    .execute(&mut *tx)
    .await?;
    let removed = sqlx::query(
        "DELETE FROM pipeline_phrase_mines
          WHERE foreign_code = $1 AND generation = $2",
    )
    .bind(foreign)
    .bind(generation)
    .execute(&mut *tx)
    .await?;
    if removed.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    tx.commit().await?;
    Ok(true)
}

async fn mine_selected(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    selection: Option<&[String]>,
) -> Result<bool, WisecrowError> {
    if !settled(pool, native, foreign, selection).await? {
        return Ok(false);
    }
    let live = evidence_checksum(pool, foreign).await?;
    let rank: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT evidence_checksum FROM pipeline_rank_stamps WHERE foreign_code = $1",
    )
    .bind(foreign)
    .fetch_optional(pool)
    .await?;
    if rank.as_deref() != Some(live.as_slice()) {
        return Ok(false);
    }
    loop {
        let page = commit_mine_page(pool, foreign, MINE_PAGE).await?;
        if !page.advanced {
            return Ok(false);
        }
        if page.empty {
            break;
        }
    }
    finish_or_abandon(pool, foreign).await
}

pub(crate) async fn mine_is_due(pool: &PgPool, foreign: &str) -> Result<bool, WisecrowError> {
    let unfinished: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM pipeline_phrase_mines WHERE foreign_code = $1
         )",
    )
    .bind(foreign)
    .fetch_one(pool)
    .await?;
    if unfinished {
        return Ok(true);
    }
    let live = evidence_checksum(pool, foreign).await?;
    let stamp: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT evidence_checksum FROM pipeline_phrase_stamps WHERE foreign_code = $1",
    )
    .bind(foreign)
    .fetch_optional(pool)
    .await?;
    Ok(stamp.as_deref() != Some(live.as_slice()))
}

async fn settled(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    selection: Option<&[String]>,
) -> Result<bool, WisecrowError> {
    let owned;
    let names: &[String] = if let Some(selection) = selection {
        selection
    } else {
        owned = super::policy::corpora_for(foreign, None)?
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        &owned
    };
    let langs = crate::Langs::new(native, foreign);
    let ledger = super::archives::ArchiveLedger::new(pool);
    for name in names {
        let corpus = crate::files::Corpus::try_from(name.as_str())?;
        let files = crate::files::LanguageFiles::new(&langs, Some(&[corpus]))?;
        let Some(file) = files.files.into_iter().next() else {
            continue;
        };
        if !ledger
            .is_settled(native, foreign, &file.target_location)
            .await?
        {
            return Ok(false);
        }
    }
    Ok(true)
}
