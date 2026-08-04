use sqlx::PgPool;
use wisecrow::sentences::sentence_for_word;

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

/// Both seeding helpers look the languages up by code, so they have to exist
/// before either runs. Doing it in `cleanup`, which every test calls first,
/// keeps that independent of the order the helpers happen to be called in — the
/// first version created them inside `seed_sentence` alone, and the two tests
/// that seed a learned word first failed on a NULL language id.
async fn ensure_languages(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('gd', 'Gaelic')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages seed");
}

/// Scoped to Gaelic, other suites owning the Welsh and Irish rows in this shared
/// database.
async fn cleanup(pool: &PgPool) {
    ensure_languages(pool).await;
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

/// Migration 007 seeds user 1 and every later migration keeps it, so the suite
/// borrows it rather than inventing a user against an auth schema it does not
/// otherwise touch. Cards are cleaned up by translation, so nothing of another
/// suite's belonging to the same user is disturbed.
const TEST_USER: i32 = 1;

/// Seeds a scored sentence, as `wisecrow score-sentences` would leave it.
async fn seed_sentence(
    pool: &PgPool,
    english: &str,
    gaelic: &str,
    tokens: &[&str],
    score: i32,
) -> i32 {
    let owned: Vec<String> = tokens.iter().map(|t| (*t).to_owned()).collect();
    sqlx::query_scalar::<_, i32>(
        "INSERT INTO translations
             (from_language_id, from_phrase, to_language_id, to_phrase,
              sentence_tokens, sentence_score)
         VALUES ((SELECT id FROM languages WHERE code='en'), $1,
                 (SELECT id FROM languages WHERE code='gd'), $2, $3, $4)
         RETURNING id",
    )
    .bind(english)
    .bind(gaelic)
    .bind(&owned)
    .bind(score)
    .fetch_one(pool)
    .await
    .expect("sentence seed")
}

/// Gives the user a card for a single-word row, which is what "knows this word"
/// means to the selection query.
async fn learn_word(pool: &PgPool, user_id: i32, english: &str, gaelic: &str) {
    let id = sqlx::query_scalar::<_, i32>(
        "INSERT INTO translations
             (from_language_id, from_phrase, to_language_id, to_phrase, corpus_frequency)
         VALUES ((SELECT id FROM languages WHERE code='en'), $1,
                 (SELECT id FROM languages WHERE code='gd'), $2, 500)
         RETURNING id",
    )
    .bind(english)
    .bind(gaelic)
    .fetch_one(pool)
    .await
    .expect("word seed");

    sqlx::query("INSERT INTO cards (translation_id, user_id) VALUES ($1, $2)")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("card seed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sentence_is_offered_only_when_one_word_in_it_is_new() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let user = TEST_USER;

    // The learner knows "tha" and "sin". Two sentences teach "math": one adds
    // only that word, the other also carries "gu" which they have never met and
    // so asks them to learn two things at once.
    learn_word(&pool, user, "is", "tha").await;
    learn_word(&pool, user, "that", "sin").await;
    let reachable = seed_sentence(
        &pool,
        "That is good",
        "Tha sin math",
        &["tha", "sin", "math"],
        6284,
    )
    .await;
    seed_sentence(
        &pool,
        "That is very good",
        "Tha sin gu math",
        &["tha", "sin", "gu", "math"],
        9000,
    )
    .await;

    let card = sentence_for_word(&pool, user, "en", "gd", "math")
        .await
        .expect("selection")
        .expect("a sentence within reach exists");

    assert_eq!(
        card.translation_id, reachable,
        "the sentence with two unknown words must lose, despite scoring higher"
    );
    assert_eq!(card.foreign_phrase, "Tha sin math");

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn nothing_is_offered_when_every_sentence_is_still_out_of_reach() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let user = TEST_USER;

    // A learner at the very start knows nothing, so every sentence asks too
    // much. That is the expected early state and must not be an error.
    seed_sentence(
        &pool,
        "That is good",
        "Tha sin math",
        &["tha", "sin", "math"],
        6284,
    )
    .await;

    let card = sentence_for_word(&pool, user, "en", "gd", "math")
        .await
        .expect("selection");

    assert!(
        card.is_none(),
        "a learner who knows nothing has no sentence within reach yet, got {card:?}"
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_most_approachable_reachable_sentence_wins() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let user = TEST_USER;

    // Both sentences are within reach; the score orders them, and a higher score
    // means the rarest word in the sentence is still comparatively common.
    learn_word(&pool, user, "is", "tha").await;
    learn_word(&pool, user, "that", "sin").await;
    seed_sentence(&pool, "Is good", "Tha math", &["tha", "math"], 900).await;
    let common = seed_sentence(
        &pool,
        "That is good",
        "Tha sin math",
        &["tha", "sin", "math"],
        6284,
    )
    .await;

    let card = sentence_for_word(&pool, user, "en", "gd", "math")
        .await
        .expect("selection")
        .expect("a sentence within reach exists");

    assert_eq!(
        card.translation_id, common,
        "the sentence whose rarest word is commonest comes first"
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sentence_already_carded_is_not_offered_again() {
    let pool = test_pool().await;
    cleanup(&pool).await;
    let user = TEST_USER;

    learn_word(&pool, user, "is", "tha").await;
    let seen = seed_sentence(&pool, "Is good", "Tha math", &["tha", "math"], 900).await;
    sqlx::query("INSERT INTO cards (translation_id, user_id) VALUES ($1, $2)")
        .bind(seen)
        .bind(user)
        .execute(&pool)
        .await
        .expect("card seed");

    let card = sentence_for_word(&pool, user, "en", "gd", "math")
        .await
        .expect("selection");

    assert!(
        card.is_none(),
        "a sentence the learner already has a card for is not new material, got {card:?}"
    );

    cleanup(&pool).await;
}
