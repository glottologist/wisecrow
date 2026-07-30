use sqlx::PgPool;
use wisecrow::vocabulary::VocabularyQuery;

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

/// Scoped to the Welsh side alone. Other suites share this database and seed
/// their own pairs off the same English row, so deleting by `from_language_id`
/// would take their fixtures with it.
async fn cleanup(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM cards WHERE translation_id IN (
            SELECT id FROM translations
             WHERE to_language_id = (SELECT id FROM languages WHERE code = 'cy')
        )",
    )
    .execute(pool)
    .await
    .expect("cards cleanup");
    sqlx::query(
        "DELETE FROM translations
          WHERE to_language_id = (SELECT id FROM languages WHERE code = 'cy')",
    )
    .execute(pool)
    .await
    .expect("translations cleanup");
}

/// Seeds a pair as ranking would leave it: a corpus count in `corpus_frequency`.
async fn seed(pool: &PgPool, english: &str, welsh: &str, corpus_frequency: i32) {
    seed_pair(pool, english, welsh, Some(corpus_frequency), 1).await;
}

/// Seeds a pair as the ingest alone would leave it: a collision count in
/// `frequency` and no corpus count at all.
async fn seed_unranked(pool: &PgPool, english: &str, welsh: &str, frequency: i32) {
    seed_pair(pool, english, welsh, None, frequency).await;
}

async fn seed_pair(
    pool: &PgPool,
    english: &str,
    welsh: &str,
    corpus_frequency: Option<i32>,
    frequency: i32,
) {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('cy', 'Welsh')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages seed");

    sqlx::query(
        "INSERT INTO translations
             (from_language_id, from_phrase, to_language_id, to_phrase, corpus_frequency, frequency)
         VALUES ((SELECT id FROM languages WHERE code='en'), $1,
                 (SELECT id FROM languages WHERE code='cy'), $2, $3, $4)",
    )
    .bind(english)
    .bind(welsh)
    .bind(corpus_frequency)
    .bind(frequency)
    .execute(pool)
    .await
    .expect("translation seed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_deck_holds_one_card_per_distinct_word_being_learned() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    // One Welsh word, four English partners, three spellings, and a single
    // corpus count shared between them so every row ties. This is the shape
    // that opened a real Gaelic deck with ten cards for the word "Tha",
    // differing only in the English beside them — two of which were the corpus
    // artefacts "AZ has" and "thatthe".
    //
    // The bare spelling is preferred only *within* the partner the corpus
    // agrees on. "Yes" partners the word three times, so its rows are ranked
    // first and the shortest target spelling among them is taken. The bare "Ie"
    // beside "AZ has" has to lose despite being shortest overall, or the choice
    // of English would be decided by punctuation.
    seed(&pool, "Yes.", "Ie.", 500).await;
    seed(&pool, "Yes?", "Ie?", 500).await;
    seed(&pool, "Yes", "Ie", 500).await;
    seed(&pool, "AZ has", "Ie", 500).await;
    seed(&pool, "thatthe", "Ie!", 500).await;
    seed(&pool, "No.", "Na.", 200).await;

    let deck = VocabularyQuery::unlearned(&pool, "en", "cy", 50)
        .await
        .expect("unlearned");

    let welsh: Vec<&str> = deck.iter().map(|e| e.to_phrase.as_str()).collect();
    assert_eq!(
        welsh,
        vec!["Ie", "Na."],
        "one card per word, most frequent first, and the bare spelling preferred"
    );
    assert_eq!(
        deck[0].from_phrase, "Yes",
        "the bare spelling has to come from the agreed partner, not from \"AZ has\""
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_native_phrase_shown_is_the_one_the_corpus_agrees_on() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    // "Yes" partners the word three times across its spellings; the corpus
    // artefacts partner it once each. Every row carries the same corpus count,
    // so nothing but agreement can separate them — and arbitrary choice is what
    // put "AZ has" on the first card of a real Gaelic deck.
    seed(&pool, "Yes.", "Ie.", 500).await;
    seed(&pool, "Yes?", "Ie?", 500).await;
    seed(&pool, "Yes!", "Ie!", 500).await;
    seed(&pool, "AZ has", "Ie", 500).await;
    seed(&pool, "thatthe", "Ie,", 500).await;

    let deck = VocabularyQuery::unlearned(&pool, "en", "cy", 50)
        .await
        .expect("unlearned");

    assert_eq!(deck.len(), 1, "still one card for the one word");
    assert!(
        deck[0].from_phrase.starts_with("Yes"),
        "expected the agreed translation, got {:?}",
        deck[0].from_phrase
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_pair_ranking_never_scored_stays_out_of_the_deck() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    // This is the Irish deck's fault reduced to two rows. The ingest upsert is
    // `ON CONFLICT ... DO UPDATE SET frequency = frequency + 1`, so a pair
    // carried by two corpora climbs that column without being common in either.
    // 3,246,887 Irish rows sat above 1 on this evidence and only 1,689 had ever
    // been ranked, so the deck served sentences ordered by duplication.
    seed_unranked(&pool, "Whatever the corpus repeated", "Beth bynnag", 5000).await;
    seed(&pool, "Yes", "Ie", 40).await;

    let deck = VocabularyQuery::unlearned(&pool, "en", "cy", 50)
        .await
        .expect("unlearned");

    let welsh: Vec<&str> = deck.iter().map(|e| e.to_phrase.as_str()).collect();
    assert_eq!(
        welsh,
        vec!["Ie"],
        "only the ranked pair belongs in a deck, however high the collision count"
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_corrupt_fragment_loses_to_a_widely_attested_native_phrase() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    // The Gaelic deck gave "Bha" the English "Bthey", "Seo" the English "Seaso"
    // and "Dè" the English "Dthat". Each of those words had only singleton
    // pairs, so agreement tied at one and the ordering fell through to the
    // shortest native phrase — which is precisely what a corrupt fragment is.
    //
    // "was" is attested against three other Welsh words here; "Bthey" against
    // one. Nothing about the two strings distinguishes them, and "Bthey" is the
    // longer, so only the breadth of attestation can decide it.
    seed(&pool, "Bthey", "Bu", 900).await;
    seed(&pool, "was", "Bu", 900).await;
    seed(&pool, "was", "Oedd", 800).await;
    seed(&pool, "was", "Roedd", 700).await;
    seed(&pool, "was", "Ydoedd", 600).await;

    let deck = VocabularyQuery::unlearned(&pool, "en", "cy", 50)
        .await
        .expect("unlearned");

    let bu = deck
        .iter()
        .find(|e| e.to_phrase == "Bu")
        .expect("the word is in the deck");
    assert_eq!(
        bu.from_phrase, "was",
        "a native phrase the corpus uses widely beats a one-off fragment"
    );

    cleanup(&pool).await;
}
