//! Learner's notes for forms that occur in a book translation.

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::llm::cache::CachedLlm;
use crate::reader::gloss::{gloss_batch, GlossRequest, GLOSS_BATCH};
use crate::reader::import::ensure_language;

/// What one gloss call stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlossStored {
    Stored { ok: usize, failed: usize },
    Done,
}

/// Notes the next batch of forms for one pair.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmUnavailable`] or a database error. An outage
/// writes no row.
pub async fn gloss_forms(
    pool: &PgPool,
    foreign_code: &str,
    native_code: &str,
    model: &CachedLlm<'_>,
) -> Result<GlossStored, WisecrowError> {
    let language_name = supported_name(foreign_code)?;
    let native_name = supported_name(native_code)?;
    let mut transaction = pool.begin().await?;
    let foreign_id = ensure_language(&mut transaction, foreign_code).await?;
    let native_id = ensure_language(&mut transaction, native_code).await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT DISTINCT ON (k.normalised) k.normalised, t.text
         FROM book_tokens k
         JOIN book_translations t ON t.id = k.translation_id
         WHERE t.language_id = $1
           AND NOT EXISTS (
               SELECT 1 FROM book_glosses g
               WHERE g.language_id = $1 AND g.native_language_id = $2
                 AND g.normalised = k.normalised
           )
         ORDER BY k.normalised
         LIMIT $3",
    )
    .bind(foreign_id)
    .bind(native_id)
    .bind(i64::try_from(GLOSS_BATCH).map_err(|_| WisecrowError::InvalidInput("batch".into()))?)
    .fetch_all(&mut *transaction)
    .await?;
    if rows.is_empty() {
        transaction.rollback().await?;
        return Ok(GlossStored::Done);
    }
    let owned: Vec<(String, String)> = rows;
    let forms: Vec<(&str, &str)> = owned
        .iter()
        .map(|(form, sentence)| (form.as_str(), sentence.as_str()))
        .collect();
    let outcome = match gloss_batch(
        model,
        &GlossRequest {
            forms: &forms,
            language_name, // from SUPPORTED_LANGUAGE_INFO, not the code
            native_name,
        },
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            transaction.rollback().await?;
            return Err(error);
        }
    };
    for note in &outcome.notes {
        sqlx::query(
            "INSERT INTO book_glosses
                 (language_id, native_language_id, normalised, meaning, lemma, class, note, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'ok')
             ON CONFLICT DO NOTHING",
        )
        .bind(foreign_id)
        .bind(native_id)
        .bind(&note.form)
        .bind(&note.meaning)
        .bind(&note.lemma)
        .bind(&note.class)
        .bind(&note.note)
        .execute(&mut *transaction)
        .await?;
    }
    for form in &outcome.failed {
        sqlx::query(
            "INSERT INTO book_glosses
                 (language_id, native_language_id, normalised, status)
             VALUES ($1, $2, $3, 'failed')
             ON CONFLICT DO NOTHING",
        )
        .bind(foreign_id)
        .bind(native_id)
        .bind(form)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(GlossStored::Stored {
        ok: outcome.notes.len(),
        failed: outcome.failed.len(),
    })
}

fn supported_name(code: &str) -> Result<&'static str, WisecrowError> {
    crate::cli::SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, name)| *name)
        .ok_or_else(|| WisecrowError::UnsupportedLanguage(code.to_owned()))
}
