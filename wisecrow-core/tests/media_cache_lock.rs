use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use sqlx::PgPool;
use wisecrow::media::cache::MediaCache;
use wisecrow::media::MediaType;

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

async fn seed_translation(pool: &PgPool) -> i32 {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('gd', 'Gaelic')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages seed");
    let (id,): (i32,) = sqlx::query_as(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, 'lock probe', 'dearbhadh glais'
         FROM languages fl, languages tl WHERE fl.code = 'en' AND tl.code = 'gd'
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
         DO UPDATE SET from_phrase = EXCLUDED.from_phrase
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("translation seed");
    sqlx::query("DELETE FROM media_cache WHERE translation_id = $1")
        .bind(id)
        .execute(pool)
        .await
        .expect("cache cleanup");
    id
}

/// Two concurrent misses for one key must produce exactly one provider call:
/// the advisory lock serialises the miss path and the second holder finds the
/// row the first wrote.
#[tokio::test]
async fn concurrent_misses_fetch_once() {
    let pool = test_pool().await;
    let translation_id = seed_translation(&pool).await;
    let cache = MediaCache::new(pool.clone()).expect("cache init");
    let cache_b = MediaCache::new(pool.clone()).expect("cache init");
    let calls = Arc::new(AtomicUsize::new(0));

    let fetcher = |calls: Arc<AtomicUsize>| {
        move || async move {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            Ok((vec![0xFFu8, 0xF3, 0x01, 0x02], None))
        }
    };

    let (a, b) = tokio::join!(
        cache.get_or_fetch_attributed(
            translation_id,
            MediaType::Audio,
            fetcher(Arc::clone(&calls))
        ),
        cache_b.get_or_fetch_attributed(
            translation_id,
            MediaType::Audio,
            fetcher(Arc::clone(&calls))
        ),
    );

    let (path_a, _) = a.expect("first fetch");
    let (path_b, _) = b.expect("second fetch");
    assert_eq!(path_a, path_b);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "provider called more than once"
    );
}
