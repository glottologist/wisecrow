//! The version-2 media route a device fetches clips and pictures through.
//!
//! One route serves both owners. A translation's media is what the web
//! serves for the same id; a sentence's clip is looked up by the fingerprint
//! the bank page named, and generated only when that fingerprint matches a
//! correct example the server itself holds. Authorisation is the
//! authenticated user's: the server keeps no per-device language state.

use dioxus::prelude::*;
use wisecrow_dto::{MobileMediaDto, MobileMediaRequestDto};

/// Returns one clip or picture for the authenticated learner.
///
/// # Errors
///
/// Returns protocol, validation, authentication, capability or sanitized
/// media errors.
#[post("/api/mobile/v2/media/fetch")]
pub async fn mobile_media_fetch(
    request: MobileMediaRequestDto,
) -> Result<MobileMediaDto, ServerFnError> {
    implementation::fetch(request).await
}

#[cfg(feature = "server")]
mod implementation {
    use axum::http::StatusCode;
    use wisecrow_dto::{MobileMediaDto, MobileMediaRequestDto, MOBILE_PROTOCOL_VERSION_V2};

    use super::ServerFnError;

    pub(super) async fn fetch(
        request: MobileMediaRequestDto,
    ) -> Result<MobileMediaDto, ServerFnError> {
        crate::server::auth::current_user().await?;
        if request.protocol_version != MOBILE_PROTOCOL_VERSION_V2 {
            return Err(crate::server::client_error(
                StatusCode::CONFLICT,
                "Unsupported mobile protocol version",
            ));
        }
        serve(request).await
    }

    #[cfg(not(any(feature = "audio", feature = "images")))]
    async fn serve(_request: MobileMediaRequestDto) -> Result<MobileMediaDto, ServerFnError> {
        Err(crate::server::client_error(
            StatusCode::NOT_IMPLEMENTED,
            "Media capability is unavailable",
        ))
    }

    #[cfg(any(feature = "audio", feature = "images"))]
    use served::serve;

    /// Serving proper, which needs a build that can voice or picture
    /// something: without either there is nothing to encode.
    #[cfg(any(feature = "audio", feature = "images"))]
    mod served {
        use axum::http::StatusCode;
        use base64::Engine as _;
        use wisecrow_dto::{
            MediaOwnerDto, MobileMediaDto, MobileMediaRequestDto, MobileMediaTypeDto,
            MOBILE_PROTOCOL_VERSION_V2,
        };

        use super::ServerFnError;

        const MAX_AUDIO_BYTES: u64 = 10 * 1024 * 1024;
        const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

        /// The file served, the identity of its bytes, its credit and the
        /// name a size error gives it.
        type Served = (
            std::path::PathBuf,
            Option<String>,
            Option<String>,
            &'static str,
        );

        pub(super) async fn serve(
            request: MobileMediaRequestDto,
        ) -> Result<MobileMediaDto, ServerFnError> {
            let (path, fingerprint, attribution, name) = match (&request.owner, request.media_type)
            {
                (MediaOwnerDto::Translation { id }, MobileMediaTypeDto::Audio) => {
                    translation_audio(*id).await?
                }
                (MediaOwnerDto::Translation { id }, MobileMediaTypeDto::Image) => {
                    translation_image(*id).await?
                }
                (MediaOwnerDto::Sentence { fingerprint }, MobileMediaTypeDto::Audio) => {
                    sentence_audio(fingerprint).await?
                }
                (MediaOwnerDto::Sentence { .. }, MobileMediaTypeDto::Image) => {
                    return Err(crate::server::client_error(
                        StatusCode::BAD_REQUEST,
                        "A sentence has no picture",
                    ));
                }
            };
            let limit = match request.media_type {
                MobileMediaTypeDto::Audio => MAX_AUDIO_BYTES,
                MobileMediaTypeDto::Image => MAX_IMAGE_BYTES,
            };
            let bytes = crate::api::media::read_bounded_file(&path, limit, name).await?;
            Ok(MobileMediaDto {
                protocol_version: MOBILE_PROTOCOL_VERSION_V2,
                owner: request.owner,
                media_type: request.media_type,
                bytes_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                fingerprint,
                attribution,
            })
        }

        fn not_found(what: &str) -> ServerFnError {
            crate::server::client_error(StatusCode::NOT_FOUND, what)
        }

        fn validate_id(id: i32) -> Result<(), ServerFnError> {
            if id <= 0 {
                return Err(crate::server::client_error(
                    StatusCode::BAD_REQUEST,
                    "Invalid media request",
                ));
            }
            Ok(())
        }

        async fn subject(id: i32) -> Result<wisecrow::media::MediaSubject, ServerFnError> {
            validate_id(id)?;
            wisecrow::media::load_media_subject(crate::server::pool()?, id)
                .await
                .map_err(|error| crate::server::internal_error("media subject load", &error))?
                .ok_or_else(|| not_found("Unknown translation"))
        }

        #[cfg(feature = "audio")]
        async fn translation_audio(id: i32) -> Result<Served, ServerFnError> {
            use wisecrow::media::cache::MediaCache;
            use wisecrow::media::MediaType;

            let subject = subject(id).await?;
            let db = crate::server::pool()?;
            let cache = MediaCache::new(db.clone()) // clone: MediaCache owns an Arc-backed pool handle
                .map_err(|error| {
                    crate::server::internal_error("audio cache initialization", &error)
                })?;
            let cereproc = wisecrow::media::cereproc::CereprocClient::from_config(
                &crate::api::media::app_config()?,
            );
            let fingerprint = wisecrow::media::audio_cache_key(&subject, cereproc.as_ref())
                .map_err(|error| crate::server::internal_error("audio fingerprint", &error))?;
            let path = cache
                .get_or_fetch(id, MediaType::Audio, &fingerprint, || {
                    wisecrow::media::audio::generate_tts(
                        &subject.to_phrase,
                        &subject.foreign_lang,
                        cereproc.as_ref(),
                    )
                })
                .await
                .map_err(|error| crate::server::internal_error("audio generation", &error))?;
            Ok((path, Some(fingerprint.as_str().to_owned()), None, "Audio"))
        }

        #[cfg(not(feature = "audio"))]
        async fn translation_audio(id: i32) -> Result<Served, ServerFnError> {
            validate_id(id)?;
            Err(crate::server::client_error(
                StatusCode::NOT_IMPLEMENTED,
                "Audio capability is unavailable",
            ))
        }

        #[cfg(feature = "images")]
        async fn translation_image(id: i32) -> Result<Served, ServerFnError> {
            use wisecrow::media::cache::MediaCache;
            use wisecrow::media::MediaType;

            let subject = subject(id).await?;
            let db = crate::server::pool()?;
            let query = subject
                .applicable_image_query()
                .ok_or_else(|| not_found("No picture applies"))?
                .to_owned();
            let fetcher = crate::api::media::image_fetcher()?;
            let fingerprint = wisecrow::media::image_cache_key(&subject, &fetcher)
                .ok_or_else(|| not_found("No picture applies"))?;
            let client = reqwest::Client::new();
            let cache = MediaCache::new(db.clone()) // clone: MediaCache owns an Arc-backed pool handle
                .map_err(|error| {
                    crate::server::internal_error("image cache initialization", &error)
                })?;
            let (path, attribution) = cache
                .get_or_fetch_attributed(id, MediaType::Image, &fingerprint, || async {
                    wisecrow::media::images::fetch_image(&client, &query, &fetcher)
                        .await
                        .map(|image| (image.bytes, image.attribution))
                })
                .await
                .map_err(|error| crate::server::internal_error("image fetch", &error))?;
            Ok((
                path,
                Some(fingerprint.as_str().to_owned()),
                attribution,
                "Image",
            ))
        }

        #[cfg(not(feature = "images"))]
        async fn translation_image(id: i32) -> Result<Served, ServerFnError> {
            validate_id(id)?;
            Err(crate::server::client_error(
                StatusCode::NOT_IMPLEMENTED,
                "Image capability is unavailable",
            ))
        }

        #[cfg(feature = "audio")]
        async fn sentence_audio(fingerprint: &str) -> Result<Served, ServerFnError> {
            use wisecrow::media::cache::MediaCache;
            use wisecrow::media::grammar_audio::{SentenceSpeaker as _, TtsSpeaker};
            use wisecrow::media::MediaType;

            if fingerprint.len() != 64 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(crate::server::client_error(
                    StatusCode::BAD_REQUEST,
                    "Invalid media request",
                ));
            }
            let db = crate::server::pool()?;
            let stored: Option<(String, String)> = sqlx::query_as(
                "SELECT language_code, sentence FROM sentence_media
                 WHERE fingerprint = $1 AND media_type = 'audio'",
            )
            .bind(fingerprint)
            .fetch_optional(db)
            .await
            .map_err(|error| crate::server::internal_error("sentence media load", &error))?;
            let speaker = TtsSpeaker::new(wisecrow::media::cereproc::CereprocClient::from_config(
                &crate::api::media::app_config()?,
            ));
            // A clip the cache has never held is served only for a sentence
            // the server itself teaches: the fingerprint must match one of
            // its own correct examples under its own voice.
            let (language, sentence) = match stored {
                Some(found) => found,
                None => matching_example(db, &speaker, fingerprint)
                    .await?
                    .ok_or_else(|| not_found("Unknown sentence"))?,
            };
            let key = speaker
                .fingerprint(&language, &sentence)
                .map_err(|error| crate::server::internal_error("sentence fingerprint", &error))?;
            if key.as_str() != fingerprint {
                // The stored row was voiced under an older profile; what the
                // device named no longer exists.
                return Err(not_found("Unknown sentence"));
            }
            let cache = MediaCache::new(db.clone()) // clone: MediaCache owns an Arc-backed pool handle
                .map_err(|error| {
                    crate::server::internal_error("audio cache initialization", &error)
                })?;
            let path = cache
                .get_or_fetch_sentence(&key, MediaType::Audio, &language, &sentence, || {
                    speaker.speak(&language, &sentence)
                })
                .await
                .map_err(|error| {
                    crate::server::internal_error("sentence audio generation", &error)
                })?;
            Ok((path, Some(fingerprint.to_owned()), None, "Audio"))
        }

        /// The correct example whose clip identity is `fingerprint`, if any.
        #[cfg(feature = "audio")]
        async fn matching_example(
            db: &sqlx::PgPool,
            speaker: &wisecrow::media::grammar_audio::TtsSpeaker,
            fingerprint: &str,
        ) -> Result<Option<(String, String)>, ServerFnError> {
            use wisecrow::media::grammar_audio::SentenceSpeaker as _;

            let languages: Vec<String> =
                sqlx::query_scalar("SELECT code FROM languages ORDER BY code")
                    .fetch_all(db)
                    .await
                    .map_err(|error| crate::server::internal_error("language list", &error))?;
            for language in languages {
                let examples =
                    wisecrow::media::grammar_audio::correct_examples(db, &language, None)
                        .await
                        .map_err(|error| crate::server::internal_error("example list", &error))?;
                if let Some(example) = examples.into_iter().find(|example| {
                    speaker
                        .fingerprint(&example.language, &example.sentence)
                        .is_ok_and(|key| key.as_str() == fingerprint)
                }) {
                    return Ok(Some((example.language, example.sentence)));
                }
            }
            Ok(None)
        }

        #[cfg(not(feature = "audio"))]
        async fn sentence_audio(_fingerprint: &str) -> Result<Served, ServerFnError> {
            Err(crate::server::client_error(
                StatusCode::NOT_IMPLEMENTED,
                "Audio capability is unavailable",
            ))
        }
    }
}
