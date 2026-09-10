pub mod cache;
pub mod fingerprint;
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
use crate::presentation::{PresentationRepository, PresentedTranslation};

#[cfg(feature = "tts")]
use crate::media::cereproc::CereprocClient;
#[cfg(feature = "images")]
use crate::media::images::ImageFetcher;

use self::fingerprint::MediaFingerprint;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Audio,
    Image,
}

/// Canonical text and media metadata for a translation, loaded server-side
/// so no client-supplied text can reach the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSubject {
    pub to_phrase: String,
    pub from_phrase: String,
    pub foreign_lang: String,
    pub image_query: Option<String>,
    pub is_phrase: bool,
}

impl From<PresentedTranslation> for MediaSubject {
    fn from(presented: PresentedTranslation) -> Self {
        Self {
            to_phrase: presented.to_phrase,
            from_phrase: presented.from_phrase,
            foreign_lang: presented.foreign_lang,
            image_query: presented.image_query,
            is_phrase: presented.is_phrase,
        }
    }
}

impl MediaSubject {
    /// Concrete image search text, or `None` when no image should be fetched.
    #[must_use]
    pub fn applicable_image_query(&self) -> Option<&str> {
        if self.is_phrase {
            None
        } else {
            self.image_query.as_deref()
        }
    }

    /// Fingerprint of the canonical speech text and selected synthesis profile.
    #[must_use]
    pub fn audio_fingerprint(
        &self,
        backend: &str,
        voice: &str,
        format_version: u32,
    ) -> MediaFingerprint {
        MediaFingerprint::for_audio(
            &self.to_phrase,
            &self.foreign_lang,
            backend,
            voice,
            format_version,
        )
    }

    /// Fingerprint of the applicable image query and provider chain.
    #[must_use]
    pub fn image_fingerprint(
        &self,
        provider_ids: &[&str],
        format_version: u32,
    ) -> Option<MediaFingerprint> {
        self.applicable_image_query().map(|query| {
            MediaFingerprint::for_image(query, &self.foreign_lang, provider_ids, format_version)
        })
    }
}

/// Loads the media subject for a translation from the presentation contract.
///
/// # Errors
///
/// Returns an error on query failure; `Ok(None)` for an unknown id.
pub async fn load_media_subject(
    pool: &PgPool,
    translation_id: i32,
) -> Result<Option<MediaSubject>, WisecrowError> {
    Ok(PresentationRepository::load(pool, translation_id)
        .await?
        .map(MediaSubject::from))
}

/// Speech-cache identity for `subject` under the same profile web, TUI, and
/// prefetch use.
///
/// # Errors
///
/// Returns an error when no TTS voice exists for the subject's language.
#[cfg(feature = "tts")]
pub fn audio_cache_key(
    subject: &MediaSubject,
    cereproc: Option<&CereprocClient>,
) -> Result<MediaFingerprint, WisecrowError> {
    let profile =
        audio::tts_profile_for_language(&subject.foreign_lang, cereproc).ok_or_else(|| {
            WisecrowError::MediaError(format!(
                "No TTS voice available for language: {}",
                subject.foreign_lang
            ))
        })?;
    Ok(subject.audio_fingerprint(
        profile.backend_id(),
        profile.voice(),
        audio::AUDIO_FORMAT_VERSION,
    ))
}

/// Image-cache identity for `subject`, or `None` when no image applies.
#[cfg(feature = "images")]
#[must_use]
pub fn image_cache_key(subject: &MediaSubject, fetcher: &ImageFetcher) -> Option<MediaFingerprint> {
    let providers = fetcher.provider_ids();
    subject.image_fingerprint(&providers, images::IMAGE_FORMAT_VERSION)
}

/// Holds shared resources for media fetching (audio + images).
///
/// Always constructible regardless of feature flags; feature-gated code
/// in `tui::app` decides which operations to perform.
pub struct MediaContext {
    pub cache: cache::MediaCache,
    pub pool: PgPool,
    pub http_client: reqwest::Client,
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
    pub fn from_config(pool: PgPool, config: &Config) -> Result<Self, WisecrowError> {
        Self::new(
            pool,
            #[cfg(feature = "images")]
            ImageFetcher::from_config(config),
            #[cfg(feature = "tts")]
            CereprocClient::from_config(config),
        )
    }

    /// Builds a media context with an explicit image fetcher (tests / custom wiring).
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created.
    pub fn new(
        pool: PgPool,
        #[cfg(feature = "images")] image_fetcher: Option<ImageFetcher>,
        #[cfg(feature = "tts")] cereproc: Option<CereprocClient>,
    ) -> Result<Self, WisecrowError> {
        let cache = cache::MediaCache::new(pool.clone())?; // clone: PgPool is Arc-backed
        let http_client = reqwest::Client::new();
        Ok(Self {
            cache,
            pool,
            http_client,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dog() -> MediaSubject {
        MediaSubject {
            to_phrase: "chien".to_owned(),
            from_phrase: "dog".to_owned(),
            foreign_lang: "fr".to_owned(),
            image_query: Some("friendly dog".to_owned()),
            is_phrase: false,
        }
    }

    #[test]
    fn web_tui_prefetch_share_fingerprints_for_the_same_subject() {
        let subject = dog();
        let providers = ["unsplash", "pexels", "pixabay"];
        let audio = subject.audio_fingerprint("edge", "fr-FR-HenriNeural", 1);
        let image = subject.image_fingerprint(&providers, 1);
        assert_eq!(
            audio,
            subject.audio_fingerprint("edge", "fr-FR-HenriNeural", 1)
        );
        assert_eq!(image, subject.image_fingerprint(&providers, 1));
        assert!(image.is_some());
        assert_eq!(subject.applicable_image_query(), Some("friendly dog"));
    }

    #[test]
    fn abstract_word_and_phrase_have_no_image_fingerprint() {
        let abstract_word = MediaSubject {
            to_phrase: "en".to_owned(),
            from_phrase: "in".to_owned(),
            foreign_lang: "fr".to_owned(),
            image_query: None,
            is_phrase: false,
        };
        let phrase = MediaSubject {
            to_phrase: "comment allez vous".to_owned(),
            from_phrase: "canonical phrase translation".to_owned(),
            foreign_lang: "fr".to_owned(),
            image_query: Some("wrong image".to_owned()),
            is_phrase: true,
        };
        let providers = ["unsplash"];
        assert!(abstract_word.image_fingerprint(&providers, 1).is_none());
        assert!(phrase.image_fingerprint(&providers, 1).is_none());
        let audio = abstract_word.audio_fingerprint("edge", "fr-FR-HenriNeural", 1);
        assert_eq!(
            audio.as_str().len(),
            64,
            "abstract words still produce speech fingerprints"
        );
    }
}
