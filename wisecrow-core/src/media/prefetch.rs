use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};
use sqlx::PgPool;
use tracing::info;

use crate::errors::WisecrowError;
use crate::media::cache::MediaCache;

#[cfg(feature = "images")]
use crate::media::images::ImageFetcher;

const MAX_CONCURRENT_FETCHES: usize = 4;
type MediaRow = (i32, String);
type PrefetchHandle = tokio::task::JoinHandle<usize>;

/// Prefetches audio and images for all translations in a language pair.
///
/// Fetching of audio requires the `tts` feature and of images the
/// `images` feature. When neither is enabled the function counts the
/// available translations without performing any network requests.
///
/// # Errors
///
/// Returns an error if the database query fails, the progress bar
/// template is invalid, or a cache / fetch operation fails.
pub async fn prefetch_media(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
    fetch_audio: bool,
    fetch_images: bool,
    #[cfg(feature = "images")] image_fetcher: Option<&ImageFetcher>,
) -> Result<usize, WisecrowError> {
    let rows = load_media_rows(pool, native_lang, foreign_lang).await?;
    if rows.is_empty() {
        info!("No translations found for {native_lang}-{foreign_lang}");
        return Ok(0);
    }
    let total = rows.len();
    info!("Prefetching media for {total} translations ({native_lang}-{foreign_lang})");
    let progress = progress_bar(total)?;
    let handles = spawn_prefetches(
        pool,
        rows,
        foreign_lang,
        fetch_audio,
        fetch_images,
        #[cfg(feature = "images")]
        image_fetcher,
        &progress,
    )
    .await?;
    let total_fetched = collect_prefetches(handles).await;
    progress.finish_with_message("done");
    info!("Prefetched {total_fetched} media items");
    Ok(total_fetched)
}

async fn load_media_rows(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
) -> Result<Vec<MediaRow>, WisecrowError> {
    sqlx::query_as::<_, MediaRow>(
        "SELECT t.id, t.to_phrase
         FROM translations t
         JOIN languages fl ON fl.id = t.from_language_id
         JOIN languages tl ON tl.id = t.to_language_id
         WHERE fl.code = $1 AND tl.code = $2
         ORDER BY t.id",
    )
    .bind(native_lang)
    .bind(foreign_lang)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

fn progress_bar(total: usize) -> Result<ProgressBar, WisecrowError> {
    let progress = ProgressBar::new(u64::try_from(total).unwrap_or(u64::MAX));
    progress.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")?,
    );
    Ok(progress)
}

async fn spawn_prefetches(
    pool: &PgPool,
    rows: Vec<MediaRow>,
    foreign_lang: &str,
    fetch_audio: bool,
    fetch_images: bool,
    #[cfg(feature = "images")] image_fetcher: Option<&ImageFetcher>,
    progress: &ProgressBar,
) -> Result<Vec<PrefetchHandle>, WisecrowError> {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_FETCHES));
    #[cfg(feature = "images")]
    let image_fetcher = image_fetcher.cloned(); // clone: ImageFetcher shares Arc providers
    let mut handles = Vec::new();
    for (translation_id, to_phrase) in rows {
        let permit = Arc::clone(&semaphore) // clone: Arc shared ownership for semaphore
            .acquire_owned()
            .await
            .map_err(|error| WisecrowError::InvalidInput(format!("Semaphore closed: {error}")))?;
        let pool_owned = pool.clone(); // clone: PgPool is Arc-based
        let foreign = String::from(foreign_lang);
        let progress_ref = progress.clone(); // clone: ProgressBar is Arc-based
        #[cfg(feature = "images")]
        let image_fetcher = image_fetcher.clone(); // clone: share provider chain across tasks
        let handle = tokio::spawn(async move {
            let fetched = prefetch_single(
                &pool_owned,
                translation_id,
                &to_phrase,
                &foreign,
                fetch_audio,
                fetch_images,
                #[cfg(feature = "images")]
                image_fetcher.as_ref(),
            )
            .await;
            progress_ref.inc(1);
            drop(permit);
            fetched
        });
        handles.push(handle);
    }
    Ok(handles)
}

async fn collect_prefetches(handles: Vec<PrefetchHandle>) -> usize {
    let mut total_fetched = 0usize;
    for handle in handles {
        match handle.await {
            Ok(count) => total_fetched = total_fetched.saturating_add(count),
            Err(error) => tracing::warn!(?error, "media prefetch task failed"),
        }
    }
    total_fetched
}

async fn prefetch_single(
    pool: &PgPool,
    translation_id: i32,
    to_phrase: &str,
    foreign_lang: &str,
    fetch_audio: bool,
    fetch_images: bool,
    #[cfg(feature = "images")] image_fetcher: Option<&ImageFetcher>,
) -> usize {
    let cache = match MediaCache::new(pool.clone()) {
        // clone: MediaCache owns an Arc-backed pool handle
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Cache init failed for translation {translation_id}: {e}");
            return 0;
        }
    };

    let audio_count =
        prefetch_audio(&cache, translation_id, to_phrase, foreign_lang, fetch_audio).await;
    let image_count = prefetch_image(
        &cache,
        translation_id,
        to_phrase,
        fetch_images,
        #[cfg(feature = "images")]
        image_fetcher,
    )
    .await;

    audio_count.saturating_add(image_count)
}

#[cfg(feature = "tts")]
async fn prefetch_audio(
    cache: &MediaCache,
    translation_id: i32,
    to_phrase: &str,
    foreign_lang: &str,
    fetch_audio: bool,
) -> usize {
    if !fetch_audio {
        return 0;
    }
    let lang = String::from(foreign_lang);
    let word = String::from(to_phrase);
    let result = cache
        .get_or_fetch(translation_id, crate::media::MediaType::Audio, || {
            crate::media::audio::generate_tts(&word, &lang)
        })
        .await;
    usize::from(result.is_ok())
}

#[cfg(not(feature = "tts"))]
async fn prefetch_audio(
    _cache: &MediaCache,
    _translation_id: i32,
    _to_phrase: &str,
    _foreign_lang: &str,
    _fetch_audio: bool,
) -> usize {
    0
}

#[cfg(feature = "images")]
async fn prefetch_image(
    cache: &MediaCache,
    translation_id: i32,
    to_phrase: &str,
    fetch_images: bool,
    image_fetcher: Option<&ImageFetcher>,
) -> usize {
    if !fetch_images {
        return 0;
    }
    let Some(fetcher) = image_fetcher else {
        return 0;
    };
    let client = reqwest::Client::new();
    let word = String::from(to_phrase);
    let result = cache
        .get_or_fetch(translation_id, crate::media::MediaType::Image, || async {
            crate::media::images::fetch_image(&client, &word, fetcher).await
        })
        .await;
    usize::from(result.is_ok())
}

#[cfg(not(feature = "images"))]
async fn prefetch_image(
    _cache: &MediaCache,
    _translation_id: i32,
    _to_phrase: &str,
    _fetch_images: bool,
) -> usize {
    0
}
