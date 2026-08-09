use dioxus::prelude::*;

/// Returns cached or generated audio for a learning phrase.
///
/// Takes only the translation id: the phrase and language are loaded
/// server-side, so no client-supplied text can reach the cache.
///
/// # Errors
///
/// Returns validation, authentication, capability, or sanitized media errors.
#[post("/api/media/audio")]
pub async fn get_audio_data(translation_id: i32) -> Result<String, ServerFnError> {
    crate::server::auth::current_user().await?;
    validate_media_request(translation_id)?;
    #[cfg(feature = "audio")]
    {
        implementation::audio(translation_id).await
    }
    #[cfg(not(feature = "audio"))]
    Err(crate::server::client_error(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "Audio capability is unavailable",
    ))
}

/// Returns a cached or fetched image for a learning word.
///
/// Takes only the translation id — see [`get_audio_data`].
///
/// # Errors
///
/// Returns validation, authentication, capability, or sanitized media errors.
#[post("/api/media/image")]
pub async fn get_image_data(
    translation_id: i32,
) -> Result<wisecrow_dto::CardImageDto, ServerFnError> {
    crate::server::auth::current_user().await?;
    validate_media_request(translation_id)?;
    #[cfg(feature = "images")]
    {
        implementation::image(translation_id).await
    }
    #[cfg(not(feature = "images"))]
    Err(crate::server::client_error(
        axum::http::StatusCode::NOT_IMPLEMENTED,
        "Image capability is unavailable",
    ))
}

#[cfg(feature = "server")]
fn validate_media_request(translation_id: i32) -> Result<(), ServerFnError> {
    if translation_id <= 0 {
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
    pub(super) async fn audio(translation_id: i32) -> Result<String, ServerFnError> {
        let db = crate::server::pool()?;
        let subject = load_subject(db, translation_id).await?;
        let cache =
            MediaCache::new(db.clone()) // clone: MediaCache owns an Arc-backed pool handle
                .map_err(|error| {
                    crate::server::internal_error("audio cache initialization", &error)
                })?;
        // Absent an account this is `None` and Edge handles the language, so a
        // deployment without CereProc credentials keeps its existing speech.
        let cereproc = wisecrow::media::cereproc::CereprocClient::from_config(&app_config()?);
        let path = cache
            .get_or_fetch(translation_id, MediaType::Audio, || {
                wisecrow::media::audio::generate_tts(
                    &subject.to_phrase,
                    &subject.foreign_lang,
                    cereproc.as_ref(),
                )
            })
            .await
            .map_err(|error| crate::server::internal_error("audio generation", &error))?;
        let bytes = read_bounded_file(&path, MAX_AUDIO_BYTES, "Audio").await?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(["data:audio/mpeg;base64,", encoded.as_str()].concat())
    }

    #[cfg(feature = "images")]
    pub(super) async fn image(
        translation_id: i32,
    ) -> Result<wisecrow_dto::CardImageDto, ServerFnError> {
        let fetcher = image_fetcher()?;
        let db = crate::server::pool()?;
        let subject = load_subject(db, translation_id).await?;
        let client = reqwest::Client::new();
        let cache =
            MediaCache::new(db.clone()) // clone: MediaCache owns an Arc-backed pool handle
                .map_err(|error| {
                    crate::server::internal_error("image cache initialization", &error)
                })?;
        let (path, attribution) = cache
            .get_or_fetch_attributed(translation_id, MediaType::Image, || async {
                wisecrow::media::images::fetch_image(&client, &subject.from_phrase, &fetcher)
                    .await
                    .map(|image| (image.bytes, image.attribution))
            })
            .await
            .map_err(|error| crate::server::internal_error("image fetch", &error))?;
        let bytes = read_bounded_file(&path, MAX_IMAGE_BYTES, "Image").await?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        Ok(wisecrow_dto::CardImageDto {
            data_url: ["data:image/jpeg;base64,", encoded.as_str()].concat(),
            attribution,
        })
    }

    async fn load_subject(
        pool: &sqlx::PgPool,
        translation_id: i32,
    ) -> Result<wisecrow::media::MediaSubject, ServerFnError> {
        wisecrow::media::load_media_subject(pool, translation_id)
            .await
            .map_err(|error| crate::server::internal_error("media subject load", &error))?
            .ok_or_else(|| {
                crate::server::client_error(
                    axum::http::StatusCode::NOT_FOUND,
                    "Unknown translation",
                )
            })
    }

    fn app_config() -> Result<wisecrow::config::Config, ServerFnError> {
        let settings = config::Config::builder()
            .add_source(config::Environment::with_prefix("WISECROW").separator("__"))
            .build()
            .map_err(|error| crate::server::internal_error("media configuration", &error))?;
        settings
            .try_deserialize()
            .map_err(|error| crate::server::internal_error("media configuration", &error))
    }

    #[cfg(feature = "images")]
    fn image_fetcher() -> Result<wisecrow::media::images::ImageFetcher, ServerFnError> {
        let cfg = app_config()?;
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
