use async_trait::async_trait;
use sqlx::PgPool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use wisecrow::errors::WisecrowError;
use wisecrow::grammar::gloss;
use wisecrow::llm::LlmProvider;

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

struct CountingProvider {
    calls: Arc<AtomicUsize>,
    response: String,
}

#[async_trait]
impl LlmProvider for CountingProvider {
    async fn generate(&self, _prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
    }

    fn name(&self) -> &str {
        "counting-mock"
    }
}

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM glosses WHERE lang_code IN ('ru', 'es', 'pt')")
        .execute(pool)
        .await
        .expect("cleanup failed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn second_call_is_cache_hit() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let calls = Arc::new(AtomicUsize::new(0));
    let provider = CountingProvider {
        calls: Arc::clone(&calls), // clone: Arc shared between test and provider
        response: "GLOSS".to_owned(),
    };

    let g1 = gloss::generate_or_lookup(&pool, &provider, "Меня зовут Иван", "ru", "Russian")
        .await
        .expect("first call failed");
    let g2 = gloss::generate_or_lookup(&pool, &provider, "Меня зовут Иван", "ru", "Russian")
        .await
        .expect("second call failed");

    assert_eq!(g1, "GLOSS");
    assert_eq!(g2, "GLOSS");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "second call must be a cache hit"
    );

    let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM glosses WHERE lang_code = 'ru'")
        .fetch_one(&pool)
        .await
        .expect("count failed");
    assert_eq!(row_count, 1);

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn different_lang_codes_cache_separately() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let calls = Arc::new(AtomicUsize::new(0));
    let provider = CountingProvider {
        calls: Arc::clone(&calls), // clone: Arc shared between test and provider
        response: "GLOSS".to_owned(),
    };

    gloss::generate_or_lookup(&pool, &provider, "casa", "es", "Spanish")
        .await
        .expect("es failed");
    gloss::generate_or_lookup(&pool, &provider, "casa", "pt", "Portuguese")
        .await
        .expect("pt failed");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "different lang codes are different cache keys"
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn refresh_bypasses_cache_and_overwrites() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let calls = Arc::new(AtomicUsize::new(0));
    let provider1 = CountingProvider {
        calls: Arc::clone(&calls), // clone: Arc shared between test and provider
        response: "OLD".to_owned(),
    };
    let provider2 = CountingProvider {
        calls: Arc::clone(&calls), // clone: shared counter across both providers
        response: "NEW".to_owned(),
    };

    let g1 =
        gloss::generate_or_lookup_with_refresh(&pool, &provider1, "casa", "es", "Spanish", false)
            .await
            .expect("first failed");
    assert_eq!(g1, "OLD");

    let g2 = gloss::generate_or_lookup_with_refresh(
        &pool, &provider2, "casa", "es", "Spanish", true, // refresh
    )
    .await
    .expect("refresh failed");
    assert_eq!(g2, "NEW");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "refresh must always re-prompt"
    );

    let g3 = gloss::generate_or_lookup(&pool, &provider1, "casa", "es", "Spanish")
        .await
        .expect("third failed");
    assert_eq!(g3, "NEW", "subsequent non-refresh reads see overwritten");

    cleanup(&pool).await;
}
