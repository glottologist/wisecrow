use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};
use sqlx::PgPool;
use tracing::info;

use crate::errors::WisecrowError;
use crate::media::cache::MediaCache;

const MAX_CONCURRENT_FETCHES: usize = 4;

/// Prefetches audio and images for all translations in a language pair.
///
/// Fetching of audio requires the `audio` feature and of images the
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
    unsplash_api_key: Option<&str>,
) -> Result<usize, WisecrowError> {
    let rows = sqlx::query_as::<_, (i32, String)>(
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
    .await?;

    let total = rows.len();
    if total == 0 {
        info!("No translations found for {native_lang}-{foreign_lang}");
        return Ok(0);
    }

    info!("Prefetching media for {total} translations ({native_lang}-{foreign_lang})");

    let pb = ProgressBar::new(u64::try_from(total).unwrap_or(u64::MAX));
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")?,
    );

    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_FETCHES));
    let mut handles = Vec::new();

    for (translation_id, to_phrase) in rows {
        let permit = Arc::clone(&semaphore) // clone: Arc shared ownership for semaphore
            .acquire_owned()
            .await
            .map_err(|e| WisecrowError::InvalidInput(format!("Semaphore closed: {e}")))?;

        let pool_owned = pool.clone(); // clone: PgPool is Arc-based
        let foreign = foreign_lang.to_owned();
        let api_key = unsplash_api_key.map(str::to_owned);
        let pb_ref = pb.clone(); // clone: ProgressBar is Arc-based

        let handle = tokio::spawn(async move {
            let fetched = prefetch_single(
                &pool_owned,
                translation_id,
                &to_phrase,
                &foreign,
                fetch_audio,
                fetch_images,
                api_key.as_deref(),
            )
            .await;

            pb_ref.inc(1);
            drop(permit);
            fetched
        });

        handles.push(handle);
    }

    let mut total_fetched = 0usize;
    for handle in handles {
        if let Ok(count) = handle.await {
            total_fetched = total_fetched.saturating_add(count);
        }
    }

    pb.finish_with_message("done");
    info!("Prefetched {total_fetched} media items");

    Ok(total_fetched)
}

async fn prefetch_single(
    pool: &PgPool,
    translation_id: i32,
    to_phrase: &str,
    foreign_lang: &str,
    fetch_audio: bool,
    fetch_images: bool,
    unsplash_api_key: Option<&str>,
) -> usize {
    let cache = match MediaCache::new(pool.clone()) {
        // clone: PgPool is Arc-based
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
        unsplash_api_key,
    )
    .await;

    audio_count.saturating_add(image_count)
}

#[cfg(feature = "audio")]
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
    let lang = foreign_lang.to_owned();
    let word = to_phrase.to_owned();
    let result = cache
        .get_or_fetch(translation_id, crate::media::MediaType::Audio, || {
            crate::media::audio::generate_tts(&word, &lang)
        })
        .await;
    usize::from(result.is_ok())
}

#[cfg(not(feature = "audio"))]
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
    unsplash_api_key: Option<&str>,
) -> usize {
    if !fetch_images {
        return 0;
    }
    let Some(key) = unsplash_api_key else {
        return 0;
    };
    let client = reqwest::Client::new();
    let word = to_phrase.to_owned();
    let key_owned = key.to_owned();
    let result = cache
        .get_or_fetch(translation_id, crate::media::MediaType::Image, || async {
            crate::media::images::fetch_image(&client, &word, &key_owned).await
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
    _unsplash_api_key: Option<&str>,
) -> usize {
    0
}
