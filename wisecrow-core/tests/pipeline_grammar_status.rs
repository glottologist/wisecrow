//! Import idleness follows the shelf schedule, and does not call a model.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use sqlx::PgPool;
use wisecrow::errors::WisecrowError;
use wisecrow::grammar::pdf_incremental::{self, IMPORTER_VERSION};
use wisecrow::llm::LlmProvider;
use wisecrow::media::cache::MediaCache;
use wisecrow::pipeline::{grammar_audio_idle, grammar_import_idle};

struct UnusedModel {
    calls: AtomicUsize,
}

#[async_trait]
impl LlmProvider for UnusedModel {
    async fn generate(&self, _prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(WisecrowError::InvalidInput(
            "grammar status must not call a model".to_owned(),
        ))
    }

    fn name(&self) -> &str {
        "unused"
    }
}

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

fn shelf_with(levels: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("shelf");
    let document = root.path().join("cy").join("pipeline-status.pdf");
    std::fs::create_dir_all(document.parent().expect("language dir")).expect("language dir");
    std::fs::write(&document, b"%PDF-1.4\npipeline-status\n").expect("pdf");
    let record = format!(
        "| File | Work | Year | Licence | Synthesis | Levels |\n\
         |------|------|-----:|---------|-----------|--------|\n\
         | `cy/pipeline-status.pdf` | Pipeline status fixture | 2026 | public domain | yes | {levels} |\n"
    );
    std::fs::write(root.path().join("SOURCES.md"), record).expect("sources");
    root
}

async fn language_id(pool: &PgPool) -> i32 {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('cy', 'Welsh') ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("language");
    sqlx::query_scalar("SELECT id FROM languages WHERE code = 'cy'")
        .fetch_one(pool)
        .await
        .expect("language id")
}

async fn level_id(pool: &PgPool, code: &str) -> i32 {
    sqlx::query_scalar("SELECT id FROM cefr_levels WHERE code = $1")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|_| panic!("cefr level {code}"))
}

async fn insert_progress(
    pool: &PgPool,
    language_id: i32,
    level_id: i32,
    hash: &[u8],
    completed: bool,
) {
    sqlx::query(
        "INSERT INTO grammar_import_progress
            (language_id, document_sha256, cefr_level_id, importer_version, document_name, next_chunk, completed)
         VALUES ($1, $2, $3, $4, 'pipeline-status.pdf', 0, $5)
         ON CONFLICT (language_id, document_sha256, cefr_level_id, importer_version)
         DO UPDATE SET completed = EXCLUDED.completed",
    )
    .bind(language_id)
    .bind(hash)
    .bind(level_id)
    .bind(IMPORTER_VERSION)
    .bind(completed)
    .execute(pool)
    .await
    .expect("progress");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_stale_hash_is_unfinished_and_a_new_level_is_unfinished() {
    let model = UnusedModel {
        calls: AtomicUsize::new(0),
    };
    let pool = test_pool().await;
    let language_id = language_id(&pool).await;
    sqlx::query(
        "DELETE FROM grammar_import_progress
          WHERE language_id = $1 AND document_name = 'pipeline-status.pdf'",
    )
    .bind(language_id)
    .execute(&pool)
    .await
    .expect("clear progress");

    let shelf = shelf_with("A1");
    let document = shelf.path().join("cy").join("pipeline-status.pdf");
    let hash = pdf_incremental::fingerprint(&document).expect("hash");
    let a1 = level_id(&pool, "A1").await;
    insert_progress(&pool, language_id, a1, &[9u8; 32], true).await;

    let dir = tempfile::tempdir().expect("cache");
    let cache = MediaCache::with_cache_dir(pool.clone(), dir.path()).expect("cache");

    let stale = grammar_import_idle(&pool, "cy", shelf.path())
        .await
        .expect("stale hash");
    assert!(
        !stale,
        "a progress row for a different file hash is unfinished"
    );
    let audio = grammar_audio_idle(&pool, "cy", &cache, None)
        .await
        .expect("audio");
    assert!(audio, "import work is not example-audio work");

    insert_progress(&pool, language_id, a1, &hash, true).await;
    let finished = grammar_import_idle(&pool, "cy", shelf.path())
        .await
        .expect("matching hash");
    assert!(finished, "a completed row for this file is idle");

    let widened = shelf_with("A1–A2");
    std::fs::copy(
        &document,
        widened.path().join("cy").join("pipeline-status.pdf"),
    )
    .expect("same bytes");
    let widened_idle = grammar_import_idle(&pool, "cy", widened.path())
        .await
        .expect("added level");
    assert!(!widened_idle, "a level with no progress row is unfinished");
    assert_eq!(
        model.calls.load(Ordering::SeqCst),
        0,
        "status does not call the model"
    );
}
