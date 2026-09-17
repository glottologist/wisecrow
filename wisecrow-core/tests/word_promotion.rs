//! Sentence-derived word promotion: schema, bounded extraction, stable-ID
//! promotion and evidence exclusion, all against the isolated database.

use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Connects to the isolated test database with a clean slate. `word_glosses`
/// is keyed by language code rather than a foreign key and would otherwise
/// carry a stale presentation across fixtures.
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

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn word_schema_is_installed() -> TestResult {
    let pool = reset_pool().await?;
    for name in [
        "word_candidates",
        "word_candidate_examples",
        "word_promotions",
        "corpus_evidence_translations",
    ] {
        let present: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(name)
            .fetch_one(&pool)
            .await?;
        assert!(present, "missing {name}");
    }
    let guarded: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM pg_trigger
             WHERE tgname = 'protect_owned_word_promotion' AND NOT tgisinternal
         )",
    )
    .fetch_one(&pool)
    .await?;
    assert!(guarded, "owned-link guard trigger missing");
    Ok(())
}

/// Five Gaelic sentences sharing the word `taigh`; every other word appears in
/// fewer than the five sources the fixture extraction threshold demands.
async fn seed_sentences(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO languages (code, name) VALUES ('en', 'English'), ('gd', 'Gaelic')")
        .execute(pool)
        .await?;
    for (native, foreign) in [
        ("the big house", "an taigh mòr"),
        ("the small house", "an taigh beag"),
        ("a warm house", "taigh blàth"),
        ("a cold house", "taigh fuar"),
        ("my house", "mo thaigh taigh"),
    ] {
        sqlx::query(
            "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
             SELECT n.id, f.id, $1, $2 FROM languages n, languages f
             WHERE n.code = 'en' AND f.code = 'gd'",
        )
        .bind(native)
        .bind(foreign)
        .execute(pool)
        .await?;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn extraction_replaces_counts_and_caps_examples() -> TestResult {
    let pool = reset_pool().await?;
    seed_sentences(&pool).await?;
    let options = wisecrow::words::ExtractionOptions::new(500, 5)?;
    for _ in 0..2 {
        let summary = wisecrow::words::extract_words(&pool, "en", "gd", &options).await?;
        assert_eq!(summary.scanned_rows, 5);
        assert_eq!(summary.candidates, 1);
    }
    let counts: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT c.occurrence_count, count(e.ordinal)
         FROM word_candidates c
         JOIN word_candidate_examples e ON e.candidate_id = c.id
         WHERE c.word = 'taigh'
         GROUP BY c.id, c.occurrence_count",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(counts, [(5, 3)]);
    let revision: i64 =
        sqlx::query_scalar("SELECT revision FROM word_candidates WHERE word = 'taigh'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(revision, 2, "a rescan bumps the revision");
    Ok(())
}

/// Answers only when the prompt carries the candidate's source context, and
/// counts how often it is asked.
struct HouseProvider {
    calls: std::sync::atomic::AtomicUsize,
}

impl HouseProvider {
    fn new() -> Self {
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl wisecrow::llm::LlmProvider for HouseProvider {
    async fn generate(
        &self,
        prompt: &str,
        _: u32,
    ) -> Result<String, wisecrow::errors::WisecrowError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if !prompt.contains("an taigh mòr") {
            return Err(wisecrow::errors::WisecrowError::LlmError(
                "Missing source context".into(),
            ));
        }
        Ok(r#"{"presentations":[{"word":"taigh","display_form":"taigh","translation":"house","teachable":true,"image_query":"house"}]}"#.into())
    }

    fn name(&self) -> &str {
        "fixture"
    }
}

async fn promoted_state(
    pool: &PgPool,
    word: &str,
) -> Result<(Option<i32>, Option<bool>, i64, String), sqlx::Error> {
    sqlx::query_as(
        "SELECT p.translation_id, p.owns_translation, c.occurrence_count, c.status
         FROM word_candidates c
         LEFT JOIN word_promotions p ON p.candidate_id = c.id
         WHERE c.word = $1",
    )
    .bind(word)
    .fetch_one(pool)
    .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn sentence_word_promotes_once_without_feedback() -> TestResult {
    let pool = reset_pool().await?;
    seed_sentences(&pool).await?;
    let extraction = wisecrow::words::ExtractionOptions::new(500, 5)?;
    wisecrow::words::extract_words(&pool, "en", "gd", &extraction).await?;
    let promotion = wisecrow::words::PromotionOptions::new(100, wisecrow::words::Refresh::Pending)?;
    let provider = HouseProvider::new();
    let result = wisecrow::words::promote_words(&pool, &provider, "en", "gd", &promotion).await?;
    assert_eq!(result.attempted, 1);
    assert_eq!(result.accepted, 1);
    assert_eq!(provider.calls(), 1);
    let (id, owns, _, status) = promoted_state(&pool, "taigh").await?;
    let id = id.ok_or("missing link")?;
    assert_eq!(
        owns,
        Some(true),
        "no corpus row spelled `taigh` existed, so the row is generated"
    );
    assert_eq!(status, "accepted");

    wisecrow::words::extract_words(&pool, "en", "gd", &extraction).await?;
    let repeated = wisecrow::words::promote_words(&pool, &provider, "en", "gd", &promotion).await?;
    assert_eq!(repeated.attempted, 0);
    assert_eq!(provider.calls(), 1, "a repeated pending run asks nothing");
    let (again, _, count, _) = promoted_state(&pool, "taigh").await?;
    assert_eq!(
        (again, count),
        (Some(id), 5),
        "generated row is not re-counted"
    );

    let eligible = wisecrow::vocabulary::VocabularyQuery::unlearned(&pool, "en", "gd", 10).await?;
    assert!(
        eligible
            .iter()
            .any(|entry| entry.translation_id == id && entry.from_phrase == "house"),
        "{eligible:?}"
    );
    Ok(())
}

/// Returns a fixed response regardless of prompt.
struct FixedProvider(&'static str);

#[async_trait::async_trait]
impl wisecrow::llm::LlmProvider for FixedProvider {
    async fn generate(&self, _: &str, _: u32) -> Result<String, wisecrow::errors::WisecrowError> {
        Ok(self.0.to_owned())
    }

    fn name(&self) -> &str {
        "fixed"
    }
}

/// Extracts `taigh` and returns the pool ready for a promotion scenario.
async fn extracted_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let pool = reset_pool().await?;
    seed_sentences(&pool).await?;
    let extraction = wisecrow::words::ExtractionOptions::new(500, 5)?;
    wisecrow::words::extract_words(&pool, "en", "gd", &extraction).await?;
    Ok(pool)
}

fn pending() -> Result<wisecrow::words::PromotionOptions, wisecrow::errors::WisecrowError> {
    wisecrow::words::PromotionOptions::new(100, wisecrow::words::Refresh::Pending)
}

async fn gloss_of(pool: &PgPool, word: &str) -> Result<Option<(String, bool)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT translation, teachable FROM word_glosses
         WHERE lang_code = 'gd' AND native_lang = 'en' AND word = $1",
    )
    .bind(word)
    .fetch_optional(pool)
    .await
}

#[rstest::rstest]
#[case::omitted(r#"{"presentations":[]}"#, (0, 0, 1), "failed")]
#[case::renamed(
    r#"{"presentations":[{"word":"taigh","display_form":"tigh","translation":"house","teachable":true,"image_query":null}]}"#,
    (0, 0, 1),
    "failed"
)]
#[case::empty_translation(
    r#"{"presentations":[{"word":"taigh","display_form":"taigh","translation":"   ","teachable":true,"image_query":null}]}"#,
    (0, 0, 1),
    "failed"
)]
#[case::unteachable(
    r#"{"presentations":[{"word":"taigh","display_form":"taigh","translation":"house","teachable":false,"image_query":null}]}"#,
    (0, 1, 0),
    "rejected"
)]
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn provider_outcomes_are_explicit(
    #[case] response: &'static str,
    #[case] expected: (u32, u32, u32),
    #[case] status: &str,
) -> TestResult {
    let pool = extracted_pool().await?;
    let summary =
        wisecrow::words::promote_words(&pool, &FixedProvider(response), "en", "gd", &pending()?)
            .await?;
    assert_eq!(
        (summary.accepted, summary.rejected, summary.failed),
        expected
    );
    assert_eq!(summary.attempted, 1);
    let (link, _, _, stored) = promoted_state(&pool, "taigh").await?;
    assert_eq!(stored, status);
    assert_eq!(link, None, "no translation is invented for {status}");
    let generated: i64 =
        sqlx::query_scalar("SELECT count(*) FROM translations WHERE to_phrase = 'taigh'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(generated, 0);
    if status == "rejected" {
        assert_eq!(
            gloss_of(&pool, "taigh").await?,
            Some(("house".into(), false))
        );
    } else {
        assert_eq!(gloss_of(&pool, "taigh").await?, None);
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn malformed_response_fails_the_batch_and_the_run() -> TestResult {
    let pool = extracted_pool().await?;
    let result =
        wisecrow::words::promote_words(&pool, &FixedProvider("not json"), "en", "gd", &pending()?)
            .await;
    assert!(result.is_err());
    let (_, _, _, status) = promoted_state(&pool, "taigh").await?;
    assert_eq!(status, "failed");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn retry_and_refresh_keep_the_same_ids() -> TestResult {
    let pool = extracted_pool().await?;
    wisecrow::words::promote_words(
        &pool,
        &FixedProvider(r#"{"presentations":[]}"#),
        "en",
        "gd",
        &pending()?,
    )
    .await?;

    // A pending run skips the failed candidate; retry-failed attempts it.
    let skipped =
        wisecrow::words::promote_words(&pool, &HouseProvider::new(), "en", "gd", &pending()?)
            .await?;
    assert_eq!(skipped.attempted, 0);
    let retry = wisecrow::words::PromotionOptions::new(100, wisecrow::words::Refresh::RetryFailed)?;
    let retried =
        wisecrow::words::promote_words(&pool, &HouseProvider::new(), "en", "gd", &retry).await?;
    assert_eq!((retried.attempted, retried.accepted), (1, 1));
    let (link, owns, _, _) = promoted_state(&pool, "taigh").await?;
    let id = link.ok_or("missing link")?;
    assert_eq!(owns, Some(true));

    // A learner reviews the card; an explicit refresh changes the meaning
    // but neither the translation ID nor the card.
    sqlx::query(
        "INSERT INTO cards (translation_id, user_id, state, reps, stability)
         VALUES ($1, 1, 2, 4, 12.5)",
    )
    .bind(id)
    .execute(&pool)
    .await?;
    let all = wisecrow::words::PromotionOptions::new(100, wisecrow::words::Refresh::All)?;
    let refreshed = wisecrow::words::promote_words(
        &pool,
        &FixedProvider(
            r#"{"presentations":[{"word":"taigh","display_form":"Taigh","translation":"home","teachable":true,"image_query":null}]}"#,
        ),
        "en",
        "gd",
        &all,
    )
    .await?;
    assert_eq!((refreshed.attempted, refreshed.accepted), (1, 1));
    let (again, _, _, _) = promoted_state(&pool, "taigh").await?;
    assert_eq!(again, Some(id));
    let row: (String, i32, i32) = sqlx::query_as(
        "SELECT from_phrase, frequency, corpus_frequency FROM translations WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(row, ("home".into(), 5, 5));
    let card: (i32, f32) = sqlx::query_as(
        "SELECT reps, stability FROM cards WHERE translation_id = $1 AND user_id = 1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(card, (4, 12.5));
    assert_eq!(gloss_of(&pool, "taigh").await?, Some(("home".into(), true)));
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM translations WHERE to_phrase = 'taigh'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(rows, 1, "refresh never merges or duplicates");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn existing_corpus_row_is_reused_and_still_counted() -> TestResult {
    let pool = reset_pool().await?;
    seed_sentences(&pool).await?;
    let corpus_row: i32 = sqlx::query_scalar(
        "INSERT INTO translations
             (from_language_id, to_language_id, from_phrase, to_phrase, frequency)
         SELECT n.id, f.id, 'House!', 'Taigh.', 3 FROM languages n, languages f
         WHERE n.code = 'en' AND f.code = 'gd'
         RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let extraction = wisecrow::words::ExtractionOptions::new(500, 5)?;
    wisecrow::words::extract_words(&pool, "en", "gd", &extraction).await?;
    let summary =
        wisecrow::words::promote_words(&pool, &HouseProvider::new(), "en", "gd", &pending()?)
            .await?;
    assert_eq!(summary.accepted, 1);
    let (link, owns, _, _) = promoted_state(&pool, "taigh").await?;
    assert_eq!((link, owns), (Some(corpus_row), Some(false)));
    let raw: (String, String, i32) =
        sqlx::query_as("SELECT from_phrase, to_phrase, frequency FROM translations WHERE id = $1")
            .bind(corpus_row)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        raw,
        ("House!".into(), "Taigh.".into(), 3),
        "raw row untouched"
    );
    let evidence: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM corpus_evidence_translations WHERE id = $1)",
    )
    .bind(corpus_row)
    .fetch_one(&pool)
    .await?;
    assert!(evidence, "a reused corpus row remains evidence");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn owned_link_survives_until_the_translation_is_gone() -> TestResult {
    let pool = extracted_pool().await?;
    let provider = HouseProvider::new();
    wisecrow::words::promote_words(&pool, &provider, "en", "gd", &pending()?).await?;
    let (link, _, _, _) = promoted_state(&pool, "taigh").await?;
    let id = link.ok_or("missing link")?;
    let candidate_id: i64 =
        sqlx::query_scalar("SELECT id FROM word_candidates WHERE word = 'taigh'")
            .fetch_one(&pool)
            .await?;

    for statement in [
        "DELETE FROM word_promotions WHERE candidate_id = $1",
        "UPDATE word_promotions SET owns_translation = false WHERE candidate_id = $1",
        "UPDATE word_promotions SET translation_id = NULL WHERE candidate_id = $1",
        "DELETE FROM word_candidates WHERE id = $1",
    ] {
        let result = sqlx::query(statement)
            .bind(candidate_id)
            .execute(&pool)
            .await;
        assert!(
            result.is_err(),
            "{statement} must be refused while the owned row exists"
        );
    }
    let excluded: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM corpus_evidence_translations WHERE id = $1)",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert!(!excluded, "generated row is not evidence");

    // Deleting the translation itself is permitted and clears the link; the
    // accepted candidate is then repaired without asking the model again.
    sqlx::query("DELETE FROM translations WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await?;
    let (cleared, _, _, status) = promoted_state(&pool, "taigh").await?;
    assert_eq!((cleared, status.as_str()), (None, "accepted"));
    let repaired =
        wisecrow::words::promote_words(&pool, &provider, "en", "gd", &pending()?).await?;
    assert_eq!((repaired.attempted, repaired.accepted), (1, 1));
    assert_eq!(
        provider.calls(),
        1,
        "current presentation reused for repair"
    );
    let (relinked, owns, _, _) = promoted_state(&pool, "taigh").await?;
    assert!(relinked.is_some() && owns == Some(true));
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn concurrent_promotion_yields_one_link() -> TestResult {
    let pool = extracted_pool().await?;
    let (options, left, right) = (pending()?, HouseProvider::new(), HouseProvider::new());
    let (first, second) = tokio::join!(
        wisecrow::words::promote_words(&pool, &left, "en", "gd", &options),
        wisecrow::words::promote_words(&pool, &right, "en", "gd", &options),
    );
    let (first, second) = (first?, second?);
    assert_eq!(first.accepted + second.accepted, 2);
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM translations WHERE to_phrase = 'taigh'")
            .fetch_one(&pool)
            .await?;
    let links: i64 = sqlx::query_scalar("SELECT count(*) FROM word_promotions")
        .fetch_one(&pool)
        .await?;
    assert_eq!((rows, links), (1, 1));
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn storage_failure_rolls_back_only_that_candidate() -> TestResult {
    let pool = reset_pool().await?;
    seed_sentences(&pool).await?;
    // Threshold two admits `an` (two sources) beside `taigh`.
    let extraction = wisecrow::words::ExtractionOptions::new(500, 2)?;
    wisecrow::words::extract_words(&pool, "en", "gd", &extraction).await?;
    // The injected constraint fails `an` at the final status write, after its
    // presentation and link were written in the same transaction.
    sqlx::query(
        "ALTER TABLE word_candidates ADD CONSTRAINT fixture_no_an
         CHECK (word <> 'an' OR status <> 'accepted')",
    )
    .execute(&pool)
    .await?;
    let provider = FixedProvider(
        r#"{"presentations":[
            {"word":"taigh","display_form":"taigh","translation":"house","teachable":true,"image_query":null},
            {"word":"an","display_form":"an","translation":"the","teachable":true,"image_query":null}]}"#,
    );
    let result = wisecrow::words::promote_words(&pool, &provider, "en", "gd", &pending()?).await;
    sqlx::query("ALTER TABLE word_candidates DROP CONSTRAINT fixture_no_an")
        .execute(&pool)
        .await?;
    assert!(result.is_err(), "a storage failure reaches the caller");
    let (taigh_link, _, _, taigh_status) = promoted_state(&pool, "taigh").await?;
    assert!(
        taigh_link.is_some() && taigh_status == "accepted",
        "earlier success kept"
    );
    let (an_link, _, _, an_status) = promoted_state(&pool, "an").await?;
    assert_eq!((an_link, an_status.as_str()), (None, "pending"));
    assert_eq!(
        gloss_of(&pool, "an").await?,
        None,
        "presentation rolled back"
    );
    let generated: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM translations WHERE to_phrase = 'an' AND from_phrase = 'the'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(generated, 0, "generated row rolled back");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn corpus_counts_ignore_generated_word() -> TestResult {
    let pool = extracted_pool().await?;
    let before = wisecrow::frequency::FrequencyUpdater::derive_counts(&pool, "gd").await?;
    wisecrow::words::promote_words(&pool, &HouseProvider::new(), "en", "gd", &pending()?).await?;
    let after = wisecrow::frequency::FrequencyUpdater::derive_counts(&pool, "gd").await?;
    assert_eq!(before.get("taigh"), Some(&5));
    assert_eq!(before.get("taigh"), after.get("taigh"));

    // Ranking from the corpus leaves the generated row's own rank alone.
    wisecrow::frequency::FrequencyUpdater::update_from_corpus(&pool, "gd").await?;
    let (link, _, _, _) = promoted_state(&pool, "taigh").await?;
    let rank: (i32, Option<i32>) =
        sqlx::query_as("SELECT frequency, corpus_frequency FROM translations WHERE id = $1")
            .bind(link.ok_or("missing link")?)
            .fetch_one(&pool)
            .await?;
    assert_eq!(rank, (5, Some(5)));
    Ok(())
}
