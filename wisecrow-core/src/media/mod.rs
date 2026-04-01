pub mod cache;
pub mod prefetch;

#[cfg(feature = "audio")]
pub mod audio;

#[cfg(feature = "images")]
pub mod images;

use sqlx::PgPool;

use crate::config::SecureString;
use crate::errors::WisecrowError;

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
    pub unsplash_api_key: Option<SecureString>,
}

impl MediaContext {
    /// Builds a media context from the database pool and config fields.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created.
    pub fn new(
        pool: PgPool,
        foreign_lang: String,
        unsplash_api_key: Option<SecureString>,
    ) -> Result<Self, WisecrowError> {
        let cache = cache::MediaCache::new(pool)?;
        let http_client = reqwest::Client::new();
        Ok(Self {
            cache,
            http_client,
            foreign_lang,
            unsplash_api_key,
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
