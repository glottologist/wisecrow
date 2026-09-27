//! Importing a document's points: what is written, what is cited, and what is
//! left alone.
//!
//! The discipline being tested is placement. An import adds to a level; it
//! never moves a point that already sits at another one, and a dry run costs
//! the model call but nothing in the database.

use std::path::PathBuf;

use sqlx::PgPool;
use wisecrow::errors::WisecrowError;
use wisecrow::grammar::pdf::{ExampleSentence, GrammarPassage};
use wisecrow::grammar::pdf_import::{import_passages, ImportOptions, ImportTarget};
use wisecrow::grammar::rules::{NewGrammarRule, RuleRepository, RuleSource};
use wisecrow::grammar::sources::{import_schedule, LevelCoverage};
use wisecrow::llm::LlmProvider;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const DOCUMENT: &str = "yo-puedo-1-2021.pdf";

async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query("TRUNCATE languages, grammar_rules, grammar_rule_aliases CASCADE")
        .execute(&pool)
        .await?;
    Ok(pool)
}

async fn seed_language(pool: &PgPool) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("INSERT INTO languages (code, name) VALUES ('es', 'Spanish') RETURNING id")
        .fetch_one(pool)
        .await
}

async fn level_id(pool: &PgPool, code: &str) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM cefr_levels WHERE code = $1")
        .bind(code)
        .fetch_one(pool)
        .await
}

/// Answers each call with the next scripted response, and the last one
/// thereafter; a stub that answers every call the same way is the one-item
/// case.
struct StubProvider {
    responses: Vec<String>,
    calls: std::sync::atomic::AtomicUsize,
}

impl StubProvider {
    fn answering(response: String) -> Self {
        Self::answering_in_turn(vec![response])
    }

    fn answering_in_turn(responses: Vec<String>) -> Self {
        assert!(!responses.is_empty(), "a stub needs at least one answer");
        Self {
            responses,
            calls: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LlmProvider for StubProvider {
    async fn generate(&self, _prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let index = call.min(self.responses.len() - 1);
        Ok(self.responses[index].clone()) // clone: the stub hands out an owned answer per call
    }

    fn name(&self) -> &str {
        "stub"
    }
}

fn passage(page: usize) -> GrammarPassage {
    GrammarPassage {
        heading: Some("Ser y estar".to_owned()),
        text: "The two verbs both translate as to be, and the choice between them is grammatical rather than stylistic. Ser names what a thing is and estar names how it is found. ".repeat(2),
        page,
        examples: vec![ExampleSentence {
            text: "Estoy cansado.".to_owned(),
            translation: Some("I am tired.".to_owned()),
        }],
    }
}

fn point(title: &str, page: usize) -> String {
    format!(
        r#"{{"title":"{title}",
            "explanation":"Ser names what a thing is and estar names how it is found. A state that can change takes estar, so a tired person is cansado with estar and not with ser.",
            "page":{page},
            "examples":[
              {{"sentence":"Estoy cansado.","translation":"I am tired.","is_correct":true}},
              {{"sentence":"Soy cansado.","translation":"I am tired.","is_correct":false}}
            ]}}"#
    )
}

/// A JSON array of `count` distinct points, titled from `first` upwards.
fn points(first: usize, count: usize, page: usize) -> String {
    let items: Vec<String> = (first..first + count)
        .map(|index| point(&format!("Point {index}"), page))
        .collect();
    format!("[{}]", items.join(","))
}

/// Seeds `count` points at `level` with the given source, slugged
/// `{source}-{index}`.
async fn seed_points(
    pool: &PgPool,
    language_id: i32,
    level: &str,
    source: RuleSource,
    count: usize,
) -> TestResult {
    let level_id = level_id(pool, level).await?;
    for index in 0..count {
        let rule = NewGrammarRule {
            slug: format!("{}-{index}", source.as_str()),
            title: format!("{} point {index}", source.as_str()),
            explanation: "Held already.".to_owned(),
            source,
            source_ref: None,
            examples: vec![],
        };
        RuleRepository::upsert_rule(pool, language_id, level_id, &rule).await?;
    }
    Ok(())
}

fn target<'a>(language_id: i32, level: &'a str) -> ImportTarget<'a> {
    ImportTarget {
        language_id,
        language_name: "Spanish",
        cefr_level: level,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_placed_point_keeps_its_citation_through_the_database() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool).await?;
    let provider = StubProvider::answering(format!("[{}]", point("Ser and estar", 148)));

    let outcome = import_passages(
        &pool,
        &provider,
        target(language_id, "B1"),
        DOCUMENT,
        &[passage(148)],
        ImportOptions::default(),
    )
    .await?;

    assert_eq!(outcome.placed, 1);
    let stored = RuleRepository::rules_for_level(&pool, language_id, "B1").await?;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].source, RuleSource::Pdf);
    assert_eq!(
        stored[0].source_ref.as_deref(),
        Some("yo-puedo-1-2021.pdf p.148"),
        "the citation must survive the round trip"
    );
    assert_eq!(stored[0].examples.len(), 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_point_already_held_at_c1_is_not_moved_to_b2_by_an_import() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool).await?;
    let c1 = level_id(&pool, "C1").await?;
    let curated = NewGrammarRule {
        slug: "ser-and-estar".to_owned(),
        title: "Ser and estar".to_owned(),
        explanation: "As the curator wrote it.".to_owned(),
        source: RuleSource::Manual,
        source_ref: None,
        examples: vec![],
    };
    RuleRepository::upsert_rule(&pool, language_id, c1, &curated).await?;

    let provider = StubProvider::answering(format!("[{}]", point("Ser and estar", 148)));
    let outcome = import_passages(
        &pool,
        &provider,
        target(language_id, "B2"),
        DOCUMENT,
        &[passage(148)],
        ImportOptions::default(),
    )
    .await?;

    assert_eq!(outcome.placed, 0);
    assert_eq!(outcome.held, 1, "the point was found at another level");
    let (level, explanation, source_ref): (i32, String, Option<String>) = sqlx::query_as(
        "SELECT cefr_level_id, explanation, source_ref FROM grammar_rules WHERE slug = 'ser-and-estar'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(level, c1, "the curated point stays at C1");
    assert_eq!(explanation, "As the curator wrote it.");
    assert_eq!(source_ref, None, "its provenance is untouched too");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_dry_run_asks_the_model_and_writes_nothing() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool).await?;
    let provider = StubProvider::answering(format!("[{}]", point("Ser and estar", 148)));

    let outcome = import_passages(
        &pool,
        &provider,
        target(language_id, "B1"),
        DOCUMENT,
        &[passage(148)],
        ImportOptions {
            dry_run: true,
            max_rules: None,
        },
    )
    .await?;

    assert_eq!(provider.calls(), 1);
    assert_eq!(outcome.synthesis.points.len(), 1, "the answer is reported");
    assert_eq!(outcome.placed, 0);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM grammar_rules")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 0, "a dry run leaves grammar_rules untouched");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_level_the_seeder_filled_is_asked_in_rounds_of_fifteen() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool).await?;
    seed_points(&pool, language_id, "B1", RuleSource::Llm, 15).await?;
    let provider = StubProvider::answering_in_turn(vec![
        points(0, 15, 148),
        points(15, 15, 149),
        points(30, 15, 150),
    ]);

    let outcome = import_passages(
        &pool,
        &provider,
        target(language_id, "B1"),
        DOCUMENT,
        &[passage(148), passage(149), passage(150)],
        ImportOptions::default(),
    )
    .await?;

    assert_eq!(
        outcome.wanted, 30,
        "the document target is measured on its own"
    );
    assert_eq!(
        provider.calls(),
        2,
        "thirty points are two rounds, never one"
    );
    assert_eq!(outcome.placed, 30);
    assert_eq!(outcome.synthesis.points.len(), 30);
    let stored = RuleRepository::rules_for_level(&pool, language_id, "B1").await?;
    assert_eq!(
        stored.len(),
        45,
        "fifteen seeded and thirty document-backed"
    );
    assert_eq!(
        stored
            .iter()
            .filter(|rule| rule.source == RuleSource::Pdf)
            .count(),
        30
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_round_that_falls_short_ends_the_import() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool).await?;
    let provider = StubProvider::answering(points(0, 4, 148));

    let outcome = import_passages(
        &pool,
        &provider,
        target(language_id, "B1"),
        DOCUMENT,
        &[passage(148)],
        ImportOptions::default(),
    )
    .await?;

    assert_eq!(outcome.wanted, 30);
    assert_eq!(
        provider.calls(),
        1,
        "four of fifteen is the document running dry"
    );
    assert_eq!(outcome.placed, 4);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_level_holding_thirty_document_points_is_not_read_to_the_model() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool).await?;
    seed_points(&pool, language_id, "B1", RuleSource::Pdf, 30).await?;
    let provider = StubProvider::answering("[]".to_owned());

    let outcome = import_passages(
        &pool,
        &provider,
        target(language_id, "B1"),
        DOCUMENT,
        &[passage(148)],
        ImportOptions::default(),
    )
    .await?;

    assert_eq!(outcome.wanted, 0);
    assert_eq!(
        provider.calls(),
        0,
        "a level at the document target costs no model call"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_seeded_slug_at_the_same_level_is_held_and_a_document_slug_is_refreshed() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool).await?;
    let b1 = level_id(&pool, "B1").await?;
    let seeded = NewGrammarRule {
        slug: "ser-and-estar".to_owned(),
        title: "Ser and estar".to_owned(),
        explanation: "Seeded prose that must survive.".to_owned(),
        source: RuleSource::Llm,
        source_ref: None,
        examples: vec![],
    };
    RuleRepository::upsert_rule(&pool, language_id, b1, &seeded).await?;
    let provider = StubProvider::answering(format!("[{}]", point("Ser and estar", 148)));

    let outcome = import_passages(
        &pool,
        &provider,
        target(language_id, "B1"),
        DOCUMENT,
        &[passage(148)],
        ImportOptions::default(),
    )
    .await?;

    assert_eq!(
        (outcome.placed, outcome.held),
        (0, 1),
        "a seeded slug is held, not rewritten"
    );
    let stored = RuleRepository::rules_for_level(&pool, language_id, "B1").await?;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].source, RuleSource::Llm);
    assert_eq!(stored[0].explanation, "Seeded prose that must survive.");
    assert!(
        stored[0].examples.is_empty(),
        "the seeded examples are untouched"
    );

    // The same proposal against the importer's own earlier work is a refresh.
    let earlier = NewGrammarRule {
        source: RuleSource::Pdf,
        source_ref: Some("older.pdf p.3".to_owned()),
        ..seeded
    };
    RuleRepository::upsert_rule(&pool, language_id, b1, &earlier).await?;
    let outcome = import_passages(
        &pool,
        &provider,
        target(language_id, "B1"),
        DOCUMENT,
        &[passage(148)],
        ImportOptions::default(),
    )
    .await?;

    assert_eq!(
        (outcome.placed, outcome.held),
        (1, 0),
        "a document slug is refreshed in place"
    );
    let stored = RuleRepository::rules_for_level(&pool, language_id, "B1").await?;
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].source_ref.as_deref(),
        Some("yo-puedo-1-2021.pdf p.148")
    );
    assert_eq!(stored[0].examples.len(), 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn max_rules_caps_what_is_kept_beneath_the_shortfall() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool).await?;
    let provider = StubProvider::answering(format!(
        "[{},{}]",
        point("Ser and estar", 148),
        point("Estar with the gerund", 148)
    ));

    let outcome = import_passages(
        &pool,
        &provider,
        target(language_id, "B1"),
        DOCUMENT,
        &[passage(148)],
        ImportOptions {
            dry_run: false,
            max_rules: Some(1),
        },
    )
    .await?;

    assert_eq!(outcome.wanted, 1);
    assert_eq!(outcome.placed, 1);
    assert_eq!(
        outcome.synthesis.rejected.len(),
        1,
        "the second point was owed no place"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_lowest_covering_level_claims_a_point_two_documents_propose() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool).await?;
    let rows = vec![
        (
            PathBuf::from("es/grammar.pdf"),
            LevelCoverage::Stated(vec!["B1"]),
        ),
        (
            PathBuf::from("es/beginner.pdf"),
            LevelCoverage::Stated(vec!["A1", "B1"]),
        ),
    ];
    let schedule = import_schedule(&rows, None)?;
    let order: Vec<(&str, &str)> = schedule
        .calls
        .iter()
        .map(|(level, document)| (*level, document.to_str().expect("utf-8")))
        .collect();
    assert_eq!(
        order,
        [
            ("A1", "es/beginner.pdf"),
            ("B1", "es/grammar.pdf"),
            ("B1", "es/beginner.pdf")
        ]
    );
    let provider = StubProvider::answering(format!("[{}]", point("Personal a", 12)));

    let mut placed = 0;
    let mut held = 0;
    for (level, document) in &schedule.calls {
        let name = document
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("name");
        let outcome = import_passages(
            &pool,
            &provider,
            target(language_id, level),
            name,
            &[passage(12)],
            ImportOptions::default(),
        )
        .await?;
        placed += outcome.placed;
        held += outcome.held;
    }

    assert_eq!((placed, held), (1, 2), "stored once, held twice");
    let a1 = RuleRepository::rules_for_level(&pool, language_id, "A1").await?;
    assert_eq!(a1.len(), 1);
    assert_eq!(a1[0].slug, "personal-a");
    assert_eq!(a1[0].source_ref.as_deref(), Some("beginner.pdf p.12"));
    assert!(RuleRepository::rules_for_level(&pool, language_id, "B1")
        .await?
        .is_empty());
    Ok(())
}
