pub mod cache;
pub mod prefetch;

#[cfg(feature = "tts")]
pub mod audio;
#[cfg(feature = "tts")]
pub mod cereproc;

#[cfg(feature = "images")]
pub mod image_provider;
#[cfg(feature = "images")]
pub mod images;
#[cfg(feature = "images")]
pub mod providers;

use sqlx::PgPool;

use crate::config::Config;
use crate::errors::WisecrowError;

#[cfg(feature = "tts")]
use crate::media::cereproc::CereprocClient;
#[cfg(feature = "images")]
use crate::media::images::ImageFetcher;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Audio,
    Image,
}

/// The authoritative text and language for a translation's media, loaded
/// server-side so no client-supplied text can reach the cache.
#[derive(Debug, PartialEq)]
pub struct MediaSubject {
    pub to_phrase: String,
    pub from_phrase: String,
    pub foreign_lang: String,
}

/// Loads the media subject for a translation.
///
/// # Errors
///
/// Returns an error on query failure; `Ok(None)` for an unknown id.
pub async fn load_media_subject(
    pool: &PgPool,
    translation_id: i32,
) -> Result<Option<MediaSubject>, WisecrowError> {
    let row = sqlx::query_as::<_, (String, String, String)>(
        "SELECT t.to_phrase, t.from_phrase, tl.code
         FROM translations t
         JOIN languages tl ON tl.id = t.to_language_id
         WHERE t.id = $1",
    )
    .bind(translation_id)
    .fetch_optional(pool)
    .await?;
    Ok(
        row.map(|(to_phrase, from_phrase, foreign_lang)| MediaSubject {
            to_phrase,
            from_phrase,
            foreign_lang,
        }),
    )
}

/// Holds shared resources for media fetching (audio + images).
///
/// Always constructible regardless of feature flags; feature-gated code
/// in `tui::app` decides which operations to perform.
pub struct MediaContext {
    pub cache: cache::MediaCache,
    pub http_client: reqwest::Client,
    pub foreign_lang: String,
    #[cfg(feature = "images")]
    pub image_fetcher: Option<ImageFetcher>,
    #[cfg(feature = "tts")]
    pub cereproc: Option<CereprocClient>,
}

impl MediaContext {
    /// Builds a media context from the database pool and full app config.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created.
    pub fn from_config(
        pool: PgPool,
        foreign_lang: impl Into<String>,
        config: &Config,
    ) -> Result<Self, WisecrowError> {
        let cache = cache::MediaCache::new(pool)?;
        let http_client = reqwest::Client::new();
        Ok(Self {
            cache,
            http_client,
            foreign_lang: foreign_lang.into(),
            #[cfg(feature = "images")]
            image_fetcher: ImageFetcher::from_config(config),
            #[cfg(feature = "tts")]
            cereproc: CereprocClient::from_config(config),
        })
    }

    /// Builds a media context with an explicit image fetcher (tests / custom wiring).
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created.
    pub fn new(
        pool: PgPool,
        foreign_lang: impl Into<String>,
        #[cfg(feature = "images")] image_fetcher: Option<ImageFetcher>,
        #[cfg(feature = "tts")] cereproc: Option<CereprocClient>,
    ) -> Result<Self, WisecrowError> {
        let cache = cache::MediaCache::new(pool)?;
        let http_client = reqwest::Client::new();
        Ok(Self {
            cache,
            http_client,
            foreign_lang: foreign_lang.into(),
            #[cfg(feature = "images")]
            image_fetcher,
            #[cfg(feature = "tts")]
            cereproc,
        })
    }
}

impl MediaType {
    /// Stable low-byte discriminant for the cache's advisory-lock key.
    #[must_use]
    pub const fn lock_discriminant(self) -> u8 {
        match self {
            Self::Audio => 0,
            Self::Image => 1,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Image => "image",
        }
    }

    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Audio => "mp3",
            Self::Image => "jpg",
        }
    }
}
