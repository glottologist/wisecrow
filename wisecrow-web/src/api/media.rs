use dioxus::prelude::*;

/// Returns cached or generated audio for a learning phrase.
///
/// # Errors
///
/// Returns validation, authentication, capability, or sanitized media errors.
#[post("/api/media/audio")]
pub async fn get_audio_data(
    translation_id: i32,
    foreign_phrase: String,
    foreign_lang: String,
) -> Result<String, ServerFnError> {
    crate::server::auth::current_user().await?;
    validate_media_request(translation_id, &foreign_phrase)?;
    crate::server::validate_lang(&foreign_lang)?;
    #[cfg(feature = "audio")]
    {
        return implementation::audio(translation_id, &foreign_phrase, &foreign_lang).await;
    }
    #[cfg(not(feature = "audio"))]
    Err(crate::server::client_error(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "Audio capability is unavailable",
    ))
}

/// Returns a cached or fetched image for a learning word.
///
/// # Errors
///
/// Returns validation, authentication, capability, or sanitized media errors.
#[post("/api/media/image")]
pub async fn get_image_data(translation_id: i32, word: String) -> Result<String, ServerFnError> {
    crate::server::auth::current_user().await?;
    validate_media_request(translation_id, &word)?;
    #[cfg(feature = "images")]
    {
        return implementation::image(translation_id, &word).await;
    }
    #[cfg(not(feature = "images"))]
    Err(crate::server::client_error(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "Image capability is unavailable",
    ))
}

#[cfg(feature = "server")]
fn validate_media_request(translation_id: i32, text: &str) -> Result<(), ServerFnError> {
    const MAX_MEDIA_TEXT_BYTES: usize = 1024;
    if translation_id <= 0 || text.is_empty() || text.len() > MAX_MEDIA_TEXT_BYTES {
        return Err(crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            "Invalid media request",
        ));
    }
    Ok(())
}

#[cfg(all(feature = "server", any(feature = "audio", feature = "images")))]
mod implementation {
    use base64::Engine as _;
    use wisecrow::media::cache::MediaCache;
    use wisecrow::media::MediaType;

    use super::ServerFnError;

    #[cfg(feature = "images")]
    const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
    #[cfg(feature = "audio")]
    const MAX_AUDIO_BYTES: u64 = 10 * 1024 * 1024;

    #[cfg(feature = "audio")]
    pub(super) async fn audio(
        translation_id: i32,
        foreign_phrase: &str,
        foreign_lang: &str,
    ) -> Result<String, ServerFnError> {
        let db = crate::server::pool()?;
        let cache =
            MediaCache::new(db.clone()) // clone: MediaCache owns an Arc-backed pool handle
                .map_err(|error| {
                    crate::server::internal_error("audio cache initialization", &error)
                })?;
        let path = cache
            .get_or_fetch(translation_id, MediaType::Audio, || {
                wisecrow::media::audio::generate_tts(foreign_phrase, foreign_lang)
            })
            .await
            .map_err(|error| crate::server::internal_error("audio generation", &error))?;
        let bytes = read_bounded_file(&path, MAX_AUDIO_BYTES, "Audio").await?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(["data:audio/mpeg;base64,", encoded.as_str()].concat())
    }

    #[cfg(feature = "images")]
    pub(super) async fn image(translation_id: i32, word: &str) -> Result<String, ServerFnError> {
        let fetcher = image_fetcher()?;
        let db = crate::server::pool()?;
        let client = reqwest::Client::new();
        let cache =
            MediaCache::new(db.clone()) // clone: MediaCache owns an Arc-backed pool handle
                .map_err(|error| {
                    crate::server::internal_error("image cache initialization", &error)
                })?;
        let path = cache
            .get_or_fetch(translation_id, MediaType::Image, || {
                wisecrow::media::images::fetch_image(&client, word, &fetcher)
            })
            .await
            .map_err(|error| crate::server::internal_error("image fetch", &error))?;
        let bytes = read_bounded_file(&path, MAX_IMAGE_BYTES, "Image").await?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(["data:image/jpeg;base64,", encoded.as_str()].concat())
    }

    #[cfg(feature = "images")]
    fn image_fetcher() -> Result<wisecrow::media::images::ImageFetcher, ServerFnError> {
        let settings = config::Config::builder()
            .add_source(config::Environment::with_prefix("WISECROW").separator("__"))
            .build()
            .map_err(|error| crate::server::internal_error("image configuration", &error))?;
        let cfg: wisecrow::config::Config = settings
            .try_deserialize()
            .map_err(|error| crate::server::internal_error("image configuration", &error))?;
        wisecrow::media::images::ImageFetcher::from_config(&cfg).ok_or_else(|| {
            crate::server::client_error(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "Image capability is not configured",
            )
        })
    }

    async fn read_bounded_file(
        path: &std::path::Path,
        maximum_bytes: u64,
        media_name: &str,
    ) -> Result<Vec<u8>, ServerFnError> {
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|error| crate::server::internal_error("media metadata read", &error))?;
        if metadata.len() > maximum_bytes {
            return Err(crate::server::client_error(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                &format!("{media_name} exceeds maximum size"),
            ));
        }
        tokio::fs::read(path)
            .await
            .map_err(|error| crate::server::internal_error("media file read", &error))
    }
}
