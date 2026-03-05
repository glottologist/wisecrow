use dioxus::prelude::*;

use super::pool;

/// Returns base64-encoded MP3 audio for a card's foreign phrase.
///
/// Generates TTS audio via wisecrow-core's media cache, falling back
/// to direct generation if uncached. Returns a data URI suitable for
/// an HTML `<audio>` element's `src` attribute.
#[cfg(feature = "audio")]
#[server]
pub async fn get_audio_data(
    translation_id: i32,
    foreign_phrase: String,
    foreign_lang: String,
) -> Result<String, ServerFnError> {
    use wisecrow::media::audio::generate_tts;
    use wisecrow::media::cache::MediaCache;
    use wisecrow::media::MediaType;

    let db = pool()?;
    let cache = MediaCache::new(db.clone()) // clone: MediaCache takes owned PgPool
        .map_err(|e| ServerFnError::new(format!("Cache init failed: {e}")))?;

    let path = cache
        .get_or_fetch(translation_id, MediaType::Audio, || {
            generate_tts(&foreign_phrase, &foreign_lang)
        })
        .await
        .map_err(|e| ServerFnError::new(format!("Audio generation failed: {e}")))?;

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read audio file: {e}")))?;

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:audio/mpeg;base64,{encoded}"))
}

/// Returns base64-encoded JPEG image for a vocabulary word.
///
/// Fetches from Unsplash via wisecrow-core's media cache. Returns a data
/// URI suitable for an HTML `<img>` element's `src` attribute.
#[cfg(feature = "images")]
#[server]
pub async fn get_image_data(
    translation_id: i32,
    word: String,
    unsplash_api_key: String,
) -> Result<String, ServerFnError> {
    use wisecrow::media::cache::MediaCache;
    use wisecrow::media::images::fetch_image;
    use wisecrow::media::MediaType;

    let db = pool()?;
    let client = reqwest::Client::new();
    let cache = MediaCache::new(db.clone()) // clone: MediaCache takes owned PgPool
        .map_err(|e| ServerFnError::new(format!("Cache init failed: {e}")))?;

    let path = cache
        .get_or_fetch(translation_id, MediaType::Image, || {
            fetch_image(&client, &word, &unsplash_api_key)
        })
        .await
        .map_err(|e| ServerFnError::new(format!("Image fetch failed: {e}")))?;

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read image file: {e}")))?;

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:image/jpeg;base64,{encoded}"))
}
