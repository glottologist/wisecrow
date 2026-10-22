//! The progress dashboard's read model.
//!
//! Every figure the dashboard shows is read live from records that other paths
//! write, so these tests seed those records directly and check the roll-up:
//! two learners, two native languages on one target, every FSRS state, reviews
//! over three days, grammar attempts, a placement that passed nothing, and
//! n-back sessions that must not count.

use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use wisecrow::dto_convert;
use wisecrow::grammar::mastery::{AttemptRecord, AttemptSource, MasteryRepository};
use wisecrow::progress::{
    LanguageScope, LevelBands, NbackSummary, ProgressQuery, VocabularyCounts,
};
use wisecrow_dto::{DailyCountDto, PairNbackDto, PlacementSummaryDto, VocabularyCountsDto};

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn test_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_review_log_is_indexed_by_learner_and_time() -> TestResult {
    let pool = test_pool().await?;
    let definition: Option<String> = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes
         WHERE tablename = 'review_events' AND indexname = 'idx_review_events_user_time'",
    )
    .fetch_optional(&pool)
    .await?;
    let definition = definition.ok_or("idx_review_events_user_time is missing")?;
    assert!(
        definition.ends_with("(user_id, occurred_at)"),
        "unexpected index definition: {definition}"
    );
    Ok(())
}

const EN: &str = "zqen";
const CY: &str = "zqcy";
const ES: &str = "zqes";
const IT: &str = "zqit";
const LEARNER: &str = "progress-learner@test.local";
const OTHER: &str = "progress-other@test.local";

/// The two learners the fixture creates.
struct Seeded {
    learner: i32,
    other: i32,
}

fn midnight(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(NaiveTime::MIN).and_utc()
}

/// When the learner's finished placement run completed: midnight yesterday.
fn placement_time(today: NaiveDate) -> DateTime<Utc> {
    midnight(today - Duration::days(1))
}

/// Removes everything a previous run left behind.
///
/// Learners go first: deleting them removes their grammar attempts, which
/// would otherwise restrict the deletion of the quiz items the languages
/// cascade into.
async fn cleanup(pool: &PgPool) -> TestResult {
    sqlx::query("DELETE FROM users WHERE email LIKE 'progress-%@test.local'")
        .execute(pool)
        .await?;
    // The corpus-change trigger reads language codes off the translation being
    // removed, so the languages have to still be there when the rows go.
    sqlx::query(
        "DELETE FROM translations
         WHERE from_language_id IN (SELECT id FROM languages WHERE code LIKE 'zq%')
            OR to_language_id IN (SELECT id FROM languages WHERE code LIKE 'zq%')",
    )
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM languages WHERE code LIKE 'zq%'")
        .execute(pool)
        .await?;
    Ok(())
}

async fn language(pool: &PgPool, code: &str) -> Result<i32, Box<dyn std::error::Error>> {
    Ok(sqlx::query_scalar::<_, i32>(
        "INSERT INTO languages (code, name) VALUES ($1, $1) RETURNING id",
    )
    .bind(code)
    .fetch_one(pool)
    .await?)
}

async fn account(pool: &PgPool, email: &str) -> Result<i32, Box<dyn std::error::Error>> {
    Ok(sqlx::query_scalar::<_, i32>(
        "INSERT INTO users (display_name, email) VALUES ('Progress', $1) RETURNING id",
    )
    .bind(email)
    .fetch_one(pool)
    .await?)
}

/// Gives a learner a card in one state, creating the translation if needed,
/// and returns the translation.
async fn card(
    pool: &PgPool,
    user_id: i32,
    (from, to): (i32, i32),
    word: &str,
    state: i16,
    due: DateTime<Utc>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let translation_id: i32 = sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, from_phrase, to_language_id, to_phrase)
         VALUES ($1, $2, $3, $2)
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
         DO UPDATE SET to_phrase = EXCLUDED.to_phrase
         RETURNING id",
    )
    .bind(from)
    .bind(word)
    .bind(to)
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO cards (user_id, translation_id, state, due) VALUES ($1, $2, $3, $4)")
        .bind(user_id)
        .bind(translation_id)
        .bind(state)
        .bind(due)
        .execute(pool)
        .await?;
    Ok(translation_id)
}

async fn reviews(
    pool: &PgPool,
    user_id: i32,
    translation_id: i32,
    occurred_at: DateTime<Utc>,
    count: usize,
) -> TestResult {
    for _ in 0..count {
        sqlx::query(
            "INSERT INTO review_events (user_id, event_id, translation_id, rating, occurred_at, source)
             VALUES ($1, $2, $3, 3, $4, 'web')",
        )
        .bind(user_id)
        .bind(Uuid::new_v4())
        .bind(translation_id)
        .bind(occurred_at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn flashcard_session(pool: &PgPool, user_id: i32, native: &str, foreign: &str) -> TestResult {
    sqlx::query(
        "INSERT INTO sessions (user_id, native_lang, foreign_lang, deck_size) VALUES ($1, $2, $3, 10)",
    )
    .bind(user_id)
    .bind(native)
    .bind(foreign)
    .execute(pool)
    .await?;
    Ok(())
}

async fn nback(
    pool: &PgPool,
    user_id: i32,
    (native, foreign): (&str, &str),
    peak: i16,
    trials: i32,
    completed: bool,
) -> TestResult {
    sqlx::query(
        "INSERT INTO dnb_sessions
             (user_id, native_lang, foreign_lang, mode, n_level_peak, trials_completed,
              interval_ms_start, interval_ms_end, completed_at)
         VALUES ($1, $2, $3, 'audio_written', $4, $5, 3000, 3000,
                 CASE WHEN $6 THEN CURRENT_TIMESTAMP END)",
    )
    .bind(user_id)
    .bind(native)
    .bind(foreign)
    .bind(peak)
    .bind(trials)
    .bind(completed)
    .execute(pool)
    .await?;
    Ok(())
}

/// Creates a grammar point with one active item, returning `(rule, item)`.
async fn rule(
    pool: &PgPool,
    language_id: i32,
    level: &str,
    slug: &str,
) -> Result<(i32, i32), Box<dyn std::error::Error>> {
    let rule_id: i32 = sqlx::query_scalar(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         SELECT $1, cl.id, $2, $2, 'why', 'llm' FROM cefr_levels cl WHERE cl.code = $3
         RETURNING id",
    )
    .bind(language_id)
    .bind(slug)
    .bind(level)
    .fetch_one(pool)
    .await?;
    let item_id: i32 = sqlx::query_scalar(
        "INSERT INTO quiz_items (rule_id, kind, prompt, answer, accepted, status, content_sha256)
         VALUES ($1, 'cloze', 'Yo ___ cansado.', 'estoy', '[]'::jsonb, 'active', $2)
         RETURNING id",
    )
    .bind(rule_id)
    .bind(format!("{:0>64}", format!("{rule_id:x}")))
    .fetch_one(pool)
    .await?;
    Ok((rule_id, item_id))
}

async fn grammar_session(
    pool: &PgPool,
    user_id: i32,
    language_id: i32,
    kind: &str,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO grammar_sessions (id, user_id, language_id, kind) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(user_id)
    .bind(language_id)
    .bind(kind)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Records answers through the real attempt path, so the mastery projection
/// is rebuilt exactly as a learner's own answers would rebuild it.
async fn answer(
    pool: &PgPool,
    user_id: i32,
    session_id: Uuid,
    (rule_id, item_id): (i32, i32),
    outcomes: &[bool],
    from: DateTime<Utc>,
) -> TestResult {
    for (index, &correct) in outcomes.iter().enumerate() {
        let ordinal = i16::try_from(index + 1)?;
        let record = AttemptRecord {
            event_id: Uuid::new_v4(),
            session_id,
            device_id: None,
            rule_id,
            item_id,
            item_revision: 1,
            ordinal,
            correct,
            hint_shown: false,
            occurred_at: from + Duration::seconds(i64::from(ordinal)),
            source: AttemptSource::Web,
        };
        MasteryRepository::record_attempt(pool, user_id, &record).await?;
    }
    Ok(())
}

async fn placement(
    pool: &PgPool,
    user_id: i32,
    language_id: i32,
    session_id: Uuid,
    completed_at: Option<DateTime<Utc>>,
) -> TestResult {
    sqlx::query(
        "INSERT INTO placement_attempts (id, user_id, language_id, session_id, completed_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(language_id)
    .bind(session_id)
    .bind(completed_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed(pool: &PgPool) -> Result<Seeded, Box<dyn std::error::Error>> {
    cleanup(pool).await?;
    let en = language(pool, EN).await?;
    let cy = language(pool, CY).await?;
    let es = language(pool, ES).await?;
    let learner = account(pool, LEARNER).await?;
    let other = account(pool, OTHER).await?;
    let now = Utc::now();
    let today = now.date_naive();
    let overdue = now - Duration::days(1);
    let later = now + Duration::days(30);

    // English to Spanish holds one card in each FSRS state; the new and
    // relearning cards are overdue, the other two are not.
    let uno = card(pool, learner, (en, es), "uno", 0, overdue).await?;
    let dos = card(pool, learner, (en, es), "dos", 1, later).await?;
    card(pool, learner, (en, es), "tres", 2, later).await?;
    card(pool, learner, (en, es), "cuatro", 3, overdue).await?;
    // A second native language on the same target.
    let cinco = card(pool, learner, (cy, es), "cinco", 0, later).await?;
    flashcard_session(pool, learner, CY, ES).await?;

    // Twelve reviews over three days: nine from English, three from Welsh.
    let two_days_ago = midnight(today - Duration::days(2)) + Duration::hours(12);
    let yesterday = midnight(today - Duration::days(1)) + Duration::hours(12);
    reviews(pool, learner, uno, two_days_ago, 3).await?;
    reviews(pool, learner, dos, yesterday, 4).await?;
    reviews(pool, learner, uno, midnight(today), 2).await?;
    reviews(pool, learner, cinco, midnight(today), 3).await?;

    // Grammar: one settled success, a placement that passed nothing, and a
    // level never asked about.
    let a1_known = rule(pool, es, "A1", "zq-a1-known").await?;
    let a1_missed = rule(pool, es, "A1", "zq-a1-missed").await?;
    rule(pool, es, "A2", "zq-a2-unasked").await?;
    let b1_missed = rule(pool, es, "B1", "zq-b1-missed").await?;
    let practice = grammar_session(pool, learner, es, "practice").await?;
    answer(
        pool,
        learner,
        practice,
        a1_known,
        &[true, true, true],
        overdue,
    )
    .await?;
    let placed_at = placement_time(today);
    let ladder = grammar_session(pool, learner, es, "placement").await?;
    answer(
        pool,
        learner,
        ladder,
        a1_missed,
        &[false, false],
        placed_at - Duration::hours(1),
    )
    .await?;
    answer(
        pool,
        learner,
        ladder,
        b1_missed,
        &[false, false],
        placed_at - Duration::minutes(30),
    )
    .await?;
    placement(pool, learner, es, ladder, Some(placed_at)).await?;
    // A newer run, abandoned part-way, must not displace the finished one.
    let abandoned = grammar_session(pool, learner, es, "placement").await?;
    placement(pool, learner, es, abandoned, None).await?;

    // N-back: only a finished session with answered trials counts.
    nback(pool, learner, (EN, ES), 4, 20, true).await?;
    nback(pool, learner, (EN, ES), 6, 5, false).await?;
    nback(pool, learner, (CY, ES), 7, 0, true).await?;
    nback(pool, learner, (EN, IT), 5, 12, false).await?;

    // The other learner shares a word and a grammar point; none of it may
    // reach the learner's figures.
    let shared = card(pool, other, (en, es), "uno", 2, overdue).await?;
    reviews(pool, other, shared, midnight(today), 5).await?;
    let theirs = grammar_session(pool, other, es, "practice").await?;
    answer(pool, other, theirs, a1_missed, &[true], overdue).await?;
    nback(pool, other, (EN, IT), 3, 10, true).await?;

    Ok(Seeded { learner, other })
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn languages_are_discovered_per_learner_from_every_kind_of_progress() -> TestResult {
    let pool = test_pool().await?;
    let seeded = seed(&pool).await?;

    let learner = ProgressQuery::languages_for_user(&pool, seeded.learner).await?;
    assert_eq!(
        learner,
        vec![LanguageScope {
            foreign: ES.into(),
            foreign_name: ES.into(),
            natives: vec![CY.into(), EN.into()],
        }],
        "an abandoned n-back session alone does not put Italian on the dashboard"
    );

    let other = ProgressQuery::languages_for_user(&pool, seeded.other).await?;
    assert_eq!(
        other,
        vec![
            LanguageScope {
                foreign: ES.into(),
                foreign_name: ES.into(),
                natives: vec![EN.into()],
            },
            LanguageScope {
                foreign: IT.into(),
                foreign_name: IT.into(),
                natives: Vec::new(),
            },
        ]
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn vocabulary_is_counted_by_state_and_reviews_by_utc_day_per_native() -> TestResult {
    let pool = test_pool().await?;
    let seeded = seed(&pool).await?;
    let today = Utc::now().date_naive();

    let english = ProgressQuery::vocabulary_counts(&pool, seeded.learner, EN, ES).await?;
    assert_eq!(
        english,
        VocabularyCounts {
            new: 1,
            learning: 1,
            review: 1,
            relearning: 1,
            due_today: 2,
        },
        "the other learner's overdue card on the same word is not the learner's"
    );
    let welsh = ProgressQuery::vocabulary_counts(&pool, seeded.learner, CY, ES).await?;
    assert_eq!(
        welsh,
        VocabularyCounts {
            new: 1,
            ..VocabularyCounts::default()
        }
    );

    let strip = ProgressQuery::daily_reviews(&pool, seeded.learner, EN, ES, 30).await?;
    assert_eq!(strip.len(), 30);
    assert_eq!(strip.last().map(|entry| entry.day), Some(today));
    let counts: Vec<i64> = strip.iter().map(|entry| entry.count).collect();
    assert_eq!(
        &counts[27..],
        &[3, 4, 2],
        "the other learner's five reviews today are not counted"
    );
    assert_eq!(counts.iter().sum::<i64>(), 9);

    let welsh_strip = ProgressQuery::daily_reviews(&pool, seeded.learner, CY, ES, 30).await?;
    assert_eq!(welsh_strip.iter().map(|entry| entry.count).sum::<i64>(), 3);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn grammar_placement_and_nback_report_only_what_the_learner_did() -> TestResult {
    let pool = test_pool().await?;
    let seeded = seed(&pool).await?;
    let today = Utc::now().date_naive();

    let bands = ProgressQuery::grammar_bands(&pool, seeded.learner, ES).await?;
    assert_eq!(
        bands,
        vec![
            LevelBands {
                level: "A1".into(),
                green: 1,
                provisional_red: 1,
                ..LevelBands::default()
            },
            LevelBands {
                level: "A2".into(),
                unseen: 1,
                ..LevelBands::default()
            },
            LevelBands {
                level: "B1".into(),
                provisional_red: 1,
                ..LevelBands::default()
            },
        ],
        "the other learner's correct answer on the missed point is theirs alone"
    );

    let placement = ProgressQuery::placement_summary(&pool, seeded.learner, ES)
        .await?
        .ok_or("the finished placement is missing")?;
    assert_eq!(placement.completed_at, placement_time(today));
    assert_eq!(
        placement.outcome.level_reached, None,
        "a run that passed nothing is still a result"
    );
    let tested: Vec<&str> = placement
        .outcome
        .tested
        .iter()
        .map(|score| score.level.as_str())
        .collect();
    assert_eq!(tested, ["A1", "B1"]);
    assert_eq!(placement.outcome.skipped, ["A2"]);
    assert!(ProgressQuery::placement_summary(&pool, seeded.other, ES)
        .await?
        .is_none());

    let nback = ProgressQuery::nback_summary(&pool, seeded.learner, ES).await?;
    assert_eq!(
        nback,
        vec![NbackSummary {
            native: EN.into(),
            highest_n: 4,
            sessions: 1,
        }],
        "abandoned and unanswered sessions do not count"
    );
    assert!(ProgressQuery::nback_summary(&pool, seeded.learner, IT)
        .await?
        .is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_dashboard_dto_carries_every_figure_for_the_learner_alone() -> TestResult {
    let pool = test_pool().await?;
    let seeded = seed(&pool).await?;
    let today = Utc::now().date_naive();

    let dto = dto_convert::dashboard(&ProgressQuery::dashboard(&pool, seeded.learner).await?);
    let [spanish] = dto.languages.as_slice() else {
        panic!("one language expected, got {:?}", dto.languages);
    };
    assert_eq!(spanish.foreign, ES);
    assert_eq!(
        spanish.foreign_name, ES,
        "an unlisted code is named by its code"
    );
    let natives: Vec<&str> = spanish
        .vocabulary
        .iter()
        .map(|pair| pair.native.as_str())
        .collect();
    assert_eq!(natives, [CY, EN]);
    let welsh = &spanish.vocabulary[0];
    let english = &spanish.vocabulary[1];
    assert_eq!(
        welsh.counts,
        VocabularyCountsDto {
            new: 1,
            ..VocabularyCountsDto::default()
        }
    );
    assert_eq!(
        english.counts,
        VocabularyCountsDto {
            new: 1,
            learning: 1,
            review: 1,
            relearning: 1,
            due_today: 2,
        }
    );
    assert_eq!(english.reviews.len(), 30);
    assert_eq!(
        english.reviews.last(),
        Some(&DailyCountDto {
            day: today,
            count: 2
        })
    );
    let levels: Vec<&str> = spanish
        .grammar
        .iter()
        .map(|level| level.level.as_str())
        .collect();
    assert_eq!(levels, ["A1", "A2", "B1"]);
    assert_eq!(spanish.grammar[0].green, 1);
    assert_eq!(
        spanish.placement,
        Some(PlacementSummaryDto {
            completed_at: placement_time(today),
            level_reached: None,
            tested: vec!["A1".into(), "B1".into()],
            skipped: vec!["A2".into()],
        })
    );
    assert_eq!(
        spanish.nback,
        vec![PairNbackDto {
            native: EN.into(),
            highest_n: 4,
            sessions: 1,
        }]
    );

    let theirs = dto_convert::dashboard(&ProgressQuery::dashboard(&pool, seeded.other).await?);
    let foreigns: Vec<&str> = theirs
        .languages
        .iter()
        .map(|language| language.foreign.as_str())
        .collect();
    assert_eq!(foreigns, [ES, IT]);
    assert_eq!(
        theirs.languages[0].vocabulary[0].counts,
        VocabularyCountsDto {
            review: 1,
            due_today: 1,
            ..VocabularyCountsDto::default()
        }
    );
    assert_eq!(theirs.languages[0].placement, None);
    assert_eq!(
        theirs.languages[1].nback,
        vec![PairNbackDto {
            native: EN.into(),
            highest_n: 3,
            sessions: 1,
        }]
    );
    Ok(())
}
