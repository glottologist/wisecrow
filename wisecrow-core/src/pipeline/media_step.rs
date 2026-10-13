//! One batch of ranked single-word rows that still need audio or a picture.
//!
//! A live file for the current fingerprint is done. A recorded empty picture
//! search is done. A phrase, a sentence, and a row with no corpus frequency
//! are not in the batch.

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::lang;
use crate::media::cache::MediaCache;
use crate::media::{load_media_subject, MediaSubject, MediaType};

const PAGE_LEN: i64 = 500;
const BATCH_LIMIT: usize = 200;

/// Speech and image identity used to decide whether a cached file is current.
///
/// A side whose `check_*` flag is false is already satisfied. That covers a
/// build without the feature, and a language that has no speech voice: the
/// picture step must not wait on audio that cannot be produced.
pub struct ProbeProfile<'a> {
    pub audio_backend: &'a str,
    pub audio_voice: &'a str,
    pub audio_format: u32,
    pub check_audio: bool,
    pub provider_ids: &'a [&'a str],
    pub image_format: u32,
    pub check_image: bool,
    /// `false` when no image provider is configured, so a fingerprint cannot
    /// be named. A live stored file still counts; a gap is unfinished work.
    pub image_providers_known: bool,
}

/// Records an authoritative empty picture search.
///
/// A timeout or a provider error must not be stored here.
///
/// # Errors
///
/// Returns an error when `fingerprint` is not 64 lowercase hex characters,
/// or when the insert fails.
pub async fn record_picture_miss(pool: &PgPool, fingerprint: &str) -> Result<(), WisecrowError> {
    if !is_lowercase_hex(fingerprint) {
        return Err(WisecrowError::InvalidInput(
            "picture miss fingerprint must be 64 lowercase hex characters".to_owned(),
        ));
    }
    sqlx::query(
        "INSERT INTO pipeline_picture_misses (fingerprint) VALUES ($1)
         ON CONFLICT (fingerprint) DO NOTHING",
    )
    .bind(fingerprint)
    .execute(pool)
    .await?;
    Ok(())
}

fn is_lowercase_hex(fingerprint: &str) -> bool {
    fingerprint.len() == 64
        && fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_single_word(phrase: &str) -> bool {
    let mut parts = phrase.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some(word), None) => !word.is_empty() && lang::is_word_length(word),
        _ => false,
    }
}

async fn needs_media(
    cache: &MediaCache,
    translation_id: i32,
    subject: &MediaSubject,
    profile: &ProbeProfile<'_>,
) -> Result<bool, WisecrowError> {
    let audio_ready = if profile.check_audio {
        let fingerprint = subject.audio_fingerprint(
            profile.audio_backend,
            profile.audio_voice,
            profile.audio_format,
        );
        cache
            .probe(translation_id, MediaType::Audio, &fingerprint)
            .await?
    } else {
        true
    };

    let image_ready = if !profile.check_image {
        true
    } else if !profile.image_providers_known {
        cache
            .stored_published(translation_id, MediaType::Image)
            .await?
            .is_some()
    } else {
        match subject.image_fingerprint(profile.provider_ids, profile.image_format) {
            None => true,
            Some(fingerprint) => {
                cache
                    .probe(translation_id, MediaType::Image, &fingerprint)
                    .await?
                    || cache.picture_miss_recorded(fingerprint.as_str()).await?
            }
        }
    };

    Ok(!audio_ready || !image_ready)
}

/// The next ranked single-word rows of this pair that still need media.
///
/// At most 200 ids are returned. Rows that are already complete are skipped,
/// so a finished prefix does not hide later work.
///
/// # Errors
///
/// Returns an error when a query or a cache probe fails.
pub async fn next_media_batch(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    cache: &MediaCache,
    profile: &ProbeProfile<'_>,
) -> Result<Vec<i32>, WisecrowError> {
    if !lang::is_valid_code(native) || !lang::is_valid_code(foreign) {
        return Err(WisecrowError::InvalidInput(
            "Invalid language code".to_owned(),
        ));
    }

    let mut after_id = 0i32;
    let mut batch = Vec::new();
    loop {
        let page: Vec<(i32, String)> = sqlx::query_as(
            "SELECT t.id, t.to_phrase
               FROM translations t
               JOIN languages fl ON fl.id = t.from_language_id
               JOIN languages tl ON tl.id = t.to_language_id
              WHERE fl.code = $1 AND tl.code = $2
                AND t.corpus_frequency >= 1
                AND position(' ' IN t.to_phrase) = 0
                AND NOT EXISTS (
                    SELECT 1 FROM phrase_translations p WHERE p.translation_id = t.id
                )
                AND t.id > $3
              ORDER BY t.id
              LIMIT $4",
        )
        .bind(native)
        .bind(foreign)
        .bind(after_id)
        .bind(PAGE_LEN)
        .fetch_all(pool)
        .await?;

        let Some((last_id, _)) = page.last() else {
            break;
        };
        after_id = *last_id;

        for (id, phrase) in &page {
            if !is_single_word(phrase) {
                continue;
            }
            let Some(subject) = load_media_subject(pool, *id).await? else {
                continue;
            };
            if subject.is_phrase || !needs_media(cache, *id, &subject, profile).await? {
                continue;
            }
            batch.push(*id);
            if batch.len() == BATCH_LIMIT {
                return Ok(batch);
            }
        }

        if page.len() < usize::try_from(PAGE_LEN).unwrap_or(usize::MAX) {
            break;
        }
    }
    Ok(batch)
}
