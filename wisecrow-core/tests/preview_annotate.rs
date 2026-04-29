use sqlx::PgPool;
use wisecrow::preview::annotate::{annotate_tokens, Status};

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

async fn seed(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('es', 'Spanish')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages");

    let mut tids = vec![];
    for (foreign, freq) in [("casa", 100), ("perro", 80), ("gato", 60)] {
        let tid: i32 = sqlx::query_scalar(
            "INSERT INTO translations (from_language_id, from_phrase, to_language_id, to_phrase, frequency)
             VALUES ((SELECT id FROM languages WHERE code='en'), 'house',
                     (SELECT id FROM languages WHERE code='es'), $1, $2) RETURNING id",
        )
        .bind(foreign)
        .bind(freq)
        .fetch_one(pool)
        .await
        .expect("translation insert");
        tids.push((foreign, tid));
    }
    sqlx::query("INSERT INTO cards (translation_id, user_id, state) VALUES ($1, 1, 2)")
        .bind(tids[0].1)
        .execute(pool)
        .await
        .expect("card 0 insert");
    sqlx::query("INSERT INTO cards (translation_id, user_id, state) VALUES ($1, 1, 1)")
        .bind(tids[1].1)
        .execute(pool)
        .await
        .expect("card 1 insert");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn annotates_tokens_with_status() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    seed(&pool).await;

    let tokens = vec![
        "casa".to_owned(),
        "perro".to_owned(),
        "gato".to_owned(),
        "desconocido".to_owned(),
    ];
    let result = annotate_tokens(&pool, "es", 1, &tokens)
        .await
        .expect("annotate");

    let by_token: std::collections::HashMap<&str, &Status> = result
        .iter()
        .map(|a| (a.token.as_str(), &a.status))
        .collect();

    assert_eq!(by_token.get("casa"), Some(&&Status::Known));
    assert_eq!(by_token.get("perro"), Some(&&Status::Learning));
    assert_eq!(by_token.get("gato"), Some(&&Status::New));
    assert_eq!(by_token.get("desconocido"), Some(&&Status::Unknown));

    cleanup(&pool).await;
}
