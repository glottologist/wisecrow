use sqlx::SqlitePool;
use tempfile::tempdir;
use wisecrow_mobile::storage::SqliteStore;

const TABLES: &[&str] = &[
    "_sqlx_migrations",
    "cached_quizzes",
    "cards",
    "language_pairs",
    "languages",
    "learn_session_cards",
    "learn_sessions",
    "media_cache",
    "nback_outbox",
    "profile_users",
    "profiles",
    "review_outbox",
    "sync_state",
    "translations",
];

async fn table_names(pool: &SqlitePool) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("table names")
}

async fn assert_pragmas_and_index(pool: &SqlitePool) {
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(pool)
        .await
        .expect("journal mode");
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await
        .expect("foreign keys");
    let index_sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master
         WHERE type = 'index' AND name = 'one_active_profile'",
    )
    .fetch_one(pool)
    .await
    .expect("active profile index");
    assert_eq!(journal_mode, "wal");
    assert_eq!(foreign_keys, 1);
    assert!(index_sql.contains("WHERE active = 1"));
}

#[tokio::test]
async fn sqlite_migrations_are_idempotent_and_enforce_connection_pragmas() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("mobile.sqlite3");
    let first = SqliteStore::open(&path).await.expect("first migration");
    drop(first);

    let second = SqliteStore::open(&path).await.expect("second migration");
    assert_eq!(table_names(second.pool()).await, TABLES);
    assert_pragmas_and_index(second.pool()).await;
}
