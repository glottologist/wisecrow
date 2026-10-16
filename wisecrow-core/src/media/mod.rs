pub mod cache;
pub mod fingerprint;
pub mod grammar_audio;
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
    pub native_lang: String,
    pub foreign_lang: String,
    pub image_query: Option<String>,
    pub is_phrase: bool,
}

impl From<PresentedTranslation> for MediaSubject {
    fn from(presented: PresentedTranslation) -> Self {
        Self {
            to_phrase: presented.to_phrase,
            from_phrase: presented.from_phrase,
            native_lang: presented.native_lang,
            foreign_lang: presented.foreign_lang,
            image_query: presented.image_query,
            is_phrase: presented.is_phrase,
        }
    }
}

impl MediaSubject {
    /// English picture search, or `None` for a phrase or a word with no native text.
    ///
    /// A stored stock-photo query wins. Otherwise the search is the native
    /// meaning, never the foreign word, so one picture serves every language
    /// taught from the same native wording.
    #[must_use]
    pub fn applicable_image_query(&self) -> Option<&str> {
        if self.is_phrase {
            return None;
        }
        if let Some(query) = self
            .image_query
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty() && !same_word(query, &self.to_phrase))
        {
            return Some(query);
        }
        native_picture_subject(&self.from_phrase)
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
            MediaFingerprint::for_image(query, &self.native_lang, provider_ids, format_version)
        })
    }
}

/// A stored query that is only the foreign word is not an English picture search.
fn same_word(query: &str, foreign: &str) -> bool {
    query.trim().eq_ignore_ascii_case(foreign.trim())
}

/// Native meaning with a trailing teaching note removed.
///
/// "the (plural)" is the picture subject "the". An empty meaning is not a search.
fn native_picture_subject(native: &str) -> Option<&str> {
    let subject = native
        .split_once('(')
        .map_or(native, |(head, _note)| head)
        .trim();
    if subject.is_empty() {
        None
    } else {
        Some(subject)
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

/// A picture already on disk, or the reason the handler must not invent one.
#[cfg(feature = "images")]
#[derive(Debug)]
pub enum CachedPicture {
    /// A phrase, or a word with no search text.
    NotApplicable,
    /// Authoritative empty search. Do not call a provider again.
    KnownMiss,
    Hit {
        path: std::path::PathBuf,
        attribution: Option<String>,
    },
    /// A picture applies and nothing live matches the current fingerprint.
    Missing,
}

/// Looks up a card picture without calling a provider.
///
/// `provider_ids` is `None` when no provider is configured. A live file is
/// still returned, so a missing key is not a 503 for a picture already stored.
///
/// # Errors
///
/// Returns an error when the cache query fails.
#[cfg(feature = "images")]
pub async fn cached_picture(
    cache: &cache::MediaCache,
    translation_id: i32,
    subject: &MediaSubject,
    provider_ids: Option<&[&str]>,
) -> Result<CachedPicture, WisecrowError> {
    if subject.applicable_image_query().is_none() {
        return Ok(CachedPicture::NotApplicable);
    }
    let Some(provider_ids) = provider_ids else {
        return Ok(
            match cache
                .stored_published(translation_id, MediaType::Image)
                .await?
            {
                Some((path, attribution)) => CachedPicture::Hit { path, attribution },
                None => CachedPicture::Missing,
            },
        );
    };
    let Some(fingerprint) = subject.image_fingerprint(provider_ids, images::IMAGE_FORMAT_VERSION)
    else {
        return Ok(CachedPicture::NotApplicable);
    };
    if let Some((path, attribution)) = cache
        .published(translation_id, MediaType::Image, &fingerprint)
        .await?
    {
        return Ok(CachedPicture::Hit { path, attribution });
    }
    if cache.picture_miss_recorded(fingerprint.as_str()).await? {
        return Ok(CachedPicture::KnownMiss);
    }
    Ok(CachedPicture::Missing)
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
    use rstest::rstest;

    use super::*;

    fn dog() -> MediaSubject {
        MediaSubject {
            to_phrase: "chien".to_owned(),
            from_phrase: "dog".to_owned(),
            native_lang: "en".to_owned(),
            foreign_lang: "fr".to_owned(),
            image_query: Some("friendly dog".to_owned()),
            is_phrase: false,
        }
    }

    fn word(foreign: &str, native: &str, image_query: Option<&str>) -> MediaSubject {
        MediaSubject {
            to_phrase: foreign.to_owned(),
            from_phrase: native.to_owned(),
            native_lang: "en".to_owned(),
            foreign_lang: "gd".to_owned(),
            image_query: image_query.map(str::to_owned),
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

    #[rstest]
    #[case("sinn", "we / us", None, Some("we / us"))]
    #[case("sinn", "we / us", Some("sinn"), Some("we / us"))]
    #[case("sinn", "we / us", Some("Sinn"), Some("we / us"))]
    #[case("sinn", "we / us", Some("  "), Some("we / us"))]
    #[case("na", "the (plural)", None, Some("the"))]
    #[case("thu", "you (singular)", None, Some("you"))]
    #[case("ann", "in it / existing (particle)", None, Some("in it / existing"))]
    #[case("casa", "house", None, Some("house"))]
    #[case("chien", "dog", Some("friendly dog"), Some("friendly dog"))]
    #[case("x", "   ", None, None)]
    #[case("x", "(particle)", None, None)]
    fn picture_query_uses_the_native_meaning(
        #[case] foreign: &str,
        #[case] native: &str,
        #[case] stored_query: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let subject = word(foreign, native, stored_query);
        assert_eq!(subject.applicable_image_query(), expected);
    }

    #[test]
    fn the_same_native_wording_shares_one_picture_across_foreign_languages() {
        let providers = ["unsplash", "pexels", "pixabay"];
        let gaelic = word("sinn", "we / us", None);
        let french = MediaSubject {
            to_phrase: "nous".to_owned(),
            foreign_lang: "fr".to_owned(),
            ..gaelic.clone() // clone: the French card is an independent subject
        };
        assert_eq!(gaelic.applicable_image_query(), Some("we / us"));
        assert_eq!(
            gaelic.image_fingerprint(&providers, 1),
            french.image_fingerprint(&providers, 1)
        );
        assert_ne!(
            gaelic.audio_fingerprint("edge", "voice", 1),
            french.audio_fingerprint("edge", "voice", 1),
            "speech stays in the foreign language"
        );
    }

    #[test]
    fn phrase_has_no_image_and_an_unglossed_word_uses_its_native_meaning() {
        let native_word = word("en", "in", None);
        let mut french = native_word.clone(); // clone: the fingerprint case names its language
        french.foreign_lang = "fr".to_owned();
        let phrase = MediaSubject {
            to_phrase: "comment allez vous".to_owned(),
            from_phrase: "canonical phrase translation".to_owned(),
            native_lang: "en".to_owned(),
            foreign_lang: "fr".to_owned(),
            image_query: Some("wrong image".to_owned()),
            is_phrase: true,
        };
        let providers = ["unsplash"];
        assert_eq!(
            french.image_fingerprint(&providers, 1),
            Some(MediaFingerprint::for_image("in", "en", &providers, 1))
        );
        assert!(phrase.applicable_image_query().is_none());
        assert!(phrase.image_fingerprint(&providers, 1).is_none());
        let audio = french.audio_fingerprint("edge", "fr-FR-HenriNeural", 1);
        assert_eq!(
            audio.as_str().len(),
            64,
            "function words still produce speech fingerprints"
        );
    }
}
