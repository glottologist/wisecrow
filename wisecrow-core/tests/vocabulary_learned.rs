use sqlx::PgPool;
use wisecrow::vocabulary::VocabularyQuery;

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

async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM cards WHERE translation_id IN (
            SELECT id FROM translations WHERE from_language_id IN (
                SELECT id FROM languages WHERE code IN ('en', 'es')
            )
        )",
    )
    .execute(pool)
    .await
    .expect("cards cleanup");
    sqlx::query(
        "DELETE FROM translations WHERE from_language_id IN (
            SELECT id FROM languages WHERE code IN ('en', 'es')
        )",
    )
    .execute(pool)
    .await
    .expect("translations cleanup");
}

async fn seed_card(pool: &PgPool, foreign: &str, native: &str, state: i16, stability: f32) {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('es', 'Spanish')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages seed");

    let tid: i32 = sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, from_phrase, to_language_id, to_phrase, frequency)
         VALUES ((SELECT id FROM languages WHERE code='en'), $1,
                 (SELECT id FROM languages WHERE code='es'), $2, 100)
         RETURNING id",
    )
    .bind(native)
    .bind(foreign)
    .fetch_one(pool)
    .await
    .expect("translation insert");

    sqlx::query(
        "INSERT INTO cards (translation_id, user_id, state, stability)
         VALUES ($1, 1, $2, $3)",
    )
    .bind(tid)
    .bind(state)
    .bind(stability)
    .execute(pool)
    .await
    .expect("card insert");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn learned_returns_only_state_2_by_default() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    seed_card(&pool, "casa", "house", 2, 5.0).await;
    seed_card(&pool, "perro", "dog", 3, 5.0).await;
    seed_card(&pool, "gato", "cat", 1, 0.0).await;

    let entries = VocabularyQuery::learned(&pool, "en", "es", 1, &[2], None, 100)
        .await
        .expect("query failed");
    let foreigns: Vec<&str> = entries.iter().map(|e| e.to_phrase.as_str()).collect();
    assert_eq!(foreigns, vec!["casa"]);

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn learned_with_states_2_and_3() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    seed_card(&pool, "casa", "house", 2, 5.0).await;
    seed_card(&pool, "perro", "dog", 3, 5.0).await;
    seed_card(&pool, "gato", "cat", 1, 0.0).await;

    let entries = VocabularyQuery::learned(&pool, "en", "es", 1, &[2, 3], None, 100)
        .await
        .expect("query failed");
    let mut foreigns: Vec<&str> = entries.iter().map(|e| e.to_phrase.as_str()).collect();
    foreigns.sort();
    assert_eq!(foreigns, vec!["casa", "perro"]);

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn learned_filters_by_min_stability() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    seed_card(&pool, "casa", "house", 2, 10.0).await;
    seed_card(&pool, "perro", "dog", 2, 3.0).await;

    let entries = VocabularyQuery::learned(&pool, "en", "es", 1, &[2], Some(7.0), 100)
        .await
        .expect("query failed");
    let foreigns: Vec<&str> = entries.iter().map(|e| e.to_phrase.as_str()).collect();
    assert_eq!(foreigns, vec!["casa"]);

    cleanup(&pool).await;
}
