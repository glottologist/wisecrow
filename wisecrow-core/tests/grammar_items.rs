//! The quiz item bank: persistence, immutability and the promotion gate.
//!
//! An item that marks a correct answer wrong does not merely annoy a learner;
//! it writes a false signal into their mastery. These tests pin down the
//! properties that protect that signal.

use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query("TRUNCATE languages, grammar_rules, grammar_rule_aliases, quiz_items CASCADE")
        .execute(&pool)
        .await?;
    Ok(pool)
}

async fn seed_rule(pool: &PgPool, slug: &str) -> Result<i32, Box<dyn std::error::Error>> {
    let language_id: i32 = sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ('es', 'Spanish')
         ON CONFLICT (code) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .fetch_one(pool)
    .await?;
    let level_id: i32 = sqlx::query_scalar("SELECT id FROM cefr_levels WHERE code = 'A1'")
        .fetch_one(pool)
        .await?;
    let rule_id: i32 = sqlx::query_scalar(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         VALUES ($1, $2, $3, 'Ser vs estar', 'x', 'llm')
         RETURNING id",
    )
    .bind(language_id)
    .bind(level_id)
    .bind(slug)
    .fetch_one(pool)
    .await?;
    Ok(rule_id)
}

async fn insert_item(
    pool: &PgPool,
    rule_id: i32,
    hash: &str,
) -> Result<i32, Box<dyn std::error::Error>> {
    let id: i32 = sqlx::query_scalar(
        "INSERT INTO quiz_items (rule_id, kind, prompt, answer, accepted, content_sha256)
         VALUES ($1, 'cloze', 'Yo ___ cansado.', 'estoy', '[]'::jsonb, $2)
         RETURNING id",
    )
    .bind(rule_id)
    .bind(hash)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_item_starts_as_a_candidate_at_revision_one() -> TestResult {
    let pool = reset_pool().await?;
    let rule_id = seed_rule(&pool, "ser-vs-estar").await?;
    let item_id = insert_item(&pool, rule_id, HASH_A).await?;

    let (status, revision): (String, i32) =
        sqlx::query_as("SELECT status, revision FROM quiz_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "candidate", "nothing reaches a learner ungated");
    assert_eq!(revision, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn identical_content_for_one_rule_is_stored_once() -> TestResult {
    let pool = reset_pool().await?;
    let rule_id = seed_rule(&pool, "ser-vs-estar").await?;
    insert_item(&pool, rule_id, HASH_A).await?;

    let duplicate = insert_item(&pool, rule_id, HASH_A).await;
    assert!(
        duplicate.is_err(),
        "the content hash deduplicates generation"
    );

    insert_item(&pool, rule_id, HASH_B).await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quiz_items WHERE rule_id = $1")
        .bind(rule_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn item_content_cannot_be_edited_in_place() -> TestResult {
    let pool = reset_pool().await?;
    let rule_id = seed_rule(&pool, "ser-vs-estar").await?;
    let item_id = insert_item(&pool, rule_id, HASH_A).await?;

    let edit = sqlx::query("UPDATE quiz_items SET prompt = 'Tú ___ cansado.' WHERE id = $1")
        .bind(item_id)
        .execute(&pool)
        .await;
    assert!(
        edit.is_err(),
        "an attempt uploaded weeks later must still grade against what the learner saw"
    );

    let promote = sqlx::query(
        "UPDATE quiz_items SET status = 'active', promoted_at = CURRENT_TIMESTAMP WHERE id = $1",
    )
    .bind(item_id)
    .execute(&pool)
    .await;
    assert!(promote.is_ok(), "status changes remain permitted");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_the_four_lifecycle_statuses_are_accepted() -> TestResult {
    let pool = reset_pool().await?;
    let rule_id = seed_rule(&pool, "ser-vs-estar").await?;
    let item_id = insert_item(&pool, rule_id, HASH_A).await?;

    for status in ["active", "rejected", "retired", "candidate"] {
        sqlx::query("UPDATE quiz_items SET status = $2 WHERE id = $1")
            .bind(item_id)
            .bind(status)
            .execute(&pool)
            .await?;
    }

    let nonsense = sqlx::query("UPDATE quiz_items SET status = 'published' WHERE id = $1")
        .bind(item_id)
        .execute(&pool)
        .await;
    assert!(nonsense.is_err());
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn exposures_record_when_a_learner_last_saw_an_item() -> TestResult {
    let pool = reset_pool().await?;
    let rule_id = seed_rule(&pool, "ser-vs-estar").await?;
    let item_id = insert_item(&pool, rule_id, HASH_A).await?;
    let user_id: i32 = sqlx::query_scalar("SELECT id FROM users ORDER BY id LIMIT 1")
        .fetch_one(&pool)
        .await?;

    sqlx::query(
        "INSERT INTO quiz_item_exposures (user_id, item_id) VALUES ($1, $2)
         ON CONFLICT (user_id, item_id) DO UPDATE SET served_at = CURRENT_TIMESTAMP",
    )
    .bind(user_id)
    .bind(item_id)
    .execute(&pool)
    .await?;

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM quiz_item_exposures WHERE user_id = $1 AND item_id = $2",
    )
    .bind(user_id)
    .bind(item_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        count, 1,
        "re-serving an item updates rather than duplicates"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn generated_items_persist_once_and_report_duplicates() -> TestResult {
    use wisecrow::grammar::items::{ItemDraft, ItemRepository};

    let pool = reset_pool().await?;
    let rule_id = seed_rule(&pool, "ser-vs-estar").await?;

    let drafts = vec![
        ItemDraft::cloze("Yo ___ cansado.", "estoy").with_hint("temporary state"),
        ItemDraft::multiple_choice(
            "Which verb describes a permanent quality?",
            &[("o1", "ser"), ("o2", "estar"), ("o3", "haber")],
            "o1",
        ),
    ];

    let first = ItemRepository::insert_candidates(&pool, rule_id, &drafts).await?;
    assert_eq!(first.inserted, 2);
    assert_eq!(first.duplicates, 0);

    let second = ItemRepository::insert_candidates(&pool, rule_id, &drafts).await?;
    assert_eq!(
        second.inserted, 0,
        "generation that repeats itself costs nothing"
    );
    assert_eq!(second.duplicates, 2);

    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quiz_items WHERE rule_id = $1")
        .bind(rule_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(stored, 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_draft_that_fails_a_gate_is_stored_rejected_with_its_reason() -> TestResult {
    use wisecrow::grammar::items::{ItemDraft, ItemRepository};

    let pool = reset_pool().await?;
    let rule_id = seed_rule(&pool, "ser-vs-estar").await?;

    // No blank: the learner would be shown the answer.
    let drafts = vec![ItemDraft::cloze("Yo estoy cansado.", "estoy")];
    let summary = ItemRepository::insert_candidates(&pool, rule_id, &drafts).await?;
    assert_eq!(summary.inserted, 0);
    assert_eq!(summary.gated, 1);

    let (status, reason): (String, Option<String>) =
        sqlx::query_as("SELECT status, gate_reason FROM quiz_items WHERE rule_id = $1")
            .bind(rule_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "rejected");
    assert!(
        reason.is_some_and(|text| text.contains("blank")),
        "a rejection records what was wrong, for the reviewer"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_promoted_items_are_servable() -> TestResult {
    use wisecrow::grammar::items::{ItemDraft, ItemRepository};

    let pool = reset_pool().await?;
    let rule_id = seed_rule(&pool, "ser-vs-estar").await?;
    ItemRepository::insert_candidates(
        &pool,
        rule_id,
        &[
            ItemDraft::cloze("Yo ___ cansado.", "estoy"),
            ItemDraft::cloze("Tú ___ alto.", "eres"),
        ],
    )
    .await?;

    assert!(
        ItemRepository::active_items_for_rule(&pool, rule_id)
            .await?
            .is_empty(),
        "a candidate is not servable"
    );

    let pending = ItemRepository::candidates_for_language(&pool, "es", None, 10).await?;
    assert_eq!(pending.len(), 2);

    ItemRepository::promote(&pool, pending[0].id).await?;
    ItemRepository::reject(&pool, pending[1].id, "distractor is also correct").await?;

    let active = ItemRepository::active_items_for_rule(&pool, rule_id).await?;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, pending[0].id);

    let (status, reason): (String, Option<String>) =
        sqlx::query_as("SELECT status, gate_reason FROM quiz_items WHERE id = $1")
            .bind(pending[1].id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "rejected");
    assert_eq!(reason.as_deref(), Some("distractor is also correct"));
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn retiring_an_item_keeps_the_row_for_later_grading() -> TestResult {
    use wisecrow::grammar::items::{ItemDraft, ItemRepository};

    let pool = reset_pool().await?;
    let rule_id = seed_rule(&pool, "ser-vs-estar").await?;
    ItemRepository::insert_candidates(
        &pool,
        rule_id,
        &[ItemDraft::cloze("Yo ___ cansado.", "estoy")],
    )
    .await?;
    let pending = ItemRepository::candidates_for_language(&pool, "es", None, 10).await?;
    ItemRepository::promote(&pool, pending[0].id).await?;
    ItemRepository::retire(&pool, pending[0].id).await?;

    assert!(ItemRepository::active_items_for_rule(&pool, rule_id)
        .await?
        .is_empty());
    let survives: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quiz_items WHERE id = $1")
        .bind(pending[0].id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        survives, 1,
        "an attempt from a stale device must still grade"
    );
    Ok(())
}
