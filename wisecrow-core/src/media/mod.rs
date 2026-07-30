pub mod cache;
pub mod prefetch;

#[cfg(feature = "tts")]
pub mod audio;

#[cfg(feature = "images")]
pub mod image_provider;
#[cfg(feature = "images")]
pub mod images;
#[cfg(feature = "images")]
pub mod providers;

use sqlx::PgPool;

use crate::config::Config;
use crate::errors::WisecrowError;

#[cfg(feature = "images")]
use crate::media::images::ImageFetcher;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Audio,
    Image,
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
    ) -> Result<Self, WisecrowError> {
        let cache = cache::MediaCache::new(pool)?;
        let http_client = reqwest::Client::new();
        Ok(Self {
            cache,
            http_client,
            foreign_lang: foreign_lang.into(),
            #[cfg(feature = "images")]
            image_fetcher,
        })
    }
}

impl MediaType {
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
