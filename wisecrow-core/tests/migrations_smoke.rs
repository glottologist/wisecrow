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

async fn assert_mobile_offline_sync_schema(pool: &PgPool) {
    for table in [
        "mobile_devices",
        "corpus_changes",
        "card_review_baselines",
        "review_events",
        "card_changes",
        "mobile_nback_uploads",
    ] {
        let exists: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(table)
            .fetch_one(pool)
            .await
            .expect("relation existence query failed");
        assert!(exists, "{table} should exist after migrations");
    }

    for (table, expected_trigger) in [
        ("translations", "wisecrow_corpus_change"),
        ("phrase_translations", "wisecrow_phrase_membership_change"),
        ("cards", "wisecrow_card_change"),
    ] {
        let triggers: Vec<String> = sqlx::query_scalar(
            "SELECT tgname
             FROM pg_trigger
             JOIN pg_class ON pg_class.oid = tgrelid
             WHERE pg_class.relname = $1
               AND NOT tgisinternal
               AND tgenabled <> 'D'
             ORDER BY tgname",
        )
        .bind(table)
        .fetch_all(pool)
        .await
        .expect("trigger query failed");
        assert_eq!(triggers, [expected_trigger]);
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn mobile_offline_sync_schema_is_idempotent() {
    let pool = test_pool().await;
    assert_mobile_offline_sync_schema(&pool).await;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("second migration run failed");
    assert_mobile_offline_sync_schema(&pool).await;
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

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn auth_and_sync_schema_present() {
    let pool = test_pool().await;

    let user_cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns WHERE table_name = 'users'",
    )
    .fetch_all(&pool)
    .await
    .expect("users column query failed");
    for col in ["email", "password_hash", "is_admin"] {
        assert!(
            user_cols.iter().any(|c| c == col),
            "users.{col} should exist after migrations"
        );
    }

    for table in ["auth_sessions", "sync_clients"] {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = $1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .expect("table existence query failed");
        assert!(exists, "{table} table should exist after migrations");
    }

    let dnb_cols: Vec<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns WHERE table_name = 'dnb_sessions'",
    )
    .fetch_all(&pool)
    .await
    .expect("dnb_sessions column query failed");
    assert!(
        dnb_cols.iter().any(|c| c == "consecutive_below_start"),
        "dnb_sessions.consecutive_below_start should exist after migrations"
    );
}
