//! Migration 043: reader storage, deck ownership and the evidence exclusion.

use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query(
        "TRUNCATE languages, translations, word_glosses, grammar_rules, media_cache,
                  sessions, users CASCADE",
    )
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO users (id, display_name) VALUES (1, 'Reader')")
        .execute(&pool)
        .await?;
    Ok(pool)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn migration_043_matches_the_import_statements_and_hides_owned_rows() -> TestResult {
    let pool = reset_pool().await?;
    let english: i32 = sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ('en', 'English') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let spanish: i32 = sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ('es', 'Spanish') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let level: i32 = sqlx::query_scalar("SELECT id FROM cefr_levels WHERE code = 'A2'")
        .fetch_one(&pool)
        .await?;

    let book_id: i32 = sqlx::query_scalar(
        "INSERT INTO books (import_key, file, file_sha256, series, title, cefr_level_id,
                            source_language_id, first_page, last_page, segmenter_version, status)
         VALUES ($1, $2, $3, 'Penguin Readers', 'Fixture', $4, $5, 6, 8, 1, 'ready')
         RETURNING id",
    )
    .bind(&[0u8; 32][..])
    .bind("Penguin Readers/Level 2/Fixture.pdf")
    .bind(&[1u8; 32][..])
    .bind(level)
    .bind(english)
    .fetch_one(&pool)
    .await?;

    let sentence_id: i32 = sqlx::query_scalar(
        "INSERT INTO book_sentences (book_id, position, chapter, paragraph, sentence, text)
         VALUES ($1, 0, 1, 1, 1, 'The cat sat.')
         RETURNING id",
    )
    .bind(book_id)
    .fetch_one(&pool)
    .await?;

    sqlx::query(
        "INSERT INTO book_import_progress (book_id, language_id) VALUES ($1, $2)
         ON CONFLICT (book_id, language_id) DO NOTHING",
    )
    .bind(book_id)
    .bind(spanish)
    .execute(&pool)
    .await?;

    let translation_row: i32 = sqlx::query_scalar(
        "INSERT INTO book_translations (sentence_id, language_id, text) VALUES ($1, $2, $3)
         ON CONFLICT (sentence_id, language_id) DO NOTHING RETURNING id",
    )
    .bind(sentence_id)
    .bind(spanish)
    .bind("El gato se sentó.")
    .fetch_one(&pool)
    .await?;

    sqlx::query(
        "INSERT INTO book_tokens (translation_id, ordinal, start_byte, end_byte, normalised)
         VALUES ($1, 0, 0, 2, 'el')",
    )
    .bind(translation_row)
    .execute(&pool)
    .await?;

    let owned: i32 = sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         VALUES ($1, $2, 'cat', 'gato') RETURNING id",
    )
    .bind(english)
    .bind(spanish)
    .fetch_one(&pool)
    .await?;
    let reused: i32 = sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         VALUES ($1, $2, 'the cat', 'el gato') RETURNING id",
    )
    .bind(english)
    .bind(spanish)
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO reader_translations
             (language_id, native_language_id, normalised, translation_id, owns_translation)
         VALUES ($1, $2, 'gato', $3, TRUE), ($1, $2, 'el gato', $4, FALSE)",
    )
    .bind(spanish)
    .bind(english)
    .bind(owned)
    .bind(reused)
    .execute(&pool)
    .await?;

    let visible: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM corpus_evidence_translations WHERE id = ANY($1)")
            .bind(vec![owned, reused])
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        visible, 1,
        "only the reused corpus row stays in the evidence view"
    );

    sqlx::query(
        "INSERT INTO reading_positions
             (user_id, file, language_id, chapter, paragraph, sentence)
         VALUES (1, 'Penguin Readers/Level 2/Fixture.pdf', $1, 1, 1, 1)",
    )
    .bind(spanish)
    .execute(&pool)
    .await?;

    let deck: i32 = sqlx::query_scalar(
        "INSERT INTO decks (user_id, name, native_lang, foreign_lang)
         VALUES (1, 'Animals', 'en', 'es') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let duplicate = sqlx::query(
        "INSERT INTO decks (user_id, name, native_lang, foreign_lang)
         VALUES (1, 'Animals', 'en', 'es')",
    )
    .execute(&pool)
    .await;
    assert!(duplicate.is_err(), "one name per user and pair");

    sqlx::query("INSERT INTO deck_cards (deck_id, translation_id) VALUES ($1, $2)")
        .bind(deck)
        .bind(reused)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE sessions SET deck_id = $1 WHERE FALSE")
        .bind(deck)
        .execute(&pool)
        .await?;
    Ok(())
}
