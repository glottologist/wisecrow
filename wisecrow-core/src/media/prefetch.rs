//! Bounded preparation of learning media.
//!
//! A run selects a finite slice of the fixed preparation deck, loads exact
//! presentations in batches of one hundred, and prepares each medium with at
//! most four requests outstanding. Preview opens the cache read-only and
//! never calls a provider; execution charges every generated payload to an
//! admission budget inside the cache's fetch callback, before publication.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};
use sqlx::PgPool;
use tokio::sync::watch;
use tracing::info;

use crate::config::Config;
use crate::errors::WisecrowError;
use crate::media::cache::MediaCache;
use crate::media::fingerprint::MediaFingerprint;
use crate::media::{MediaSubject, MediaType};
use crate::presentation::{
    PresentationRepository, PresentedTranslation, CURRENT_PRESENTATION_VERSION, SELECTED_BATCH,
};
use crate::vocabulary::VocabularyQuery;
use crate::Langs;

#[cfg(feature = "tts")]
use crate::media::audio_cache_key;
#[cfg(feature = "tts")]
use crate::media::cereproc::CereprocClient;
#[cfg(feature = "images")]
use crate::media::image_cache_key;
#[cfg(feature = "images")]
use crate::media::images::ImageFetcher;

const MAX_CONCURRENT_FETCHES: usize = 4;

/// Furthest position of the fixed preparation deck a range may reach.
const MAX_RANGE_END: u32 = 10_000;
/// Most entries one invocation may select.
const MAX_RANGE_LIMIT: u32 = 5000;
/// Largest admission budget one invocation may carry: 5 GiB.
const MAX_BUDGET_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// Whether a run inspects the cache or generates into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefetchMode {
    /// Read-only: report what is cached and what would be generated.
    Preview,
    /// Generate and publish missing media within the budget.
    Execute,
}

/// Which media a run prepares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestedMedia {
    Audio,
    Images,
    Both,
}

impl RequestedMedia {
    const fn audio(self) -> bool {
        matches!(self, Self::Audio | Self::Both)
    }

    const fn images(self) -> bool {
        matches!(self, Self::Images | Self::Both)
    }
}

/// A finite slice of the preparation deck and the byte budget for it.
#[derive(Debug)]
pub struct PrefetchOptions {
    limit: u32,
    offset: u32,
    max_bytes: u64,
    media: RequestedMedia,
    mode: PrefetchMode,
}

impl PrefetchOptions {
    /// Validates a finite preparation range and generated-byte budget.
    ///
    /// # Errors
    ///
    /// Rejects a limit outside 1–5,000, a range reaching past position
    /// 10,000, or a budget outside 1 byte–5 GiB.
    pub fn new(
        limit: u32,
        offset: u32,
        max_bytes: u64,
        media: RequestedMedia,
        mode: PrefetchMode,
    ) -> Result<Self, WisecrowError> {
        let end = offset
            .checked_add(limit)
            .ok_or_else(|| WisecrowError::InvalidInput("Media range overflow".into()))?;
        if !(1..=MAX_RANGE_LIMIT).contains(&limit)
            || end > MAX_RANGE_END
            || !(1..=MAX_BUDGET_BYTES).contains(&max_bytes)
        {
            return Err(WisecrowError::InvalidInput(
                "Invalid media range or byte budget".into(),
            ));
        }
        Ok(Self {
            limit,
            offset,
            max_bytes,
            media,
            mode,
        })
    }

    /// The run's mode.
    #[must_use]
    pub const fn mode(&self) -> PrefetchMode {
        self.mode
    }

    /// First deck position of the range.
    #[must_use]
    pub const fn offset(&self) -> u32 {
        self.offset
    }

    /// Most entries the range covers.
    #[must_use]
    pub const fn limit(&self) -> u32 {
        self.limit
    }
}

/// Provider handles a run may use, each present only when its feature is
/// compiled and its configuration supplied.
#[derive(Default)]
pub struct MediaProviders {
    #[cfg(feature = "images")]
    image_fetcher: Option<ImageFetcher>,
    #[cfg(feature = "tts")]
    cereproc: Option<CereprocClient>,
}

impl MediaProviders {
    /// Builds every provider the configuration and build support.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        // Without either feature the configuration has nothing to offer.
        #[cfg(not(any(feature = "images", feature = "tts")))]
        let _ = config;
        Self {
            #[cfg(feature = "images")]
            image_fetcher: ImageFetcher::from_config(config),
            #[cfg(feature = "tts")]
            cereproc: CereprocClient::from_config(config),
        }
    }

    fn audio_supported(&self, foreign_lang: &str) -> bool {
        #[cfg(feature = "tts")]
        {
            crate::media::audio::tts_profile_for_language(foreign_lang, self.cereproc.as_ref())
                .is_some()
        }
        #[cfg(not(feature = "tts"))]
        {
            let _ = foreign_lang;
            false
        }
    }

    fn images_supported(&self) -> bool {
        #[cfg(feature = "images")]
        {
            self.image_fetcher.is_some()
        }
        #[cfg(not(feature = "images"))]
        {
            false
        }
    }
}

/// What happened to one medium of one translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaOutcome {
    /// A live file for the current fingerprint already exists.
    Cached,
    /// Preview only: nothing cached; execution would generate it.
    Missing,
    /// Generated and published in this run.
    Generated { bytes: u64 },
    /// No media applies, such as an image for a phrase or an abstract word.
    NotApplicable,
    /// The feature or provider for this medium is unavailable in this build
    /// or configuration.
    Unsupported,
    /// Generation or publication failed; see the log for the category.
    Failed,
    /// The payload did not fit the remaining budget; nothing was written.
    BudgetExhausted,
}

impl MediaOutcome {
    /// Whether the outcome means the range needs another attempt.
    #[must_use]
    pub const fn needs_attention(self) -> bool {
        matches!(
            self,
            Self::Unsupported | Self::Failed | Self::BudgetExhausted
        )
    }
}

/// Outcomes for one selected translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemOutcome {
    pub translation_id: i32,
    /// `None` when audio was not requested.
    pub audio: Option<MediaOutcome>,
    /// `None` when images were not requested.
    pub image: Option<MediaOutcome>,
}

impl ItemOutcome {
    fn outcomes(&self) -> impl Iterator<Item = MediaOutcome> {
        self.audio.into_iter().chain(self.image)
    }
}

/// What one run selected, did and admitted.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PrefetchSummary {
    /// Deck entries in the requested range.
    pub selected: u32,
    /// Entries whose media were inspected or prepared.
    pub processed: u32,
    /// Where the next range starts while the deck has more entries.
    pub next_offset: Option<u32>,
    /// Bytes charged to the budget, refunded never.
    pub admitted_bytes: u64,
    /// Bytes of payloads that were also published.
    pub generated_bytes: u64,
    /// One entry per processed translation, in completion order.
    pub outcomes: Vec<ItemOutcome>,
}

impl PrefetchSummary {
    /// How many media outcomes of `kind` the run produced.
    #[must_use]
    pub fn count(&self, kind: MediaOutcome) -> usize {
        self.outcomes
            .iter()
            .flat_map(ItemOutcome::outcomes)
            .filter(|outcome| *outcome == kind)
            .count()
    }

    /// Whether any medium failed, was refused by the budget, or is
    /// unsupported. A preview's `Missing` entries are expected, not failures.
    #[must_use]
    pub fn needs_attention(&self) -> bool {
        self.outcomes
            .iter()
            .flat_map(ItemOutcome::outcomes)
            .any(MediaOutcome::needs_attention)
    }

    /// IDs with at least one medium needing attention, in completion order.
    #[must_use]
    pub fn attention_ids(&self) -> Vec<i32> {
        self.outcomes
            .iter()
            .filter(|item| item.outcomes().any(MediaOutcome::needs_attention))
            .map(|item| item.translation_id)
            .collect()
    }
}

/// Prepares media for `options`' slice of the pair's preparation deck.
///
/// Preview performs read-only cache probes and no provider calls. Execute
/// generates missing media with at most four requests outstanding, charging
/// each payload to the byte budget before it is published; once a payload is
/// refused, later misses are reported as budget-exhausted without a call.
/// Setting `cancel` to `true` aborts outstanding workers and returns once
/// they have stopped.
///
/// # Errors
///
/// Returns an error when a language is unsupported, execution requests a
/// medium this build or configuration cannot produce, the selection times
/// out or its presentations changed, a worker cannot be joined, or the run
/// is cancelled.
pub async fn prefetch_media(
    pool: &PgPool,
    langs: &Langs,
    options: &PrefetchOptions,
    providers: &MediaProviders,
    cancel: watch::Receiver<bool>,
) -> Result<PrefetchSummary, WisecrowError> {
    let (native, foreign) = (langs.native_code(), langs.foreign_code());
    if !crate::cli::is_supported_language(native) || !crate::cli::is_supported_language(foreign) {
        return Err(WisecrowError::InvalidInput(format!(
            "Unsupported language pair {native}/{foreign}"
        )));
    }
    let capabilities = Capabilities {
        audio: options
            .media
            .audio()
            .then(|| providers.audio_supported(foreign)),
        images: options.media.images().then(|| providers.images_supported()),
    };
    if options.mode == PrefetchMode::Execute {
        if capabilities.audio == Some(false) {
            return Err(WisecrowError::MediaError(format!(
                "No speech synthesis is available for {foreign}; run with --audio=false"
            )));
        }
        if capabilities.images == Some(false) {
            return Err(WisecrowError::MediaError(
                "No image provider is configured; run with --images=false".into(),
            ));
        }
    }

    let deck = VocabularyQuery::preparation_ids(pool, native, foreign).await?;
    let offset = usize::try_from(options.offset)
        .map_err(|_| WisecrowError::InvalidInput("Media range offset overflow".into()))?;
    let limit = usize::try_from(options.limit)
        .map_err(|_| WisecrowError::InvalidInput("Media range limit overflow".into()))?;
    let selected: Vec<i32> = deck.iter().copied().skip(offset).take(limit).collect();
    let selected_count = u32::try_from(selected.len())
        .map_err(|_| WisecrowError::InvalidInput("Media selection overflow".into()))?;
    let next_offset = (deck.len() > offset.saturating_add(selected.len()))
        .then(|| options.offset.saturating_add(selected_count));
    info!(
        "Preparing media for {} of {} {native}-{foreign} entries from offset {}",
        selected.len(),
        deck.len(),
        options.offset
    );

    let cache = match options.mode {
        PrefetchMode::Preview => MediaCache::open_existing(pool.clone()), // clone: PgPool is Arc-based
        PrefetchMode::Execute => MediaCache::new(pool.clone())?, // clone: PgPool is Arc-based
    };
    let operation = Arc::new(Operation {
        cache,
        budget: ByteBudget::new(options.max_bytes),
        mode: options.mode,
        capabilities,
        budget_refused: AtomicBool::new(false),
        #[cfg(feature = "images")]
        http: reqwest::Client::new(),
        #[cfg(feature = "images")]
        image_fetcher: providers.image_fetcher.clone(), // clone: ImageFetcher shares Arc providers
        #[cfg(feature = "tts")]
        cereproc: providers.cereproc.clone(), // clone: CereprocClient shares one Arc access token
    });

    let progress = progress_bar(selected.len())?;
    let mut summary = PrefetchSummary {
        selected: selected_count,
        next_offset,
        ..PrefetchSummary::default()
    };
    for batch in selected.chunks(SELECTED_BATCH) {
        let rows = PresentationRepository::load_selected(pool, batch, native, foreign).await?;
        if let Some(changed) = rows.iter().find(|row| {
            !row.teachable
                || (!row.is_phrase && row.presentation_version < CURRENT_PRESENTATION_VERSION)
        }) {
            return Err(WisecrowError::InvalidInput(format!(
                "Presentation {} changed since selection; preview again from offset 0",
                changed.translation_id
            )));
        }
        let outcomes = run_batch(
            rows,
            |row| {
                let operation = Arc::clone(&operation);
                let progress = progress.clone(); // clone: ProgressBar is Arc-based
                async move {
                    let outcome = prepare_item(&operation, row).await;
                    progress.inc(1);
                    outcome
                }
            },
            cancel.clone(),
        )
        .await?;
        for outcome in outcomes {
            summary.processed = summary.processed.saturating_add(1);
            summary.generated_bytes = outcome
                .outcomes()
                .filter_map(|medium| match medium {
                    MediaOutcome::Generated { bytes } => Some(bytes),
                    _ => None,
                })
                .fold(summary.generated_bytes, u64::saturating_add);
            summary.outcomes.push(outcome);
        }
    }
    progress.finish_with_message("done");
    summary.admitted_bytes = operation.budget.used();
    Ok(summary)
}

fn progress_bar(total: usize) -> Result<ProgressBar, WisecrowError> {
    let progress = ProgressBar::new(u64::try_from(total).unwrap_or(u64::MAX));
    progress.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")?,
    );
    Ok(progress)
}

/// Per-medium availability for this run: `None` when not requested.
#[derive(Debug, Clone, Copy)]
struct Capabilities {
    audio: Option<bool>,
    images: Option<bool>,
}

/// Everything the workers of one run share.
struct Operation {
    cache: MediaCache,
    budget: ByteBudget,
    mode: PrefetchMode,
    capabilities: Capabilities,
    /// Set once a payload was refused: later misses stop calling providers.
    budget_refused: AtomicBool,
    #[cfg(feature = "images")]
    http: reqwest::Client,
    #[cfg(feature = "images")]
    image_fetcher: Option<ImageFetcher>,
    #[cfg(feature = "tts")]
    cereproc: Option<CereprocClient>,
}

async fn prepare_item(operation: &Operation, presented: PresentedTranslation) -> ItemOutcome {
    let translation_id = presented.translation_id;
    let subject = MediaSubject::from(presented);
    let audio = match operation.capabilities.audio {
        None => None,
        Some(false) => Some(MediaOutcome::Unsupported),
        Some(true) => Some(prepare_audio(operation, translation_id, &subject).await),
    };
    let image = match operation.capabilities.images {
        None => None,
        Some(false) => Some(MediaOutcome::Unsupported),
        Some(true) => Some(prepare_image(operation, translation_id, &subject).await),
    };
    ItemOutcome {
        translation_id,
        audio,
        image,
    }
}

/// Probe-or-generate for one medium. Preview only probes; execution probes
/// once the budget has refused a payload, so hits stay identifiable while
/// no further provider call is made.
async fn prepare_medium<F, Fut>(
    operation: &Operation,
    translation_id: i32,
    media_type: MediaType,
    fingerprint: &MediaFingerprint,
    fetch: F,
) -> MediaOutcome
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<u8>, Option<String>), WisecrowError>>,
{
    let refused = operation.budget_refused.load(Ordering::Relaxed);
    if operation.mode == PrefetchMode::Preview || refused {
        return match operation
            .cache
            .probe(translation_id, media_type, fingerprint)
            .await
        {
            Ok(true) => MediaOutcome::Cached,
            Ok(false) if refused => MediaOutcome::BudgetExhausted,
            Ok(false) => MediaOutcome::Missing,
            Err(error) => {
                tracing::warn!(
                    translation_id,
                    media_type = media_type.as_str(),
                    error_kind = error_category(&error),
                    "Media cache probe failed"
                );
                MediaOutcome::Failed
            }
        };
    }
    let outcome = fetch_with_budget(
        &operation.cache,
        translation_id,
        media_type,
        fingerprint,
        &operation.budget,
        fetch,
    )
    .await;
    if outcome == MediaOutcome::BudgetExhausted {
        operation.budget_refused.store(true, Ordering::Relaxed);
    }
    outcome
}

#[cfg(feature = "tts")]
async fn prepare_audio(
    operation: &Operation,
    translation_id: i32,
    subject: &MediaSubject,
) -> MediaOutcome {
    let cereproc = operation.cereproc.as_ref();
    let fingerprint = match audio_cache_key(subject, cereproc) {
        Ok(fingerprint) => fingerprint,
        Err(_) => return MediaOutcome::Unsupported,
    };
    prepare_medium(
        operation,
        translation_id,
        MediaType::Audio,
        &fingerprint,
        || async {
            crate::media::audio::generate_tts(&subject.to_phrase, &subject.foreign_lang, cereproc)
                .await
                .map(|bytes| (bytes, None))
        },
    )
    .await
}

#[cfg(not(feature = "tts"))]
async fn prepare_audio(
    _operation: &Operation,
    _translation_id: i32,
    _subject: &MediaSubject,
) -> MediaOutcome {
    MediaOutcome::Unsupported
}

#[cfg(feature = "images")]
async fn prepare_image(
    operation: &Operation,
    translation_id: i32,
    subject: &MediaSubject,
) -> MediaOutcome {
    let Some(fetcher) = operation.image_fetcher.as_ref() else {
        return MediaOutcome::Unsupported;
    };
    let Some(query) = subject.applicable_image_query() else {
        return MediaOutcome::NotApplicable;
    };
    let Some(fingerprint) = image_cache_key(subject, fetcher) else {
        return MediaOutcome::NotApplicable;
    };
    prepare_medium(
        operation,
        translation_id,
        MediaType::Image,
        &fingerprint,
        || async {
            crate::media::images::fetch_image(&operation.http, query, fetcher)
                .await
                .map(|image| (image.bytes, image.attribution))
        },
    )
    .await
}

#[cfg(not(feature = "images"))]
async fn prepare_image(
    _operation: &Operation,
    _translation_id: i32,
    _subject: &MediaSubject,
) -> MediaOutcome {
    MediaOutcome::Unsupported
}

/// Runs `launch` over `items` with at most four tasks outstanding, collecting
/// results in completion order. `tasks.len()` counts finished-but-uncollected
/// tasks too, so both executing and retained tasks stay bounded. A worker
/// that cannot be joined, or a cancellation, aborts and drains the rest
/// before returning an error; per-item failures are the worker's own result.
async fn run_batch<T, R, F, Fut>(
    items: Vec<T>,
    launch: F,
    mut cancel: watch::Receiver<bool>,
) -> Result<Vec<R>, WisecrowError>
where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> Fut,
    Fut: std::future::Future<Output = R> + Send + 'static,
{
    let mut pending = items.into_iter();
    let mut tasks = tokio::task::JoinSet::new();
    let mut completed = Vec::new();
    loop {
        while tasks.len() < MAX_CONCURRENT_FETCHES {
            let Some(item) = pending.next() else { break };
            tasks.spawn(launch(item));
        }
        if tasks.is_empty() {
            return Ok(completed);
        }
        let stop = async {
            // A closed sender means the caller is gone; treat it as a stop.
            while cancel.changed().await.is_ok() {
                if *cancel.borrow() {
                    break;
                }
            }
        };
        let failure = tokio::select! {
            joined = tasks.join_next() => match joined {
                Some(Ok(outcome)) => {
                    completed.push(outcome);
                    continue;
                }
                Some(Err(error)) => format!("Media worker failed: {error}"),
                None => return Ok(completed),
            },
            () = stop => "Media preparation cancelled".to_owned(),
        };
        tasks.abort_all();
        while let Some(stopped) = tasks.join_next().await {
            if let Err(join_error) = stopped {
                if !join_error.is_cancelled() {
                    tracing::warn!(%join_error, "Media worker failed during shutdown");
                }
            }
        }
        return Err(WisecrowError::MediaError(failure));
    }
}

/// Runs the cache's fetch-or-hit path while charging the generated payload
/// to `budget` inside the fetch callback, before publication. A hit costs
/// nothing; a refused payload is never written; a payload admitted and then
/// lost to a publication error stays charged.
async fn fetch_with_budget<F, Fut>(
    cache: &MediaCache,
    translation_id: i32,
    media_type: MediaType,
    fingerprint: &MediaFingerprint,
    budget: &ByteBudget,
    fetch: F,
) -> MediaOutcome
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<u8>, Option<String>), WisecrowError>>,
{
    let mut generated = None;
    let mut rejected = false;
    let result = cache
        .get_or_fetch_attributed(translation_id, media_type, fingerprint, || async {
            let (bytes, attribution) = fetch().await?;
            let length = u64::try_from(bytes.len()).map_err(|_| {
                WisecrowError::MediaError("Generated payload length cannot be represented".into())
            })?;
            if !budget.charge(length) {
                rejected = true;
                return Err(WisecrowError::MediaError(
                    "Media publication budget exhausted".into(),
                ));
            }
            generated = Some(length);
            Ok((bytes, attribution))
        })
        .await;
    match result {
        Ok(_) => generated.map_or(MediaOutcome::Cached, |bytes| MediaOutcome::Generated {
            bytes,
        }),
        Err(_) if rejected => MediaOutcome::BudgetExhausted,
        Err(error) => {
            tracing::warn!(
                translation_id,
                media_type = media_type.as_str(),
                error_kind = error_category(&error),
                "Media preparation failed"
            );
            MediaOutcome::Failed
        }
    }
}

/// Coarse class of a media failure for the log; never the message, which
/// may quote a provider response.
fn error_category(error: &WisecrowError) -> &'static str {
    match error {
        WisecrowError::PersistenceConnectionError(_) => "database",
        WisecrowError::UnableToCreateFile(_) => "filesystem",
        WisecrowError::UnableToGetFile(_) | WisecrowError::HttpStatus { .. } => "network",
        _ => "provider_or_media",
    }
}

/// Admission counter for generated payload bytes. A charge either fits
/// entirely or is refused; nothing is refunded, so a payload admitted and
/// then lost to a publication error still counts against the run.
struct ByteBudget {
    limit: u64,
    used: std::sync::atomic::AtomicU64,
}

impl ByteBudget {
    fn new(limit: u64) -> Self {
        Self {
            limit,
            used: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn used(&self) -> u64 {
        // Relaxed suffices: the counter is the whole decision, not a signal
        // that publishes other memory.
        self.used.load(Ordering::Relaxed)
    }

    fn charge(&self, bytes: u64) -> bool {
        self.used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.limit)
            })
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn admissions_never_exceed_limit(
            limit in 1u64..10_000,
            sizes in proptest::collection::vec(0u64..20_000, 0..100),
        ) {
            let budget = ByteBudget::new(limit);
            let mut expected = 0u64;
            for size in sizes {
                let accepted = size <= limit - expected;
                prop_assert_eq!(budget.charge(size), accepted);
                if accepted {
                    expected += size;
                }
                prop_assert_eq!(budget.used(), expected);
                prop_assert!(budget.used() <= limit);
            }
        }
    }

    #[tokio::test]
    async fn concurrent_admission_is_exact() -> Result<(), Box<dyn std::error::Error>> {
        let budget = std::sync::Arc::new(ByteBudget::new(100));
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let shared = std::sync::Arc::clone(&budget);
            tasks.spawn(async move { shared.charge(25) });
        }
        let mut accepted = 0;
        while let Some(result) = tasks.join_next().await {
            accepted += u32::from(result?);
        }
        assert_eq!((accepted, budget.used()), (4, 100));
        assert!(!budget.charge(1));
        assert!(!budget.charge(u64::MAX));
        Ok(())
    }

    async fn fixture_pool() -> Result<sqlx::PgPool, Box<dyn std::error::Error>> {
        let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned()
        });
        let pool = sqlx::PgPool::connect(&url).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(pool)
    }

    /// One translation row whose media the tests may publish; media rows for
    /// it are cleared so each run starts from a miss.
    async fn fixture_translation(pool: &sqlx::PgPool) -> Result<i32, Box<dyn std::error::Error>> {
        sqlx::query(
            "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('fr', 'French')
             ON CONFLICT (code) DO NOTHING",
        )
        .execute(pool)
        .await?;
        let id: i32 = sqlx::query_scalar(
            "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
             SELECT n.id, f.id, 'budget probe', 'sonde budget'
             FROM languages n, languages f WHERE n.code = 'en' AND f.code = 'fr'
             ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
             DO UPDATE SET from_phrase = EXCLUDED.from_phrase
             RETURNING id",
        )
        .fetch_one(pool)
        .await?;
        sqlx::query("DELETE FROM media_cache WHERE translation_id = $1")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(id)
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn preview_and_rejected_payload_do_not_write() -> Result<(), Box<dyn std::error::Error>> {
        use crate::media::{fingerprint::MediaFingerprint, MediaType};
        let pool = fixture_pool().await?;
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("absent");
        let cache = MediaCache::open_existing_at(pool, &root);
        let key = MediaFingerprint::for_audio("chien", "fr", "fixture", "test", 1);
        assert!(!cache.probe(i32::MAX, MediaType::Audio, &key).await?);
        assert!(!root.exists());
        let budget = ByteBudget::new(1);
        let outcome = fetch_with_budget(
            &cache,
            i32::MAX,
            MediaType::Audio,
            &key,
            &budget,
            || async { Ok((vec![1u8, 2], None)) },
        )
        .await;
        assert_eq!(outcome, MediaOutcome::BudgetExhausted);
        assert_eq!(budget.used(), 0);
        assert!(!root.exists());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn exact_fit_is_charged_once_and_then_hits() -> Result<(), Box<dyn std::error::Error>> {
        use crate::media::{fingerprint::MediaFingerprint, MediaType};
        let pool = fixture_pool().await?;
        let id = fixture_translation(&pool).await?;
        let directory = tempfile::tempdir()?;
        let cache = MediaCache::with_cache_dir(pool.clone(), directory.path())?;
        let key = MediaFingerprint::for_audio("sonde budget", "fr", "fixture", "test", 1);
        let budget = ByteBudget::new(1);
        let generated = fetch_with_budget(&cache, id, MediaType::Audio, &key, &budget, || async {
            Ok((vec![7u8], None))
        })
        .await;
        assert_eq!(generated, MediaOutcome::Generated { bytes: 1 });
        assert_eq!(budget.used(), 1);

        // A hit costs nothing and never asks the provider, even with an
        // exhausted budget and a read-only handle on the same root.
        let readonly = MediaCache::open_existing_at(pool, directory.path());
        assert!(readonly.probe(id, MediaType::Audio, &key).await?);
        let hit = fetch_with_budget(&readonly, id, MediaType::Audio, &key, &budget, || async {
            Err(WisecrowError::MediaError(
                "provider must not be called".into(),
            ))
        })
        .await;
        assert_eq!(hit, MediaOutcome::Cached);
        assert_eq!(budget.used(), 1);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn lost_file_and_changed_fingerprint_regenerate_only_themselves(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use crate::media::{fingerprint::MediaFingerprint, MediaType};
        let pool = fixture_pool().await?;
        let id = fixture_translation(&pool).await?;
        let directory = tempfile::tempdir()?;
        let cache = MediaCache::with_cache_dir(pool.clone(), directory.path())?;
        let audio = MediaFingerprint::for_audio("sonde budget", "fr", "fixture", "test", 1);
        let image = MediaFingerprint::for_image("probe", "fr", &["fixture"], 1);
        let budget = ByteBudget::new(100);
        for (kind, key) in [(MediaType::Audio, &audio), (MediaType::Image, &image)] {
            let outcome = fetch_with_budget(&cache, id, kind, key, &budget, || async {
                Ok((vec![1u8, 2, 3], None))
            })
            .await;
            assert_eq!(outcome, MediaOutcome::Generated { bytes: 3 });
        }
        assert_eq!(budget.used(), 6);

        // The audio file goes missing: only audio regenerates.
        let audio_path: String = sqlx::query_scalar(
            "SELECT file_path FROM media_cache WHERE translation_id = $1 AND media_type = 'audio'",
        )
        .bind(id)
        .fetch_one(&pool)
        .await?;
        std::fs::remove_file(&audio_path)?;
        let regenerated =
            fetch_with_budget(&cache, id, MediaType::Audio, &audio, &budget, || async {
                Ok((vec![9u8], None))
            })
            .await;
        assert_eq!(regenerated, MediaOutcome::Generated { bytes: 1 });
        let still_cached =
            fetch_with_budget(&cache, id, MediaType::Image, &image, &budget, || async {
                Err(WisecrowError::MediaError(
                    "image must not be refetched".into(),
                ))
            })
            .await;
        assert_eq!(still_cached, MediaOutcome::Cached);

        // A changed canonical input changes the fingerprint: the old entry is
        // a miss for the new key and only that medium is regenerated.
        let renamed = MediaFingerprint::for_audio("sonde budget!", "fr", "fixture", "test", 1);
        assert!(!cache.probe(id, MediaType::Audio, &renamed).await?);
        let refreshed =
            fetch_with_budget(&cache, id, MediaType::Audio, &renamed, &budget, || async {
                Ok((vec![4u8, 4], None))
            })
            .await;
        assert_eq!(refreshed, MediaOutcome::Generated { bytes: 2 });
        assert!(cache.probe(id, MediaType::Image, &image).await?);
        assert_eq!(budget.used(), 9);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn publication_failure_keeps_the_charge() -> Result<(), Box<dyn std::error::Error>> {
        use crate::media::{fingerprint::MediaFingerprint, MediaType};
        let pool = fixture_pool().await?;
        let id = fixture_translation(&pool).await?;
        let directory = tempfile::tempdir()?;
        let cache = MediaCache::with_cache_dir(pool, directory.path())?;
        // The audio directory vanishes after the cache opened, so the
        // admitted payload cannot be published.
        std::fs::remove_dir_all(directory.path().join(MediaType::Audio.as_str()))?;
        let key = MediaFingerprint::for_audio("sonde budget", "fr", "fixture", "lost", 1);
        let budget = ByteBudget::new(10);
        let outcome = fetch_with_budget(&cache, id, MediaType::Audio, &key, &budget, || async {
            Ok((vec![0u8; 4], None))
        })
        .await;
        assert_eq!(outcome, MediaOutcome::Failed);
        assert_eq!(budget.used(), 4, "a charge is never refunded");
        Ok(())
    }

    #[tokio::test]
    async fn scheduler_never_exceeds_four() -> Result<(), Box<dyn std::error::Error>> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (_stop, cancel) = tokio::sync::watch::channel(false);
        let completed = run_batch(
            (0..100).collect(),
            |id| {
                let active = Arc::clone(&active);
                let peak = Arc::clone(&peak);
                async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    id
                }
            },
            cancel,
        )
        .await?;
        assert_eq!(completed.len(), 100);
        assert!(peak.load(Ordering::SeqCst) <= 4);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        Ok(())
    }

    /// Decrements a live-worker counter when dropped, so an aborted worker
    /// still reports that it stopped.
    struct LiveGuard(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for LiveGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn cancellation_drains_every_worker() -> Result<(), Box<dyn std::error::Error>> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let live = Arc::new(AtomicUsize::new(0));
        let (stop, cancel) = tokio::sync::watch::channel(false);
        let runner = run_batch(
            (0..20).collect(),
            |id: u32| {
                let live = Arc::clone(&live);
                async move {
                    live.fetch_add(1, Ordering::SeqCst);
                    let _guard = LiveGuard(live);
                    // Workers wait for a stop that only cancellation delivers.
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    id
                }
            },
            cancel,
        );
        let stopper = async {
            tokio::task::yield_now().await;
            stop.send_replace(true);
        };
        let (result, ()) = tokio::join!(runner, stopper);
        assert!(
            matches!(result, Err(WisecrowError::MediaError(_))),
            "{result:?}"
        );
        assert_eq!(live.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn delayed_worker_error_is_an_outcome_not_a_crash(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_stop, cancel) = tokio::sync::watch::channel(false);
        let outcomes = run_batch(
            vec![1u8, 2, 3],
            |id| async move {
                tokio::time::sleep(std::time::Duration::from_millis(u64::from(id))).await;
                if id == 2 {
                    Err(WisecrowError::MediaError("provider down".into()))
                } else {
                    Ok(id)
                }
            },
            cancel,
        )
        .await?;
        assert_eq!(outcomes.len(), 3);
        assert_eq!(
            outcomes.iter().filter(|outcome| outcome.is_err()).count(),
            1
        );
        Ok(())
    }

    #[rstest::rstest]
    #[case(0, 0, 1, false)]
    #[case(5001, 0, 1, false)]
    #[case(100, 9901, 1, false)]
    #[case(1, u32::MAX, 1, false)]
    #[case(100, 0, 0, false)]
    #[case(100, 0, 5 * 1024 * 1024 * 1024 + 1, false)]
    #[case(100, 0, 64 * 1024 * 1024, true)]
    #[case(5000, 5000, 1, true)]
    fn option_bounds(
        #[case] limit: u32,
        #[case] offset: u32,
        #[case] bytes: u64,
        #[case] valid: bool,
    ) {
        assert_eq!(
            PrefetchOptions::new(
                limit,
                offset,
                bytes,
                RequestedMedia::Both,
                PrefetchMode::Preview
            )
            .is_ok(),
            valid
        );
    }
}
