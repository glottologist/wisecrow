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
    sqlx::query("DELETE FROM word_glosses WHERE lang_code = 'cy'")
        .execute(pool)
        .await
        .expect("word_glosses cleanup");
}

/// Stores a translation for a word, as `wisecrow gloss-deck` would.
async fn seed_gloss(pool: &PgPool, welsh_word: &str, english: &str) {
    sqlx::query(
        "INSERT INTO word_glosses (lang_code, word, native_lang, translation)
         VALUES ('cy', $1, 'en', $2)
         ON CONFLICT (lang_code, word, native_lang) DO UPDATE
           SET translation = EXCLUDED.translation",
    )
    .bind(welsh_word)
    .bind(english)
    .execute(pool)
    .await
    .expect("gloss seed");
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
async fn a_word_the_corpus_states_once_is_shown_its_gloss_instead() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    // The 200-of-315 case. "Marw" has exactly one English partner and it is an
    // alignment failure — this is `Die! → An!` from the real Irish deck, which
    // no rule over the Welsh side can see, because the Welsh side is fine.
    seed(&pool, "Die!", "Marw", 500).await;
    seed_gloss(&pool, "marw", "dead").await;

    let deck = VocabularyQuery::unlearned(&pool, "en", "cy", 50)
        .await
        .expect("unlearned");

    assert_eq!(deck.len(), 1, "still one card for the one word");
    assert_eq!(
        deck[0].from_phrase, "dead",
        "an uncorroborated pairing loses to a stored translation"
    );
    assert_eq!(
        deck[0].to_phrase, "Marw",
        "only the prompt is substituted; the word being taught is untouched"
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_pairing_the_corpus_repeats_beats_a_gloss() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    // The corroborated case, and the reason the substitution is conditional. Two
    // rows agree on "Yes", so the corpus is speaking from evidence rather than
    // from a single accident, and authentic evidence has to win — otherwise
    // every card in the deck would come from a model.
    seed(&pool, "Yes.", "Ie.", 500).await;
    seed(&pool, "Yes?", "Ie?", 500).await;
    seed_gloss(&pool, "ie", "affirmative").await;

    let deck = VocabularyQuery::unlearned(&pool, "en", "cy", 50)
        .await
        .expect("unlearned");

    assert_eq!(deck.len(), 1, "still one card for the one word");
    assert!(
        deck[0].from_phrase.starts_with("Yes"),
        "expected the corroborated corpus partner, got {:?}",
        deck[0].from_phrase
    );

    cleanup(&pool).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_word_with_no_gloss_keeps_whatever_the_corpus_gave_it() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    // Glossing is incremental — the command runs against a limit and a second
    // run picks up where the first stopped — so an uncorroborated word with no
    // gloss yet must still produce a card rather than vanish from the deck.
    seed(&pool, "Die!", "Marw", 500).await;

    let deck = VocabularyQuery::unlearned(&pool, "en", "cy", 50)
        .await
        .expect("unlearned");

    assert_eq!(deck.len(), 1, "the word is still taught");
    assert_eq!(
        deck[0].from_phrase, "Die!",
        "with the corpus partner it had"
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

/// Returns the seeded translation's id so phrase/card fixtures can reference it.
async fn seed_returning_id(
    pool: &PgPool,
    english: &str,
    welsh: &str,
    corpus_frequency: i32,
) -> i32 {
    seed(pool, english, welsh, corpus_frequency).await;
    let (id,): (i32,) = sqlx::query_as(
        "SELECT id FROM translations
          WHERE from_phrase = $1 AND to_phrase = $2
            AND to_language_id = (SELECT id FROM languages WHERE code='cy')",
    )
    .bind(english)
    .bind(welsh)
    .fetch_one(pool)
    .await
    .expect("seeded id");
    id
}

async fn seed_card(pool: &PgPool, translation_id: i32) {
    sqlx::query("INSERT INTO cards (translation_id, user_id, state) VALUES ($1, 1, 1)")
        .bind(translation_id)
        .execute(pool)
        .await
        .expect("card seed");
}

/// Marks a translation as a promoted phrase: a `phrases` row plus the
/// `phrase_translations` link the deck queries test membership against.
async fn seed_phrase_link(pool: &PgPool, translation_id: i32, phrase: &str) {
    let (phrase_id,): (i32,) = sqlx::query_as(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         VALUES ((SELECT id FROM languages WHERE code='cy'), $1, 2, 10)
         ON CONFLICT (language_id, phrase)
           DO UPDATE SET sentence_count = EXCLUDED.sentence_count
         RETURNING id",
    )
    .bind(phrase)
    .fetch_one(pool)
    .await
    .expect("phrase seed");
    sqlx::query(
        "INSERT INTO phrase_translations (phrase_id, native_language_id, translation, translation_id)
         VALUES ($1, (SELECT id FROM languages WHERE code='en'), 'seeded', $2)
         ON CONFLICT (phrase_id, native_language_id)
           DO UPDATE SET translation_id = EXCLUDED.translation_id",
    )
    .bind(phrase_id)
    .bind(translation_id)
    .execute(pool)
    .await
    .expect("phrase link seed");
}

async fn cleanup_phrases(pool: &PgPool) {
    sqlx::query(
        "DELETE FROM phrases WHERE language_id = (SELECT id FROM languages WHERE code='cy')",
    )
    .execute(pool)
    .await
    .expect("phrases cleanup"); // phrase_translations rows cascade
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ranked_candidates_carded_switch_controls_card_exclusion() {
    use wisecrow::vocabulary::{IncludeCarded, PhraseFilter};
    let pool = test_pool().await;
    cleanup(&pool).await;
    cleanup_phrases(&pool).await;

    let carded = seed_returning_id(&pool, "water", "dŵr", 900).await;
    seed_card(&pool, carded).await;
    seed_returning_id(&pool, "fire", "tân", 800).await;

    let all = VocabularyQuery::ranked_candidates(
        &pool,
        "en",
        "cy",
        50,
        IncludeCarded::Yes,
        PhraseFilter::All,
    )
    .await
    .expect("include carded");
    let welsh: Vec<&str> = all.iter().map(|e| e.to_phrase.as_str()).collect();
    assert_eq!(welsh, vec!["dŵr", "tân"]);

    let uncarded = VocabularyQuery::ranked_candidates(
        &pool,
        "en",
        "cy",
        50,
        IncludeCarded::No,
        PhraseFilter::All,
    )
    .await
    .expect("exclude carded");
    let welsh: Vec<&str> = uncarded.iter().map(|e| e.to_phrase.as_str()).collect();
    assert_eq!(welsh, vec!["tân"]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ranked_candidates_phrase_filter_separates_and_orders() {
    use wisecrow::vocabulary::{IncludeCarded, PhraseFilter};
    let pool = test_pool().await;
    cleanup(&pool).await;
    cleanup_phrases(&pool).await;

    seed_returning_id(&pool, "house", "tŷ mawr un", 700).await;
    seed_returning_id(&pool, "dog", "ci bach un", 600).await;
    seed_returning_id(&pool, "cat", "cath fach un", 500).await;
    let p1 = seed_returning_id(&pool, "how are you", "sut wyt ti", 90).await;
    let p2 = seed_returning_id(&pool, "thank you very much", "diolch yn fawr", 80).await;
    seed_phrase_link(&pool, p1, "sut wyt ti").await;
    seed_phrase_link(&pool, p2, "diolch yn fawr").await;

    let words = VocabularyQuery::ranked_candidates(
        &pool,
        "en",
        "cy",
        50,
        IncludeCarded::Yes,
        PhraseFilter::Exclude,
    )
    .await
    .expect("words");
    let got: Vec<&str> = words.iter().map(|e| e.to_phrase.as_str()).collect();
    assert_eq!(got, vec!["tŷ mawr un", "ci bach un", "cath fach un"]);

    let phrases = VocabularyQuery::ranked_candidates(
        &pool,
        "en",
        "cy",
        50,
        IncludeCarded::Yes,
        PhraseFilter::Only,
    )
    .await
    .expect("phrases");
    let got: Vec<&str> = phrases.iter().map(|e| e.to_phrase.as_str()).collect();
    assert_eq!(got, vec!["sut wyt ti", "diolch yn fawr"]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ranked_candidates_deduplicates_spellings_when_carded_included() {
    use wisecrow::vocabulary::{IncludeCarded, PhraseFilter};
    let pool = test_pool().await;
    cleanup(&pool).await;
    cleanup_phrases(&pool).await;

    // Same normalised Welsh surface, two English partners; the corroborated
    // partner (two agreeing rows) must win and the surface appear once.
    seed(&pool, "bread", "bara", 400).await;
    seed(&pool, "bread", "bara.", 400).await;
    seed(&pool, "brxad", "bara!", 400).await;

    let all = VocabularyQuery::ranked_candidates(
        &pool,
        "en",
        "cy",
        50,
        IncludeCarded::Yes,
        PhraseFilter::All,
    )
    .await
    .expect("dedup");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].from_phrase, "bread");
}
