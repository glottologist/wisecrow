use dioxus::prelude::*;

use super::pool;

const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_AUDIO_BYTES: u64 = 10 * 1024 * 1024;

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

    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read audio metadata: {e}")))?;
    if metadata.len() > MAX_AUDIO_BYTES {
        return Err(ServerFnError::new("Audio exceeds maximum size of 10 MB"));
    }

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read audio file: {e}")))?;

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:audio/mpeg;base64,{encoded}"))
}

#[cfg(feature = "images")]
#[server]
pub async fn get_image_data(translation_id: i32, word: String) -> Result<String, ServerFnError> {
    use wisecrow::media::cache::MediaCache;
    use wisecrow::media::images::fetch_image;
    use wisecrow::media::MediaType;

    let api_key = {
        let settings = config::Config::builder()
            .add_source(config::Environment::default())
            .build()
            .map_err(|e| ServerFnError::new(format!("Config error: {e}")))?;
        settings
            .get_string("UNSPLASH_API_KEY")
            .map_err(|_| ServerFnError::new("UNSPLASH_API_KEY not configured"))?
    };

    let db = pool()?;
    let client = reqwest::Client::new();
    let cache = MediaCache::new(db.clone()) // clone: MediaCache takes owned PgPool
        .map_err(|e| ServerFnError::new(format!("Cache init failed: {e}")))?;

    let path = cache
        .get_or_fetch(translation_id, MediaType::Image, || {
            fetch_image(&client, &word, &api_key)
        })
        .await
        .map_err(|e| ServerFnError::new(format!("Image fetch failed: {e}")))?;

    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read image metadata: {e}")))?;
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(ServerFnError::new("Image exceeds maximum size of 5 MB"));
    }

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to read image file: {e}")))?;

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:image/jpeg;base64,{encoded}"))
}
