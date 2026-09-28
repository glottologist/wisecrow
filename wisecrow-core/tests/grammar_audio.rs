//! Voicing a language's example sentences.
//!
//! Only correct examples are spoken, each sentence once however many points
//! quote it, and a prune takes only what no current example needs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use tokio::sync::watch;
use wisecrow::errors::WisecrowError;
use wisecrow::media::cache::MediaCache;
use wisecrow::media::fingerprint::MediaFingerprint;
use wisecrow::media::grammar_audio::{
    correct_examples, prefetch_grammar_audio, GrammarAudioOptions, GrammarAudioSummary,
    SentenceOutcome, SentenceSpeaker,
};
use wisecrow::media::prefetch::PrefetchMode;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query("TRUNCATE languages, grammar_rules, sentence_media CASCADE")
        .execute(&pool)
        .await?;
    Ok(pool)
}

/// Seeds one point with its examples; `examples` are `(sentence, is_correct)`.
/// `grammar_rules` cascades into `rule_examples` (`006_cefr_grammar.sql:31`),
/// so deleting a rule takes its examples with it.
async fn seed_rule(
    pool: &PgPool,
    language: &str,
    level: &str,
    slug: &str,
    examples: &[(&str, bool)],
) -> Result<i32, sqlx::Error> {
    let language_id: i32 = sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ($1, $1)
         ON CONFLICT (code) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .bind(language)
    .fetch_one(pool)
    .await?;
    let rule_id: i32 = sqlx::query_scalar(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         SELECT $1, cl.id, $2, $2, 'Prose.', 'llm' FROM cefr_levels cl WHERE cl.code = $3
         RETURNING id",
    )
    .bind(language_id)
    .bind(slug)
    .bind(level)
    .fetch_one(pool)
    .await?;
    for (sentence, is_correct) in examples {
        sqlx::query(
            "INSERT INTO rule_examples (rule_id, sentence, translation, is_correct)
             VALUES ($1, $2, NULL, $3)",
        )
        .bind(rule_id)
        .bind(sentence)
        .bind(is_correct)
        .execute(pool)
        .await?;
    }
    Ok(rule_id)
}

/// Speaks every sentence as the same bytes and counts the requests.
struct StubSpeaker(AtomicUsize);

#[async_trait]
impl SentenceSpeaker for StubSpeaker {
    fn fingerprint(
        &self,
        language: &str,
        sentence: &str,
    ) -> Result<MediaFingerprint, WisecrowError> {
        Ok(MediaFingerprint::for_audio(
            sentence, language, "stub", "voice", 1,
        ))
    }

    async fn speak(&self, _language: &str, _sentence: &str) -> Result<Vec<u8>, WisecrowError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(b"clip".to_vec())
    }
}

/// A cache in a fresh directory and a counting speaker, shared the way the
/// runner shares them.
fn harness(
    pool: &PgPool,
) -> Result<(tempfile::TempDir, Arc<MediaCache>, Arc<StubSpeaker>), WisecrowError> {
    let dir = tempfile::tempdir()?;
    let cache = Arc::new(MediaCache::with_cache_dir(pool.clone(), dir.path())?); // clone: PgPool is Arc-based
    Ok((dir, cache, Arc::new(StubSpeaker(AtomicUsize::new(0)))))
}

fn options(mode: PrefetchMode, level: Option<&str>, prune: bool) -> GrammarAudioOptions {
    GrammarAudioOptions {
        language: "es".to_owned(),
        level: level.map(str::to_owned),
        mode,
        prune,
    }
}

async fn run(
    pool: &PgPool,
    cache: &Arc<MediaCache>,
    speaker: &Arc<StubSpeaker>,
    options: &GrammarAudioOptions,
) -> Result<GrammarAudioSummary, WisecrowError> {
    let (_stop, cancel) = watch::channel(false);
    let speaker: Arc<dyn SentenceSpeaker> = Arc::<StubSpeaker>::clone(speaker); // clone: the runner shares the speaker with its workers
    prefetch_grammar_audio(pool, Arc::clone(cache), speaker, options, cancel).await
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_correct_examples_of_the_language_and_level_are_selected_in_order() -> TestResult {
    let pool = reset_pool().await?;
    seed_rule(
        &pool,
        "es",
        "B1",
        "ser-y-estar",
        &[("Estoy cansado.", true), ("Soy cansado.", false)],
    )
    .await?;
    seed_rule(&pool, "es", "B1", "articulos", &[("El agua.", true)]).await?;
    seed_rule(&pool, "es", "A1", "hola", &[("Hola.", true)]).await?;
    seed_rule(&pool, "fr", "B1", "etre", &[("Je suis fatigué.", true)]).await?;

    let all = correct_examples(&pool, "es", None).await?;
    assert_eq!(
        all.iter()
            .map(|example| example.sentence.as_str())
            .collect::<Vec<_>>(),
        vec!["Hola.", "El agua.", "Estoy cansado."],
        "level order, then slug order; no incorrect and no other language"
    );
    assert!(all.iter().all(|example| example.language == "es"));

    let b1 = correct_examples(&pool, "es", Some("B1")).await?;
    assert_eq!(b1.len(), 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sentence_quoted_twice_is_spoken_once_and_a_re_import_costs_nothing() -> TestResult {
    let pool = reset_pool().await?;
    seed_rule(
        &pool,
        "es",
        "B1",
        "ser-y-estar",
        &[("Estoy cansado.", true), ("Soy cansado.", false)],
    )
    .await?;
    seed_rule(
        &pool,
        "es",
        "B1",
        "estar-estados",
        &[("Estoy cansado.", true), ("El agua.", true)],
    )
    .await?;
    let (_dir, cache, speaker) = harness(&pool)?;

    let preview = run(
        &pool,
        &cache,
        &speaker,
        &options(PrefetchMode::Preview, None, false),
    )
    .await?;
    assert_eq!(preview.selected, 2, "two distinct sentences");
    assert_eq!(preview.examples, 3, "from three correct example rows");
    assert_eq!(preview.count(SentenceOutcome::Missing), 2);
    assert_eq!(
        speaker.0.load(Ordering::SeqCst),
        0,
        "a preview speaks nothing"
    );

    let voiced = run(
        &pool,
        &cache,
        &speaker,
        &options(PrefetchMode::Execute, None, false),
    )
    .await?;
    assert_eq!(voiced.count(SentenceOutcome::Generated), 2);
    assert_eq!(
        speaker.0.load(Ordering::SeqCst),
        2,
        "one clip per distinct sentence"
    );

    // The point is re-seeded with the same sentences: new example ids, same clips.
    sqlx::query("DELETE FROM grammar_rules")
        .execute(&pool)
        .await?;
    seed_rule(
        &pool,
        "es",
        "B1",
        "ser-y-estar",
        &[("Estoy cansado.", true), ("El agua.", true)],
    )
    .await?;
    let again = run(
        &pool,
        &cache,
        &speaker,
        &options(PrefetchMode::Execute, None, false),
    )
    .await?;
    assert_eq!(again.count(SentenceOutcome::Cached), 2);
    assert_eq!(speaker.0.load(Ordering::SeqCst), 2, "nothing regenerated");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_edited_sentence_is_one_new_clip_and_prune_reclaims_one() -> TestResult {
    let pool = reset_pool().await?;
    let rule = seed_rule(
        &pool,
        "es",
        "B1",
        "ser-y-estar",
        &[("Estoy cansado.", true), ("El agua.", true)],
    )
    .await?;
    let (_dir, cache, speaker) = harness(&pool)?;
    run(
        &pool,
        &cache,
        &speaker,
        &options(PrefetchMode::Execute, None, false),
    )
    .await?;
    assert_eq!(speaker.0.load(Ordering::SeqCst), 2);

    sqlx::query("UPDATE rule_examples SET sentence = 'Estoy cansada.' WHERE rule_id = $1 AND sentence = 'Estoy cansado.'")
        .bind(rule)
        .execute(&pool)
        .await?;
    let pruned = run(
        &pool,
        &cache,
        &speaker,
        &options(PrefetchMode::Execute, None, true),
    )
    .await?;

    assert_eq!(pruned.count(SentenceOutcome::Generated), 1);
    assert_eq!(pruned.count(SentenceOutcome::Cached), 1);
    assert_eq!(pruned.pruned, 1, "the old sentence's clip is reclaimed");
    assert_eq!(speaker.0.load(Ordering::SeqCst), 3);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sentence_media")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 2);
    Ok(())
}
