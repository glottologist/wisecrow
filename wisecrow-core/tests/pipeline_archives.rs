use sqlx::PgPool;
use wisecrow::ingesting::parsing::TranslationPair;
use wisecrow::ingesting::persisting::DatabasePersister;
use wisecrow::pipeline::archives::ArchiveLedger;

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
async fn unavailable_and_ingested_archives_are_not_downloaded_again() {
    let pool = test_pool().await;
    sqlx::query("DELETE FROM pipeline_archives WHERE archive_url LIKE 'https://example.test/%'")
        .execute(&pool)
        .await
        .expect("clear archive fixtures");
    let ledger = ArchiveLedger::new(&pool);
    let native = "en";
    let foreign = "es";
    let missing = "https://example.test/open-subtitles-es-missing";
    let gone = "https://example.test/cc-aligned-es-404";
    let done = "https://example.test/paracrawl-es-done";

    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('es', 'Spanish')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(&pool)
    .await
    .expect("languages");
    sqlx::query(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, 'archive-marker-probe', 'sonda'
         FROM languages fl, languages tl
         WHERE fl.code = 'en' AND tl.code = 'es'
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
         DO NOTHING",
    )
    .execute(&pool)
    .await
    .expect("existing rows do not imply an archive");

    assert!(ledger
        .needs_download(native, foreign, missing)
        .await
        .expect("probe"));

    ledger
        .mark_unavailable(native, foreign, "cc_aligned", gone)
        .await
        .expect("404");
    assert!(ledger
        .is_settled(native, foreign, gone)
        .await
        .expect("settled"));
    assert!(!ledger
        .needs_download(native, foreign, gone)
        .await
        .expect("404 skip"));

    ledger
        .mark_ingested(native, foreign, "paracrawl", done)
        .await
        .expect("ingested");
    ledger
        .mark_ingested(native, foreign, "paracrawl", done)
        .await
        .expect("second mark");
    assert!(!ledger
        .needs_download(native, foreign, done)
        .await
        .expect("done"));
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pipeline_archives
         WHERE native_code = $1 AND foreign_code = $2 AND archive_url = $3",
    )
    .bind(native)
    .bind(foreign)
    .bind(done)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn replaying_an_archive_does_not_increment_frequency() {
    let pool = test_pool().await;
    let persister = DatabasePersister::new(pool.clone());
    let from_id = persister
        .ensure_language("en", "English")
        .await
        .expect("english");
    let to_id = persister
        .ensure_language("es", "Spanish")
        .await
        .expect("spanish");

    sqlx::query(
        "DELETE FROM translations
         WHERE from_phrase IN ('pipeline-once-kept', 'pipeline-once-fresh')",
    )
    .execute(&pool)
    .await
    .expect("clear pairs");
    sqlx::query(
        "INSERT INTO translations (from_language_id, from_phrase, to_language_id, to_phrase, frequency)
         VALUES ($1, 'pipeline-once-kept', $2, 'sonda-once', 3)",
    )
    .bind(from_id)
    .bind(to_id)
    .execute(&pool)
    .await
    .expect("seed");

    let kept = TranslationPair {
        source_text: "pipeline-once-kept".to_owned(),
        target_text: "sonda-once".to_owned(),
    };
    persister
        .persist_translations_once(&[kept], from_id, to_id)
        .await
        .expect("replay");
    let (rows, frequency): (i64, i32) = sqlx::query_as(
        "SELECT COUNT(*), MAX(frequency) FROM translations
         WHERE from_language_id = $1 AND to_language_id = $2 AND from_phrase = 'pipeline-once-kept'",
    )
    .bind(from_id)
    .bind(to_id)
    .fetch_one(&pool)
    .await
    .expect("kept row");
    assert_eq!(rows, 1);
    assert_eq!(frequency, 3);

    let fresh = TranslationPair {
        source_text: "pipeline-once-fresh".to_owned(),
        target_text: "nueva".to_owned(),
    };
    persister
        .persist_translations_once(&[fresh], from_id, to_id)
        .await
        .expect("new pair");
    let inserted: i32 = sqlx::query_scalar(
        "SELECT frequency FROM translations
         WHERE from_language_id = $1 AND to_language_id = $2 AND from_phrase = 'pipeline-once-fresh'",
    )
    .bind(from_id)
    .bind(to_id)
    .fetch_one(&pool)
    .await
    .expect("fresh row");
    assert_eq!(inserted, 1);
}
