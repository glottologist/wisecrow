//! Which grammar point a session serves next, and which item of it.
//!
//! The ordering is the feature: a learner who keeps failing a point should
//! meet it again before one they have merely not seen lately, and a point
//! actually due should come before either.

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use wisecrow::errors::WisecrowError;
use wisecrow::grammar::mastery::AttemptSource;
use wisecrow::grammar::selection::{record_exposure, select_practice};
use wisecrow::grammar::session::{
    GrammarSessionManager, SessionKind, SubmissionContext, SESSION_LENGTH,
};
use wisecrow_learning::grading::{Answer, Submission};

fn context() -> SubmissionContext {
    SubmissionContext {
        occurred_at: Utc::now(),
        device_id: None,
        source: AttemptSource::Web,
    }
}

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

async fn seed_rule(
    pool: &PgPool,
    slug: &str,
    level: &str,
) -> Result<i32, Box<dyn std::error::Error>> {
    let rule_id: i32 = sqlx::query_scalar(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         SELECT l.id, cl.id, $1, $2, 'why', 'llm'
         FROM languages l, cefr_levels cl
         WHERE l.code = 'es' AND cl.code = $3
         RETURNING id",
    )
    .bind(slug)
    .bind(slug)
    .bind(level)
    .fetch_one(pool)
    .await?;
    Ok(rule_id)
}

/// Adds one item to a rule. The hash only has to be unique per rule.
async fn seed_item(
    pool: &PgPool,
    rule_id: i32,
    marker: &str,
    status: &str,
) -> Result<i32, Box<dyn std::error::Error>> {
    let item_id: i32 = sqlx::query_scalar(
        "INSERT INTO quiz_items (rule_id, kind, prompt, answer, accepted, status, content_sha256)
         VALUES ($1, 'cloze', $2, 'estoy', '[]'::jsonb, $3, $4)
         RETURNING id",
    )
    .bind(rule_id)
    .bind(format!("Yo ___ {marker}."))
    .bind(status)
    .bind(format!("{marker:0>64}").replace(|c: char| !c.is_ascii_hexdigit(), "0"))
    .fetch_one(pool)
    .await?;
    Ok(item_id)
}

async fn seed_mastery(
    pool: &PgPool,
    user_id: i32,
    rule_id: i32,
    accuracy: f64,
    due: DateTime<Utc>,
) -> TestResult {
    sqlx::query(
        "INSERT INTO grammar_mastery (user_id, rule_id, accuracy, attempts, reps, due)
         VALUES ($1, $2, $3, 5, 5, $4)",
    )
    .bind(user_id)
    .bind(rule_id)
    .bind(accuracy)
    .bind(due)
    .execute(pool)
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn due_beats_unseen_which_beats_low_accuracy() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    let now = Utc::now();

    // Slugs are deliberately in the opposite alphabetical order to the one
    // expected, so that a selection falling back to syllabus order fails.
    let due = seed_rule(&pool, "zdue", "A1").await?;
    let unseen = seed_rule(&pool, "munseen", "A1").await?;
    let low = seed_rule(&pool, "alow", "A1").await?;
    let high = seed_rule(&pool, "bhigh", "A1").await?;
    for (index, rule) in [due, unseen, low, high].into_iter().enumerate() {
        seed_item(&pool, rule, &format!("a{index}"), "active").await?;
    }

    seed_mastery(&pool, user_id, due, 0.9, now - Duration::days(1)).await?;
    seed_mastery(&pool, user_id, low, 0.2, now + Duration::days(7)).await?;
    seed_mastery(&pool, user_id, high, 0.9, now + Duration::days(7)).await?;

    let selected = select_practice(&pool, user_id, "es", None, 10).await?;
    let order: Vec<&str> = selected.iter().map(|item| item.slug.as_str()).collect();
    assert_eq!(order, vec!["zdue", "munseen", "alow", "bhigh"]);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn equally_due_points_surface_the_weaker_one_first() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    let overdue = Utc::now() - Duration::days(1);

    let strong = seed_rule(&pool, "astrong", "A1").await?;
    let weak = seed_rule(&pool, "bweak", "A1").await?;
    seed_item(&pool, strong, "a0", "active").await?;
    seed_item(&pool, weak, "a1", "active").await?;
    seed_mastery(&pool, user_id, strong, 0.95, overdue).await?;
    seed_mastery(&pool, user_id, weak, 0.30, overdue).await?;

    let selected = select_practice(&pool, user_id, "es", None, 10).await?;
    let order: Vec<&str> = selected.iter().map(|item| item.slug.as_str()).collect();
    assert_eq!(order, vec!["bweak", "astrong"]);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_promoted_items_reach_a_learner() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;

    let ungated = seed_rule(&pool, "aungated", "A1").await?;
    let ready = seed_rule(&pool, "bready", "A1").await?;
    seed_item(&pool, ungated, "a0", "candidate").await?;
    seed_item(&pool, ungated, "a1", "rejected").await?;
    let active = seed_item(&pool, ready, "a2", "active").await?;
    seed_item(&pool, ready, "a3", "retired").await?;

    let selected = select_practice(&pool, user_id, "es", None, 10).await?;
    assert_eq!(
        selected.len(),
        1,
        "a rule with no active item is not served"
    );
    assert_eq!(selected[0].slug, "bready");
    assert_eq!(selected[0].item_id, active);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn exposure_spreads_across_a_rules_bank() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    let rule_id = seed_rule(&pool, "aonly", "A1").await?;
    let first = seed_item(&pool, rule_id, "a0", "active").await?;
    let second = seed_item(&pool, rule_id, "a1", "active").await?;

    let unserved = select_practice(&pool, user_id, "es", None, 10).await?;
    assert_eq!(
        unserved[0].item_id, first,
        "with nothing served, the lowest identifier breaks the tie"
    );

    record_exposure(&pool, user_id, first).await?;
    let after_first = select_practice(&pool, user_id, "es", None, 10).await?;
    assert_eq!(
        after_first[0].item_id, second,
        "an unserved item comes first"
    );

    record_exposure(&pool, user_id, second).await?;
    let after_both = select_practice(&pool, user_id, "es", None, 10).await?;
    assert_eq!(
        after_both[0].item_id, first,
        "with both served, the least recent comes first"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_level_restricts_selection_and_a_limit_bounds_it() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;

    for (slug, level) in [("aone", "A1"), ("btwo", "A1"), ("cthree", "A2")] {
        let rule_id = seed_rule(&pool, slug, level).await?;
        seed_item(&pool, rule_id, slug, "active").await?;
    }

    let elementary = select_practice(&pool, user_id, "es", Some("A2"), 10).await?;
    let slugs: Vec<&str> = elementary.iter().map(|item| item.slug.as_str()).collect();
    assert_eq!(slugs, vec!["cthree"]);

    let bounded = select_practice(&pool, user_id, "es", None, 2).await?;
    assert_eq!(bounded.len(), 2);
    Ok(())
}

// --- Session lifecycle ---

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_session_serves_items_records_answers_and_closes() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    for slug in ["aone", "btwo", "cthree"] {
        let rule_id = seed_rule(&pool, slug, "A1").await?;
        seed_item(&pool, rule_id, slug, "active").await?;
    }

    let session = GrammarSessionManager::create(
        &pool,
        user_id,
        "es",
        None,
        SessionKind::Practice,
        SESSION_LENGTH,
    )
    .await?
    .expect("a bank with active items yields a session");
    assert_eq!(session.items.len(), 3);

    let kind: String = sqlx::query_scalar("SELECT kind FROM grammar_sessions WHERE id = $1")
        .bind(session.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(kind, "practice");

    for (index, item) in session.items.iter().enumerate() {
        // The first answer is wrong, the rest right, so the recorded verdicts
        // cannot all have come from the same branch.
        let typed = if index == 0 { "nope" } else { "ESTOY" };
        let verdict = GrammarSessionManager::submit(
            &pool,
            user_id,
            &Submission {
                item_id: item.item_id,
                revision: item.revision,
                session_id: session.id,
                event_id: Uuid::new_v4(),
                answer: Answer::Text(String::from(typed)),
                hint_shown: false,
                ordinal: 1,
            },
            context(),
        )
        .await?;
        assert_eq!(verdict.correct, index != 0);
    }

    let recorded: Vec<(bool, i16)> = sqlx::query_as(
        "SELECT correct, ordinal FROM grammar_attempts WHERE session_id = $1 ORDER BY occurred_at",
    )
    .bind(session.id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(recorded.len(), 3);
    assert_eq!(recorded.iter().filter(|(correct, _)| *correct).count(), 2);

    GrammarSessionManager::complete(&pool, user_id, session.id).await?;
    let completed: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT completed_at FROM grammar_sessions WHERE id = $1")
            .bind(session.id)
            .fetch_one(&pool)
            .await?;
    assert!(completed.is_some());
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_completed_session_accepts_nothing_further() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    let rule_id = seed_rule(&pool, "aone", "A1").await?;
    seed_item(&pool, rule_id, "aone", "active").await?;

    let session =
        GrammarSessionManager::create(&pool, user_id, "es", None, SessionKind::Practice, 12)
            .await?
            .expect("session");
    let item = &session.items[0];
    GrammarSessionManager::complete(&pool, user_id, session.id).await?;

    let refused = GrammarSessionManager::submit(
        &pool,
        user_id,
        &Submission {
            item_id: item.item_id,
            revision: item.revision,
            session_id: session.id,
            event_id: Uuid::new_v4(),
            answer: Answer::Text(String::from("estoy")),
            hint_shown: false,
            ordinal: 1,
        },
        context(),
    )
    .await;
    assert!(
        matches!(refused, Err(WisecrowError::Conflict(_))),
        "a closed session cannot gain attempts: {refused:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_submission_naming_another_revision_is_refused() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    let rule_id = seed_rule(&pool, "aone", "A1").await?;
    seed_item(&pool, rule_id, "aone", "active").await?;

    let session =
        GrammarSessionManager::create(&pool, user_id, "es", None, SessionKind::Practice, 12)
            .await?
            .expect("session");
    let item = &session.items[0];

    let refused = GrammarSessionManager::submit(
        &pool,
        user_id,
        &Submission {
            item_id: item.item_id,
            revision: item.revision.saturating_add(1),
            session_id: session.id,
            event_id: Uuid::new_v4(),
            answer: Answer::Text(String::from("estoy")),
            hint_shown: false,
            ordinal: 1,
        },
        context(),
    )
    .await;
    assert!(
        matches!(refused, Err(WisecrowError::Conflict(_))),
        "grading must happen against the revision the learner saw: {refused:?}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_language_without_promoted_items_yields_no_session() -> TestResult {
    let pool = reset_pool().await?;
    let user_id = seed_user(&pool).await?;
    let rule_id = seed_rule(&pool, "aone", "A1").await?;
    seed_item(&pool, rule_id, "aone", "candidate").await?;

    let session =
        GrammarSessionManager::create(&pool, user_id, "es", None, SessionKind::Practice, 12)
            .await?;
    assert!(session.is_none(), "an empty bank is a state, not a failure");

    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM grammar_sessions")
        .fetch_one(&pool)
        .await?;
    assert_eq!(sessions, 0, "no session row is written for an empty bank");
    Ok(())
}
