use sqlx::PgPool;
use wisecrow::pipeline::evidence_checksum;
use wisecrow::pipeline::lock::{pair_lock_key, with_pair_lock};

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
async fn replacing_a_sentence_changes_the_checksum_even_when_the_count_does_not() {
    let pool = test_pool().await;
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('cy', 'Welsh')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(&pool)
    .await
    .expect("languages");
    sqlx::query(
        "DELETE FROM translations
         WHERE from_phrase IN ('pipeline-rank-alpha', 'pipeline-rank-beta', 'pipeline-rank-gamma')",
    )
    .execute(&pool)
    .await
    .expect("clear fixtures");

    sqlx::query(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, phrase, target
         FROM languages fl
         JOIN languages tl ON tl.code = 'cy'
         JOIN (VALUES ('pipeline-rank-alpha', 'alpha-one'), ('pipeline-rank-beta', 'beta-one'))
              AS seed(phrase, target) ON true
         WHERE fl.code = 'en'",
    )
    .execute(&pool)
    .await
    .expect("seed");

    let original = evidence_checksum(&pool, "cy").await.expect("original");

    sqlx::query(
        "UPDATE translations SET to_phrase = 'alpha-two'
         WHERE from_phrase = 'pipeline-rank-alpha'",
    )
    .execute(&pool)
    .await
    .expect("replace");
    let replaced = evidence_checksum(&pool, "cy").await.expect("replaced");
    assert_ne!(original, replaced);

    sqlx::query("DELETE FROM translations WHERE from_phrase = 'pipeline-rank-beta'")
        .execute(&pool)
        .await
        .expect("delete");
    sqlx::query(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, 'pipeline-rank-gamma', 'beta-one'
         FROM languages fl, languages tl
         WHERE fl.code = 'en' AND tl.code = 'cy'",
    )
    .execute(&pool)
    .await
    .expect("restore count");
    let restored = evidence_checksum(&pool, "cy")
        .await
        .expect("restored count");
    assert_ne!(original, restored);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn pair_lock_blocks_another_connection_until_release() {
    let pool = test_pool().await;
    let key = pair_lock_key("en", "cy");

    with_pair_lock(&pool, "en", "cy", || async {
        let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(key)
            .fetch_one(&pool)
            .await
            .expect("probe");
        if acquired {
            sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(key)
                .execute(&pool)
                .await
                .expect("unlock leaked session lock");
        }
        assert!(
            !acquired,
            "the pair lock must be visible to other connections"
        );
        Ok(())
    })
    .await
    .expect("lock");

    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(key)
        .fetch_one(&pool)
        .await
        .expect("after release");
    if acquired {
        sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(key)
            .execute(&pool)
            .await
            .expect("unlock");
    }
    assert!(acquired, "releasing the transaction releases the lock");
}
