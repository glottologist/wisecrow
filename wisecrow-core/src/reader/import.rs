//! A book's import identity, and the transactions that store its sentences
//! and each batch of work done on it.

use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};

use super::segment::SegmentedSentence;
use super::shelf::ReaderEntry;
use super::tokens::spans;
use super::translate::run_length;
use crate::errors::WisecrowError;

/// The page range of a row whose `Pages` cell is empty, as it enters the key.
pub const AUTO_PAGES: &str = "auto";
/// Language the shelf's books are written in.
pub const BOOK_SOURCE_LANGUAGE: &str = "en";
const KEY_DOMAIN: &[u8] = b"wisecrow-reader-import-v1";

/// What makes one revision of a book.
#[derive(Debug, Clone, Copy)]
pub struct ImportIdentity<'a> {
    pub file_sha256: [u8; 32],
    /// The ledger's page range label, or [`AUTO_PAGES`].
    pub pages: &'a str,
    pub segmenter_version: i16,
}

impl ImportIdentity<'_> {
    /// The revision's key.
    #[must_use]
    pub fn key(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(KEY_DOMAIN);
        hasher.update(self.file_sha256);
        hasher.update([0u8]);
        hasher.update(self.pages.as_bytes());
        hasher.update([0u8]);
        hasher.update(self.segmenter_version.to_be_bytes());
        hasher.finalize().into()
    }
}

/// How an extraction ended, as `books.status` records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookStatus {
    Ready,
    NeedsPages,
    Thin,
}

impl BookStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NeedsPages => "needs_pages",
            Self::Thin => "thin",
        }
    }
}

/// One revision to store.
#[derive(Debug, Clone, Copy)]
pub struct NewBook<'a> {
    pub entry: &'a ReaderEntry,
    pub identity: ImportIdentity<'a>,
    pub cefr_level: &'static str,
    pub status: BookStatus,
    /// First and last printed page of the body, when one was found.
    pub pages: Option<(u32, u32)>,
    pub sentences: &'a [SegmentedSentence],
}

/// A book's translation progress in one language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub stage: String,
    /// Position of the next sentence to translate.
    pub cursor: i32,
    /// Start positions of runs that failed twice.
    pub failed_runs: Vec<i32>,
}

/// Stores a revision and its sentences in one transaction.
///
/// Returns the new book's id, or `None` when a revision with this key is
/// already stored.
///
/// # Errors
///
/// Returns an error when an ordinal does not fit the database or a write
/// fails; nothing is stored then.
pub async fn store_book(pool: &PgPool, book: &NewBook<'_>) -> Result<Option<i32>, WisecrowError> {
    let mut transaction = pool.begin().await?;
    let source = ensure_language(&mut transaction, BOOK_SOURCE_LANGUAGE).await?;
    let (first_page, last_page) = match book.pages {
        Some((first, last)) => (Some(to_i32(first)?), Some(to_i32(last)?)),
        None => (None, None),
    };
    let inserted: Option<i32> = sqlx::query_scalar(
        "INSERT INTO books (import_key, file, file_sha256, series, title, cefr_level_id,
                            source_language_id, first_page, last_page, segmenter_version, status)
         SELECT $1, $2, $3, $4, $5, l.id, $7, $8, $9, $10, $11
         FROM cefr_levels l WHERE l.code = $6
         ON CONFLICT (import_key) DO NOTHING
         RETURNING id",
    )
    .bind(book.identity.key().as_slice())
    .bind(&book.entry.file)
    .bind(book.identity.file_sha256.as_slice())
    .bind(&book.entry.series)
    .bind(&book.entry.title)
    .bind(book.cefr_level)
    .bind(source)
    .bind(first_page)
    .bind(last_page)
    .bind(book.identity.segmenter_version)
    .bind(book.status.as_str())
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(book_id) = inserted else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let mut positions = Vec::with_capacity(book.sentences.len());
    let mut chapters = Vec::with_capacity(book.sentences.len());
    let mut paragraphs = Vec::with_capacity(book.sentences.len());
    let mut ordinals = Vec::with_capacity(book.sentences.len());
    let mut texts = Vec::with_capacity(book.sentences.len());
    for (position, sentence) in book.sentences.iter().enumerate() {
        positions.push(i32::try_from(position).map_err(|_| too_large("position"))?);
        chapters.push(to_i32(sentence.chapter)?);
        paragraphs.push(to_i32(sentence.paragraph)?);
        ordinals.push(to_i32(sentence.sentence)?);
        texts.push(sentence.text.as_str());
    }
    sqlx::query(
        "INSERT INTO book_sentences (book_id, position, chapter, paragraph, sentence, text)
         SELECT $1, unnest($2::int[]), unnest($3::int[]), unnest($4::int[]),
                unnest($5::int[]), unnest($6::text[])",
    )
    .bind(book_id)
    .bind(&positions)
    .bind(&chapters)
    .bind(&paragraphs)
    .bind(&ordinals)
    .bind(&texts)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some(book_id))
}

/// Whether a revision with this key is stored.
///
/// # Errors
///
/// Returns an error when the query fails.
pub async fn book_exists(pool: &PgPool, key: &[u8; 32]) -> Result<bool, WisecrowError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM books WHERE import_key = $1)")
            .bind(key.as_slice())
            .fetch_one(pool)
            .await?,
    )
}

/// Takes the progress row for a book and language, creating it at the
/// start, and holds it until `transaction` ends.
///
/// # Errors
///
/// Returns an error when the query fails.
pub async fn lock_progress(
    transaction: &mut Transaction<'_, Postgres>,
    book_id: i32,
    language_id: i32,
) -> Result<Progress, WisecrowError> {
    sqlx::query(
        "INSERT INTO book_import_progress (book_id, language_id) VALUES ($1, $2)
         ON CONFLICT (book_id, language_id) DO NOTHING",
    )
    .bind(book_id)
    .bind(language_id)
    .execute(&mut **transaction)
    .await?;
    let (stage, cursor, failed_runs): (String, i32, Vec<i32>) = sqlx::query_as(
        "SELECT stage, cursor, failed_runs FROM book_import_progress
         WHERE book_id = $1 AND language_id = $2 FOR UPDATE",
    )
    .bind(book_id)
    .bind(language_id)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(Progress {
        stage,
        cursor,
        failed_runs,
    })
}

/// Stores one run's translations, each with its word spans.
///
/// # Errors
///
/// Returns an error when a span offset does not fit the database or a write
/// fails.
pub async fn save_run(
    transaction: &mut Transaction<'_, Postgres>,
    language_id: i32,
    translated: &[(i32, String)],
) -> Result<(), WisecrowError> {
    for (sentence_id, text) in translated {
        let inserted: Option<i32> = sqlx::query_scalar(
            "INSERT INTO book_translations (sentence_id, language_id, text) VALUES ($1, $2, $3)
             ON CONFLICT (sentence_id, language_id) DO NOTHING RETURNING id",
        )
        .bind(sentence_id)
        .bind(language_id)
        .bind(text)
        .fetch_optional(&mut **transaction)
        .await?;
        let Some(translation_id) = inserted else {
            continue;
        };
        let words = spans(text);
        let mut ordinals = Vec::with_capacity(words.len());
        let mut starts = Vec::with_capacity(words.len());
        let mut ends = Vec::with_capacity(words.len());
        let mut keys = Vec::with_capacity(words.len());
        for (ordinal, word) in words.iter().enumerate() {
            ordinals.push(i32::try_from(ordinal).map_err(|_| too_large("token ordinal"))?);
            starts.push(i32::try_from(word.start).map_err(|_| too_large("token offset"))?);
            ends.push(i32::try_from(word.end).map_err(|_| too_large("token offset"))?);
            keys.push(word.normalised.as_str());
        }
        sqlx::query(
            "INSERT INTO book_tokens (translation_id, ordinal, start_byte, end_byte, normalised)
             SELECT $1, unnest($2::int[]), unnest($3::int[]), unnest($4::int[]), unnest($5::text[])",
        )
        .bind(translation_id)
        .bind(&ordinals)
        .bind(&starts)
        .bind(&ends)
        .bind(&keys)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

/// Moves the cursor past a run, recording the run as failed when
/// `failed_run` names its start, and marks the translation complete once
/// the cursor reaches `total`.
///
/// # Errors
///
/// Returns an error when the update fails.
pub async fn advance(
    transaction: &mut Transaction<'_, Postgres>,
    book_id: i32,
    language_id: i32,
    cursor: i32,
    failed_run: Option<i32>,
    total: i32,
) -> Result<(), WisecrowError> {
    sqlx::query(
        "UPDATE book_import_progress
         SET cursor = $3,
             failed_runs = CASE WHEN $4::int IS NULL THEN failed_runs
                                ELSE array_append(failed_runs, $4::int) END,
             stage = CASE WHEN $3 >= $5 THEN 'complete' ELSE 'translate' END,
             completed_at = CASE WHEN $3 >= $5 THEN COALESCE(completed_at, CURRENT_TIMESTAMP) END,
             updated_at = CURRENT_TIMESTAMP
         WHERE book_id = $1 AND language_id = $2",
    )
    .bind(book_id)
    .bind(language_id)
    .bind(cursor)
    .bind(failed_run)
    .bind(total)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// The id of a supported language, creating its row when it is missing.
///
/// # Errors
///
/// Returns an error for an unsupported code or a failed write.
pub async fn ensure_language(
    transaction: &mut Transaction<'_, Postgres>,
    code: &str,
) -> Result<i32, WisecrowError> {
    let name = crate::cli::SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, name)| *name)
        .ok_or_else(|| WisecrowError::UnsupportedLanguage(code.to_owned()))?;
    Ok(sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ($1, $2)
         ON CONFLICT (code) DO UPDATE SET code = EXCLUDED.code
         RETURNING id",
    )
    .bind(code)
    .bind(name)
    .fetch_one(&mut **transaction)
    .await?)
}

fn to_i32(value: u32) -> Result<i32, WisecrowError> {
    i32::try_from(value).map_err(|_| too_large("ordinal"))
}

fn too_large(what: &str) -> WisecrowError {
    WisecrowError::InvalidInput(format!("a book {what} is too large to store"))
}

/// What one committed call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStored {
    Advanced { cursor: i32 },
    Rejected { cursor: i32, start: i32 },
    Done,
}

/// Translates the next run of one book into `language_code` and commits it
/// with the cursor, or records the run as failed after two refusals.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmUnavailable`] without moving the cursor, or a
/// database error. The book transaction is rolled back on either.
pub async fn translate_book_run(
    pool: &PgPool,
    book_id: i32,
    language_code: &str,
    language_name: &str,
    cefr: &str,
    model: &crate::llm::cache::CachedLlm<'_>,
) -> Result<RunStored, WisecrowError> {
    let mut transaction = pool.begin().await?;
    let language_id = ensure_language(&mut transaction, language_code).await?;
    let progress = lock_progress(&mut transaction, book_id, language_id).await?;
    let rows: Vec<(i32, i32, String)> = sqlx::query_as(
        "SELECT id, position, text FROM book_sentences
         WHERE book_id = $1 AND position >= $2 ORDER BY position",
    )
    .bind(book_id)
    .bind(progress.cursor)
    .fetch_all(&mut *transaction)
    .await?;
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM book_sentences WHERE book_id = $1")
        .bind(book_id)
        .fetch_one(&mut *transaction)
        .await?;
    let total = i32::try_from(total).map_err(|_| too_large("sentence count"))?;
    if rows.is_empty() {
        if progress.cursor < total {
            advance(&mut transaction, book_id, language_id, total, None, total).await?;
            transaction.commit().await?;
        } else {
            transaction.rollback().await?;
        }
        return Ok(RunStored::Done);
    }
    let texts: Vec<&str> = rows.iter().map(|(_, _, text)| text.as_str()).collect();
    let count = run_length(&texts);
    let run = &rows[..count];
    let sentences: Vec<&str> = run.iter().map(|(_, _, text)| text.as_str()).collect();
    let start = run[0].1;
    let next = run[count - 1]
        .1
        .checked_add(1)
        .ok_or_else(|| too_large("cursor"))?;
    let outcome = crate::reader::translate::translate_run(
        model,
        &crate::reader::translate::RunRequest {
            sentences: &sentences,
            language_name,
            cefr_level: cefr,
        },
    )
    .await;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            transaction.rollback().await?;
            return Err(error);
        }
    };
    match outcome {
        crate::reader::translate::RunOutcome::Translated(translations) => {
            let pairs: Vec<(i32, String)> = run
                .iter()
                .zip(translations)
                .map(|(row, text)| (row.0, text))
                .collect();
            save_run(&mut transaction, language_id, &pairs).await?;
            advance(&mut transaction, book_id, language_id, next, None, total).await?;
            transaction.commit().await?;
            Ok(RunStored::Advanced { cursor: next })
        }
        crate::reader::translate::RunOutcome::Failed(_) => {
            advance(
                &mut transaction,
                book_id,
                language_id,
                next,
                Some(start),
                total,
            )
            .await?;
            transaction.commit().await?;
            Ok(RunStored::Rejected {
                cursor: next,
                start,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(pages: &str, version: i16, byte: u8) -> ImportIdentity<'_> {
        ImportIdentity {
            file_sha256: [byte; 32],
            pages,
            segmenter_version: version,
        }
    }

    #[test]
    fn the_key_changes_with_the_file_the_pages_and_the_rules() {
        let base = identity(AUTO_PAGES, 1, 7).key();

        assert_eq!(base, identity(AUTO_PAGES, 1, 7).key(), "the key is stable");
        assert_ne!(
            base,
            identity("4-41", 1, 7).key(),
            "a page range is a new revision"
        );
        assert_ne!(
            base,
            identity(AUTO_PAGES, 2, 7).key(),
            "new rules are a new revision"
        );
        assert_ne!(
            base,
            identity(AUTO_PAGES, 1, 8).key(),
            "a changed file is a new revision"
        );
    }

    #[test]
    fn a_status_is_stored_by_its_name() {
        assert_eq!(BookStatus::Ready.as_str(), "ready");
        assert_eq!(BookStatus::NeedsPages.as_str(), "needs_pages");
        assert_eq!(BookStatus::Thin.as_str(), "thin");
    }
}
