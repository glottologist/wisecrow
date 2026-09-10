use async_trait::async_trait;
use sqlx::PgPool;
use wisecrow::errors::WisecrowError;
use wisecrow::glossing::{enrich_presentations, pending_presentations};
use wisecrow::llm::LlmProvider;
use wisecrow::presentation::{PresentationRepository, PresentedTranslation};

const NATIVE_CODE: &str = "en";
const FOREIGN_CODE: &str = "fr";

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

async fn seed_translation(pool: &PgPool, native: &str, foreign: &str) -> i32 {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('fr', 'French')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages seed");

    sqlx::query_scalar(
        "INSERT INTO translations
             (from_language_id, from_phrase, to_language_id, to_phrase, corpus_frequency)
         SELECT nl.id, $1, fl.id, $2, 100
         FROM languages nl, languages fl
         WHERE nl.code = $3 AND fl.code = $4
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
         DO UPDATE SET corpus_frequency = EXCLUDED.corpus_frequency
         RETURNING id",
    )
    .bind(native)
    .bind(foreign)
    .bind(NATIVE_CODE)
    .bind(FOREIGN_CODE)
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
    version: i32,
) {
    sqlx::query(
        "INSERT INTO word_glosses
             (lang_code, word, native_lang, translation, display_form,
              teachable, image_query, presentation_version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (lang_code, word, native_lang) DO UPDATE
         SET translation = EXCLUDED.translation,
             display_form = EXCLUDED.display_form,
             teachable = EXCLUDED.teachable,
             image_query = EXCLUDED.image_query,
             presentation_version = EXCLUDED.presentation_version",
    )
    .bind(FOREIGN_CODE)
    .bind(word)
    .bind(NATIVE_CODE)
    .bind(translation)
    .bind(display_form)
    .bind(teachable)
    .bind(image_query)
    .bind(version)
    .execute(pool)
    .await
    .expect("presentation seed");
}

async fn mark_as_phrase(pool: &PgPool, translation_id: i32, phrase: &str) {
    let phrase_id: i32 = sqlx::query_scalar(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, $1, 3, 50 FROM languages WHERE code = $2
         ON CONFLICT (language_id, phrase) DO UPDATE
         SET sentence_count = EXCLUDED.sentence_count
         RETURNING id",
    )
    .bind(phrase)
    .bind(FOREIGN_CODE)
    .fetch_one(pool)
    .await
    .expect("phrase seed");

    sqlx::query(
        "INSERT INTO phrase_translations
             (phrase_id, native_language_id, translation, translation_id)
         SELECT $1, id, $2, $3 FROM languages WHERE code = $4
         ON CONFLICT (phrase_id, native_language_id) DO UPDATE
         SET translation = EXCLUDED.translation,
             translation_id = EXCLUDED.translation_id",
    )
    .bind(phrase_id)
    .bind("canonical phrase translation")
    .bind(translation_id)
    .bind(NATIVE_CODE)
    .execute(pool)
    .await
    .expect("phrase translation seed");
}

struct PresentationFixture {
    fallback: i32,
    canonical: i32,
    concrete: i32,
    rejected: i32,
    phrase: i32,
}

async fn seed_fixture(pool: &PgPool) -> PresentationFixture {
    let fallback = seed_translation(pool, "raw fallback", "motbrut").await;
    let canonical = seed_translation(pool, "China?", "En...").await;
    seed_presentation(pool, "en", "in", "en", true, None, 1).await;
    let concrete = seed_translation(pool, "raw animal", "Chien.").await;
    seed_presentation(pool, "chien", "dog", "chien", true, Some("friendly dog"), 1).await;
    let rejected = seed_translation(pool, "[FANGS cackles]", "Es...").await;
    seed_presentation(pool, "es", "is", "es", false, None, 1).await;
    let phrase = seed_translation(pool, "raw phrase", "comment allez vous").await;
    seed_presentation(
        pool,
        "comment allez vous",
        "wrong word override",
        "wrong display",
        false,
        Some("wrong image"),
        1,
    )
    .await;
    mark_as_phrase(pool, phrase, "comment allez vous").await;
    PresentationFixture {
        fallback,
        canonical,
        concrete,
        rejected,
        phrase,
    }
}

async fn load(pool: &PgPool, translation_id: i32) -> PresentedTranslation {
    PresentationRepository::load(pool, translation_id)
        .await
        .expect("presentation query")
        .expect("presentation row")
}

fn assert_presentations(rows: &[PresentedTranslation]) {
    let [fallback, canonical, concrete, rejected, phrase] = rows else {
        panic!("expected all presentation fixtures");
    };
    assert_eq!(fallback.from_phrase, "raw fallback");
    assert_eq!(fallback.to_phrase, "motbrut");
    assert_eq!(fallback.presentation_version, 0);
    assert!(fallback.teachable && !fallback.is_phrase);
    assert_eq!(canonical.from_phrase, "in");
    assert_eq!(canonical.to_phrase, "en");
    assert_eq!(canonical.image_query, None);
    assert_eq!(concrete.image_query.as_deref(), Some("friendly dog"));
    assert!(!rejected.teachable);
    assert_eq!(phrase.from_phrase, "canonical phrase translation");
    assert_eq!(phrase.to_phrase, "comment allez vous");
    assert!(phrase.image_query.is_none() && phrase.teachable && phrase.is_phrase);
}

async fn assert_legacy_cache_has_no_fingerprint(pool: &PgPool, translation_id: i32) {
    sqlx::query(
        "INSERT INTO media_cache (translation_id, media_type, file_path)
         VALUES ($1, 'audio', '/tmp/legacy-presentation-probe.mp3')
         ON CONFLICT (translation_id, media_type) DO UPDATE
         SET file_path = EXCLUDED.file_path, source_fingerprint = NULL",
    )
    .bind(translation_id)
    .execute(pool)
    .await
    .expect("legacy cache seed");
    let fingerprint: Option<String> = sqlx::query_scalar(
        "SELECT source_fingerprint FROM media_cache
         WHERE translation_id = $1 AND media_type = 'audio'",
    )
    .bind(translation_id)
    .fetch_one(pool)
    .await
    .expect("legacy cache query");
    assert_eq!(fingerprint, None);
}

struct StubProvider {
    response: String,
}

#[async_trait]
impl LlmProvider for StubProvider {
    async fn generate(&self, _prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        Ok(self.response.clone()) // clone: test provider may serve more than one batch
    }

    fn name(&self) -> &str {
        "stub"
    }
}

async fn clear_enrichment_fixture(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM word_glosses
         WHERE lang_code = 'fr' AND native_lang = 'en'
           AND word = ANY($1)",
    )
    .bind(["avec", "le", "chien"])
    .execute(pool)
    .await
    .expect("presentation cleanup");
}

async fn set_frequency(pool: &PgPool, translation_id: i32, frequency: i32) {
    sqlx::query("UPDATE translations SET corpus_frequency = $1 WHERE id = $2")
        .bind(frequency)
        .bind(translation_id)
        .execute(pool)
        .await
        .expect("frequency seed");
}

async fn seed_enrichment_fixture(pool: &PgPool) {
    clear_enrichment_fixture(pool).await;
    for (native, foreign, frequency) in [
        ("Starring.", "Avec.", 900),
        ("Starring?", "Avec?", 900),
        ("The", "Le", 800),
        ("Dog", "Chien", 700),
    ] {
        let id = seed_translation(pool, native, foreign).await;
        set_frequency(pool, id, frequency).await;
    }
    seed_presentation(pool, "le", "legacy the", "Le", true, None, 0).await;
    seed_presentation(pool, "chien", "legacy dog", "Chien", true, None, 0).await;
}

fn candidate_words(candidates: &[wisecrow::glossing::PresentationCandidate]) -> Vec<&str> {
    candidates.iter().map(|item| item.word.as_str()).collect()
}

async fn stored_presentation(
    pool: &PgPool,
    word: &str,
) -> (String, String, bool, Option<String>, i32) {
    sqlx::query_as(
        "SELECT display_form, translation, teachable, image_query, presentation_version
         FROM word_glosses
         WHERE lang_code = 'fr' AND native_lang = 'en' AND word = $1",
    )
    .bind(word)
    .fetch_one(pool)
    .await
    .expect("stored presentation")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn pending_presentations_advance_by_version_and_retry_omissions() {
    let pool = test_pool().await;
    seed_enrichment_fixture(&pool).await;
    let pending = pending_presentations(&pool, "en", "fr", 3)
        .await
        .expect("pending presentations");
    assert_eq!(candidate_words(&pending), vec!["avec", "le", "chien"]);

    let provider = StubProvider {
        response: r#"{"presentations":[{"word":"Avec.","display_form":"avec","translation":"with","teachable":true,"image_query":null},{"word":"le","display_form":"le","translation":"   ","teachable":true,"image_query":null}]}"#.to_owned(),
    };
    let written = enrich_presentations(&pool, &provider, &pending, "en", "fr", "English", "French")
        .await
        .expect("presentation enrichment");
    assert_eq!(written, 1);

    let next = pending_presentations(&pool, "en", "fr", 2)
        .await
        .expect("next pending presentations");
    assert_eq!(candidate_words(&next), vec!["le", "chien"]);
    assert_eq!(
        stored_presentation(&pool, "avec").await,
        ("avec".into(), "with".into(), true, None, 1)
    );
    assert_eq!(
        stored_presentation(&pool, "le").await,
        ("Le".into(), "legacy the".into(), true, None, 0)
    );
    assert_eq!(
        stored_presentation(&pool, "chien").await,
        ("Chien".into(), "legacy dog".into(), true, None, 0)
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn presentation_contract_resolves_words_phrases_and_legacy_cache() {
    let pool = test_pool().await;
    let fixture = seed_fixture(&pool).await;
    let rows = vec![
        load(&pool, fixture.fallback).await,
        load(&pool, fixture.canonical).await,
        load(&pool, fixture.concrete).await,
        load(&pool, fixture.rejected).await,
        load(&pool, fixture.phrase).await,
    ];
    assert_presentations(&rows);
    assert_legacy_cache_has_no_fingerprint(&pool, fixture.fallback).await;
}
