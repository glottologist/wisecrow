use sqlx::PgPool;
use wisecrow::config::SecureString;
use wisecrow::media::images::{ImageFetcher, ImageProviderMode};
use wisecrow::media::{audio_cache_key, image_cache_key, load_media_subject, MediaSubject};
use wisecrow::presentation::CURRENT_PRESENTATION_VERSION;

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

async fn seed_pair(pool: &PgPool, from: &str, to: &str) -> i32 {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('fr', 'French')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages seed");
    sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, $1, $2
         FROM languages fl, languages tl WHERE fl.code = 'en' AND tl.code = 'fr'
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
         DO UPDATE SET from_phrase = EXCLUDED.from_phrase
         RETURNING id",
    )
    .bind(from)
    .bind(to)
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
         VALUES ('fr', $1, 'en', $2, $3, $4, $5, $6)
         ON CONFLICT (lang_code, word, native_lang) DO UPDATE
         SET translation = EXCLUDED.translation,
             display_form = EXCLUDED.display_form,
             teachable = EXCLUDED.teachable,
             image_query = EXCLUDED.image_query,
             presentation_version = EXCLUDED.presentation_version",
    )
    .bind(word)
    .bind(translation)
    .bind(display_form)
    .bind(teachable)
    .bind(image_query)
    .bind(CURRENT_PRESENTATION_VERSION)
    .execute(pool)
    .await
    .expect("presentation seed");
}

async fn mark_as_phrase(pool: &PgPool, translation_id: i32, phrase: &str) {
    let phrase_id: i32 = sqlx::query_scalar(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, $1, 3, 50 FROM languages WHERE code = 'fr'
         ON CONFLICT (language_id, phrase) DO UPDATE
         SET sentence_count = EXCLUDED.sentence_count
         RETURNING id",
    )
    .bind(phrase)
    .fetch_one(pool)
    .await
    .expect("phrase seed");
    sqlx::query(
        "INSERT INTO phrase_translations
             (phrase_id, native_language_id, translation, translation_id)
         SELECT $1, id, $2, $3 FROM languages WHERE code = 'en'
         ON CONFLICT (phrase_id, native_language_id) DO UPDATE
         SET translation = EXCLUDED.translation,
             translation_id = EXCLUDED.translation_id",
    )
    .bind(phrase_id)
    .bind("canonical phrase translation")
    .bind(translation_id)
    .execute(pool)
    .await
    .expect("phrase translation seed");
}

async fn load(pool: &PgPool, translation_id: i32) -> MediaSubject {
    load_media_subject(pool, translation_id)
        .await
        .expect("query")
        .expect("row exists")
}

fn stock_fetcher() -> ImageFetcher {
    ImageFetcher::from_keys(
        ImageProviderMode::Auto,
        Some(&SecureString::from("u".to_owned())),
        Some(&SecureString::from("p".to_owned())),
        Some(&SecureString::from("x".to_owned())),
    )
    .expect("stock providers")
}

/// The served text comes from the presentation contract, never from the caller.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn concrete_word_yields_canonical_tts_text_and_image_query() {
    let pool = test_pool().await;
    let id = seed_pair(&pool, "raw animal", "Chien.").await;
    seed_presentation(&pool, "chien", "dog", "chien", true, Some("friendly dog")).await;
    let subject = load(&pool, id).await;
    assert_eq!(subject.to_phrase, "chien");
    assert_eq!(subject.from_phrase, "dog");
    assert_eq!(subject.applicable_image_query(), Some("friendly dog"));
    assert!(!subject.is_phrase);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn abstract_word_has_no_applicable_image() {
    let pool = test_pool().await;
    let id = seed_pair(&pool, "China?", "En...").await;
    seed_presentation(&pool, "en", "in", "en", true, None).await;
    let subject = load(&pool, id).await;
    assert_eq!(subject.to_phrase, "en");
    assert_eq!(subject.from_phrase, "in");
    assert!(subject.applicable_image_query().is_none());
    assert!(image_cache_key(&subject, &stock_fetcher()).is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn promoted_phrase_has_audio_text_and_no_image() {
    let pool = test_pool().await;
    let id = seed_pair(&pool, "raw phrase", "comment allez vous").await;
    seed_presentation(
        &pool,
        "comment allez vous",
        "wrong word override",
        "wrong display",
        false,
        Some("wrong image"),
    )
    .await;
    mark_as_phrase(&pool, id, "comment allez vous").await;
    let subject = load(&pool, id).await;
    assert_eq!(subject.to_phrase, "comment allez vous");
    assert_eq!(subject.from_phrase, "canonical phrase translation");
    assert!(subject.is_phrase);
    assert!(subject.applicable_image_query().is_none());
    assert!(audio_cache_key(&subject, None).is_ok());
    assert!(image_cache_key(&subject, &stock_fetcher()).is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn unknown_id_is_none() {
    let pool = test_pool().await;
    assert!(load_media_subject(&pool, 999_999_999)
        .await
        .expect("query")
        .is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn web_tui_prefetch_audio_keys_match_for_loaded_subject() {
    let pool = test_pool().await;
    let id = seed_pair(&pool, "raw animal", "Chien.").await;
    seed_presentation(&pool, "chien", "dog", "chien", true, Some("friendly dog")).await;
    let subject = load(&pool, id).await;
    let web = audio_cache_key(&subject, None).expect("web key");
    let tui = audio_cache_key(&subject, None).expect("tui key");
    let prefetch = audio_cache_key(&subject, None).expect("prefetch key");
    assert_eq!(web, tui);
    assert_eq!(tui, prefetch);
    let fetcher = stock_fetcher();
    let image = image_cache_key(&subject, &fetcher).expect("image key");
    assert_eq!(image, image_cache_key(&subject, &fetcher).expect("repeat"));
}
