//! Speech for a translated sentence and for a word's normalised form.
//!
//! Both clips are `sentence_media` rows. One spelling is spoken once. The
//! fingerprint is stored only after the clip is written, so a form the
//! speaker refuses stays null and is not required again.

use std::path::{Path, PathBuf};

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::media::cache::MediaCache;
use crate::media::fingerprint::MediaFingerprint;
use crate::media::grammar_audio::SentenceSpeaker;
use crate::media::MediaType;

/// Sentences voiced in one prepare-readers step.
pub const SENTENCE_BATCH: i64 = 100;
/// Word glosses voiced in one prepare-readers step.
pub const WORD_BATCH: i64 = 200;

/// Speaks up to `limit` distinct unvoiced translations of `language_code`.
///
/// Every row of a text just spoken receives the clip's fingerprint. A form
/// the speaker refuses is left null. Returns how many rows were marked.
///
/// # Errors
///
/// Returns the speaker's error when synthesis fails before a clip is
/// written, or a database error.
pub async fn voice_sentence_batch(
    pool: &PgPool,
    language_code: &str,
    speaker: &dyn SentenceSpeaker,
    cache_dir: &Path,
    limit: i64,
) -> Result<usize, WisecrowError> {
    if limit < 1 {
        return Ok(0);
    }
    let texts: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT bt.text
         FROM book_translations bt
         JOIN languages l ON l.id = bt.language_id
         WHERE l.code = $1 AND bt.audio_fingerprint IS NULL
         ORDER BY bt.text
         LIMIT $2",
    )
    .bind(language_code)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    if texts.is_empty() {
        return Ok(0);
    }
    let cache = writable_cache(pool, cache_dir)?;
    let mut stored = 0usize;
    for text in &texts {
        let Some(fingerprint) = speak_once(&cache, speaker, language_code, text).await? else {
            continue;
        };
        let updated = sqlx::query(
            "UPDATE book_translations bt
             SET audio_fingerprint = $1
             FROM languages l
             WHERE bt.language_id = l.id
               AND l.code = $2
               AND bt.text = $3
               AND bt.audio_fingerprint IS NULL",
        )
        .bind(fingerprint.as_str())
        .bind(language_code)
        .bind(text)
        .execute(pool)
        .await?
        .rows_affected();
        stored = stored.saturating_add(usize::try_from(updated).unwrap_or(usize::MAX));
    }
    Ok(stored)
}

/// Speaks up to `limit` distinct unvoiced glosses of `language_code`.
///
/// Only `status = 'ok'` rows are spoken, and the clip is the normalised
/// form. Returns how many rows were marked.
///
/// # Errors
///
/// Returns the speaker's error when synthesis fails before a clip is
/// written, or a database error.
pub async fn voice_word_batch(
    pool: &PgPool,
    language_code: &str,
    speaker: &dyn SentenceSpeaker,
    cache_dir: &Path,
    limit: i64,
) -> Result<usize, WisecrowError> {
    if limit < 1 {
        return Ok(0);
    }
    let forms: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT bg.normalised
         FROM book_glosses bg
         JOIN languages l ON l.id = bg.language_id
         WHERE l.code = $1 AND bg.status = 'ok' AND bg.audio_fingerprint IS NULL
         ORDER BY bg.normalised
         LIMIT $2",
    )
    .bind(language_code)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    if forms.is_empty() {
        return Ok(0);
    }
    let cache = writable_cache(pool, cache_dir)?;
    let mut stored = 0usize;
    for form in &forms {
        let Some(fingerprint) = speak_once(&cache, speaker, language_code, form).await? else {
            continue;
        };
        let updated = sqlx::query(
            "UPDATE book_glosses bg
             SET audio_fingerprint = $1
             FROM languages l
             WHERE bg.language_id = l.id
               AND l.code = $2
               AND bg.normalised = $3
               AND bg.status = 'ok'
               AND bg.audio_fingerprint IS NULL",
        )
        .bind(fingerprint.as_str())
        .bind(language_code)
        .bind(form)
        .execute(pool)
        .await?
        .rows_affected();
        stored = stored.saturating_add(usize::try_from(updated).unwrap_or(usize::MAX));
    }
    Ok(stored)
}

fn writable_cache(pool: &PgPool, cache_dir: &Path) -> Result<MediaCache, WisecrowError> {
    std::fs::create_dir_all(cache_dir.join(MediaType::Audio.as_str()))?;
    Ok(MediaCache::open_existing_at(pool.clone(), cache_dir)) // clone: the cache shares the pool
}

/// The clip for one translation. A stored fingerprint is not spoken again.
///
/// # Errors
///
/// Returns [`WisecrowError::InvalidInput`] when the translation is missing,
/// [`WisecrowError::MediaError`] when there is no voice or the clip is
/// missing, or a database error.
pub async fn sentence_clip(
    pool: &PgPool,
    cache: &MediaCache,
    translation_id: i32,
    speaker: &dyn SentenceSpeaker,
) -> Result<PathBuf, WisecrowError> {
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT bt.text, l.code, bt.audio_fingerprint
         FROM book_translations bt
         JOIN languages l ON l.id = bt.language_id
         WHERE bt.id = $1",
    )
    .bind(translation_id)
    .fetch_optional(pool)
    .await?;
    let Some((text, language, fingerprint)) = row else {
        return Err(WisecrowError::InvalidInput(
            "that sentence is not translated".into(),
        ));
    };
    if let Some(fingerprint) = fingerprint {
        return stored_clip(pool, &fingerprint).await;
    }
    let Some(fingerprint) = speak_once(cache, speaker, &language, &text).await? else {
        return Err(WisecrowError::MediaError(
            "no voice for this sentence".into(),
        ));
    };
    sqlx::query(
        "UPDATE book_translations SET audio_fingerprint = $1
         WHERE id = $2 AND audio_fingerprint IS NULL",
    )
    .bind(fingerprint.as_str())
    .bind(translation_id)
    .execute(pool)
    .await?;
    stored_clip(pool, fingerprint.as_str()).await
}

/// The clip for one gloss. A stored fingerprint is not spoken again.
///
/// # Errors
///
/// Returns [`WisecrowError::InvalidInput`] when the form has no ok note,
/// [`WisecrowError::MediaError`] when there is no voice or the clip is
/// missing, or a database error.
pub async fn word_clip(
    pool: &PgPool,
    cache: &MediaCache,
    native: &str,
    foreign: &str,
    normalised: &str,
    speaker: &dyn SentenceSpeaker,
) -> Result<PathBuf, WisecrowError> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT bg.audio_fingerprint
         FROM book_glosses bg
         JOIN languages fl ON fl.id = bg.language_id
         JOIN languages nl ON nl.id = bg.native_language_id
         WHERE fl.code = $1 AND nl.code = $2 AND bg.normalised = $3 AND bg.status = 'ok'",
    )
    .bind(foreign)
    .bind(native)
    .bind(normalised)
    .fetch_optional(pool)
    .await?;
    let Some((fingerprint,)) = row else {
        return Err(WisecrowError::InvalidInput("that word has no note".into()));
    };
    if let Some(fingerprint) = fingerprint {
        return stored_clip(pool, &fingerprint).await;
    }
    let Some(fingerprint) = speak_once(cache, speaker, foreign, normalised).await? else {
        return Err(WisecrowError::MediaError("no voice for this word".into()));
    };
    sqlx::query(
        "UPDATE book_glosses bg
         SET audio_fingerprint = $1
         FROM languages fl, languages nl
         WHERE bg.language_id = fl.id AND bg.native_language_id = nl.id
           AND fl.code = $2 AND nl.code = $3 AND bg.normalised = $4
           AND bg.status = 'ok' AND bg.audio_fingerprint IS NULL",
    )
    .bind(fingerprint.as_str())
    .bind(foreign)
    .bind(native)
    .bind(normalised)
    .execute(pool)
    .await?;
    stored_clip(pool, fingerprint.as_str()).await
}

async fn stored_clip(pool: &PgPool, fingerprint: &str) -> Result<PathBuf, WisecrowError> {
    let path: Option<String> = sqlx::query_scalar(
        "SELECT file_path FROM sentence_media WHERE fingerprint = $1 AND media_type = $2",
    )
    .bind(fingerprint)
    .bind(MediaType::Audio.as_str())
    .fetch_optional(pool)
    .await?;
    path.map(PathBuf::from)
        .ok_or_else(|| WisecrowError::MediaError("the stored clip is missing".into()))
}

/// `None` when the speaker has no voice for this text. The column stays null.
async fn speak_once(
    cache: &MediaCache,
    speaker: &dyn SentenceSpeaker,
    language_code: &str,
    text: &str,
) -> Result<Option<MediaFingerprint>, WisecrowError> {
    let fingerprint = match speaker.fingerprint(language_code, text) {
        Ok(fingerprint) => fingerprint,
        Err(_) => return Ok(None),
    };
    cache
        .get_or_fetch_sentence(&fingerprint, MediaType::Audio, language_code, text, || {
            speaker.speak(language_code, text)
        })
        .await?;
    Ok(Some(fingerprint))
}
