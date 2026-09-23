//! Change feeds a device can follow without skipping a row.
//!
//! `BIGSERIAL` hands out a sequence number when the row is inserted, not when
//! the transaction commits, so a client that advances its cursor past a higher
//! number committed early would never come back for the lower number committed
//! late. These tests pin the visibility rule that prevents it: a change is
//! served only once every transaction that could still be holding an earlier
//! sequence has finished.

use sqlx::PgPool;
use wisecrow::grammar::sync::{visible_grammar_changes, visible_item_changes, ChangeOperation};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const PAGE: i64 = 100;

async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    // `users` is deliberately absent: migration 007 seeds a default user that
    // other suites depend on, and truncating the table would take it with them.
    sqlx::query(
        "TRUNCATE languages, grammar_rules, grammar_rule_aliases, quiz_items,
                  grammar_sessions, grammar_changes, quiz_item_changes CASCADE",
    )
    .execute(&pool)
    .await?;
    Ok(pool)
}

struct Fixture {
    user_id: i32,
    rule_a: i32,
    rule_b: i32,
}

async fn seed_two_rules(pool: &PgPool) -> Result<Fixture, Box<dyn std::error::Error>> {
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

    let mut rules = Vec::new();
    for slug in ["ser-vs-estar", "por-vs-para"] {
        let rule_id: i32 = sqlx::query_scalar(
            "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
             VALUES ($1, $2, $3, $3, 'x', 'llm')
             RETURNING id",
        )
        .bind(language_id)
        .bind(level_id)
        .bind(slug)
        .fetch_one(pool)
        .await?;
        rules.push(rule_id);
    }

    Ok(Fixture {
        user_id,
        rule_a: rules[0],
        rule_b: rules[1],
    })
}

/// Writes a mastery row inside the caller's transaction, firing the feed
/// trigger with that transaction's identifier.
async fn touch_mastery(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: i32,
    rule_id: i32,
) -> TestResult {
    sqlx::query(
        "INSERT INTO grammar_mastery (user_id, rule_id, attempts)
         VALUES ($1, $2, 1)
         ON CONFLICT (user_id, rule_id) DO UPDATE SET attempts = grammar_mastery.attempts + 1",
    )
    .bind(user_id)
    .bind(rule_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn interleaved_commits_are_never_skipped() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed_two_rules(&pool).await?;

    // Transaction A writes first but commits last.
    let mut tx_a = pool.begin().await?;
    touch_mastery(&mut tx_a, fixture.user_id, fixture.rule_a).await?;

    let mut tx_b = pool.begin().await?;
    touch_mastery(&mut tx_b, fixture.user_id, fixture.rule_b).await?;
    tx_b.commit().await?;

    let first_page = visible_grammar_changes(&pool, fixture.user_id, 0, PAGE).await?;
    let cursor = first_page.next_cursor;
    assert!(
        !first_page
            .changes
            .iter()
            .any(|row| row.rule_id == fixture.rule_b),
        "a change whose predecessor is still in flight must not be served yet"
    );

    tx_a.commit().await?;

    let second_page = visible_grammar_changes(&pool, fixture.user_id, cursor, PAGE).await?;
    let seen: Vec<i32> = second_page.changes.iter().map(|row| row.rule_id).collect();
    assert!(
        seen.contains(&fixture.rule_a),
        "the late commit must still arrive"
    );
    assert!(seen.contains(&fixture.rule_b), "and so must the early one");
    Ok(())
}

/// The cursor a page reports must be safe to resume from, which means it can
/// never run ahead of what that page actually served.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_empty_page_leaves_the_cursor_where_it_was() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed_two_rules(&pool).await?;

    let mut tx = pool.begin().await?;
    touch_mastery(&mut tx, fixture.user_id, fixture.rule_a).await?;

    let page = visible_grammar_changes(&pool, fixture.user_id, 0, PAGE).await?;
    assert!(page.changes.is_empty(), "the only writer is still open");
    assert_eq!(page.next_cursor, 0);
    assert!(!page.has_more);

    tx.commit().await?;
    let page = visible_grammar_changes(&pool, fixture.user_id, 0, PAGE).await?;
    assert_eq!(page.changes.len(), 1);
    assert!(page.next_cursor > 0, "a served row advances the cursor");
    Ok(())
}

/// One learner's mastery is not another's business.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_mastery_feed_is_scoped_to_one_learner() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed_two_rules(&pool).await?;
    let other: i32 =
        sqlx::query_scalar("INSERT INTO users (display_name) VALUES ('Other') RETURNING id")
            .fetch_one(&pool)
            .await?;

    let mut tx = pool.begin().await?;
    touch_mastery(&mut tx, fixture.user_id, fixture.rule_a).await?;
    tx.commit().await?;

    let mine = visible_grammar_changes(&pool, fixture.user_id, 0, PAGE).await?;
    assert_eq!(mine.changes.len(), 1);
    let theirs = visible_grammar_changes(&pool, other, 0, PAGE).await?;
    assert!(theirs.changes.is_empty());
    Ok(())
}

/// A page hands back only what was asked for, and says when more is waiting.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_full_page_reports_that_more_is_waiting() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed_two_rules(&pool).await?;

    let mut tx = pool.begin().await?;
    touch_mastery(&mut tx, fixture.user_id, fixture.rule_a).await?;
    touch_mastery(&mut tx, fixture.user_id, fixture.rule_b).await?;
    tx.commit().await?;

    let page = visible_grammar_changes(&pool, fixture.user_id, 0, 1).await?;
    assert_eq!(page.changes.len(), 1);
    assert!(page.has_more, "a second change is waiting");

    let rest = visible_grammar_changes(&pool, fixture.user_id, page.next_cursor, 1).await?;
    assert_eq!(rest.changes.len(), 1);
    assert!(!rest.has_more);
    assert_ne!(rest.changes[0].rule_id, page.changes[0].rule_id);
    Ok(())
}

/// The item bank feed is keyed by language, because a device syncs the bank
/// for the languages it is learning rather than for a user.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_item_feed_reports_the_language_of_the_point_it_belongs_to() -> TestResult {
    let pool = reset_pool().await?;
    let fixture = seed_two_rules(&pool).await?;

    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO quiz_items (rule_id, kind, prompt, answer, accepted, status, content_sha256)
         VALUES ($1, 'cloze', 'Yo ___ cansado.', 'estoy', '[]'::jsonb, 'candidate', $2)",
    )
    .bind(fixture.rule_a)
    .bind("a".repeat(64))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let page = visible_item_changes(&pool, "es", 0, PAGE).await?;
    assert_eq!(page.changes.len(), 1);
    assert_eq!(page.changes[0].language_code, "es");
    assert_eq!(page.changes[0].operation, ChangeOperation::Upsert);

    let other = visible_item_changes(&pool, "fr", 0, PAGE).await?;
    assert!(other.changes.is_empty(), "another language sees nothing");
    Ok(())
}
