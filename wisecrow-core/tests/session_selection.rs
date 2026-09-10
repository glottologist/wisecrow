use chrono::{Duration, Utc};
use sqlx::PgPool;
use wisecrow::srs::scheduler::CardManager;
use wisecrow::srs::session::SessionManager;

const EMAIL: &str = "session-selection@test.local";

struct Fixture {
    user_id: i32,
    due_id: i32,
    rejected_id: i32,
    new_ids: [i32; 3],
}

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await.expect("database connection");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

async fn reset_fixture(pool: &PgPool) -> i32 {
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(pool)
        .await
        .expect("user cleanup");
    sqlx::query(
        "DELETE FROM translations
         WHERE to_language_id = (SELECT id FROM languages WHERE code = 'br')",
    )
    .execute(pool)
    .await
    .expect("translation cleanup");
    sqlx::query("DELETE FROM word_glosses WHERE lang_code = 'br' AND native_lang = 'en'")
        .execute(pool)
        .await
        .expect("presentation cleanup");
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('br', 'Breton')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("language seed");
    sqlx::query_scalar(
        "INSERT INTO users (display_name, email) VALUES ('Session selection', $1)
         RETURNING id",
    )
    .bind(EMAIL)
    .fetch_one(pool)
    .await
    .expect("user seed")
}

async fn seed_translation(pool: &PgPool, native: &str, foreign: &str, frequency: i32) -> i32 {
    sqlx::query_scalar(
        "INSERT INTO translations
             (from_language_id, from_phrase, to_language_id, to_phrase,
              corpus_frequency, frequency)
         VALUES ((SELECT id FROM languages WHERE code = 'en'), $1,
                 (SELECT id FROM languages WHERE code = 'br'), $2, $3, 1)
         RETURNING id",
    )
    .bind(native)
    .bind(foreign)
    .bind(frequency)
    .fetch_one(pool)
    .await
    .expect("translation seed")
}

async fn seed_presentation(
    pool: &PgPool,
    word: &str,
    translation: &str,
    display_form: &str,
    teachable: bool,
    image_query: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO word_glosses
             (lang_code, word, native_lang, translation, display_form,
              teachable, image_query, presentation_version)
         VALUES ('br', $1, 'en', $2, $3, $4, $5, 1)",
    )
    .bind(word)
    .bind(translation)
    .bind(display_form)
    .bind(teachable)
    .bind(image_query)
    .execute(pool)
    .await
    .expect("presentation seed");
}

async fn make_due(pool: &PgPool, user_id: i32, translation_id: i32) {
    CardManager::ensure_cards(pool, &[translation_id], user_id)
        .await
        .expect("card seed");
    sqlx::query(
        "UPDATE cards SET state = 3, due = $1
         WHERE user_id = $2 AND translation_id = $3",
    )
    .bind(Utc::now() - Duration::days(1))
    .bind(user_id)
    .bind(translation_id)
    .execute(pool)
    .await
    .expect("due seed");
}

async fn seed_fixture(pool: &PgPool) -> Fixture {
    let user_id = reset_fixture(pool).await;
    let due_id = seed_translation(pool, "subtitle review", "Adolygu.", 1_000).await;
    let rejected_id = seed_translation(pool, "[noise]", "Torri.", 950).await;
    let dog_id = seed_translation(pool, "subtitle dog", "Ci.", 900).await;
    let but_id = seed_translation(pool, "subtitle but", "Ond.", 800).await;
    let now_id = seed_translation(pool, "subtitle now", "Nawr.", 700).await;
    seed_presentation(pool, "adolygu", "review", "adolygu", true, None).await;
    seed_presentation(pool, "torri", "broken fragment", "torri", false, None).await;
    seed_presentation(pool, "ci", "dog", "ci", true, Some("friendly dog")).await;
    seed_presentation(pool, "ond", "but", "ond", true, None).await;
    seed_presentation(pool, "nawr", "now", "nawr", true, None).await;
    make_due(pool, user_id, due_id).await;
    make_due(pool, user_id, rejected_id).await;
    Fixture {
        user_id,
        due_id,
        rejected_id,
        new_ids: [dog_id, but_id, now_id],
    }
}

fn assert_selected_cards(session: &wisecrow::srs::session::Session, fixture: &Fixture) {
    let expected = [
        fixture.due_id,
        fixture.new_ids[0],
        fixture.new_ids[1],
        fixture.new_ids[2],
    ];
    let actual: Vec<i32> = session
        .cards
        .iter()
        .map(|card| card.translation_id)
        .collect();
    assert_eq!(actual, expected);
    assert!(!actual.contains(&fixture.rejected_id));
    assert_eq!(actual.iter().filter(|id| **id == fixture.due_id).count(), 1);
    assert_eq!(session.cards[0].from_phrase, "review");
    assert_eq!(session.cards[0].to_phrase, "adolygu");
    assert_eq!(session.cards[1].from_phrase, "dog");
    assert_eq!(session.cards[1].to_phrase, "ci");
    assert!(session.cards[1].image_allowed);
    assert!(session
        .cards
        .iter()
        .enumerate()
        .all(|(index, card)| index == 1 || !card.image_allowed));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn session_uses_due_once_and_exact_ordered_new_cards() {
    let pool = test_pool().await;
    let fixture = seed_fixture(&pool).await;
    let session = SessionManager::create(&pool, fixture.user_id, "en", "br", 4, 3_000)
        .await
        .expect("session creation");
    assert_selected_cards(&session, &fixture);
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT card_id)
         FROM session_cards WHERE session_id = $1",
    )
    .bind(session.id)
    .fetch_one(&pool)
    .await
    .expect("session card counts");
    assert_eq!(counts, (4, 4));
    let direct = CardManager::get_card_by_id(&pool, session.cards[1].card_id)
        .await
        .expect("card by id");
    assert_eq!(
        (direct.from_phrase.as_str(), direct.to_phrase.as_str()),
        ("dog", "ci")
    );
    let by_translation =
        CardManager::card_for_translation(&pool, fixture.new_ids[0], fixture.user_id)
            .await
            .expect("card by translation")
            .expect("seeded card");
    assert!(by_translation.image_allowed);

    SessionManager::pause(&pool, session.id, fixture.user_id)
        .await
        .expect("pause");
    let resumed = SessionManager::resume(&pool, fixture.user_id, "en", "br")
        .await
        .expect("resume")
        .expect("paused session");
    assert_selected_cards(&resumed, &fixture);
}
