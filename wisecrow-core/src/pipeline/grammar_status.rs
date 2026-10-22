//! Whether a language's grammar shelf still needs a model call or a clip.
//!
//! Import idleness is the importer's own schedule joined to progress for the
//! current file hash and importer version. Audio idleness is a separate
//! probe of the clips that schedule does not describe. Neither path calls a
//! model.

use std::path::{Path, PathBuf};

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::frequency::MATCH_TRIM_SQL;
use crate::grammar::library::{self, documents};
use crate::grammar::pdf_import::ImportTarget;
use crate::grammar::pdf_incremental::{self, is_complete};
use crate::grammar::sources::{self, import_schedule, Clearance, LevelCoverage};
use crate::lang;
use crate::media::cache::MediaCache;
use crate::media::grammar_audio::{correct_examples, verb_forms_voiced, SentenceSpeaker};
use crate::media::MediaType;

/// `true` when every scheduled document and level for `lang` is already imported.
///
/// A document the schedule skips, and a progress row for an older file hash
/// or importer version, do not count. An empty schedule is idle.
///
/// # Errors
///
/// Returns an error when `lang` is not a well-formed code, the shelf cannot
/// be read, or a query fails.
pub async fn grammar_import_idle(
    pool: &PgPool,
    lang: &str,
    shelf: &Path,
) -> Result<bool, WisecrowError> {
    if !lang::is_valid_code(lang) {
        return Err(WisecrowError::InvalidInput(format!(
            "Invalid language code: {lang}"
        )));
    }

    let rows = scheduled_documents(lang, shelf)?;
    let schedule = import_schedule(&rows, None)?;
    if schedule.calls.is_empty() {
        return Ok(true);
    }

    let language_id: Option<i32> = sqlx::query_scalar("SELECT id FROM languages WHERE code = $1")
        .bind(lang)
        .fetch_optional(pool)
        .await?;
    let Some(language_id) = language_id else {
        return Ok(false);
    };

    for (level, document) in schedule.calls {
        let hash = pdf_incremental::fingerprint(document)?;
        let target = ImportTarget {
            language_id,
            language_name: "",
            cefr_level: level,
        };
        if !is_complete(pool, target, &hash).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn scheduled_documents(
    lang: &str,
    shelf: &Path,
) -> Result<Vec<(PathBuf, LevelCoverage)>, WisecrowError> {
    let mut rows = Vec::new();
    for document in documents(shelf)? {
        if library::language_from_path(&document) != Some(lang) {
            continue;
        }
        let found = sources::lookup(&document);
        if matches!(found.clearance, Clearance::Permitted) {
            rows.push((document, found.levels));
        }
    }
    Ok(rows)
}

/// `true` when every correct example already has a live clip, or there are none.
///
/// `speaker` is `None` when this build cannot voice examples; that is idle,
/// not a pending job. A language with no voice is not idle, so the audio
/// script can report it. This function does not synthesise audio.
///
/// # Errors
///
/// Returns an error when the example query or a cache probe fails.
pub async fn grammar_audio_idle(
    pool: &PgPool,
    lang: &str,
    cache: &MediaCache,
    speaker: Option<&dyn SentenceSpeaker>,
) -> Result<bool, WisecrowError> {
    let Some(speaker) = speaker else {
        return Ok(true);
    };
    if !lang::is_valid_code(lang) {
        return Err(WisecrowError::InvalidInput(format!(
            "Invalid language code: {lang}"
        )));
    }

    let examples = correct_examples(pool, lang, None).await?;
    for example in &examples {
        let fingerprint = match speaker.fingerprint(&example.language, &example.sentence) {
            Ok(fingerprint) => fingerprint,
            Err(_) => return Ok(false),
        };
        if !cache.probe_sentence(&fingerprint, MediaType::Audio).await? {
            return Ok(false);
        }
    }
    verb_forms_voiced(pool, lang, speaker).await
}

/// `true` when this pair has no area, class, paradigm or verb-form clip left to do.
///
/// Audio idleness stays in [`grammar_audio_idle`]. This function does not open a cache.
///
/// # Errors
///
/// Returns an error when a language code is malformed or a query fails.
pub async fn grammar_areas_idle(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
) -> Result<bool, WisecrowError> {
    if !lang::is_valid_code(native_lang) || !lang::is_valid_code(foreign_lang) {
        return Err(WisecrowError::InvalidInput(format!(
            "Invalid language code: {native_lang}/{foreign_lang}"
        )));
    }
    let untagged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM grammar_rules gr
         JOIN languages l ON l.id = gr.language_id
         WHERE l.code = $1 AND gr.area IS NULL",
    )
    .bind(foreign_lang)
    .fetch_one(pool)
    .await?;
    if untagged > 0 {
        return Ok(false);
    }
    if unclassified_window(pool, native_lang, foreign_lang).await? > 0 {
        return Ok(false);
    }
    // A language with no slot table has no paradigm to generate. `paradigm_work`
    // reports that as invalid input; status must treat it as idle, not as work.
    if crate::grammar::paradigms::slots_for(foreign_lang).is_err() {
        return Ok(true);
    }
    let paradigm = crate::grammar::paradigms::paradigm_work(
        pool,
        native_lang,
        foreign_lang,
        crate::grammar::paradigms::PARADIGM_VERSION,
        1,
    )
    .await?;
    Ok(paradigm.is_empty())
}

async fn unclassified_window(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
) -> Result<i64, WisecrowError> {
    let sql = format!(
        "WITH ranked AS (
             SELECT lower(btrim(t.to_phrase, '{trim}')) AS word,
                    max(t.corpus_frequency) AS freq
             FROM corpus_evidence_translations t
             JOIN languages native ON native.id = t.from_language_id
             JOIN languages \"foreign\" ON \"foreign\".id = t.to_language_id
             WHERE native.code = $1 AND \"foreign\".code = $2
             GROUP BY 1
         ),
         windowed AS (
             SELECT word, freq
             FROM ranked
             WHERE char_length(word) BETWEEN 1 AND 64
               AND word !~ '[[:space:]]'
             ORDER BY freq DESC NULLS LAST, word
             LIMIT $3
         )
         SELECT count(*)
         FROM windowed
         WHERE NOT EXISTS (
             SELECT 1
             FROM word_classes wc
             JOIN languages l ON l.id = wc.language_id
             WHERE l.code = $2 AND wc.word = windowed.word
         )",
        trim = MATCH_TRIM_SQL
    );
    Ok(sqlx::query_scalar(&sql)
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(crate::words::classes::WORD_CLASS_WINDOW)
        .fetch_one(pool)
        .await?)
}
