//! Grammar attempts and the two mastery projections.
//!
//! The attempt stream is the record and both mastery numbers are projections
//! of it. These tests pin that relationship down: an attempt recorded twice
//! must count once, and replaying from a baseline must reproduce exactly what
//! the pure code in `wisecrow-learning` computes.

use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use wisecrow::grammar::mastery::{AttemptRecord, AttemptSource, MasteryRepository};
use wisecrow_learning::mastery::accuracy;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    // `users` is deliberately absent: migration 007 seeds a default user that
    // other suites depend on, and truncating the table would take it with them.
    // Each test here creates its own learner instead.
    sqlx::query(
        "TRUNCATE languages, grammar_rules, grammar_rule_aliases, quiz_items,
                  grammar_sessions CASCADE",
    )
    .execute(&pool)
    .await?;
    Ok(pool)
}

struct Fixture {
    user_id: i32,
    rule_id: i32,
    item_id: i32,
    session_id: Uuid,
}

async fn seed(pool: &PgPool) -> Result<Fixture, Box<dyn std::error::Error>> {
    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (display_name) VALUES ('Learner') RETURNING id")
            .fetch_one(pool)
            .await?;
    let language_id: i32 = sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ('es', 'Spanish') RETURNING id",
    )
    .fetch_one(pool)
    .await?;
    let level_id: i32 = sqlx::query_scalar("SELECT id FROM cefr_levels WHERE code = 'A1'")
        .fetch_one(pool)
        .await?;
    let rule_id: i32 = sqlx::query_scalar(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         VALUES ($1, $2, 'ser-vs-estar', 'Ser vs estar', 'x', 'llm')
         RETURNING id",
    )
    .bind(language_id)
    .bind(level_id)
    .fetch_one(pool)
    .await?;
    let item_id: i32 = sqlx::query_scalar(
        "INSERT INTO quiz_items (rule_id, kind, prompt, answer, accepted, status, content_sha256)
         VALUES ($1, 'cloze', 'Yo ___ cansado.', 'estoy', '[]'::jsonb, 'active', $2)
         RETURNING id",
    )
    .bind(rule_id)
    .bind("a".repeat(64))
    .fetch_one(pool)
    .await?;
    let session_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO grammar_sessions (id, user_id, language_id, cefr_level_id, kind)
         VALUES ($1, $2, $3, $4, 'practice')",
    )
    .bind(session_id)
    .bind(user_id)
    .bind(language_id)
    .bind(level_id)
    .execute(pool)
    .await?;

    Ok(Fixture {
        user_id,
        rule_id,
        item_id,
        session_id,
    })
}

async fn mastery_of(
    pool: &PgPool,
    user_id: i32,
    slug: &str,
) -> Result<wisecrow::grammar::mastery::MasteryRow, Box<dyn std::error::Error>> {
    let rows = MasteryRepository::mastery_for_language(pool, user_id, "es").await?;
    rows.into_iter()
        .find(|row| row.slug == slug)
        .ok_or_else(|| format!("no mastery row for {slug}").into())
}

fn at(offset: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0)
        .single()
        .expect("valid timestamp")
        + Duration::seconds(offset)
}

fn attempt(
    fixture: &Fixture,
    ordinal: i16,
    correct: bool,
    hint: bool,
    offset: i64,
) -> AttemptRecord {
    AttemptRecord {
        event_id: Uuid::new_v4(),
        session_id: fixture.session_id,
        device_id: None,
        rule_id: fixture.rule_id,
        item_id: fixture.item_id,
        item_revision: 1,
        ordinal,
        correct,
        hint_shown: hint,
        occurred_at: at(offset),
        source: AttemptSource::Web,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_schema_carries_sessions_attempts_and_both_projections() -> TestResult {
    let pool = reset_pool().await?;

    for table in [
        "grammar_sessions",
        "grammar_attempts",
        "grammar_mastery",
        "grammar_review_baselines",
        "placement_attempts",
    ] {
        let present: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(table)
            .fetch_one(&pool)
            .await?;
        assert!(present, "{table} is missing");
    }

    let key: Vec<String> = sqlx::query_scalar(
        "SELECT a.attname
         FROM pg_index i
         JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
         WHERE i.indrelid = 'grammar_attempts'::regclass AND i.indisprimary
         ORDER BY a.attname",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        key,
        vec![String::from("event_id"), String::from("user_id")],
        "an attempt is identified by its user and client-generated event"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_schema_refuses_impossible_projection_values() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed(&pool).await?;

    let bad_state =
        sqlx::query("INSERT INTO grammar_mastery (user_id, rule_id, state) VALUES ($1, $2, 9)")
            .bind(fixture.user_id)
            .bind(fixture.rule_id)
            .execute(&pool)
            .await;
    assert!(bad_state.is_err(), "FSRS state is confined to 0..=3");

    let bad_accuracy = sqlx::query(
        "INSERT INTO grammar_mastery (user_id, rule_id, accuracy) VALUES ($1, $2, 1.5)",
    )
    .bind(fixture.user_id)
    .bind(fixture.rule_id)
    .execute(&pool)
    .await;
    assert!(bad_accuracy.is_err(), "accuracy is a proportion");

    let bad_ordinal = sqlx::query(
        "INSERT INTO grammar_attempts
             (user_id, event_id, session_id, rule_id, item_id, item_revision, ordinal,
              correct, occurred_at, source)
         VALUES ($1, $2, $3, $4, $5, 1, 0, TRUE, CURRENT_TIMESTAMP, 'web')",
    )
    .bind(fixture.user_id)
    .bind(Uuid::new_v4())
    .bind(fixture.session_id)
    .bind(fixture.rule_id)
    .bind(fixture.item_id)
    .execute(&pool)
    .await;
    assert!(bad_ordinal.is_err(), "an interaction's first answer is one");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_interaction_records_every_attempt_and_schedules_once() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed(&pool).await?;

    for record in [
        attempt(&fixture, 1, false, false, 10),
        attempt(&fixture, 2, false, true, 20),
        attempt(&fixture, 3, true, true, 30),
    ] {
        MasteryRepository::record_attempt(&pool, fixture.user_id, &record).await?;
    }

    let attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM grammar_attempts WHERE user_id = $1")
            .bind(fixture.user_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(attempts, 3, "accuracy consumes every submission");

    let (reps, recorded, counted): (i32, Option<f64>, i32) = sqlx::query_as(
        "SELECT reps, accuracy, attempts FROM grammar_mastery WHERE user_id = $1 AND rule_id = $2",
    )
    .bind(fixture.user_id)
    .bind(fixture.rule_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(reps, 1, "scheduling consumes one rating per interaction");
    assert_eq!(counted, 3);
    assert_eq!(recorded, Some(accuracy(&[false, false, true])));
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_same_event_recorded_twice_counts_once() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed(&pool).await?;
    let record = attempt(&fixture, 1, true, false, 10);

    MasteryRepository::record_attempt(&pool, fixture.user_id, &record).await?;
    MasteryRepository::record_attempt(&pool, fixture.user_id, &record).await?;

    let attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM grammar_attempts WHERE user_id = $1")
            .bind(fixture.user_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        attempts, 1,
        "a resent upload must not advance mastery twice"
    );

    let (reps, counted): (i32, i32) = sqlx::query_as(
        "SELECT reps, attempts FROM grammar_mastery WHERE user_id = $1 AND rule_id = $2",
    )
    .bind(fixture.user_id)
    .bind(fixture.rule_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!((reps, counted), (1, 1));
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_late_attempt_reforms_its_interaction() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed(&pool).await?;

    // The correct answer arrives first; the failure that preceded it follows,
    // as it would from a device that had been offline.
    MasteryRepository::record_attempt(
        &pool,
        fixture.user_id,
        &attempt(&fixture, 2, true, false, 20),
    )
    .await?;
    let early = mastery_of(&pool, fixture.user_id, "ser-vs-estar").await?;
    assert_eq!(early.accuracy, Some(1.0));

    MasteryRepository::record_attempt(
        &pool,
        fixture.user_id,
        &attempt(&fixture, 1, false, false, 10),
    )
    .await?;
    let reformed = mastery_of(&pool, fixture.user_id, "ser-vs-estar").await?;

    assert_eq!(reformed.attempts, 2);
    assert_eq!(reformed.accuracy, Some(accuracy(&[false, true])));
    assert_eq!(
        reformed.reps, 1,
        "the interaction is rerated, not scheduled again"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn mastery_for_a_language_reports_unattempted_points() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed(&pool).await?;
    let language_id: i32 = sqlx::query_scalar("SELECT id FROM languages WHERE code = 'es'")
        .fetch_one(&pool)
        .await?;
    let level_id: i32 = sqlx::query_scalar("SELECT id FROM cefr_levels WHERE code = 'A1'")
        .fetch_one(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         VALUES ($1, $2, 'gender-agreement', 'Gender agreement', 'x', 'llm')",
    )
    .bind(language_id)
    .bind(level_id)
    .execute(&pool)
    .await?;

    MasteryRepository::record_attempt(
        &pool,
        fixture.user_id,
        &attempt(&fixture, 1, true, false, 10),
    )
    .await?;

    let rows = MasteryRepository::mastery_for_language(&pool, fixture.user_id, "es").await?;
    assert_eq!(
        rows.len(),
        2,
        "every syllabus point appears, attempted or not"
    );

    let unattempted = rows
        .iter()
        .find(|row| row.slug == "gender-agreement")
        .expect("the untouched point");
    assert_eq!(
        unattempted.accuracy, None,
        "never attempted is never coloured"
    );
    assert_eq!(unattempted.attempts, 0);
    Ok(())
}
