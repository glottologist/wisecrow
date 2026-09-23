//! The placement ladder: how far it climbs, when it stops, and what it
//! refuses to assume.
//!
//! The discipline being tested is negative. A placement must not invent
//! knowledge: passing A2 says nothing about the A1 points it never asked
//! about, and a level whose bank is too thin to judge is untested rather than
//! failed.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;
use wisecrow::grammar::mastery::AttemptSource;
use wisecrow::grammar::placement::{self, PlacementState};
use wisecrow::grammar::session::SubmissionContext;
use wisecrow_learning::grading::{Answer, Submission};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ANSWER: &str = "estoy";

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

fn context() -> SubmissionContext {
    SubmissionContext {
        occurred_at: Utc::now(),
        device_id: None,
        source: AttemptSource::Web,
    }
}

async fn seed_user(pool: &PgPool) -> Result<i32, Box<dyn std::error::Error>> {
    let user_id: i32 =
        sqlx::query_scalar("INSERT INTO users (display_name) VALUES ('Learner') RETURNING id")
            .fetch_one(pool)
            .await?;
    sqlx::query("INSERT INTO languages (code, name) VALUES ('es', 'Spanish')")
        .execute(pool)
        .await?;
    Ok(user_id)
}

/// Gives a level `count` points, each with one promoted item.
async fn seed_level(pool: &PgPool, level: &str, count: usize) -> TestResult {
    for index in 0..count {
        let slug = format!("{}-{index}", level.to_lowercase());
        let rule_id: i32 = sqlx::query_scalar(
            "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
             SELECT l.id, cl.id, $1, $1, 'why', 'llm'
             FROM languages l, cefr_levels cl
             WHERE l.code = 'es' AND cl.code = $2
             RETURNING id",
        )
        .bind(&slug)
        .bind(level)
        .fetch_one(pool)
        .await?;

        sqlx::query(
            "INSERT INTO quiz_items
                 (rule_id, kind, prompt, answer, accepted, status, content_sha256)
             VALUES ($1, 'cloze', 'Yo ___ cansado.', $2, '[]'::jsonb, 'active', $3)",
        )
        .bind(rule_id)
        .bind(ANSWER)
        .bind(format!("{:0>64}", format!("{rule_id:x}")))
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Answers a step, getting the first `correct` items right.
fn answers(step: &placement::PlacementStep, correct: usize) -> Vec<Submission> {
    step.items
        .iter()
        .enumerate()
        .map(|(index, item)| Submission {
            item_id: item.item_id,
            revision: item.revision,
            session_id: step.session_id,
            event_id: Uuid::new_v4(),
            answer: Answer::Text(String::from(if index < correct { ANSWER } else { "nope" })),
            hint_shown: false,
            ordinal: 1,
        })
        .collect()
}

fn testing(state: PlacementState) -> placement::PlacementStep {
    match state {
        PlacementState::Testing(step) => step,
        PlacementState::Finished(outcome) => panic!("expected another level, got {outcome:?}"),
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_ladder_reports_the_highest_level_passed_and_stops_there() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    for level in ["A1", "A2", "B1", "B2", "C1"] {
        seed_level(&pool, level, 6).await?;
    }

    let first = testing(placement::start(&pool, user_id, "es").await?);
    assert_eq!(first.level, "A1");

    // A1 and A2 pass at four of six; B1 and B2 fail at three and two.
    let second = testing(
        placement::submit(
            &pool,
            user_id,
            first.attempt_id,
            &answers(&first, 4),
            context(),
        )
        .await?,
    );
    assert_eq!(second.level, "A2");

    let third = testing(
        placement::submit(
            &pool,
            user_id,
            second.attempt_id,
            &answers(&second, 4),
            context(),
        )
        .await?,
    );
    assert_eq!(third.level, "B1");

    let fourth = testing(
        placement::submit(
            &pool,
            user_id,
            third.attempt_id,
            &answers(&third, 3),
            context(),
        )
        .await?,
    );
    assert_eq!(fourth.level, "B2");

    let finished = placement::submit(
        &pool,
        user_id,
        fourth.attempt_id,
        &answers(&fourth, 2),
        context(),
    )
    .await?;
    let PlacementState::Finished(outcome) = finished else {
        panic!("two failures in a row must end the ladder");
    };

    assert_eq!(outcome.level_reached.as_deref(), Some("A2"));
    assert_eq!(
        outcome
            .tested
            .iter()
            .map(|score| score.level.as_str())
            .collect::<Vec<_>>(),
        vec!["A1", "A2", "B1", "B2"]
    );

    let stored: Option<String> =
        sqlx::query_scalar("SELECT level_reached FROM placement_attempts WHERE id = $1")
            .bind(outcome.attempt_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(stored.as_deref(), Some("A2"));

    let untouched: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM grammar_rules gr
         JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
         WHERE cl.code = 'C1'
           AND gr.id IN (SELECT rule_id FROM grammar_attempts WHERE user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(untouched, 0, "the ladder never drew C1");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn mastery_is_seeded_only_for_the_points_actually_tested() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    seed_level(&pool, "A1", 6).await?;
    seed_level(&pool, "A2", 6).await?;

    let first = testing(placement::start(&pool, user_id, "es").await?);
    placement::submit(
        &pool,
        user_id,
        first.attempt_id,
        &answers(&first, 6),
        context(),
    )
    .await?;

    let seeded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM grammar_mastery WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(seeded, 6, "only the six points asked about gain mastery");

    let ratings: Vec<i32> =
        sqlx::query_scalar("SELECT reps FROM grammar_mastery WHERE user_id = $1 ORDER BY rule_id")
            .bind(user_id)
            .fetch_all(&pool)
            .await?;
    assert!(
        ratings.iter().all(|reps| *reps == 1),
        "each tested point is rated exactly once"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_thin_level_is_skipped_rather_than_failed() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    seed_level(&pool, "A1", 3).await?;
    seed_level(&pool, "A2", 6).await?;

    let first = testing(placement::start(&pool, user_id, "es").await?);
    assert_eq!(first.level, "A2", "a bank of three cannot judge A1");

    let finished = placement::submit(
        &pool,
        user_id,
        first.attempt_id,
        &answers(&first, 6),
        context(),
    )
    .await?;
    let PlacementState::Finished(outcome) = finished else {
        panic!("nothing remains above A2");
    };

    assert_eq!(outcome.level_reached.as_deref(), Some("A2"));
    assert!(
        outcome.skipped.contains(&String::from("A1")),
        "A1 is reported untested: {:?}",
        outcome.skipped
    );
    assert!(
        !outcome.tested.iter().any(|score| score.level == "A1"),
        "untested is not failed"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn failing_the_first_two_levels_places_nobody() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    seed_level(&pool, "A1", 6).await?;
    seed_level(&pool, "A2", 6).await?;

    let first = testing(placement::start(&pool, user_id, "es").await?);
    let second = testing(
        placement::submit(
            &pool,
            user_id,
            first.attempt_id,
            &answers(&first, 0),
            context(),
        )
        .await?,
    );
    let finished = placement::submit(
        &pool,
        user_id,
        second.attempt_id,
        &answers(&second, 0),
        context(),
    )
    .await?;

    let PlacementState::Finished(outcome) = finished else {
        panic!("two failures end the ladder");
    };
    assert_eq!(outcome.level_reached, None);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_finished_run_accepts_nothing_further() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    seed_level(&pool, "A1", 6).await?;

    let first = testing(placement::start(&pool, user_id, "es").await?);
    let submissions = answers(&first, 6);
    placement::submit(&pool, user_id, first.attempt_id, &submissions, context()).await?;

    let refused = placement::submit(
        &pool,
        user_id,
        first.attempt_id,
        &answers(&first, 6),
        context(),
    )
    .await;
    assert!(
        refused.is_err(),
        "a closed run cannot gain answers: {refused:?}"
    );
    Ok(())
}
