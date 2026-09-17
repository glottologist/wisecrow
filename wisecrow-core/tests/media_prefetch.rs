//! Bounded media preparation: the fixed preparation deck, exact presentation
//! loading, and the read-only preview and budgeted execution paths.

use sqlx::PgPool;
use wisecrow::presentation::{PresentationRepository, CURRENT_PRESENTATION_VERSION};
use wisecrow::vocabulary::VocabularyQuery;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Connects to the isolated test database with a clean slate for the
/// tables these fixtures touch.
async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query("TRUNCATE translations, languages, word_glosses, phrases, media_cache CASCADE")
        .execute(&pool)
        .await?;
    Ok(pool)
}

async fn seed_pair(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO languages (code, name) VALUES ('en', 'English'), ('fr', 'French')")
        .execute(pool)
        .await?;
    Ok(())
}

async fn seed_word(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    rank: i32,
    ready: bool,
) -> Result<i32, sqlx::Error> {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO translations
             (from_language_id, to_language_id, from_phrase, to_phrase, corpus_frequency)
         SELECT n.id, f.id, $1, $2, $3 FROM languages n, languages f
         WHERE n.code = 'en' AND f.code = 'fr'
         RETURNING id",
    )
    .bind(native)
    .bind(foreign)
    .bind(rank)
    .fetch_one(pool)
    .await?;
    if ready {
        sqlx::query(
            "INSERT INTO word_glosses
                 (lang_code, word, native_lang, translation, display_form, teachable,
                  presentation_version)
             VALUES ('fr', $1, 'en', $2, $1, true, $3)",
        )
        .bind(foreign)
        .bind(native)
        .bind(CURRENT_PRESENTATION_VERSION)
        .execute(pool)
        .await?;
    }
    Ok(id)
}

/// Seeds one unassessed noise row and two ready words; returns the ready IDs
/// in rank order.
async fn seed_ready_words(pool: &PgPool) -> Result<Vec<i32>, sqlx::Error> {
    seed_pair(pool).await?;
    let mut ids = Vec::new();
    for (native, foreign, rank, ready) in [
        ("noise", "bruit", 1000, false),
        ("dog", "chien", 20, true),
        ("cat", "chat", 10, true),
    ] {
        let id = seed_word(pool, native, foreign, rank, ready).await?;
        if ready {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Promotes a translation row as a phrase, the way `translate-phrases`
/// links one.
async fn seed_phrase(pool: &PgPool, translation_id: i32, phrase: &str) -> Result<(), sqlx::Error> {
    let phrase_id: i32 = sqlx::query_scalar(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, $1, 2, 50 FROM languages WHERE code = 'fr'
         RETURNING id",
    )
    .bind(phrase)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO phrase_translations
             (phrase_id, native_language_id, translation, translation_id)
         SELECT $1, id, 'phrase meaning', $2 FROM languages WHERE code = 'en'",
    )
    .bind(phrase_id)
    .bind(translation_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn preparation_selects_ready_ids_in_order() -> TestResult {
    let pool = reset_pool().await?;
    let ids = seed_ready_words(&pool).await?;
    let selected = VocabularyQuery::preparation_ids(&pool, "en", "fr").await?;
    assert_eq!(
        selected, ids,
        "noise without a current gloss is not prepared"
    );

    let reversed: Vec<i32> = ids.into_iter().rev().collect();
    let rows = PresentationRepository::load_selected(&pool, &reversed, "en", "fr").await?;
    assert_eq!(
        rows.iter()
            .map(|row| row.translation_id)
            .collect::<Vec<_>>(),
        reversed,
        "input order is preserved"
    );
    assert!(
        PresentationRepository::load_selected(&pool, &reversed, "en", "gd")
            .await
            .is_err()
    );
    assert!(
        PresentationRepository::load_selected(&pool, &[reversed[0], reversed[0]], "en", "fr")
            .await
            .is_err()
    );
    assert!(
        PresentationRepository::load_selected(&pool, &[reversed[0], i32::MAX], "en", "fr")
            .await
            .is_err()
    );
    assert!(
        PresentationRepository::load_selected(&pool, &[], "en", "fr")
            .await?
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn preparation_deck_interleaves_phrases_at_fixed_positions() -> TestResult {
    let pool = reset_pool().await?;
    seed_pair(&pool).await?;
    let mut words = Vec::new();
    for index in 0..6 {
        let rank = 100 - index;
        words.push(
            seed_word(
                &pool,
                &format!("word {index}"),
                &format!("mot{index}"),
                rank,
                true,
            )
            .await?,
        );
    }
    let phrase_a = seed_word(&pool, "good morning", "bon matin", 90, false).await?;
    seed_phrase(&pool, phrase_a, "bon matin").await?;
    let phrase_b = seed_word(&pool, "good evening", "bon soir", 80, false).await?;
    seed_phrase(&pool, phrase_b, "bon soir").await?;

    let deck = VocabularyQuery::preparation_ids(&pool, "en", "fr").await?;
    // Every fifth slot takes a phrase while any remain; words fill the rest.
    assert_eq!(
        deck,
        [words[0], words[1], words[2], words[3], phrase_a, words[4], words[5], phrase_b]
    );
    assert!(deck.len() <= 10_000);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn empty_pair_prepares_nothing() -> TestResult {
    let pool = reset_pool().await?;
    seed_pair(&pool).await?;
    assert!(VocabularyQuery::preparation_ids(&pool, "en", "fr")
        .await?
        .is_empty());
    Ok(())
}

mod preview {
    use super::*;
    use wisecrow::media::prefetch::{
        prefetch_media, MediaOutcome, MediaProviders, PrefetchMode, PrefetchOptions, RequestedMedia,
    };
    use wisecrow::Langs;

    fn never_cancelled() -> (
        tokio::sync::watch::Sender<bool>,
        tokio::sync::watch::Receiver<bool>,
    ) {
        tokio::sync::watch::channel(false)
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn preview_reports_misses_and_pages_the_fixed_deck() -> TestResult {
        let pool = reset_pool().await?;
        let ids = seed_ready_words(&pool).await?;
        let langs = Langs::new("en", "fr");
        let providers = MediaProviders::default();
        let (_stop, cancel) = never_cancelled();

        // No image provider is configured, so images are unsupported; French
        // speech exists, so audio would be generated on execution.
        let options =
            PrefetchOptions::new(100, 0, 1024, RequestedMedia::Both, PrefetchMode::Preview)?;
        let summary = prefetch_media(&pool, &langs, &options, &providers, cancel.clone()).await?;
        assert_eq!((summary.selected, summary.processed), (2, 2));
        assert_eq!(summary.next_offset, None);
        assert_eq!(summary.count(MediaOutcome::Missing), 2, "{summary:?}");
        assert_eq!(summary.count(MediaOutcome::Unsupported), 2, "{summary:?}");
        assert_eq!((summary.admitted_bytes, summary.generated_bytes), (0, 0));
        let mut seen: Vec<i32> = summary
            .outcomes
            .iter()
            .map(|item| item.translation_id)
            .collect();
        seen.sort_unstable();
        let mut expected = ids.clone();
        expected.sort_unstable();
        assert_eq!(seen, expected);
        assert!(
            summary.needs_attention(),
            "unsupported images need attention"
        );

        // A page of one from offset zero leaves one more; the next page ends
        // the deck.
        let first = PrefetchOptions::new(1, 0, 1024, RequestedMedia::Audio, PrefetchMode::Preview)?;
        let page = prefetch_media(&pool, &langs, &first, &providers, cancel.clone()).await?;
        assert_eq!((page.selected, page.next_offset), (1, Some(1)));
        assert_eq!(page.outcomes[0].translation_id, ids[0]);
        assert!(!page.needs_attention(), "missing is expected in a preview");
        let second =
            PrefetchOptions::new(1, 1, 1024, RequestedMedia::Audio, PrefetchMode::Preview)?;
        let page = prefetch_media(&pool, &langs, &second, &providers, cancel.clone()).await?;
        assert_eq!((page.selected, page.next_offset), (1, None));
        assert_eq!(page.outcomes[0].translation_id, ids[1]);
        let beyond =
            PrefetchOptions::new(5, 2, 1024, RequestedMedia::Audio, PrefetchMode::Preview)?;
        let page = prefetch_media(&pool, &langs, &beyond, &providers, cancel).await?;
        assert_eq!(
            (page.selected, page.processed, page.next_offset),
            (0, 0, None)
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn execution_refuses_an_unavailable_medium_before_any_call() -> TestResult {
        let pool = reset_pool().await?;
        seed_ready_words(&pool).await?;
        let providers = MediaProviders::default();
        let (_stop, cancel) = never_cancelled();
        let images =
            PrefetchOptions::new(100, 0, 1024, RequestedMedia::Images, PrefetchMode::Execute)?;
        let refused = prefetch_media(
            &pool,
            &Langs::new("en", "fr"),
            &images,
            &providers,
            cancel.clone(),
        )
        .await;
        assert!(
            matches!(refused, Err(wisecrow::errors::WisecrowError::MediaError(_))),
            "{refused:?}"
        );
        // Breton has no speech voice: audio execution is refused, preview
        // reports it unsupported rather than ready.
        sqlx::query("INSERT INTO languages (code, name) VALUES ('br', 'Breton')")
            .execute(&pool)
            .await?;
        let audio =
            PrefetchOptions::new(100, 0, 1024, RequestedMedia::Audio, PrefetchMode::Execute)?;
        let refused = prefetch_media(
            &pool,
            &Langs::new("en", "br"),
            &audio,
            &providers,
            cancel.clone(),
        )
        .await;
        assert!(
            matches!(refused, Err(wisecrow::errors::WisecrowError::MediaError(_))),
            "{refused:?}"
        );
        let unsupported =
            prefetch_media(&pool, &Langs::new("en", "xx"), &audio, &providers, cancel).await;
        assert!(
            unsupported.is_err(),
            "unsupported code rejected at the library boundary"
        );
        Ok(())
    }
}
