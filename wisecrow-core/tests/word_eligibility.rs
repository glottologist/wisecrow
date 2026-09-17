//! Single-character forms reach candidate discovery on their own, but reach a
//! learner only once a current, teachable presentation vouches for them.

use sqlx::PgPool;
use wisecrow::presentation::CURRENT_PRESENTATION_VERSION;
use wisecrow::vocabulary::{IncludeCarded, PhraseFilter, VocabularyQuery};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Connects to the isolated test database and clears every table these
/// fixtures touch. `word_glosses` is keyed by language code rather than a
/// foreign key, so truncating `languages` alone would leave stale glosses to
/// collide with the next fixture.
async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query("TRUNCATE translations, languages, word_glosses CASCADE")
        .execute(&pool)
        .await?;
    Ok(pool)
}

async fn seed_pair(pool: &PgPool, native: &str, foreign: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ($1, $1), ($2, $2)
         ON CONFLICT (code) DO NOTHING",
    )
    .bind(native)
    .bind(foreign)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_translation(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    from_phrase: &str,
    to_phrase: &str,
    frequency: i32,
) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO translations
             (from_language_id, to_language_id, from_phrase, to_phrase, corpus_frequency)
         SELECT n.id, f.id, $3, $4, $5
         FROM languages n, languages f
         WHERE n.code = $1 AND f.code = $2
         RETURNING id",
    )
    .bind(native)
    .bind(foreign)
    .bind(from_phrase)
    .bind(to_phrase)
    .bind(frequency)
    .fetch_one(pool)
    .await
}

async fn seed_gloss(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    word: &str,
    teachable: bool,
    version: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO word_glosses
             (lang_code, word, native_lang, translation, display_form, teachable,
              presentation_version)
         VALUES ($1, $2, $3, 'meaning', $2, $4, $5)",
    )
    .bind(foreign)
    .bind(word)
    .bind(native)
    .bind(teachable)
    .bind(version)
    .execute(pool)
    .await?;
    Ok(())
}

async fn selected_ids(
    pool: &PgPool,
    native: &str,
    foreign: &str,
) -> Result<Vec<i32>, wisecrow::errors::WisecrowError> {
    let entries = VocabularyQuery::ranked_candidates(
        pool,
        native,
        foreign,
        10,
        IncludeCarded::Yes,
        PhraseFilter::Exclude,
    )
    .await?;
    Ok(entries.iter().map(|entry| entry.translation_id).collect())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn single_character_learning_requires_current_meaning() -> TestResult {
    let pool = reset_pool().await?;
    seed_pair(&pool, "en", "zh").await?;
    let id = seed_translation(&pool, "en", "zh", "I", "我", 30).await?;
    assert!(selected_ids(&pool, "en", "zh").await?.is_empty());

    seed_gloss(&pool, "en", "zh", "我", true, CURRENT_PRESENTATION_VERSION).await?;
    assert_eq!(selected_ids(&pool, "en", "zh").await?, [id]);

    sqlx::query("UPDATE word_glosses SET teachable = false WHERE lang_code = 'zh'")
        .execute(&pool)
        .await?;
    assert!(selected_ids(&pool, "en", "zh").await?.is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn one_character_admission_cases() -> TestResult {
    let pool = reset_pool().await?;
    seed_pair(&pool, "en", "zh").await?;
    seed_pair(&pool, "en", "fr").await?;

    // A version-1 presentation predates the identity contract and does not admit.
    let stale = seed_translation(&pool, "en", "zh", "you", "你", 40).await?;
    seed_gloss(
        &pool,
        "en",
        "zh",
        "你",
        true,
        CURRENT_PRESENTATION_VERSION - 1,
    )
    .await?;
    // Current but rejected.
    let rejected = seed_translation(&pool, "en", "zh", "of", "的", 35).await?;
    seed_gloss(&pool, "en", "zh", "的", false, CURRENT_PRESENTATION_VERSION).await?;
    // Current and teachable.
    let admitted = seed_translation(&pool, "en", "zh", "I", "我", 30).await?;
    seed_gloss(&pool, "en", "zh", "我", true, CURRENT_PRESENTATION_VERSION).await?;
    // A one-character native side needs the gate as much as a one-character
    // target: either short side is what the old floor excluded.
    let short_native = seed_translation(&pool, "en", "zh", "I", "我们", 20).await?;
    let selected = selected_ids(&pool, "en", "zh").await?;
    assert_eq!(selected, [admitted]);
    assert!(!selected.contains(&stale) && !selected.contains(&rejected));
    seed_gloss(
        &pool,
        "en",
        "zh",
        "我们",
        true,
        CURRENT_PRESENTATION_VERSION,
    )
    .await?;
    assert_eq!(
        selected_ids(&pool, "en", "zh").await?,
        [admitted, short_native]
    );

    // Ordinary French is unchanged by the gate.
    let french = seed_translation(&pool, "en", "fr", "dog", "chien", 50).await?;
    assert_eq!(selected_ids(&pool, "en", "fr").await?, [french]);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ranked_index_admits_single_characters() -> TestResult {
    let pool = reset_pool().await?;
    let definition: String = sqlx::query_scalar(
        "SELECT pg_get_indexdef(indexrelid) FROM pg_index
         WHERE indexrelid = 'idx_translations_ranked_to_word_v2'::regclass",
    )
    .fetch_one(&pool)
    .await?;
    assert!(
        definition.contains("length(from_phrase) >= 1"),
        "{definition}"
    );
    assert!(
        definition.contains("length(to_phrase) >= 1"),
        "{definition}"
    );
    assert!(
        definition.contains("INCLUDE (to_phrase, from_phrase)"),
        "{definition}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn one_character_is_an_enrichment_candidate() -> TestResult {
    let pool = reset_pool().await?;
    seed_pair(&pool, "en", "zh").await?;
    seed_translation(&pool, "en", "zh", "I", "我", 30).await?;
    let candidates = wisecrow::glossing::pending_presentations(&pool, "en", "zh", 10).await?;
    assert!(candidates.iter().any(|candidate| candidate.word == "我"));
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn pending_pages_report_rejections_and_continue() -> TestResult {
    let pool = reset_pool().await?;
    seed_pair(&pool, "en", "zh").await?;
    // A punctuation-only token outranks the real word; it is scanned and
    // rejected rather than sent to the model or looped over forever.
    seed_translation(&pool, "en", "zh", "...", "……", 90).await?;
    seed_translation(&pool, "en", "zh", "I", "我", 30).await?;

    let first = wisecrow::glossing::pending_page(&pool, "en", "zh", 1, 0).await?;
    assert_eq!(first.scanned, 1);
    assert!(first.candidates.is_empty());
    assert_eq!(first.next_offset, Some(1));

    let second = wisecrow::glossing::pending_page(&pool, "en", "zh", 1, 1).await?;
    assert_eq!(second.scanned, 1);
    assert_eq!(
        second
            .candidates
            .iter()
            .map(|candidate| candidate.word.as_str())
            .collect::<Vec<_>>(),
        ["我"]
    );
    assert_eq!(second.next_offset, Some(2));

    let exhausted = wisecrow::glossing::pending_page(&pool, "en", "zh", 10, 0).await?;
    assert_eq!(exhausted.scanned, 2);
    assert_eq!(exhausted.next_offset, None);

    assert!(wisecrow::glossing::pending_page(&pool, "en", "zh", 0, 0)
        .await
        .is_err());
    assert!(wisecrow::glossing::pending_page(&pool, "en", "zh", 1001, 0)
        .await
        .is_err());
    assert!(
        wisecrow::glossing::pending_page(&pool, "en", "zh", 1000, 9001)
            .await
            .is_err()
    );
    Ok(())
}
