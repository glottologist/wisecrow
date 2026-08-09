use sqlx::PgPool;
use wisecrow::media::load_media_subject;

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

async fn seed_pair(pool: &PgPool, from: &str, to: &str) -> i32 {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('gd', 'Gaelic')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages seed");
    let (id,): (i32,) = sqlx::query_as(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, $1, $2
         FROM languages fl, languages tl WHERE fl.code = 'en' AND tl.code = 'gd'
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
         DO UPDATE SET from_phrase = EXCLUDED.from_phrase
         RETURNING id",
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await
    .expect("translation seed");
    id
}

/// The served text comes from the database row, never from the caller — the
/// property that makes cache poisoning impossible.
#[tokio::test]
async fn subject_is_the_rows_own_text() {
    let pool = test_pool().await;
    let id = seed_pair(&pool, "subject probe water", "uisge dearbhaidh").await;
    let subject = load_media_subject(&pool, id)
        .await
        .expect("query")
        .expect("row exists");
    assert_eq!(subject.to_phrase, "uisge dearbhaidh");
    assert_eq!(subject.from_phrase, "subject probe water");
    assert_eq!(subject.foreign_lang, "gd");
}

#[tokio::test]
async fn unknown_id_is_none() {
    let pool = test_pool().await;
    assert!(load_media_subject(&pool, 999_999_999)
        .await
        .expect("query")
        .is_none());
}

/// A different id yields that pair's own text, not anything a caller chose.
#[tokio::test]
async fn each_id_maps_to_its_own_pair() {
    let pool = test_pool().await;
    let first = seed_pair(&pool, "subject probe fire", "teine dearbhaidh").await;
    let second = seed_pair(&pool, "subject probe stone", "clach dearbhaidh").await;
    let a = load_media_subject(&pool, first)
        .await
        .expect("query")
        .expect("row");
    let b = load_media_subject(&pool, second)
        .await
        .expect("query")
        .expect("row");
    assert_eq!(a.to_phrase, "teine dearbhaidh");
    assert_eq!(b.to_phrase, "clach dearbhaidh");
    assert_ne!(a.from_phrase, b.from_phrase);
}
