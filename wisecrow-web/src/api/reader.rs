//! The story a learner reads, and the audio for one sentence or one word.

use dioxus::prelude::*;
use wisecrow_dto::{ChapterDto, GlossDto, LibraryDto};

#[cfg(feature = "server")]
fn map_error(operation: &'static str, error: wisecrow::errors::WisecrowError) -> ServerFnError {
    match error {
        wisecrow::errors::WisecrowError::InvalidInput(message) => {
            crate::server::client_error(axum::http::StatusCode::BAD_REQUEST, &message)
        }
        other => crate::server::internal_error(operation, &other),
    }
}

/// Ready books for the pair, with the learner's place in each file.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/reader/library")]
pub async fn library(native: String, foreign: String) -> Result<LibraryDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    let books = wisecrow::reader::library::library_books(crate::server::pool()?, user.id, &foreign)
        .await
        .map_err(|error| map_error("reader library", error))?;
    Ok(LibraryDto { books })
}

/// One chapter, once every sentence of the book is translated.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/reader/chapter")]
pub async fn chapter(
    native: String,
    foreign: String,
    book_id: i32,
    chapter_number: i32,
) -> Result<ChapterDto, ServerFnError> {
    crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    wisecrow::reader::library::chapter_text(
        crate::server::pool()?,
        &native,
        &foreign,
        book_id,
        chapter_number,
    )
    .await
    .map_err(|error| map_error("reader chapter", error))
}

/// The stored note for a word. A missing note is pending and is not generated.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/reader/gloss")]
pub async fn gloss(
    native: String,
    foreign: String,
    normalised: String,
) -> Result<GlossDto, ServerFnError> {
    crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    wisecrow::reader::library::gloss_for(crate::server::pool()?, &native, &foreign, &normalised)
        .await
        .map_err(|error| map_error("reader gloss", error))
}

/// Remembers the learner's place in the book's file.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/reader/position")]
pub async fn save_position(
    book_id: i32,
    chapter: i32,
    paragraph: i32,
    sentence: i32,
) -> Result<(), ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    wisecrow::reader::library::save_reading_position(
        crate::server::pool()?,
        user.id,
        book_id,
        chapter,
        paragraph,
        sentence,
    )
    .await
    .map_err(|error| map_error("reader position", error))
}

/// A data URL for one translated sentence. A stored clip is not spoken again.
///
/// # Errors
///
/// Returns validation, authentication, capability, or sanitized media errors.
#[post("/api/reader/sentence-audio")]
pub async fn sentence_audio(translation_id: i32) -> Result<String, ServerFnError> {
    crate::server::auth::current_user().await?;
    #[cfg(feature = "audio")]
    {
        sentence_audio_url(translation_id).await
    }
    #[cfg(not(feature = "audio"))]
    Err(crate::server::client_error(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "Audio capability is unavailable",
    ))
}

/// A data URL for one word. A stored clip is not spoken again.
///
/// # Errors
///
/// Returns validation, authentication, capability, or sanitized media errors.
#[post("/api/reader/word-audio")]
pub async fn word_audio(
    native: String,
    foreign: String,
    normalised: String,
) -> Result<String, ServerFnError> {
    crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    #[cfg(feature = "audio")]
    {
        word_audio_url(&native, &foreign, &normalised).await
    }
    #[cfg(not(feature = "audio"))]
    Err(crate::server::client_error(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "Audio capability is unavailable",
    ))
}

#[cfg(all(feature = "server", feature = "audio"))]
const MAX_AUDIO_BYTES: u64 = 10 * 1024 * 1024;

#[cfg(all(feature = "server", feature = "audio"))]
async fn sentence_audio_url(translation_id: i32) -> Result<String, ServerFnError> {
    let db = crate::server::pool()?;
    let cache =
        wisecrow::media::cache::MediaCache::new(db.clone()) // clone: the cache and the query share the pool
            .map_err(|error| crate::server::internal_error("reader audio cache", &error))?;
    let speaker = speaker()?;
    let path = wisecrow::reader::audio::sentence_clip(db, &cache, translation_id, &speaker)
        .await
        .map_err(|error| map_error("reader sentence audio", error))?;
    data_url(&path).await
}

#[cfg(all(feature = "server", feature = "audio"))]
async fn word_audio_url(
    native: &str,
    foreign: &str,
    normalised: &str,
) -> Result<String, ServerFnError> {
    let db = crate::server::pool()?;
    let cache =
        wisecrow::media::cache::MediaCache::new(db.clone()) // clone: the cache and the query share the pool
            .map_err(|error| crate::server::internal_error("reader audio cache", &error))?;
    let speaker = speaker()?;
    let path =
        wisecrow::reader::audio::word_clip(db, &cache, native, foreign, normalised, &speaker)
            .await
            .map_err(|error| map_error("reader word audio", error))?;
    data_url(&path).await
}

#[cfg(all(feature = "server", feature = "audio"))]
fn speaker() -> Result<wisecrow::media::grammar_audio::TtsSpeaker, ServerFnError> {
    let cereproc =
        wisecrow::media::cereproc::CereprocClient::from_config(&super::media::app_config()?);
    Ok(wisecrow::media::grammar_audio::TtsSpeaker::new(cereproc))
}

#[cfg(all(feature = "server", feature = "audio"))]
async fn data_url(path: &std::path::Path) -> Result<String, ServerFnError> {
    use base64::Engine as _;

    let bytes = super::media::read_bounded_file(path, MAX_AUDIO_BYTES, "Audio").await?;
    let mime = if is_wav(&bytes) {
        "audio/wav"
    } else {
        "application/octet-stream"
    };
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(["data:", mime, ";base64,", encoded.as_str()].concat())
}

#[cfg(all(feature = "server", feature = "audio"))]
fn is_wav(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE"
}
