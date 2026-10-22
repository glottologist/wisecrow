//! Custom reader decks against PostgreSQL.

use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('es', 'Spanish')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(&pool)
    .await?;
    sqlx::query("TRUNCATE decks, reader_translations, book_glosses, translations CASCADE")
        .execute(&pool)
        .await?;
    Ok(pool)
}

async fn language(pool: &PgPool, code: &str) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM languages WHERE code = $1")
        .bind(code)
        .fetch_one(pool)
        .await
}

async fn user(pool: &PgPool, email: &str) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO users (display_name, email) VALUES ($1, $2)
         ON CONFLICT (email) DO UPDATE SET display_name = EXCLUDED.display_name
         RETURNING id",
    )
    .bind(email)
    .bind(email)
    .fetch_one(pool)
    .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn adding_a_word_owns_a_new_row_and_reuses_a_colliding_one() -> TestResult {
    let pool = reset_pool().await?;
    let english = language(&pool, "en").await?;
    let spanish = language(&pool, "es").await?;
    let owner = user(&pool, "owner@localhost.test").await?;
    let other = user(&pool, "other@localhost.test").await?;
    let deck = wisecrow::decks::DeckRepository::create(&pool, owner, "en", "es", "Keeper").await?;
    for (form, meaning) in [("mar", "sea"), ("rio", "river")] {
        sqlx::query(
            "INSERT INTO book_glosses
                 (language_id, native_language_id, normalised, meaning, lemma, class, note, status)
             VALUES ($1, $2, $3, $4, $3, 'noun', 'A short note.', 'ok')",
        )
        .bind(spanish)
        .bind(english)
        .bind(form)
        .bind(meaning)
        .execute(&pool)
        .await?;
    }
    sqlx::query(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase, frequency)
         VALUES ($1, $2, 'river', 'rio', 1)",
    )
    .bind(english)
    .bind(spanish)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO word_glosses (lang_code, word, native_lang, translation, display_form)
         VALUES ('es', 'mar', 'en', 'ocean', 'mar')",
    )
    .execute(&pool)
    .await?;

    let created = wisecrow::decks::DeckRepository::add_word(&pool, owner, deck, "mar").await?;
    let reused = wisecrow::decks::DeckRepository::add_word(&pool, owner, deck, "rio").await?;

    assert!(created.owns_translation);
    assert!(!reused.owns_translation);
    let evidence: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM corpus_evidence_translations WHERE to_phrase IN ('mar', 'rio')",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        evidence, 1,
        "the owned row is hidden and the reused row was already evidence"
    );
    let face: String = sqlx::query_scalar(
        "SELECT from_phrase FROM translation_presentations WHERE translation_id = $1",
    )
    .bind(created.translation_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(face, "sea", "the note wins over the word gloss");
    let image: Option<String> = sqlx::query_scalar(
        "SELECT image_query FROM translation_presentations WHERE translation_id = $1",
    )
    .bind(created.translation_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(image, None);
    let cards: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cards WHERE user_id = $1 AND translation_id = $2")
            .bind(owner)
            .bind(created.translation_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(cards, 1);
    sqlx::query("DELETE FROM deck_cards WHERE deck_id = $1")
        .bind(deck)
        .execute(&pool)
        .await?;
    let link: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM reader_translations WHERE normalised = 'mar'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(link, 1, "removing the card does not remove the link");
    let refused = wisecrow::decks::DeckRepository::add_word(&pool, other, deck, "mar").await;
    assert!(refused.is_err());
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn due_cards_keep_a_deck_card_inside_the_limit() -> TestResult {
    use chrono::Duration;
    use wisecrow::decks::{DeckFilter, DeckRepository};
    use wisecrow::srs::scheduler::CardManager;

    let pool = reset_pool().await?;
    let english = language(&pool, "en").await?;
    let spanish = language(&pool, "es").await?;
    let owner = user(&pool, "due-deck@localhost.test").await?;
    let deck = DeckRepository::create(&pool, owner, "en", "es", "Due").await?;
    let sooner: i32 = sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         VALUES ($1, $2, 'sooner', 'antes') RETURNING id",
    )
    .bind(english)
    .bind(spanish)
    .fetch_one(&pool)
    .await?;
    let later: i32 = sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         VALUES ($1, $2, 'later', 'despues') RETURNING id",
    )
    .bind(english)
    .bind(spanish)
    .fetch_one(&pool)
    .await?;
    CardManager::ensure_cards(&pool, &[sooner, later], owner).await?;
    sqlx::query("INSERT INTO deck_cards (deck_id, translation_id) VALUES ($1, $2)")
        .bind(deck)
        .bind(later)
        .execute(&pool)
        .await?;
    let now = chrono::Utc::now();
    sqlx::query("UPDATE cards SET state = 2, due = $1 WHERE user_id = $2 AND translation_id = $3")
        .bind(now - Duration::hours(2))
        .bind(owner)
        .bind(sooner)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE cards SET state = 2, due = $1 WHERE user_id = $2 AND translation_id = $3")
        .bind(now - Duration::hours(1))
        .bind(owner)
        .bind(later)
        .execute(&pool)
        .await?;

    let open = CardManager::due_cards(&pool, "en", "es", owner, 1, None, DeckFilter::All).await?;
    let filtered =
        CardManager::due_cards(&pool, "en", "es", owner, 1, None, DeckFilter::Only(deck)).await?;

    assert_eq!(open[0].translation_id, sooner);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].translation_id, later);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn two_paused_sessions_resume_by_deck() -> TestResult {
    use wisecrow::decks::DeckRepository;
    use wisecrow::srs::session::{SessionChoice, SessionManager};

    let pool = reset_pool().await?;
    let english = language(&pool, "en").await?;
    let spanish = language(&pool, "es").await?;
    let owner = user(&pool, "resume-owner@localhost.test").await?;
    let other = user(&pool, "resume-other@localhost.test").await?;
    let deck = DeckRepository::create(&pool, owner, "en", "es", "Resume").await?;
    sqlx::query(
        "INSERT INTO book_glosses
             (language_id, native_language_id, normalised, meaning, lemma, class, note, status)
         VALUES ($1, $2, 'mar', 'sea', 'mar', 'noun', 'A short note.', 'ok')",
    )
    .bind(spanish)
    .bind(english)
    .execute(&pool)
    .await?;
    let added = DeckRepository::add_word(&pool, owner, deck, "mar").await?;
    let deck_word = added.translation_id;
    sqlx::query(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase, frequency)
         VALUES ($1, $2, 'house', 'casa', 4)",
    )
    .bind(english)
    .bind(spanish)
    .execute(&pool)
    .await?;

    let deck_session = SessionManager::create(
        &pool,
        owner,
        "en",
        "es",
        5,
        1000,
        SessionChoice {
            area: None,
            deck_id: Some(deck),
        },
    )
    .await?;
    let open_session = SessionManager::create(
        &pool,
        owner,
        "en",
        "es",
        5,
        1000,
        SessionChoice {
            area: None,
            deck_id: None,
        },
    )
    .await?;
    sqlx::query("UPDATE sessions SET paused_at = CURRENT_TIMESTAMP WHERE id = $1")
        .bind(deck_session.id)
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE sessions SET paused_at = CURRENT_TIMESTAMP WHERE id = $1")
        .bind(open_session.id)
        .execute(&pool)
        .await?;

    let resumed_deck = SessionManager::resume(&pool, owner, "en", "es", Some(deck))
        .await?
        .expect("deck");
    let resumed_open = SessionManager::resume(&pool, owner, "en", "es", None)
        .await?
        .expect("open");

    assert_eq!(resumed_deck.id, deck_session.id);
    assert_eq!(resumed_open.id, open_session.id);
    assert_ne!(resumed_deck.id, resumed_open.id);
    assert!(resumed_deck
        .cards
        .iter()
        .all(|card| card.translation_id == deck_word));
    let stolen = SessionManager::create(
        &pool,
        other,
        "en",
        "es",
        5,
        1000,
        SessionChoice {
            area: None,
            deck_id: Some(deck),
        },
    )
    .await;
    assert!(stolen.is_err());
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn book_progress_counts_a_finished_file() -> TestResult {
    let pool = reset_pool().await?;
    let english = language(&pool, "en").await?;
    let spanish = language(&pool, "es").await?;
    let reader = user(&pool, "books-progress@localhost.test").await?;
    let level: i32 = sqlx::query_scalar("SELECT id FROM cefr_levels WHERE code = 'A2'")
        .fetch_one(&pool)
        .await?;
    sqlx::query("DELETE FROM reading_positions WHERE user_id = $1")
        .bind(reader)
        .execute(&pool)
        .await?;

    let finished_file = "readers/finished.pdf";
    let middle_file = "readers/middle.pdf";
    sqlx::query("DELETE FROM books WHERE file = $1 OR file = $2")
        .bind(finished_file)
        .bind(middle_file)
        .execute(&pool)
        .await?;

    let finished = insert_book(
        &pool,
        &[2u8; 32],
        finished_file,
        level,
        english,
        &[(1, 1, 1)],
    )
    .await?;
    let middle = insert_book(
        &pool,
        &[3u8; 32],
        middle_file,
        level,
        english,
        &[(1, 1, 1), (1, 1, 2)],
    )
    .await?;
    translate_book(&pool, finished, spanish).await?;
    translate_book(&pool, middle, spanish).await?;
    save_place(&pool, reader, english, finished_file, 1, 1, 1).await?;
    save_place(&pool, reader, english, middle_file, 1, 1, 1).await?;

    let progress = wisecrow::progress::ProgressQuery::dashboard(&pool, reader).await?;
    let spanish_progress = progress
        .iter()
        .find(|language| language.foreign == "es")
        .expect("spanish");
    assert_eq!(spanish_progress.books.started, 2);
    assert_eq!(spanish_progress.books.finished, 1);
    Ok(())
}

async fn insert_book(
    pool: &PgPool,
    key: &[u8],
    file: &str,
    level: i32,
    source: i32,
    sentences: &[(i32, i32, i32)],
) -> Result<i32, sqlx::Error> {
    let book_id: i32 = sqlx::query_scalar(
        "INSERT INTO books (import_key, file, file_sha256, series, title, cefr_level_id,
                            source_language_id, segmenter_version, status)
         VALUES ($1, $2, $3, 'Fixture', $2, $4, $5, 1, 'ready')
         RETURNING id",
    )
    .bind(key)
    .bind(file)
    .bind(&[4u8; 32][..])
    .bind(level)
    .bind(source)
    .fetch_one(pool)
    .await?;
    for (index, (chapter, paragraph, sentence)) in sentences.iter().enumerate() {
        let position = i32::try_from(index).map_or(0, |value| value.saturating_add(1));
        sqlx::query(
            "INSERT INTO book_sentences (book_id, position, chapter, paragraph, sentence, text)
             VALUES ($1, $2, $3, $4, $5, 'A synthetic sentence.')",
        )
        .bind(book_id)
        .bind(position)
        .bind(chapter)
        .bind(paragraph)
        .bind(sentence)
        .execute(pool)
        .await?;
    }
    Ok(book_id)
}

async fn translate_book(pool: &PgPool, book_id: i32, foreign: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO book_translations (sentence_id, language_id, text)
         SELECT id, $2, 'Una frase sintética.' FROM book_sentences WHERE book_id = $1",
    )
    .bind(book_id)
    .bind(foreign)
    .execute(pool)
    .await?;
    Ok(())
}

async fn save_place(
    pool: &PgPool,
    user_id: i32,
    language_id: i32,
    file: &str,
    chapter: i32,
    paragraph: i32,
    sentence: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO reading_positions (user_id, file, language_id, chapter, paragraph, sentence)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(file)
    .bind(language_id)
    .bind(chapter)
    .bind(paragraph)
    .bind(sentence)
    .execute(pool)
    .await?;
    Ok(())
}
