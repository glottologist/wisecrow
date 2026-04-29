use sqlx::{PgPool, Row};

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

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn glosses_table_exists() {
    let pool = test_pool().await;

    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'glosses')",
    )
    .fetch_one(&pool)
    .await
    .expect("query failed");
    assert!(exists, "glosses table should exist after migrations");

    let rows = sqlx::query(
        "SELECT column_name FROM information_schema.columns
         WHERE table_name = 'glosses' ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .expect("column query failed");

    let cols: Vec<String> = rows
        .iter()
        .map(|r| r.get::<String, _>("column_name"))
        .collect();
    assert_eq!(
        cols,
        vec![
            "id",
            "sentence_hash",
            "lang_code",
            "gloss_text",
            "created_at"
        ]
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn glosses_unique_constraint_enforced() {
    let pool = test_pool().await;

    sqlx::query("DELETE FROM glosses WHERE sentence_hash = $1")
        .bind("a".repeat(64))
        .execute(&pool)
        .await
        .expect("cleanup failed");

    let hash = "a".repeat(64);
    sqlx::query("INSERT INTO glosses (sentence_hash, lang_code, gloss_text) VALUES ($1, $2, $3)")
        .bind(&hash)
        .bind("ru")
        .bind("first")
        .execute(&pool)
        .await
        .expect("first insert failed");

    let dup = sqlx::query(
        "INSERT INTO glosses (sentence_hash, lang_code, gloss_text) VALUES ($1, $2, $3)",
    )
    .bind(&hash)
    .bind("ru")
    .bind("second")
    .execute(&pool)
    .await;
    assert!(dup.is_err(), "duplicate (hash, lang) should be rejected");

    sqlx::query("DELETE FROM glosses WHERE sentence_hash = $1")
        .bind(&hash)
        .execute(&pool)
        .await
        .expect("cleanup failed");
}
