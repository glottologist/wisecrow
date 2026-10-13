//! One bounded corpus step: a single archive, one ranking, or one media batch.

use std::path::PathBuf;

use crate::config::Config;
use crate::downloader::DownloadConfig;
use crate::errors::WisecrowError;
use crate::files::{Corpus, LanguageFileInfo, LanguageFiles};
use crate::ingesting::Ingester;
use crate::media::cache::MediaCache;
use crate::Langs;
use sqlx::PgPool;

use super::archives::ArchiveLedger;
use super::config::PairSpec;
use super::lock::with_pair_lock;
use super::media_step::{next_media_batch, record_picture_miss, ProbeProfile};
use super::policy::corpora_for;
use super::ranking::rank_if_stale;

const DOWNLOAD_CEILING_MB: u64 = 102_400;
const DECOMPRESS_CEILING_MB: u64 = 65_536;

/// What one invocation of `prepare-corpus` accomplished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusStep {
    /// An archive was read, a 404 was recorded, a ranking finished, or a
    /// media batch was stored.
    Worked,
    /// Every configured pair is caught up.
    Idle,
    /// A download, a voice, or a picture search should be tried again later.
    /// Nothing was recorded as finished.
    Retry,
}

/// Network and provider failures the unit should sleep on.
///
/// A 404 is not one of these: it is recorded and the step is [`CorpusStep::Worked`].
/// A file past the size ceiling is not one of these either.
#[must_use]
pub fn is_pipeline_retry(error: &WisecrowError) -> bool {
    match error {
        WisecrowError::MediaError(_)
        | WisecrowError::UnableToGetFile(_)
        | WisecrowError::DownloadRetriesExhausted => true,
        WisecrowError::HttpStatus { status, .. } => {
            *status == 408 || *status == 429 || (500..600).contains(status)
        }
        _ => false,
    }
}

fn is_archive_missing(error: &WisecrowError) -> bool {
    matches!(error, WisecrowError::HttpStatus { status: 404, .. })
}

fn pipeline_download() -> DownloadConfig {
    DownloadConfig {
        max_retries: 3,
        timeout_seconds: 300,
        max_file_size_mb: DOWNLOAD_CEILING_MB,
        max_decompressed_mb: DECOMPRESS_CEILING_MB,
        unpack: true,
    }
}

/// `/ingest-tmp` when that mount exists. This function does not create it.
fn scratch_dir() -> PathBuf {
    let mounted = PathBuf::from("/ingest-tmp");
    if mounted.is_dir() {
        mounted
    } else {
        std::env::temp_dir()
    }
}

fn discard_archive(path: &str) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!("failed to remove {path} after ingest: {error}"),
    }
}

/// Advances the first configured pair that still has work, by one step.
///
/// # Errors
///
/// Returns an error for a bad corpus name, a database failure, a parse
/// failure, or an archive that exceeds the size ceiling. Those are not retried.
pub async fn prepare_one_step(
    pool: &PgPool,
    config: &Config,
    pairs: &[PairSpec],
) -> Result<CorpusStep, WisecrowError> {
    for pair in pairs {
        if let Some(step) = archive_step(pool, pair).await? {
            return Ok(step);
        }
        if rank_if_stale(pool, &pair.native, &pair.foreign).await? {
            return Ok(CorpusStep::Worked);
        }
        match media_step(pool, config, pair).await? {
            None => {}
            Some(step) => return Ok(step),
        }
    }
    Ok(CorpusStep::Idle)
}

async fn archive_step(pool: &PgPool, pair: &PairSpec) -> Result<Option<CorpusStep>, WisecrowError> {
    let selected = pair
        .corpora
        .as_ref()
        .map(|names| names.iter().map(String::as_str).collect::<Vec<_>>());
    let corpora = corpora_for(&pair.foreign, selected.as_deref())?;
    let langs = Langs::new(&pair.native, &pair.foreign);
    let ledger = ArchiveLedger::new(pool);

    for corpus_name in corpora {
        let corpus = Corpus::try_from(corpus_name)?;
        let files = LanguageFiles::new(&langs, Some(&[corpus]))?;
        let Some(file) = files.files.into_iter().next() else {
            continue;
        };
        if !ledger
            .needs_download(&pair.native, &pair.foreign, &file.target_location)
            .await?
        {
            continue;
        }
        return Ok(Some(read_archive(pool, pair, corpus_name, &file).await?));
    }
    Ok(None)
}

async fn read_archive(
    pool: &PgPool,
    pair: &PairSpec,
    corpus_name: &str,
    file: &LanguageFileInfo,
) -> Result<CorpusStep, WisecrowError> {
    let download = Ingester::download_to_dir(&pipeline_download(), file, &scratch_dir()).await;
    let path = match download {
        Err(error) if is_archive_missing(&error) => {
            with_pair_lock(pool, &pair.native, &pair.foreign, || async {
                ArchiveLedger::new(pool)
                    .mark_unavailable(
                        &pair.native,
                        &pair.foreign,
                        corpus_name,
                        &file.target_location,
                    )
                    .await
            })
            .await?;
            return Ok(CorpusStep::Worked);
        }
        Err(error) if is_pipeline_retry(&error) => return Ok(CorpusStep::Retry),
        Err(error) => return Err(error),
        Ok(path) => path,
    };

    let ingested = with_pair_lock(pool, &pair.native, &pair.foreign, || async {
        let ledger = ArchiveLedger::new(pool);
        if !ledger
            .needs_download(&pair.native, &pair.foreign, &file.target_location)
            .await?
        {
            return Ok(false);
        }
        let ingester = Ingester::new(pool.clone(), pipeline_download()); // clone: PgPool is Arc-based
        ingester
            .ingest_from_file_once(&path, &file.file_name, &file.languages)
            .await?;
        ledger
            .mark_ingested(
                &pair.native,
                &pair.foreign,
                corpus_name,
                &file.target_location,
            )
            .await?;
        Ok(true)
    })
    .await;

    match ingested {
        Ok(_) => {
            discard_archive(&path);
            Ok(CorpusStep::Worked)
        }
        Err(error) => {
            discard_archive(&path);
            if is_pipeline_retry(&error) {
                Ok(CorpusStep::Retry)
            } else {
                Err(error)
            }
        }
    }
}

async fn media_step(
    pool: &PgPool,
    config: &Config,
    pair: &PairSpec,
) -> Result<Option<CorpusStep>, WisecrowError> {
    if !cfg!(any(feature = "tts", feature = "images")) {
        return Ok(None);
    }

    let identity = media_identity(config, &pair.foreign);
    let cache = MediaCache::new(pool.clone())?; // clone: PgPool is Arc-based
    let profile = ProbeProfile {
        audio_backend: identity.backend,
        audio_voice: &identity.voice,
        audio_format: identity.audio_format,
        check_audio: identity.check_audio,
        provider_ids: &identity.providers,
        image_format: identity.image_format,
        check_image: identity.check_image,
        image_providers_known: identity.providers_known,
    };
    let batch = next_media_batch(pool, &pair.native, &pair.foreign, &cache, &profile).await?;
    if batch.is_empty() {
        return Ok(None);
    }

    match fulfill_batch(pool, config, &cache, &identity, &batch).await {
        Ok(()) => Ok(Some(CorpusStep::Worked)),
        Err(error) if is_pipeline_retry(&error) => Ok(Some(CorpusStep::Retry)),
        Err(error) => Err(error),
    }
}

struct MediaIdentity {
    backend: &'static str,
    voice: String,
    audio_format: u32,
    check_audio: bool,
    providers: Vec<&'static str>,
    image_format: u32,
    check_image: bool,
    providers_known: bool,
}

fn media_identity(config: &Config, foreign: &str) -> MediaIdentity {
    let (backend, voice, audio_format, check_audio) = audio_identity(config, foreign);
    let (providers, image_format, check_image, providers_known) = image_identity(config);
    MediaIdentity {
        backend,
        voice,
        audio_format,
        check_audio,
        providers,
        image_format,
        check_image,
        providers_known,
    }
}

#[cfg(feature = "tts")]
fn audio_identity(config: &Config, foreign: &str) -> (&'static str, String, u32, bool) {
    use crate::media::audio::{self, AUDIO_FORMAT_VERSION};
    use crate::media::cereproc::CereprocClient;

    let cereproc = CereprocClient::from_config(config);
    match audio::tts_profile_for_language(foreign, cereproc.as_ref()) {
        Some(profile) => (
            profile.backend_id(),
            profile.voice().to_owned(), // clone: profile borrows the client that ends with this call
            AUDIO_FORMAT_VERSION,
            true,
        ),
        None => {
            tracing::warn!(
                "no TTS voice for {foreign}; pictures will not wait on audio that cannot be made"
            );
            ("none", String::new(), AUDIO_FORMAT_VERSION, false)
        }
    }
}

#[cfg(not(feature = "tts"))]
fn audio_identity(_config: &Config, foreign: &str) -> (&'static str, String, u32, bool) {
    let _ = foreign;
    ("none", String::new(), 0, false)
}

#[cfg(feature = "images")]
fn image_identity(config: &Config) -> (Vec<&'static str>, u32, bool, bool) {
    use crate::media::images::{ImageFetcher, IMAGE_FORMAT_VERSION};

    match ImageFetcher::from_config(config) {
        Some(fetcher) => (fetcher.provider_ids(), IMAGE_FORMAT_VERSION, true, true),
        None => (Vec::new(), IMAGE_FORMAT_VERSION, true, false),
    }
}

#[cfg(not(feature = "images"))]
fn image_identity(_config: &Config) -> (Vec<&'static str>, u32, bool, bool) {
    (Vec::new(), 0, false, false)
}

async fn fulfill_batch(
    pool: &PgPool,
    config: &Config,
    cache: &MediaCache,
    identity: &MediaIdentity,
    ids: &[i32],
) -> Result<(), WisecrowError> {
    for id in ids {
        let Some(subject) = crate::media::load_media_subject(pool, *id).await? else {
            continue;
        };
        ensure_audio(config, cache, identity, *id, &subject).await?;
        ensure_image(pool, config, cache, identity, *id, &subject).await?;
    }
    Ok(())
}

#[cfg(feature = "tts")]
async fn ensure_audio(
    config: &Config,
    cache: &MediaCache,
    identity: &MediaIdentity,
    translation_id: i32,
    subject: &crate::media::MediaSubject,
) -> Result<(), WisecrowError> {
    use crate::media::audio;
    use crate::media::cereproc::CereprocClient;
    use crate::media::MediaType;

    if !identity.check_audio {
        return Ok(());
    }
    let fingerprint =
        subject.audio_fingerprint(identity.backend, &identity.voice, identity.audio_format);
    if cache
        .probe(translation_id, MediaType::Audio, &fingerprint)
        .await?
    {
        return Ok(());
    }
    let cereproc = CereprocClient::from_config(config);
    let generated =
        audio::generate_tts(&subject.to_phrase, &subject.foreign_lang, cereproc.as_ref()).await?;
    cache
        .get_or_fetch(translation_id, MediaType::Audio, &fingerprint, || async {
            Ok(generated)
        })
        .await?;
    Ok(())
}

#[cfg(not(feature = "tts"))]
async fn ensure_audio(
    _config: &Config,
    _cache: &MediaCache,
    _identity: &MediaIdentity,
    _translation_id: i32,
    _subject: &crate::media::MediaSubject,
) -> Result<(), WisecrowError> {
    Ok(())
}

#[cfg(feature = "images")]
async fn ensure_image(
    pool: &PgPool,
    config: &Config,
    cache: &MediaCache,
    identity: &MediaIdentity,
    translation_id: i32,
    subject: &crate::media::MediaSubject,
) -> Result<(), WisecrowError> {
    use crate::media::images::{ImageFetcher, ImageSearch};
    use crate::media::MediaType;

    if !identity.check_image {
        return Ok(());
    }
    if !identity.providers_known {
        if cache
            .stored_published(translation_id, MediaType::Image)
            .await?
            .is_some()
        {
            return Ok(());
        }
        return Err(WisecrowError::MediaError(
            "No image provider is configured".to_owned(),
        ));
    }
    let Some(query) = subject.applicable_image_query() else {
        return Ok(());
    };
    let Some(fingerprint) = subject.image_fingerprint(&identity.providers, identity.image_format)
    else {
        return Ok(());
    };
    if cache
        .probe(translation_id, MediaType::Image, &fingerprint)
        .await?
    {
        return Ok(());
    }
    let fetcher = ImageFetcher::from_config(config)
        .ok_or_else(|| WisecrowError::MediaError("No image provider is configured".to_owned()))?;
    let client = reqwest::Client::new();
    let query = crate::media::image_provider::ImageQuery::new(query);
    match fetcher.search_outcome(&client, &query).await {
        ImageSearch::Found(image) => {
            let bytes = image.bytes;
            let attribution = image.attribution;
            cache
                .get_or_fetch_attributed(translation_id, MediaType::Image, &fingerprint, || async {
                    Ok((bytes, attribution))
                })
                .await?;
            Ok(())
        }
        ImageSearch::Empty => {
            record_picture_miss(pool, fingerprint.as_str()).await?;
            Ok(())
        }
        ImageSearch::Retryable(error) => Err(error),
    }
}

#[cfg(not(feature = "images"))]
async fn ensure_image(
    _pool: &PgPool,
    _config: &Config,
    _cache: &MediaCache,
    _identity: &MediaIdentity,
    _translation_id: i32,
    _subject: &crate::media::MediaSubject,
) -> Result<(), WisecrowError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_pipeline_retry;
    use crate::errors::WisecrowError;

    #[test]
    fn only_transient_failures_are_retries() {
        assert!(is_pipeline_retry(&WisecrowError::MediaError(
            "timeout".to_owned()
        )));
        assert!(is_pipeline_retry(&WisecrowError::DownloadRetriesExhausted));
        assert!(is_pipeline_retry(&WisecrowError::HttpStatus {
            status: 503,
            url: "https://example.test/a".to_owned(),
        }));
        assert!(is_pipeline_retry(&WisecrowError::HttpStatus {
            status: 429,
            url: "https://example.test/a".to_owned(),
        }));
        assert!(!is_pipeline_retry(&WisecrowError::HttpStatus {
            status: 404,
            url: "https://example.test/missing".to_owned(),
        }));
        assert!(!is_pipeline_retry(&WisecrowError::FileTooLarge(
            "too big".to_owned()
        )));
        assert!(!is_pipeline_retry(&WisecrowError::InvalidInput(
            "parse".to_owned()
        )));
    }
}
