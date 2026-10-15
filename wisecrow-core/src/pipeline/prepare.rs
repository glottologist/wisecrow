//! One corpus wake: unsettled archives, then stale ranks, then one mine, then
//! one turn of gloss, phrase speech, or word audio.

use std::path::PathBuf;

use crate::config::Config;
use crate::downloader::DownloadConfig;
use crate::errors::WisecrowError;
use crate::files::{Corpus, LanguageFileInfo, LanguageFiles};
use crate::ingesting::Ingester;
use crate::lang;
use crate::llm::LlmProvider;
use crate::media::cache::MediaCache;
use crate::media::{load_media_subject, MediaType};
use crate::phrases::{self, Refresh};
use crate::Langs;
use sqlx::PgPool;

use super::archives::ArchiveLedger;
use super::config::PairSpec;
use super::lock::with_pair_lock;
use super::media_step::{next_media_batch, record_picture_miss, ProbeProfile};
use super::phrase_mine::{mine_is_due, run_mine_if_due};
use super::policy::corpora_for;
use super::ranking::{rank_if_stale, rank_is_current};
use super::turn::{settle, Attempt, Turn};

const DOWNLOAD_CEILING_MB: u64 = 102_400;
const DECOMPRESS_CEILING_MB: u64 = 65_536;
/// Phrases glossed on one corpus wake. The hand command's default, not a ceiling.
const GLOSS_LIMIT: u32 = 100;
const PHRASE_AUDIO_LIMIT: usize = 100;
const PHRASE_AUDIO_PAGE: i64 = 500;

/// One pair's repair-and-gloss pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlossStep {
    /// Gloss rows whose translation link was missing and is now restored.
    pub relinked: usize,
    /// New native links written by the model.
    pub glossed: usize,
}

/// Repairs broken links, then glosses at most [`GLOSS_LIMIT`] phrases that have none.
///
/// An existing English gloss is not sent to the model. [`WisecrowError::LlmUnavailable`]
/// and [`WisecrowError::ConfigurationError`] are returned to the caller.
///
/// # Errors
///
/// Returns an error when a language is unknown, the provider fails, or a write fails.
pub async fn gloss_pair(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    native: &str,
    foreign: &str,
) -> Result<GlossStep, WisecrowError> {
    let relinked = phrases::relink_orphans(pool, foreign, native).await?;
    let glossed =
        phrases::translate_phrases(pool, provider, foreign, native, GLOSS_LIMIT, Refresh::No)
            .await?;
    Ok(GlossStep { relinked, glossed })
}

/// Glossed phrases of this pair that have no current clip, at most 100.
///
/// Unglossed phrases are not selected. A live file for this profile is skipped.
///
/// # Errors
///
/// Returns an error when a language code, query, or cache probe fails.
pub async fn next_phrase_audio_batch(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    profile: &ProbeProfile<'_>,
    limit: usize,
) -> Result<Vec<i32>, WisecrowError> {
    if !lang::is_valid_code(native) || !lang::is_valid_code(foreign) {
        return Err(WisecrowError::InvalidInput(
            "Invalid language code".to_owned(),
        ));
    }
    let limit = limit.min(PHRASE_AUDIO_LIMIT);
    if limit == 0 {
        return Ok(Vec::new());
    }

    let cache = MediaCache::new(pool.clone())?; // clone: PgPool is Arc-based
    let mut after_count = i32::MAX;
    let mut after_id = 0i32;
    let mut batch = Vec::new();
    loop {
        let page: Vec<(i32, i32)> = sqlx::query_as(
            "SELECT link.translation_id, p.sentence_count
               FROM phrase_translations link
               JOIN phrases p ON p.id = link.phrase_id
               JOIN languages fl ON fl.id = p.language_id
               JOIN languages nl ON nl.id = link.native_language_id
              WHERE fl.code = $1 AND nl.code = $2
                AND link.translation_id IS NOT NULL
                AND (p.sentence_count < $3
                     OR (p.sentence_count = $3 AND link.translation_id > $4))
              ORDER BY p.sentence_count DESC, link.translation_id
              LIMIT $5",
        )
        .bind(foreign)
        .bind(native)
        .bind(after_count)
        .bind(after_id)
        .bind(PHRASE_AUDIO_PAGE)
        .fetch_all(pool)
        .await?;

        let Some((last_id, last_count)) = page.last().copied() else {
            break;
        };
        let page_len = page.len();
        after_id = last_id;
        after_count = last_count;

        for (translation_id, _) in page {
            let Some(subject) = load_media_subject(pool, translation_id).await? else {
                continue;
            };
            let fingerprint = subject.audio_fingerprint(
                profile.audio_backend,
                profile.audio_voice,
                profile.audio_format,
            );
            if cache
                .probe(translation_id, MediaType::Audio, &fingerprint)
                .await?
            {
                continue;
            }
            batch.push(translation_id);
            if batch.len() == limit {
                return Ok(batch);
            }
        }

        if i64::try_from(page_len).unwrap_or(PHRASE_AUDIO_PAGE) < PHRASE_AUDIO_PAGE {
            break;
        }
    }
    Ok(batch)
}

/// Voices one batch of glossed phrases. Pictures are not fetched.
///
/// No speech voice for this language is [`CorpusStep::Retry`] and writes nothing.
/// An empty batch is [`CorpusStep::Idle`].
///
/// # Errors
///
/// Returns an error when a query fails or speech generation fails for a reason
/// the unit does not sleep on.
pub async fn voice_phrase_batch(
    pool: &PgPool,
    config: &Config,
    native: &str,
    foreign: &str,
) -> Result<CorpusStep, WisecrowError> {
    let identity = media_identity(config, foreign);
    if !identity.check_audio {
        return Ok(CorpusStep::Retry);
    }
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
    let batch =
        next_phrase_audio_batch(pool, native, foreign, &profile, PHRASE_AUDIO_LIMIT).await?;
    if batch.is_empty() {
        return Ok(CorpusStep::Idle);
    }
    let cache = MediaCache::new(pool.clone())?; // clone: PgPool is Arc-based
    for id in batch {
        let Some(subject) = load_media_subject(pool, id).await? else {
            continue;
        };
        match ensure_audio(config, &cache, &identity, id, &subject).await {
            Ok(()) => {}
            Err(error) if is_pipeline_retry(&error) => return Ok(CorpusStep::Retry),
            Err(error) => return Err(error),
        }
    }
    Ok(CorpusStep::Worked)
}

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
        | WisecrowError::DownloadRetriesExhausted
        | WisecrowError::LlmUnavailable(_) => true,
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

/// The work that beats the gloss and audio cycle. Reading this does not download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leading<'a> {
    /// An archive for `foreign` is not recorded yet.
    Archive { foreign: &'a str },
    /// The rank stamp does not match the live evidence checksum.
    Rank { foreign: &'a str },
    /// A mine is unfinished, or the phrase stamp does not match.
    Mine { foreign: &'a str },
    /// Archives, ranks, and mines are caught up.
    Cycle,
}

/// The first archive, rank, or mine still due, in that order across every pair.
///
/// An unsettled archive on a later pair beats a mine already open on an earlier
/// pair. This function does not construct a downloader.
///
/// # Errors
///
/// Returns an error when a corpus name is unknown or a ledger query fails.
pub async fn leading_work<'a>(
    pool: &PgPool,
    pairs: &'a [PairSpec],
) -> Result<Leading<'a>, WisecrowError> {
    for pair in pairs {
        if next_pending_archive(pool, pair).await?.is_some() {
            return Ok(Leading::Archive {
                foreign: pair.foreign.as_str(),
            });
        }
    }
    for pair in pairs {
        if !rank_is_current(pool, &pair.foreign).await? {
            return Ok(Leading::Rank {
                foreign: pair.foreign.as_str(),
            });
        }
    }
    for pair in pairs {
        if mine_is_due(pool, &pair.foreign).await? {
            return Ok(Leading::Mine {
                foreign: pair.foreign.as_str(),
            });
        }
    }
    Ok(Leading::Cycle)
}

/// Runs the next corpus step.
///
/// Every unsettled archive comes first, then every stale rank, then one mine,
/// then one turn of gloss, phrase speech, or word audio.
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
    match leading_work(pool, pairs).await? {
        Leading::Archive { foreign } => {
            let pair = pair_named(pairs, foreign)?;
            match archive_step(pool, pair).await? {
                Some(step) => Ok(step),
                None => Ok(CorpusStep::Idle),
            }
        }
        Leading::Rank { foreign } => {
            let pair = pair_named(pairs, foreign)?;
            if rank_if_stale(pool, &pair.native, &pair.foreign).await? {
                Ok(CorpusStep::Worked)
            } else {
                Ok(CorpusStep::Idle)
            }
        }
        Leading::Mine { foreign } => {
            let pair = pair_named(pairs, foreign)?;
            if run_mine_if_due(pool, std::slice::from_ref(pair)).await? {
                Ok(CorpusStep::Worked)
            } else {
                Ok(CorpusStep::Retry)
            }
        }
        Leading::Cycle => run_cycle(pool, config, pairs).await,
    }
}

fn pair_named<'a>(pairs: &'a [PairSpec], foreign: &str) -> Result<&'a PairSpec, WisecrowError> {
    pairs
        .iter()
        .find(|pair| pair.foreign == foreign)
        .ok_or_else(|| {
            WisecrowError::InvalidInput(format!("language pair {foreign} is not configured"))
        })
}

async fn run_cycle(
    pool: &PgPool,
    config: &Config,
    pairs: &[PairSpec],
) -> Result<CorpusStep, WisecrowError> {
    let mut current = turn_kind(pool).await?;
    for _ in 0..3 {
        let attempt = match current {
            Turn::Gloss => gloss_cycle(pool, config, pairs).await?,
            Turn::PhraseAudio => phrase_audio_cycle(pool, config, pairs).await?,
            Turn::WordAudio => word_audio_cycle(pool, config, pairs).await?,
        };
        let (next, attempt) = settle(current, attempt);
        match attempt {
            Attempt::Empty => current = next,
            Attempt::Retry => {
                store_turn_kind(pool, next).await?;
                return Ok(CorpusStep::Retry);
            }
            Attempt::Worked => {
                store_turn_kind(pool, next).await?;
                return Ok(CorpusStep::Worked);
            }
        }
    }
    store_turn_kind(pool, Turn::Gloss).await?;
    Ok(CorpusStep::Idle)
}

async fn gloss_cycle(
    pool: &PgPool,
    config: &Config,
    pairs: &[PairSpec],
) -> Result<Attempt, WisecrowError> {
    let provider = match crate::llm::create_provider(config) {
        Ok(provider) => provider,
        Err(WisecrowError::ConfigurationError(_)) => return Ok(Attempt::Retry),
        Err(error) => return Err(error),
    };
    let cursor = pair_cursor(pool, "gloss_pair").await?;
    let start = rotation_start(pairs, &cursor);
    for pair in pairs[start..].iter().chain(pairs[..start].iter()) {
        if !pair_needs_gloss(pool, pair).await? {
            continue;
        }
        match gloss_pair(pool, provider.as_ref(), &pair.native, &pair.foreign).await {
            Ok(_) => {
                store_pair_cursor(pool, "gloss_pair", following_foreign(pairs, &pair.foreign))
                    .await?;
                return Ok(Attempt::Worked);
            }
            Err(WisecrowError::LlmUnavailable(_) | WisecrowError::ConfigurationError(_)) => {
                return Ok(Attempt::Retry);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(Attempt::Empty)
}

async fn phrase_audio_cycle(
    pool: &PgPool,
    config: &Config,
    pairs: &[PairSpec],
) -> Result<Attempt, WisecrowError> {
    let cursor = pair_cursor(pool, "phrase_audio_pair").await?;
    let start = rotation_start(pairs, &cursor);
    for pair in pairs[start..].iter().chain(pairs[..start].iter()) {
        match voice_phrase_batch(pool, config, &pair.native, &pair.foreign).await? {
            CorpusStep::Idle => continue,
            CorpusStep::Retry => return Ok(Attempt::Retry),
            CorpusStep::Worked => {
                store_pair_cursor(
                    pool,
                    "phrase_audio_pair",
                    following_foreign(pairs, &pair.foreign),
                )
                .await?;
                return Ok(Attempt::Worked);
            }
        }
    }
    Ok(Attempt::Empty)
}

async fn word_audio_cycle(
    pool: &PgPool,
    config: &Config,
    pairs: &[PairSpec],
) -> Result<Attempt, WisecrowError> {
    for pair in pairs {
        match media_step(pool, config, pair).await? {
            None | Some(CorpusStep::Idle) => continue,
            Some(CorpusStep::Worked) => return Ok(Attempt::Worked),
            Some(CorpusStep::Retry) => return Ok(Attempt::Retry),
        }
    }
    Ok(Attempt::Empty)
}

fn rotation_start(pairs: &[PairSpec], cursor: &str) -> usize {
    pairs
        .iter()
        .position(|pair| pair.foreign == cursor)
        .unwrap_or(0)
}

fn following_foreign<'a>(pairs: &'a [PairSpec], foreign: &str) -> &'a str {
    let Some(index) = pairs.iter().position(|pair| pair.foreign == foreign) else {
        return "";
    };
    index
        .checked_add(1)
        .and_then(|next| pairs.get(next))
        .map(|pair| pair.foreign.as_str())
        .unwrap_or("")
}

async fn pair_needs_gloss(pool: &PgPool, pair: &PairSpec) -> Result<bool, WisecrowError> {
    let needed: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM phrase_translations pt
             JOIN phrases p ON p.id = pt.phrase_id
             JOIN languages fl ON fl.id = p.language_id
             JOIN languages nl ON nl.id = pt.native_language_id
             WHERE fl.code = $1 AND nl.code = $2 AND pt.translation_id IS NULL
         ) OR EXISTS (
             SELECT 1
             FROM phrases p
             JOIN languages fl ON fl.id = p.language_id
             WHERE fl.code = $1
               AND NOT EXISTS (
                   SELECT 1 FROM phrase_translations pt
                   JOIN languages nl ON nl.id = pt.native_language_id
                   WHERE pt.phrase_id = p.id AND nl.code = $2
               )
         )",
    )
    .bind(&pair.foreign)
    .bind(&pair.native)
    .fetch_one(pool)
    .await?;
    Ok(needed)
}

async fn turn_kind(pool: &PgPool) -> Result<Turn, WisecrowError> {
    let kind: String = sqlx::query_scalar("SELECT kind FROM pipeline_corpus_turn WHERE id")
        .fetch_one(pool)
        .await?;
    match kind.as_str() {
        "gloss" => Ok(Turn::Gloss),
        "phrase_audio" => Ok(Turn::PhraseAudio),
        "word_audio" => Ok(Turn::WordAudio),
        other => Err(WisecrowError::InvalidInput(format!(
            "unknown corpus turn {other}"
        ))),
    }
}

async fn store_turn_kind(pool: &PgPool, turn: Turn) -> Result<(), WisecrowError> {
    let kind = match turn {
        Turn::Gloss => "gloss",
        Turn::PhraseAudio => "phrase_audio",
        Turn::WordAudio => "word_audio",
    };
    let updated = sqlx::query("UPDATE pipeline_corpus_turn SET kind = $1 WHERE id")
        .bind(kind)
        .execute(pool)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(WisecrowError::InvalidInput(
            "corpus turn row is missing".to_owned(),
        ));
    }
    Ok(())
}

async fn pair_cursor(pool: &PgPool, column: &str) -> Result<String, WisecrowError> {
    let statement = match column {
        "gloss_pair" => "SELECT gloss_pair FROM pipeline_corpus_turn WHERE id",
        "phrase_audio_pair" => "SELECT phrase_audio_pair FROM pipeline_corpus_turn WHERE id",
        other => {
            return Err(WisecrowError::InvalidInput(format!(
                "unknown corpus cursor {other}"
            )))
        }
    };
    let cursor: String = sqlx::query_scalar(statement).fetch_one(pool).await?;
    Ok(cursor)
}

async fn store_pair_cursor(pool: &PgPool, column: &str, value: &str) -> Result<(), WisecrowError> {
    let statement = match column {
        "gloss_pair" => "UPDATE pipeline_corpus_turn SET gloss_pair = $1 WHERE id",
        "phrase_audio_pair" => "UPDATE pipeline_corpus_turn SET phrase_audio_pair = $1 WHERE id",
        other => {
            return Err(WisecrowError::InvalidInput(format!(
                "unknown corpus cursor {other}"
            )))
        }
    };
    let updated = sqlx::query(statement).bind(value).execute(pool).await?;
    if updated.rows_affected() == 0 {
        return Err(WisecrowError::InvalidInput(
            "corpus turn row is missing".to_owned(),
        ));
    }
    Ok(())
}

async fn next_pending_archive(
    pool: &PgPool,
    pair: &PairSpec,
) -> Result<Option<(String, LanguageFileInfo)>, WisecrowError> {
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
        if ledger
            .needs_download(&pair.native, &pair.foreign, &file.target_location)
            .await?
        {
            // clone: the corpus name outlives the selection slice
            return Ok(Some((corpus_name.to_owned(), file)));
        }
    }
    Ok(None)
}

async fn archive_step(pool: &PgPool, pair: &PairSpec) -> Result<Option<CorpusStep>, WisecrowError> {
    let Some((corpus_name, file)) = next_pending_archive(pool, pair).await? else {
        return Ok(None);
    };
    Ok(Some(read_archive(pool, pair, &corpus_name, &file).await?))
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

    #[test]
    fn llm_outage_is_a_pipeline_retry() {
        let error = WisecrowError::LlmUnavailable("down".into());
        assert!(is_pipeline_retry(&error));
        assert!(!is_pipeline_retry(&WisecrowError::LlmError(
            "bad json".into()
        )));
    }
}
