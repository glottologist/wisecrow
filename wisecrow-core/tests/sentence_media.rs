//! The sentence-keyed half of the media cache.
//!
//! A clip belongs to what is spoken, not to the row that asked for it: two
//! examples quoting one sentence share a file, a re-imported point finds its
//! clips where it left them, and only an explicit prune reclaims a clip no
//! current example needs.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use sqlx::PgPool;
use wisecrow::errors::WisecrowError;
use wisecrow::media::cache::MediaCache;
use wisecrow::media::fingerprint::MediaFingerprint;
use wisecrow::media::MediaType;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query("TRUNCATE sentence_media")
        .execute(&pool)
        .await?;
    Ok(pool)
}

fn fingerprint_of(sentence: &str) -> MediaFingerprint {
    MediaFingerprint::for_audio(sentence, "es", "edge", "es-ES-ElviraNeural", 1)
}

/// Counts how often the cache asked for bytes.
struct Generator(AtomicUsize);

impl Generator {
    fn speak(&self) -> impl std::future::Future<Output = Result<Vec<u8>, WisecrowError>> + '_ {
        self.0.fetch_add(1, Ordering::SeqCst);
        async { Ok(b"mp3 bytes".to_vec()) }
    }

    fn calls(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

async fn stored_path(pool: &PgPool, fingerprint: &MediaFingerprint) -> Option<String> {
    sqlx::query_scalar("SELECT file_path FROM sentence_media WHERE fingerprint = $1")
        .bind(fingerprint.as_str())
        .fetch_optional(pool)
        .await
        .expect("row query")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sentence_is_generated_once_and_shared_by_every_example_that_quotes_it() -> TestResult {
    let pool = reset_pool().await?;
    let dir = tempfile::tempdir()?;
    let cache = MediaCache::with_cache_dir(pool.clone(), dir.path())?;
    let generator = Generator(AtomicUsize::new(0));
    let fingerprint = fingerprint_of("Estoy cansado.");

    let first = cache
        .get_or_fetch_sentence(
            &fingerprint,
            MediaType::Audio,
            "es",
            "Estoy cansado.",
            || generator.speak(),
        )
        .await?;
    let second = cache
        .get_or_fetch_sentence(
            &fingerprint,
            MediaType::Audio,
            "es",
            "Estoy cansado.",
            || generator.speak(),
        )
        .await?;

    assert_eq!(generator.calls(), 1, "the second call is a hit");
    assert_eq!(first, second);
    assert_eq!(
        first.file_name().and_then(|name| name.to_str()),
        Some(format!("sentence-{}.mp3", fingerprint.as_str()).as_str()),
        "the file is named after the sentence, not a row"
    );
    assert!(first.starts_with(dir.path().join("audio")));
    assert_eq!(tokio::fs::read(&first).await?, b"mp3 bytes");
    assert!(cache.probe_sentence(&fingerprint, MediaType::Audio).await?);
    assert!(
        !cache
            .probe_sentence(&fingerprint_of("Soy cansado."), MediaType::Audio)
            .await?
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_row_whose_file_is_gone_misses_and_regenerates() -> TestResult {
    let pool = reset_pool().await?;
    let dir = tempfile::tempdir()?;
    let cache = MediaCache::with_cache_dir(pool.clone(), dir.path())?;
    let generator = Generator(AtomicUsize::new(0));
    let fingerprint = fingerprint_of("Estoy cansado.");

    let path = cache
        .get_or_fetch_sentence(
            &fingerprint,
            MediaType::Audio,
            "es",
            "Estoy cansado.",
            || generator.speak(),
        )
        .await?;
    tokio::fs::remove_file(&path).await?;
    assert!(!cache.probe_sentence(&fingerprint, MediaType::Audio).await?);

    let again = cache
        .get_or_fetch_sentence(
            &fingerprint,
            MediaType::Audio,
            "es",
            "Estoy cansado.",
            || generator.speak(),
        )
        .await?;
    assert_eq!(generator.calls(), 2);
    assert_eq!(again, path);
    assert!(Path::new(&again).is_file());
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn prune_removes_only_what_no_kept_fingerprint_names() -> TestResult {
    let pool = reset_pool().await?;
    let dir = tempfile::tempdir()?;
    let cache = MediaCache::with_cache_dir(pool.clone(), dir.path())?;
    let generator = Generator(AtomicUsize::new(0));
    let kept = fingerprint_of("Estoy cansado.");
    let stale = fingerprint_of("Estoy cansada.");
    let kept_path = cache
        .get_or_fetch_sentence(&kept, MediaType::Audio, "es", "Estoy cansado.", || {
            generator.speak()
        })
        .await?;
    let stale_path = cache
        .get_or_fetch_sentence(&stale, MediaType::Audio, "es", "Estoy cansada.", || {
            generator.speak()
        })
        .await?;

    let pruned = cache.prune_sentences(std::slice::from_ref(&kept)).await?;

    assert_eq!(pruned, 1);
    assert!(kept_path.is_file());
    assert!(
        !stale_path.exists(),
        "the stale clip's file goes with its row"
    );
    assert!(stored_path(&pool, &kept).await.is_some());
    assert!(stored_path(&pool, &stale).await.is_none());
    assert_eq!(
        cache.prune_sentences(&[kept]).await?,
        0,
        "a second prune finds nothing"
    );
    Ok(())
}
