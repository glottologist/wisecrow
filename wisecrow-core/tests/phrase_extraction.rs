use sqlx::PgPool;
use wisecrow::phrases::extract_phrases;

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

async fn ensure_languages(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('gd', 'Gaelic')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages seed");
}

/// Scoped to the Gaelic side; other suites own the Welsh and Irish rows in
/// this shared database. `phrase_translations` rows cascade from `phrases`.
async fn cleanup(pool: &PgPool) {
    ensure_languages(pool).await;
    sqlx::query(
        "DELETE FROM phrases WHERE language_id = (SELECT id FROM languages WHERE code = 'gd')",
    )
    .execute(pool)
    .await
    .expect("phrases cleanup");
    sqlx::query(
        "DELETE FROM cards WHERE translation_id IN (
            SELECT id FROM translations
             WHERE to_language_id = (SELECT id FROM languages WHERE code = 'gd')
        )",
    )
    .execute(pool)
    .await
    .expect("cards cleanup");
    sqlx::query(
        "DELETE FROM translations
          WHERE to_language_id = (SELECT id FROM languages WHERE code = 'gd')",
    )
    .execute(pool)
    .await
    .expect("translations cleanup");
}

async fn seed_surface(pool: &PgPool, english: &str, gaelic: &str) {
    sqlx::query(
        "INSERT INTO translations
             (from_language_id, from_phrase, to_language_id, to_phrase, corpus_frequency)
         VALUES ((SELECT id FROM languages WHERE code='en'), $1,
                 (SELECT id FROM languages WHERE code='gd'), $2, 100)
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase) DO NOTHING",
    )
    .bind(english)
    .bind(gaelic)
    .execute(pool)
    .await
    .expect("surface seed");
}

async fn phrase_count(pool: &PgPool, phrase: &str) -> Option<i32> {
    sqlx::query_as::<_, (i32,)>(
        "SELECT sentence_count FROM phrases
          WHERE phrase = $1
            AND language_id = (SELECT id FROM languages WHERE code='gd')",
    )
    .bind(phrase)
    .fetch_optional(pool)
    .await
    .expect("phrase lookup")
    .map(|(count,)| count)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn extraction_counts_distinct_surfaces_across_pages() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    // Six distinct surfaces sharing "tha mi", spread across keyset pages of
    // two. Two rows carry the *same* normalised surface ("tha mi sgìth" with
    // and without trailing punctuation) — they must count once.
    seed_surface(&pool, "I am tired", "tha mi sgìth").await;
    seed_surface(&pool, "I am tired.", "tha mi sgìth.").await;
    seed_surface(&pool, "I am happy", "tha mi toilichte").await;
    seed_surface(&pool, "I am cold", "tha mi fuar").await;
    seed_surface(&pool, "I am warm", "tha mi blàth").await;
    seed_surface(&pool, "I am going", "tha mi a' dol").await;
    seed_surface(&pool, "I am here", "tha mi an seo").await;
    // A bigram present in only two surfaces: below the threshold.
    seed_surface(&pool, "big house", "taigh mòr").await;
    seed_surface(&pool, "the big house", "an taigh mòr").await;

    let count = extract_phrases(&pool, "gd", 2).await.expect("extract");
    assert!(count >= 1, "at least one qualifying phrase");

    assert_eq!(phrase_count(&pool, "tha mi").await, Some(6));
    assert_eq!(phrase_count(&pool, "taigh mòr").await, None);

    // Idempotent re-run: same counts, no duplicate rows.
    let (rows_before,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM phrases
          WHERE language_id = (SELECT id FROM languages WHERE code='gd')",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    extract_phrases(&pool, "gd", 2).await.expect("re-extract");
    let (rows_after,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM phrases
          WHERE language_id = (SELECT id FROM languages WHERE code='gd')",
    )
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(rows_before, rows_after);
    assert_eq!(phrase_count(&pool, "tha mi").await, Some(6));
}

use async_trait::async_trait;
use wisecrow::errors::WisecrowError;
use wisecrow::llm::LlmProvider;
use wisecrow::phrases::{translate_phrases, Refresh};

struct StubProvider {
    response: String,
}

#[async_trait]
impl LlmProvider for StubProvider {
    async fn generate(&self, _prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        Ok(self.response.clone()) // clone: provider may be called per batch
    }
    fn name(&self) -> &str {
        "stub"
    }
}

async fn seed_phrase(pool: &PgPool, phrase: &str, sentence_count: i32) -> i32 {
    let (id,): (i32,) = sqlx::query_as(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, $1, 2, $2 FROM languages WHERE code = 'gd'
         ON CONFLICT (language_id, phrase)
           DO UPDATE SET sentence_count = EXCLUDED.sentence_count
         RETURNING id",
    )
    .bind(phrase)
    .bind(sentence_count)
    .fetch_one(pool)
    .await
    .expect("phrase seed");
    id
}

async fn linked_translation(pool: &PgPool, phrase: &str) -> Option<(i32, String, i32)> {
    sqlx::query_as::<_, (i32, String, i32)>(
        "SELECT t.id, t.from_phrase, t.corpus_frequency
         FROM phrase_translations pt
         JOIN phrases p ON p.id = pt.phrase_id
         JOIN translations t ON t.id = pt.translation_id
         WHERE p.phrase = $1",
    )
    .bind(phrase)
    .fetch_optional(pool)
    .await
    .expect("linked translation lookup")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn translation_promotes_and_reruns_are_idempotent() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    seed_phrase(&pool, "tha mi", 40).await;
    seed_phrase(&pool, "ciamar a tha", 30).await;

    let stub = StubProvider {
        response: r#"{"translations":[
            {"phrase":"tha mi","translation":"I am"},
            {"phrase":"ciamar a tha","translation":"how is"}
        ]}"#
        .to_owned(),
    };
    let written = translate_phrases(&pool, &stub, "gd", "en", 10, Refresh::No)
        .await
        .expect("translate");
    assert_eq!(written, 2);

    let (tid, from, freq) = linked_translation(&pool, "tha mi").await.expect("link");
    assert_eq!(from, "I am");
    assert_eq!(freq, 40);
    assert!(tid > 0);

    // Idempotent: nothing left untranslated, so a re-run writes nothing.
    let again = translate_phrases(&pool, &stub, "gd", "en", 10, Refresh::No)
        .await
        .expect("re-run");
    assert_eq!(again, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn refresh_updates_the_linked_row_in_place() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    seed_phrase(&pool, "tha mi", 40).await;

    let first = StubProvider {
        response: r#"{"translations":[{"phrase":"tha mi","translation":"I am"}]}"#.to_owned(),
    };
    translate_phrases(&pool, &first, "gd", "en", 10, Refresh::No)
        .await
        .expect("initial");
    let (original_id, _, _) = linked_translation(&pool, "tha mi").await.expect("link");

    let reworded = StubProvider {
        response: r#"{"translations":[{"phrase":"tha mi","translation":"I am (present)"}]}"#
            .to_owned(),
    };
    let written = translate_phrases(&pool, &reworded, "gd", "en", 10, Refresh::Yes)
        .await
        .expect("refresh");
    assert_eq!(written, 1);

    let (same_id, from, _) = linked_translation(&pool, "tha mi").await.expect("link");
    assert_eq!(
        same_id, original_id,
        "refresh must update, not insert a sibling"
    );
    assert_eq!(from, "I am (present)");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn response_hygiene_drops_bad_entries_and_keeps_omissions_pending() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    seed_phrase(&pool, "tha mi", 40).await;
    seed_phrase(&pool, "ciamar a tha", 30).await;
    seed_phrase(&pool, "an taigh mòr", 20).await;

    // Hallucinated phrase, empty translation, a duplicate (first wins), and
    // "an taigh mòr" omitted entirely.
    let stub = StubProvider {
        response: r#"{"translations":[
            {"phrase":"not a requested phrase","translation":"ghost"},
            {"phrase":"ciamar a tha","translation":"   "},
            {"phrase":"tha mi","translation":"I am"},
            {"phrase":"tha mi","translation":"I exist"}
        ]}"#
        .to_owned(),
    };
    let written = translate_phrases(&pool, &stub, "gd", "en", 10, Refresh::No)
        .await
        .expect("translate");
    assert_eq!(written, 1);
    let (_, from, _) = linked_translation(&pool, "tha mi").await.expect("link");
    assert_eq!(from, "I am", "first answer wins");
    assert!(linked_translation(&pool, "ciamar a tha").await.is_none());
    assert!(linked_translation(&pool, "an taigh mòr").await.is_none());

    // Malformed JSON: the batch is skipped rather than failing the run, so
    // nothing is written and its phrases stay pending for the next run.
    let broken = StubProvider {
        response: "not json at all".to_owned(),
    };
    let written = translate_phrases(&pool, &broken, "gd", "en", 10, Refresh::No)
        .await
        .expect("a malformed response skips its batch");
    assert_eq!(written, 0);
    assert!(linked_translation(&pool, "ciamar a tha").await.is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn pruned_translation_leaves_link_reclaimable() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    seed_phrase(&pool, "tha mi", 40).await;

    let stub = StubProvider {
        response: r#"{"translations":[{"phrase":"tha mi","translation":"I am"}]}"#.to_owned(),
    };
    translate_phrases(&pool, &stub, "gd", "en", 10, Refresh::No)
        .await
        .expect("initial");
    let (tid, _, _) = linked_translation(&pool, "tha mi").await.expect("link");

    // Prune the promoted row directly, as pruning.rs does.
    sqlx::query("DELETE FROM translations WHERE id = $1")
        .bind(tid)
        .execute(&pool)
        .await
        .expect("prune");

    // The link survives with translation_id nulled…
    let (orphaned,): (Option<i32>,) = sqlx::query_as(
        "SELECT pt.translation_id FROM phrase_translations pt
         JOIN phrases p ON p.id = pt.phrase_id WHERE p.phrase = 'tha mi'",
    )
    .fetch_one(&pool)
    .await
    .expect("orphan lookup");
    assert!(orphaned.is_none());

    // …and a refresh run re-promotes it.
    let written = translate_phrases(&pool, &stub, "gd", "en", 10, Refresh::Yes)
        .await
        .expect("re-promote");
    assert_eq!(written, 1);
    let (new_tid, from, _) = linked_translation(&pool, "tha mi").await.expect("relink");
    assert_ne!(new_tid, tid);
    assert_eq!(from, "I am");
}

use wisecrow::srs::session::SessionManager;

async fn ensure_user(pool: &PgPool) -> i32 {
    sqlx::query_scalar(
        "INSERT INTO users (display_name, email) VALUES ('Seeder', 'seeder@phrase.test')
         ON CONFLICT (email) DO UPDATE SET display_name = EXCLUDED.display_name
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("user seed")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn srs_seeding_interleaves_words_and_phrases() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let user_id = ensure_user(&pool).await;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("sessions cleanup");

    // Eight ranked words…
    for (i, (en, gd)) in [
        ("water", "uisge an-diugh"),
        ("fire", "teine an-diugh"),
        ("stone", "clach an-diugh"),
        ("house", "taigh an-diugh"),
        ("dog", "cù an-diugh"),
        ("cat", "cat an-diugh"),
        ("sun", "grian an-diugh"),
        ("moon", "gealach an-diugh"),
    ]
    .iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO translations
                 (from_language_id, from_phrase, to_language_id, to_phrase, corpus_frequency)
             VALUES ((SELECT id FROM languages WHERE code='en'), $1,
                     (SELECT id FROM languages WHERE code='gd'), $2, $3)
             ON CONFLICT DO NOTHING",
        )
        .bind(en)
        .bind(gd)
        .bind(1000 - i32::try_from(i).unwrap_or(0))
        .execute(&pool)
        .await
        .expect("word seed");
    }
    // …and two promoted phrases via the real promotion path.
    seed_phrase(&pool, "tha mi", 40).await;
    seed_phrase(&pool, "ciamar a tha", 30).await;
    let stub = StubProvider {
        response: r#"{"translations":[
            {"phrase":"tha mi","translation":"I am"},
            {"phrase":"ciamar a tha","translation":"how is"}
        ]}"#
        .to_owned(),
    };
    translate_phrases(&pool, &stub, "gd", "en", 10, Refresh::No)
        .await
        .expect("promote");

    let session = SessionManager::create(&pool, user_id, "en", "gd", 10, 3000)
        .await
        .expect("session");

    let phrase_positions: Vec<usize> = session
        .cards
        .iter()
        .enumerate()
        .filter(|(_, card)| card.is_phrase)
        .map(|(position, _)| position)
        .collect();
    assert_eq!(session.cards.len(), 10);
    assert_eq!(
        phrase_positions.len(),
        2,
        "an 80/20 deck of ten holds two phrases"
    );
    let phrases: Vec<&str> = session
        .cards
        .iter()
        .filter(|card| card.is_phrase)
        .map(|card| card.to_phrase.as_str())
        .collect();
    assert!(phrases.contains(&"tha mi"));
    assert!(phrases.contains(&"ciamar a tha"));
}

/// A response that ran out of `max_tokens` mid-JSON: the string is cut off, so
/// the batch cannot be parsed at all.
struct TruncatedFirstBatchProvider {
    calls: std::sync::atomic::AtomicUsize,
    later_response: String,
}

#[async_trait]
impl LlmProvider for TruncatedFirstBatchProvider {
    async fn generate(&self, _prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return Ok(r#"{"translations":[{"phrase":"p00","translation":"tr"#.to_owned());
        }
        Ok(self.later_response.clone())
    }
    fn name(&self) -> &str {
        "truncated-first-batch"
    }
}

/// Production hit this: one 25-phrase batch came back truncated and took the
/// whole 2,000-phrase run down with it, leaving Gaelic stuck at 302 translated.
/// A bad batch must cost only its own phrases.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_failed_batch_does_not_abandon_the_batches_behind_it() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    // 30 phrases over a batch size of 25, seeded with descending sentence
    // counts so the selection order — and therefore the batch split — is fixed.
    for i in 0..30 {
        seed_phrase(&pool, &format!("p{i:02}"), 130 - i).await;
    }

    let second_batch: String = (25..30)
        .map(|i| format!(r#"{{"phrase":"p{i:02}","translation":"t{i:02}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let provider = TruncatedFirstBatchProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
        later_response: format!(r#"{{"translations":[{second_batch}]}}"#),
    };

    let written = translate_phrases(&pool, &provider, "gd", "en", 30, Refresh::No)
        .await
        .expect("a truncated batch must not fail the run");

    assert_eq!(written, 5, "the second batch's phrases are still written");
    assert!(
        linked_translation(&pool, "p00").await.is_none(),
        "the truncated batch writes nothing"
    );
    assert!(
        linked_translation(&pool, "p29").await.is_some(),
        "the batch queued behind it still runs"
    );
}
