//! Corpus-unit phrase mining, glossing, and speech.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use sqlx::PgPool;
use wisecrow::errors::WisecrowError;
use wisecrow::llm::LlmProvider;
use wisecrow::media::cache::MediaCache;
use wisecrow::media::{load_media_subject, MediaType};
use wisecrow::phrases::link_phrase_translation;
use wisecrow::pipeline::evidence_checksum;
use wisecrow::pipeline::{
    archives_settled, commit_mine_page, commit_mine_page_at, finish_or_abandon, gloss_pair,
    leading_work, mine_pair, next_phrase_audio_batch, Leading, PairSpec, ProbeProfile,
};

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

async fn seed_languages(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('cy', 'Welsh')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn owned_phrase_rows_leave_the_evidence_view_and_reused_rows_stay() {
    let pool = test_pool().await;
    seed_languages(&pool).await;
    sqlx::query(
        "DELETE FROM phrase_translations
          WHERE phrase_id IN (
              SELECT id FROM phrases WHERE phrase IN ('pipeline phrase keep', 'pipeline phrase drop')
          )",
    )
    .execute(&pool)
    .await
    .expect("clear links");
    sqlx::query(
        "DELETE FROM phrases WHERE phrase IN ('pipeline phrase keep', 'pipeline phrase drop')",
    )
    .execute(&pool)
    .await
    .expect("clear phrases");
    sqlx::query(
        "DELETE FROM translations
          WHERE to_phrase IN ('pipeline-phrase-keep', 'pipeline-phrase-drop')",
    )
    .execute(&pool)
    .await
    .expect("clear translations");

    sqlx::query(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, seed.native, seed.target
         FROM languages fl
         JOIN languages tl ON tl.code = 'cy'
         JOIN (VALUES ('keep the line', 'pipeline-phrase-keep'),
                      ('drop the line', 'pipeline-phrase-drop')) AS seed(native, target) ON true
         WHERE fl.code = 'en'",
    )
    .execute(&pool)
    .await
    .expect("seed translations");

    let columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns
          WHERE table_name = 'pipeline_phrase_mines'
            AND column_name IN ('surface_cursor', 'generation')",
    )
    .fetch_one(&pool)
    .await
    .expect("mine columns");
    assert_eq!(columns, 2, "a mine records its cursor and generation");

    sqlx::query(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, phrase, 3, 8 FROM languages
         JOIN (VALUES ('pipeline phrase keep'), ('pipeline phrase drop')) AS seed(phrase) ON true
         WHERE code = 'cy'",
    )
    .execute(&pool)
    .await
    .expect("seed phrases");

    sqlx::query(
        "INSERT INTO phrase_translations
             (phrase_id, native_language_id, translation, translation_id, owns_translation)
         SELECT p.id, native.id, t.from_phrase, t.id, seed.owned
         FROM phrases p
         JOIN (VALUES ('pipeline phrase keep', 'pipeline-phrase-keep', false),
                      ('pipeline phrase drop', 'pipeline-phrase-drop', true))
              AS seed(phrase, target, owned) ON p.phrase = seed.phrase
         JOIN translations t ON t.to_phrase = seed.target
         JOIN languages native ON native.code = 'en'
         WHERE p.language_id = (SELECT id FROM languages WHERE code = 'cy')",
    )
    .execute(&pool)
    .await
    .expect("seed links");

    let in_view: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM corpus_evidence_translations
          WHERE to_phrase IN ('pipeline-phrase-keep', 'pipeline-phrase-drop')",
    )
    .fetch_one(&pool)
    .await
    .expect("evidence count");
    assert_eq!(in_view, 1, "only the reused corpus line stays in the view");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn corpus_turn_rejects_an_unknown_kind() {
    let pool = test_pool().await;
    let error = sqlx::query("UPDATE pipeline_corpus_turn SET kind = 'nope'")
        .execute(&pool)
        .await
        .expect_err("kind check");
    let code = error
        .as_database_error()
        .and_then(|db| db.code().map(|code| code.into_owned()));
    assert_eq!(code.as_deref(), Some("23514"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn promoting_a_phrase_that_matches_a_corpus_line_keeps_the_checksum() {
    let pool = test_pool().await;
    seed_languages(&pool).await;
    sqlx::query(
        "DELETE FROM phrase_translations
          WHERE phrase_id IN (
              SELECT id FROM phrases
              WHERE phrase IN ('pipeline phrase line', 'pipeline phrase fresh')
          )",
    )
    .execute(&pool)
    .await
    .expect("clear link");
    sqlx::query(
        "DELETE FROM phrases WHERE phrase IN ('pipeline phrase line', 'pipeline phrase fresh')",
    )
    .execute(&pool)
    .await
    .expect("clear phrase");
    sqlx::query(
        "DELETE FROM translations
          WHERE to_phrase IN ('pipeline phrase line', 'pipeline phrase fresh')",
    )
    .execute(&pool)
    .await
    .expect("clear translation");

    sqlx::query(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, 'I am here', 'pipeline phrase line'
         FROM languages fl, languages tl
         WHERE fl.code = 'en' AND tl.code = 'cy'",
    )
    .execute(&pool)
    .await
    .expect("seed corpus line");
    let (original_id,): (i32,) =
        sqlx::query_as("SELECT id FROM translations WHERE to_phrase = 'pipeline phrase line'")
            .fetch_one(&pool)
            .await
            .expect("corpus id");
    let before = evidence_checksum(&pool, "cy").await.expect("before");

    let (phrase_id,): (i32,) = sqlx::query_as(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, 'pipeline phrase line', 3, 8 FROM languages WHERE code = 'cy'
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("phrase");
    let (link_id,): (i32,) = sqlx::query_as(
        "INSERT INTO phrase_translations (phrase_id, native_language_id, translation)
         SELECT $1, id, 'I am here' FROM languages WHERE code = 'en'
         RETURNING id",
    )
    .bind(phrase_id)
    .fetch_one(&pool)
    .await
    .expect("gloss row");

    link_phrase_translation(
        &pool,
        link_id,
        "pipeline phrase line",
        "I am here",
        8,
        "cy",
        "en",
    )
    .await
    .expect("link reused line");

    let after = evidence_checksum(&pool, "cy").await.expect("after");
    assert_eq!(before, after);
    let (linked_id, owns, from_phrase, frequency): (i32, bool, String, i32) = sqlx::query_as(
        "SELECT t.id, pt.owns_translation, t.from_phrase, t.corpus_frequency
         FROM phrase_translations pt
         JOIN translations t ON t.id = pt.translation_id
         WHERE pt.id = $1",
    )
    .bind(link_id)
    .fetch_one(&pool)
    .await
    .expect("reused link");
    assert_eq!(linked_id, original_id);
    assert!(!owns);
    assert_eq!(from_phrase, "I am here");
    assert_eq!(frequency, 8);

    let (fresh_phrase,): (i32,) = sqlx::query_as(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, 'pipeline phrase fresh', 3, 4 FROM languages WHERE code = 'cy'
         ON CONFLICT (language_id, phrase) DO UPDATE SET sentence_count = EXCLUDED.sentence_count
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("fresh phrase");
    sqlx::query("DELETE FROM phrase_translations WHERE phrase_id = $1")
        .bind(fresh_phrase)
        .execute(&pool)
        .await
        .expect("clear fresh link");
    let (fresh_link,): (i32,) = sqlx::query_as(
        "INSERT INTO phrase_translations (phrase_id, native_language_id, translation)
         SELECT $1, id, 'a fresh gloss' FROM languages WHERE code = 'en'
         RETURNING id",
    )
    .bind(fresh_phrase)
    .fetch_one(&pool)
    .await
    .expect("fresh gloss");
    link_phrase_translation(
        &pool,
        fresh_link,
        "pipeline phrase fresh",
        "a fresh gloss",
        4,
        "cy",
        "en",
    )
    .await
    .expect("link fresh");
    let (fresh_owns, in_view): (bool, bool) = sqlx::query_as(
        "SELECT pt.owns_translation,
                EXISTS (
                    SELECT 1 FROM corpus_evidence_translations t
                    WHERE t.id = pt.translation_id
                )
         FROM phrase_translations pt
         WHERE pt.id = $1",
    )
    .bind(fresh_link)
    .fetch_one(&pool)
    .await
    .expect("fresh ownership");
    assert!(fresh_owns);
    assert!(!in_view);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_stale_generation_does_not_add_counts_and_two_claims_do_not_double() {
    let pool = test_pool().await;
    seed_languages(&pool).await;
    let surfaces = [
        "000 alpha beta one",
        "000 alpha beta two",
        "000 alpha beta three",
        "000 alpha beta four",
        "000 alpha beta five",
    ];
    sqlx::query("DELETE FROM pipeline_phrase_counts WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear counts");
    sqlx::query("DELETE FROM pipeline_phrase_mines WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear mine");
    sqlx::query("DELETE FROM translations WHERE to_phrase = ANY($1)")
        .bind(&surfaces[..])
        .execute(&pool)
        .await
        .expect("clear surfaces");
    sqlx::query(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, 'mine page ' || seed.surface, seed.surface
         FROM languages fl
         JOIN languages tl ON tl.code = 'cy'
         JOIN unnest($1::text[]) AS seed(surface) ON true
         WHERE fl.code = 'en'",
    )
    .bind(&surfaces[..])
    .execute(&pool)
    .await
    .expect("seed surfaces");

    let first = commit_mine_page(&pool, "cy", 2).await.expect("first page");
    assert!(first.advanced);
    let stolen = commit_mine_page_at(&pool, "cy", first.generation, &first.cursor_before, 2)
        .await
        .expect("stolen page");
    assert!(!stolen.advanced);
    let count: i32 = sqlx::query_scalar(
        "SELECT sentence_count FROM pipeline_phrase_counts
          WHERE foreign_code = 'cy' AND phrase = 'alpha beta'",
    )
    .fetch_one(&pool)
    .await
    .expect("alpha beta count");
    assert_eq!(count, 2, "the losing claim must not add the page again");
}

fn cy_archive_urls() -> Vec<(String, String)> {
    archive_urls("cy")
}

fn archive_urls(foreign: &str) -> Vec<(String, String)> {
    let corpora = wisecrow::pipeline::corpora_for(foreign, None).expect("corpora");
    let langs = wisecrow::Langs::new("en", foreign);
    corpora
        .into_iter()
        .map(|name| {
            let corpus = wisecrow::files::Corpus::try_from(name).expect("corpus");
            let files =
                wisecrow::files::LanguageFiles::new(&langs, Some(&[corpus])).expect("files");
            let file = files.files.into_iter().next().expect("archive");
            (name.to_owned(), file.target_location)
        })
        .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn mine_waits_until_every_archive_is_recorded() {
    let pool = test_pool().await;
    seed_languages(&pool).await;
    sqlx::query("DELETE FROM pipeline_archives WHERE native_code = 'en' AND foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear archives");
    sqlx::query("DELETE FROM pipeline_phrase_counts WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear counts");

    let urls = cy_archive_urls();
    assert!(!urls.is_empty());
    assert!(!archives_settled(&pool, "en", "cy")
        .await
        .expect("unsettled"));
    mine_pair(&pool, "en", "cy").await.expect("mine waits");
    let counts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pipeline_phrase_counts WHERE foreign_code = 'cy'")
            .fetch_one(&pool)
            .await
            .expect("counts");
    assert_eq!(counts, 0, "an unrecorded archive does not mine");

    let ledger = wisecrow::pipeline::archives::ArchiveLedger::new(&pool);
    for (name, url) in &urls {
        ledger
            .mark_unavailable("en", "cy", name, url)
            .await
            .expect("record archive");
    }
    assert!(archives_settled(&pool, "en", "cy").await.expect("settled"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn publish_updates_the_linked_frequency_and_leaves_a_phrase_below_five() {
    let pool = test_pool().await;
    seed_languages(&pool).await;
    for phrase in [
        "publish keep phrase",
        "publish rare phrase",
        "publish absent phrase",
    ] {
        sqlx::query(
            "DELETE FROM phrase_translations
              WHERE phrase_id IN (SELECT id FROM phrases WHERE phrase = $1)",
        )
        .bind(phrase)
        .execute(&pool)
        .await
        .expect("clear link");
        sqlx::query("DELETE FROM phrases WHERE phrase = $1")
            .bind(phrase)
            .execute(&pool)
            .await
            .expect("clear phrase");
        sqlx::query("DELETE FROM translations WHERE to_phrase = $1")
            .bind(phrase)
            .execute(&pool)
            .await
            .expect("clear translation");
    }
    sqlx::query("DELETE FROM pipeline_phrase_counts WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear counts");
    sqlx::query("DELETE FROM pipeline_phrase_mines WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear mine");
    sqlx::query("DELETE FROM pipeline_phrase_stamps WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear stamp");

    sqlx::query(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, phrase, 3, sentences
         FROM languages
         JOIN (VALUES ('publish keep phrase', 1), ('publish rare phrase', 9))
              AS seed(phrase, sentences) ON true
         WHERE code = 'cy'",
    )
    .execute(&pool)
    .await
    .expect("seed phrases");
    let (translation_id,): (i32,) = sqlx::query_as(
        "INSERT INTO translations
             (from_language_id, to_language_id, from_phrase, to_phrase, corpus_frequency)
         SELECT fl.id, tl.id, 'keep the english', 'publish keep phrase', 1
         FROM languages fl, languages tl
         WHERE fl.code = 'en' AND tl.code = 'cy'
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("seed translation");
    sqlx::query(
        "INSERT INTO phrase_translations (phrase_id, native_language_id, translation, translation_id)
         SELECT p.id, native.id, 'keep the english', $1
         FROM phrases p, languages native
         WHERE p.phrase = 'publish keep phrase' AND native.code = 'en'",
    )
    .bind(translation_id)
    .execute(&pool)
    .await
    .expect("seed link");
    sqlx::query(
        "INSERT INTO pipeline_phrase_counts
             (foreign_code, phrase, token_count, sentence_count)
         VALUES ('cy', 'publish keep phrase', 3, 6),
                ('cy', 'publish rare phrase', 3, 4),
                ('cy', 'publish absent phrase', 3, 4)",
    )
    .execute(&pool)
    .await
    .expect("seed counts");
    let checksum = evidence_checksum(&pool, "cy").await.expect("checksum");
    sqlx::query(
        "INSERT INTO pipeline_phrase_mines (foreign_code, evidence_checksum)
         VALUES ('cy', $1)",
    )
    .bind(checksum.as_slice())
    .execute(&pool)
    .await
    .expect("seed mine");

    finish_or_abandon(&pool, "cy").await.expect("publish");

    let (kept,): (i32,) =
        sqlx::query_as("SELECT sentence_count FROM phrases WHERE phrase = 'publish keep phrase'")
            .fetch_one(&pool)
            .await
            .expect("kept phrase");
    assert_eq!(kept, 6);
    let (rare,): (i32,) =
        sqlx::query_as("SELECT sentence_count FROM phrases WHERE phrase = 'publish rare phrase'")
            .fetch_one(&pool)
            .await
            .expect("rare phrase");
    assert_eq!(rare, 9);
    let absent: i64 =
        sqlx::query_scalar("SELECT count(*) FROM phrases WHERE phrase = 'publish absent phrase'")
            .fetch_one(&pool)
            .await
            .expect("absent phrase");
    assert_eq!(absent, 0);
    let (same_id, from_phrase, frequency): (i32, String, i32) =
        sqlx::query_as("SELECT id, from_phrase, corpus_frequency FROM translations WHERE id = $1")
            .bind(translation_id)
            .fetch_one(&pool)
            .await
            .expect("linked translation");
    assert_eq!(same_id, translation_id);
    assert_eq!(from_phrase, "keep the english");
    assert_eq!(frequency, 6);
    let stamp: Vec<u8> = sqlx::query_scalar(
        "SELECT evidence_checksum FROM pipeline_phrase_stamps WHERE foreign_code = 'cy'",
    )
    .fetch_one(&pool)
    .await
    .expect("stamp");
    assert_eq!(stamp, checksum);
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pipeline_phrase_counts WHERE foreign_code = 'cy'")
            .fetch_one(&pool)
            .await
            .expect("counts cleared");
    assert_eq!(remaining, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_changed_checksum_drops_the_partial_mine() {
    let pool = test_pool().await;
    seed_languages(&pool).await;
    sqlx::query("DELETE FROM pipeline_phrase_counts WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear counts");
    sqlx::query("DELETE FROM pipeline_phrase_mines WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear mine");
    sqlx::query("DELETE FROM pipeline_phrase_stamps WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear stamp");
    sqlx::query(
        "DELETE FROM translations
          WHERE to_phrase IN ('checksum drop phrase', 'checksum drop moved')",
    )
    .execute(&pool)
    .await
    .expect("clear surface");
    sqlx::query(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, 'checksum drop english', 'checksum drop phrase'
         FROM languages fl, languages tl
         WHERE fl.code = 'en' AND tl.code = 'cy'",
    )
    .execute(&pool)
    .await
    .expect("seed surface");

    let first = commit_mine_page(&pool, "cy", 50).await.expect("page");
    assert!(first.advanced);
    sqlx::query(
        "UPDATE translations SET to_phrase = 'checksum drop moved'
          WHERE to_phrase = 'checksum drop phrase'",
    )
    .execute(&pool)
    .await
    .expect("change evidence");

    finish_or_abandon(&pool, "cy").await.expect("abandon");

    let stamp: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pipeline_phrase_stamps WHERE foreign_code = 'cy'")
            .fetch_one(&pool)
            .await
            .expect("stamp");
    assert_eq!(stamp, 0);
    let counts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pipeline_phrase_counts WHERE foreign_code = 'cy'")
            .fetch_one(&pool)
            .await
            .expect("counts");
    assert_eq!(counts, 0);
    let generation: i64 = sqlx::query_scalar(
        "SELECT generation FROM pipeline_phrase_mines WHERE foreign_code = 'cy'",
    )
    .fetch_one(&pool)
    .await
    .expect("generation");
    assert!(generation > first.generation);
}

struct OutageProvider<'a> {
    calls: &'a AtomicUsize,
}

#[async_trait]
impl LlmProvider for OutageProvider<'_> {
    async fn generate(&self, _prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(WisecrowError::LlmUnavailable("down".into()))
    }

    fn name(&self) -> &str {
        "outage"
    }
}

struct EchoProvider;

#[async_trait]
impl LlmProvider for EchoProvider {
    async fn generate(&self, prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        let mut entries = Vec::new();
        let mut listing = false;
        for line in prompt.lines() {
            if line == "Phrases:" {
                listing = true;
                continue;
            }
            if !listing {
                continue;
            }
            if line.is_empty() {
                break;
            }
            let Some((_, phrase)) = line.split_once(". ") else {
                continue;
            };
            let translation = format!("en {phrase}");
            entries.push(format!(
                r#"{{"phrase":{},"translation":{}}}"#,
                serde_json::to_string(phrase).expect("phrase json"),
                serde_json::to_string(&translation).expect("translation json"),
            ));
        }
        Ok(format!(r#"{{"translations":[{}]}}"#, entries.join(",")))
    }

    fn name(&self) -> &str {
        "echo"
    }
}

async fn linked_translation(pool: &PgPool, phrase: &str) -> Option<(i32, String, i32)> {
    sqlx::query_as(
        "SELECT t.id, t.from_phrase, t.corpus_frequency
         FROM phrase_translations pt
         JOIN phrases p ON p.id = pt.phrase_id
         JOIN translations t ON t.id = pt.translation_id
         WHERE p.phrase = $1
           AND p.language_id = (SELECT id FROM languages WHERE code = 'cy')",
    )
    .bind(phrase)
    .fetch_optional(pool)
    .await
    .expect("linked translation")
}

async fn delete_unglossed_cy_phrases(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM phrases p USING languages l
          WHERE p.language_id = l.id AND l.code = 'cy'
            AND NOT EXISTS (
                SELECT 1 FROM phrase_translations pt WHERE pt.phrase_id = p.id
            )",
    )
    .execute(pool)
    .await
    .expect("clear unglossed phrases");
}

async fn delete_cy_phrase(pool: &PgPool, phrase: &str) {
    sqlx::query(
        "DELETE FROM phrase_translations
          WHERE phrase_id IN (
              SELECT p.id FROM phrases p
              JOIN languages l ON l.id = p.language_id
              WHERE l.code = 'cy' AND p.phrase = $1
          )",
    )
    .bind(phrase)
    .execute(pool)
    .await
    .expect("clear link");
    sqlx::query(
        "DELETE FROM phrases p USING languages l
          WHERE p.language_id = l.id AND l.code = 'cy' AND p.phrase = $1",
    )
    .bind(phrase)
    .execute(pool)
    .await
    .expect("clear phrase");
    sqlx::query("DELETE FROM translations WHERE to_phrase = $1")
        .bind(phrase)
        .execute(pool)
        .await
        .expect("clear translation");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn orphan_relink_does_not_call_the_model_and_gloss_skips_a_linked_phrase() {
    let pool = test_pool().await;
    seed_languages(&pool).await;
    delete_cy_phrase(&pool, "tha mi").await;

    let (phrase_id,): (i32,) = sqlx::query_as(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, 'tha mi', 2, 10 FROM languages WHERE code = 'cy'
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("orphan phrase");
    sqlx::query(
        "INSERT INTO phrase_translations (phrase_id, native_language_id, translation)
         SELECT $1, id, 'I am' FROM languages WHERE code = 'en'",
    )
    .bind(phrase_id)
    .execute(&pool)
    .await
    .expect("orphan gloss");
    delete_unglossed_cy_phrases(&pool).await;

    let calls = AtomicUsize::new(0);
    let provider = OutageProvider { calls: &calls };
    let step = gloss_pair(&pool, &provider, "en", "cy").await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(step.relinked >= 1);
    let (_id, from, _) = linked_translation(&pool, "tha mi").await.unwrap();
    assert_eq!(from, "I am");

    for index in 0..101 {
        let phrase = format!("g{index:03} xx");
        delete_cy_phrase(&pool, &phrase).await;
        sqlx::query(
            "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
             SELECT id, $1, 2, $2 FROM languages WHERE code = 'cy'",
        )
        .bind(&phrase)
        .bind(101 - index)
        .execute(&pool)
        .await
        .expect("unglossed phrase");
    }
    delete_cy_phrase(&pool, "stay put").await;
    let (kept_phrase,): (i32,) = sqlx::query_as(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, 'stay put', 2, 500 FROM languages WHERE code = 'cy'
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("kept phrase");
    let (kept_translation,): (i32,) = sqlx::query_as(
        "INSERT INTO translations
             (from_language_id, to_language_id, from_phrase, to_phrase, corpus_frequency)
         SELECT fl.id, tl.id, 'do not move', 'stay put', 500
         FROM languages fl, languages tl
         WHERE fl.code = 'en' AND tl.code = 'cy'
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("kept translation");
    sqlx::query(
        "INSERT INTO phrase_translations
             (phrase_id, native_language_id, translation, translation_id)
         SELECT $1, id, 'do not move', $2 FROM languages WHERE code = 'en'",
    )
    .bind(kept_phrase)
    .bind(kept_translation)
    .execute(&pool)
    .await
    .expect("kept link");

    let echo = EchoProvider;
    gloss_pair(&pool, &echo, "en", "cy").await.unwrap();

    let new_links: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM phrase_translations pt
         JOIN phrases p ON p.id = pt.phrase_id
         JOIN languages l ON l.id = p.language_id
         WHERE l.code = 'cy'
           AND p.phrase LIKE 'g___ xx'
           AND pt.translation_id IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("new links");
    assert!(new_links <= 100);
    assert_eq!(new_links, 100, "the gloss slice is 100, not the whole tail");
    let left_out: Option<i32> = sqlx::query_scalar(
        "SELECT pt.translation_id
         FROM phrases p
         LEFT JOIN phrase_translations pt ON pt.phrase_id = p.id
         JOIN languages l ON l.id = p.language_id
         WHERE l.code = 'cy' AND p.phrase = 'g100 xx'",
    )
    .fetch_one(&pool)
    .await
    .expect("tail phrase");
    assert_eq!(left_out, None);
    let (_id, kept, _) = linked_translation(&pool, "stay put").await.unwrap();
    assert_eq!(kept, "do not move");
}

async fn seed_cy_translation(pool: &PgPool, english: &str, phrase: &str, frequency: i32) -> i32 {
    let (id,): (i32,) = sqlx::query_as(
        "INSERT INTO translations
             (from_language_id, to_language_id, from_phrase, to_phrase, corpus_frequency)
         SELECT fl.id, tl.id, $1, $2, $3
         FROM languages fl, languages tl
         WHERE fl.code = 'en' AND tl.code = 'cy'
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
         DO UPDATE SET corpus_frequency = EXCLUDED.corpus_frequency
         RETURNING id",
    )
    .bind(english)
    .bind(phrase)
    .bind(frequency)
    .fetch_one(pool)
    .await
    .expect("translation");
    id
}

async fn seed_cy_gloss(pool: &PgPool, english: &str, phrase: &str, sentence_count: i32) -> i32 {
    delete_cy_phrase(pool, phrase).await;
    let translation_id = seed_cy_translation(pool, english, phrase, sentence_count).await;
    let (phrase_id,): (i32,) = sqlx::query_as(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, $1, 2, $2 FROM languages WHERE code = 'cy'
         RETURNING id",
    )
    .bind(phrase)
    .bind(sentence_count)
    .fetch_one(pool)
    .await
    .expect("phrase");
    sqlx::query(
        "INSERT INTO phrase_translations
             (phrase_id, native_language_id, translation, translation_id)
         SELECT $1, id, $2, $3 FROM languages WHERE code = 'en'",
    )
    .bind(phrase_id)
    .bind(english)
    .bind(translation_id)
    .execute(pool)
    .await
    .expect("gloss");
    translation_id
}

fn phrase_audio_profile() -> ProbeProfile<'static> {
    ProbeProfile {
        audio_backend: "edge",
        audio_voice: "test-voice",
        audio_format: 1,
        check_audio: true,
        provider_ids: &[],
        image_format: 1,
        check_image: false,
        image_providers_known: false,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn phrase_audio_selects_glossed_phrases_without_a_current_clip() {
    let pool = test_pool().await;
    seed_languages(&pool).await;
    sqlx::query(
        "DELETE FROM phrase_translations pt
          USING phrases p, languages l
          WHERE pt.phrase_id = p.id AND p.language_id = l.id AND l.code = 'cy'",
    )
    .execute(&pool)
    .await
    .expect("clear cy glosses");

    let cached_id = seed_cy_gloss(&pool, "cached english", "cached clip", 40).await;
    let uncached_id = seed_cy_gloss(&pool, "uncached english", "uncached clip", 80).await;
    let unglossed_id = seed_cy_translation(&pool, "bare english", "bare staged", 70).await;
    delete_cy_phrase(&pool, "bare staged").await;
    sqlx::query(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, 'bare staged', 2, 70 FROM languages WHERE code = 'cy'",
    )
    .execute(&pool)
    .await
    .expect("unglossed phrase");

    let xdg = tempfile::tempdir().expect("xdg");
    let previous = std::env::var_os("XDG_DATA_HOME");
    std::env::set_var("XDG_DATA_HOME", xdg.path());
    let cache_dir = xdg.path().join("wisecrow").join("cache");
    let cache = MediaCache::with_cache_dir(pool.clone(), &cache_dir).expect("cache");
    let subject = load_media_subject(&pool, cached_id)
        .await
        .expect("subject")
        .expect("cached subject");
    let profile = phrase_audio_profile();
    let fingerprint = subject.audio_fingerprint(
        profile.audio_backend,
        profile.audio_voice,
        profile.audio_format,
    );
    let path = cache_dir.join(MediaType::Audio.as_str()).join(format!(
        "{cached_id}-{}.{}",
        fingerprint.as_str(),
        MediaType::Audio.extension()
    ));
    tokio::fs::write(&path, b"audio").await.expect("clip");
    sqlx::query(
        "INSERT INTO media_cache (translation_id, media_type, file_path, source_fingerprint)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (translation_id, media_type)
         DO UPDATE SET file_path = $3, source_fingerprint = $4",
    )
    .bind(cached_id)
    .bind(MediaType::Audio.as_str())
    .bind(path.to_str().expect("utf8 path"))
    .bind(fingerprint.as_str())
    .execute(&pool)
    .await
    .expect("cache row");
    let _ = cache;

    let ids = next_phrase_audio_batch(&pool, "en", "cy", &profile, 100)
        .await
        .unwrap();
    assert_eq!(ids, vec![uncached_id]);
    assert!(!ids.contains(&unglossed_id));
    assert!(!ids.contains(&cached_id));

    for index in 0..100 {
        let phrase = format!("clip{index:03} xx");
        seed_cy_gloss(&pool, &format!("en {phrase}"), &phrase, 30).await;
    }
    let capped = next_phrase_audio_batch(&pool, "en", "cy", &profile, 100)
        .await
        .unwrap();
    assert_eq!(capped.len(), 100);
    assert!(!capped.contains(&cached_id));
    assert!(!capped.contains(&unglossed_id));

    if let Some(previous) = previous {
        std::env::set_var("XDG_DATA_HOME", previous);
    } else {
        std::env::remove_var("XDG_DATA_HOME");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_unsettled_later_pair_is_named_before_an_open_mine() {
    let pool = test_pool().await;
    sqlx::query(
        "DELETE FROM pipeline_archives
          WHERE native_code = 'en' AND foreign_code IN ('cy', 'gd')",
    )
    .execute(&pool)
    .await
    .expect("clear archives");
    sqlx::query("DELETE FROM pipeline_phrase_mines WHERE foreign_code = 'cy'")
        .execute(&pool)
        .await
        .expect("clear mine");

    let ledger = wisecrow::pipeline::archives::ArchiveLedger::new(&pool);
    for (name, url) in archive_urls("cy") {
        ledger
            .mark_unavailable("en", "cy", &name, &url)
            .await
            .expect("record cy archive");
    }
    let gd_urls = archive_urls("gd");
    let recorded = gd_urls.len().saturating_sub(1);
    for (name, url) in gd_urls.iter().take(recorded) {
        ledger
            .mark_unavailable("en", "gd", name, url)
            .await
            .expect("record gd archive");
    }
    assert!(gd_urls.len() > recorded, "one gd archive stays unrecorded");

    sqlx::query(
        "INSERT INTO pipeline_phrase_mines
             (foreign_code, evidence_checksum, surface_cursor)
         VALUES ('cy', $1, 'left-as-seeded')",
    )
    .bind([0u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect("seed mine");

    let pairs = [
        PairSpec {
            native: "en".to_owned(),
            foreign: "cy".to_owned(),
            corpora: None,
        },
        PairSpec {
            native: "en".to_owned(),
            foreign: "gd".to_owned(),
            corpora: None,
        },
    ];
    let leading = leading_work(&pool, &pairs).await.unwrap();
    assert_eq!(leading, Leading::Archive { foreign: "gd" });
    let cursor: String = sqlx::query_scalar(
        "SELECT surface_cursor FROM pipeline_phrase_mines WHERE foreign_code = 'cy'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(cursor, "left-as-seeded");
}
