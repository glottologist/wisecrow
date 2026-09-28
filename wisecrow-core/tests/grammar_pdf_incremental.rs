//! Durable PDF progress and preservation of existing syllabus content.

use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

use serde_json::{json, Value};
use sqlx::PgPool;
use wisecrow::errors::WisecrowError;
use wisecrow::grammar::pdf::GrammarPassage;
use wisecrow::grammar::pdf_import::ImportTarget;
use wisecrow::grammar::pdf_incremental::{import_passages, is_complete, DocumentIdentity};
use wisecrow::llm::LlmProvider;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Model {
    answers: Mutex<VecDeque<String>>,
    calls: AtomicUsize,
    reject_title: Option<&'static str>,
    review_answer: Option<&'static str>,
}

impl Model {
    fn new(answers: Vec<String>) -> Self {
        Self {
            answers: Mutex::new(answers.into()),
            calls: AtomicUsize::new(0),
            reject_title: None,
            review_answer: None,
        }
    }
}

#[async_trait::async_trait]
impl LlmProvider for Model {
    async fn generate(&self, prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some((_, data)) = prompt.split_once("\nNOVELTY_DATA:\n") {
            if let Some(answer) = self.review_answer {
                return Ok(answer.to_owned());
            }
            let data: Value = serde_json::from_str(data)
                .map_err(|error| WisecrowError::LlmError(error.to_string()))?;
            let candidates = data["candidates"]
                .as_array()
                .ok_or_else(|| WisecrowError::LlmError("missing candidates".into()))?;
            let keep: Vec<&Value> = candidates
                .iter()
                .filter(|candidate| candidate["title"].as_str() != self.reject_title)
                .map(|candidate| &candidate["index"])
                .collect();
            return Ok(json!({"keep": keep}).to_string());
        }
        self.answers
            .lock()
            .map_err(|_| WisecrowError::LlmError("test model lock poisoned".into()))?
            .pop_front()
            .ok_or_else(|| WisecrowError::LlmError("unexpected synthesis call".into()))
    }

    fn name(&self) -> &str {
        "incremental test model"
    }
}

fn passage(page: usize, length: usize) -> GrammarPassage {
    GrammarPassage {
        page,
        heading: None,
        text: "A grammar passage with Gaelic à and ò. ".repeat(length),
        examples: Vec::new(),
    }
}

fn point(title: &str, page: usize) -> Value {
    json!({
        "title": title,
        "explanation": "This rule describes a specific condition under which the verb form changes. Apply this form only when the stated grammatical condition is present in the sentence.",
        "page": page,
        "examples": [
            {"sentence": "Tha mi an seo.", "translation": "I am here.", "is_correct": true},
            {"sentence": "Mi tha an seo.", "translation": "Incorrect word order.", "is_correct": false}
        ]
    })
}

fn points(start: usize, count: usize, page: usize) -> String {
    let values: Vec<Value> = (start..start + count)
        .map(|index| point(&format!("Rule {index}"), page))
        .collect();
    json!(values).to_string()
}

async fn language(pool: &PgPool) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ('gd', 'Scottish Gaelic') RETURNING id",
    )
    .fetch_one(pool)
    .await
}

fn target(language_id: i32, cefr_level: &str) -> ImportTarget<'_> {
    ImportTarget {
        language_id,
        language_name: "Scottish Gaelic",
        cefr_level,
    }
}

fn document(name: &str) -> DocumentIdentity<'_> {
    DocumentIdentity {
        name,
        fingerprint: [1; 32],
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires PostgreSQL with database creation privileges"]
async fn completed_and_renamed_files_cost_no_calls_and_preserve_examples(
    pool: PgPool,
) -> TestResult {
    let lang = language(&pool).await?;
    let model = Model::new(vec![points(0, 2, 1)]);
    let first = import_passages(
        &pool,
        &model,
        target(lang, "A1"),
        document("book.pdf"),
        &[passage(1, 10)],
    )
    .await?;
    assert_eq!(first.placed, 2);
    let before: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, sentence FROM rule_examples ORDER BY id")
            .fetch_all(&pool)
            .await?;
    let calls = model.calls.load(Ordering::SeqCst);
    let second = import_passages(
        &pool,
        &model,
        target(lang, "A1"),
        document("renamed.pdf"),
        &[passage(1, 10)],
    )
    .await?;
    assert!(second.already_complete);
    assert_eq!(model.calls.load(Ordering::SeqCst), calls);
    let after: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, sentence FROM rule_examples ORDER BY id")
            .fetch_all(&pool)
            .await?;
    assert_eq!(before, after);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires PostgreSQL with database creation privileges"]
async fn imports_beyond_thirty_and_visits_later_chunks(pool: PgPool) -> TestResult {
    let lang = language(&pool).await?;
    let model = Model::new(vec![
        points(0, 15, 1),
        points(15, 15, 1),
        points(30, 5, 1),
        points(35, 1, 2),
    ]);
    let passages = [passage(1, 1200), passage(2, 1200)];
    let result = import_passages(
        &pool,
        &model,
        target(lang, "A1"),
        document("book.pdf"),
        &passages,
    )
    .await?;
    assert_eq!(result.placed, 36);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM grammar_rules")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 36);
    assert!(is_complete(&pool, target(lang, "A1"), &[1; 32]).await?);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires PostgreSQL with database creation privileges"]
async fn preserves_exact_duplicates_and_rejects_rewordings_across_levels(
    pool: PgPool,
) -> TestResult {
    let lang = language(&pool).await?;
    let first = Model::new(vec![json!([point("Original rule", 1)]).to_string()]);
    import_passages(
        &pool,
        &first,
        target(lang, "A1"),
        document("old.pdf"),
        &[passage(1, 10)],
    )
    .await?;
    let before: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, sentence FROM rule_examples ORDER BY id")
            .fetch_all(&pool)
            .await?;
    let mut second = Model::new(vec![json!([
        point("Original rule", 1),
        point("Reworded rule", 1),
        point("Distinct rule", 1)
    ])
    .to_string()]);
    second.reject_title = Some("Reworded rule");
    let result = import_passages(
        &pool,
        &second,
        target(lang, "B1"),
        document("new.pdf"),
        &[passage(1, 10)],
    )
    .await?;
    assert_eq!((result.placed, result.duplicates), (1, 2));
    let after: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, sentence FROM rule_examples ORDER BY id LIMIT 2")
            .fetch_all(&pool)
            .await?;
    assert_eq!(before, after);
    let source: String =
        sqlx::query_scalar("SELECT source_ref FROM grammar_rules WHERE title = 'Original rule'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(source, "old.pdf p.1");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires PostgreSQL with database creation privileges"]
async fn failure_retries_only_unfinished_chunks_and_changed_content_is_new(
    pool: PgPool,
) -> TestResult {
    let lang = language(&pool).await?;
    let passages = [passage(1, 1200), passage(2, 1200)];
    let failing = Model::new(vec![points(0, 1, 1)]);
    assert!(import_passages(
        &pool,
        &failing,
        target(lang, "A1"),
        document("book.pdf"),
        &passages
    )
    .await
    .is_err());
    assert!(!is_complete(&pool, target(lang, "A1"), &[1; 32]).await?);
    let resumed = Model::new(vec![points(1, 1, 2)]);
    let result = import_passages(
        &pool,
        &resumed,
        target(lang, "A1"),
        document("book.pdf"),
        &passages,
    )
    .await?;
    assert_eq!(result.placed, 1);
    assert_eq!(resumed.calls.load(Ordering::SeqCst), 2);
    let changed = DocumentIdentity {
        fingerprint: [2; 32],
        ..document("book.pdf")
    };
    let additional = Model::new(vec![points(2, 1, 1)]);
    let result = import_passages(
        &pool,
        &additional,
        target(lang, "A1"),
        changed,
        &[passage(1, 10)],
    )
    .await?;
    assert_eq!(result.placed, 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires PostgreSQL with database creation privileges"]
async fn concurrent_importers_share_one_completed_pass(pool: PgPool) -> TestResult {
    let lang = language(&pool).await?;
    let model = Model::new(vec![points(0, 1, 1)]);
    let passages = [passage(1, 10)];
    let (first, second) = tokio::join!(
        import_passages(
            &pool,
            &model,
            target(lang, "A1"),
            document("book.pdf"),
            &passages
        ),
        import_passages(
            &pool,
            &model,
            target(lang, "A1"),
            document("book.pdf"),
            &passages
        )
    );
    assert_eq!(first?.placed + second?.placed, 1);
    assert_eq!(model.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires PostgreSQL with database creation privileges"]
async fn unfinished_rounds_resume_and_exact_same_level_rules_are_unchanged(
    pool: PgPool,
) -> TestResult {
    let lang = language(&pool).await?;
    let passages = [passage(1, 10)];
    let first = Model::new(vec![points(0, 15, 1)]);
    assert!(import_passages(
        &pool,
        &first,
        target(lang, "A1"),
        document("book.pdf"),
        &passages
    )
    .await
    .is_err());
    let examples: Vec<i32> = sqlx::query_scalar("SELECT id FROM rule_examples ORDER BY id")
        .fetch_all(&pool)
        .await?;
    assert_eq!(examples.len(), 30);
    let resumed = Model::new(vec![points(0, 2, 1)]);
    let result = import_passages(
        &pool,
        &resumed,
        target(lang, "A1"),
        document("book.pdf"),
        &passages,
    )
    .await?;
    assert_eq!((result.placed, result.duplicates), (0, 2));
    let after: Vec<i32> = sqlx::query_scalar("SELECT id FROM rule_examples ORDER BY id")
        .fetch_all(&pool)
        .await?;
    assert_eq!(examples, after);
    assert!(is_complete(&pool, target(lang, "A1"), &[1; 32]).await?);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires PostgreSQL with database creation privileges"]
async fn invalid_novelty_and_example_write_failure_leave_the_round_retryable(
    pool: PgPool,
) -> TestResult {
    let lang = language(&pool).await?;
    let passages = [passage(1, 10)];
    let mut invalid = Model::new(vec![points(0, 1, 1)]);
    invalid.review_answer = Some(r#"{"keep":[99]}"#);
    assert!(import_passages(
        &pool,
        &invalid,
        target(lang, "A1"),
        document("book.pdf"),
        &passages
    )
    .await
    .is_err());
    sqlx::query("ALTER TABLE rule_examples ADD CONSTRAINT test_reject_example CHECK (sentence <> 'Rejected example')")
        .execute(&pool).await?;
    let mut broken = point("Rule 0", 1);
    broken["examples"][1]["sentence"] = json!("Rejected example");
    let failing = Model::new(vec![json!([broken]).to_string()]);
    assert!(import_passages(
        &pool,
        &failing,
        target(lang, "A1"),
        document("book.pdf"),
        &passages
    )
    .await
    .is_err());
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT COUNT(*) FROM grammar_rules), (SELECT COUNT(*) FROM rule_examples), (SELECT COUNT(*) FROM grammar_import_progress)")
        .fetch_one(&pool).await?;
    assert_eq!(counts, (0, 0, 0));
    let repaired = Model::new(vec![points(0, 1, 1)]);
    let result = import_passages(
        &pool,
        &repaired,
        target(lang, "A1"),
        document("book.pdf"),
        &passages,
    )
    .await?;
    assert_eq!(result.placed, 1);
    Ok(())
}

struct SuspendedReview {
    entered: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl LlmProvider for SuspendedReview {
    async fn generate(&self, prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        if prompt.contains("\nNOVELTY_DATA:\n") {
            self.entered.notify_one();
            return std::future::pending().await;
        }
        Ok(points(0, 1, 1))
    }

    fn name(&self) -> &str {
        "suspended review"
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires PostgreSQL with database creation privileges"]
async fn cancellation_releases_the_lock_and_does_not_complete_the_chunk(
    pool: PgPool,
) -> TestResult {
    let lang = language(&pool).await?;
    let model = SuspendedReview {
        entered: tokio::sync::Notify::new(),
    };
    let passages = [passage(1, 10)];
    let mut import = Box::pin(import_passages(
        &pool,
        &model,
        target(lang, "A1"),
        document("book.pdf"),
        &passages,
    ));
    tokio::select! {
        _ = model.entered.notified() => {},
        result = &mut import => { return Err(format!("import finished before cancellation: {result:?}").into()); },
    }
    drop(import);
    assert!(!is_complete(&pool, target(lang, "A1"), &[1; 32]).await?);
    let resumed = Model::new(vec![points(0, 1, 1)]);
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        import_passages(
            &pool,
            &resumed,
            target(lang, "A1"),
            document("book.pdf"),
            &passages,
        ),
    )
    .await??;
    assert_eq!(result.placed, 1);
    Ok(())
}
