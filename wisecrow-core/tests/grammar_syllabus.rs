//! Grammar syllabus identity: slugs, aliases and provenance tiers.
//!
//! Mastery is keyed on a grammar point, so a point needs an identity that
//! survives re-seeding with different model wording. These tests pin that
//! identity down at the schema level.

use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

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

async fn seed_language(pool: &PgPool, code: &str, name: &str) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("INSERT INTO languages (code, name) VALUES ($1, $2) RETURNING id")
        .bind(code)
        .bind(name)
        .fetch_one(pool)
        .await
}

async fn level_id(pool: &PgPool, code: &str) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM cefr_levels WHERE code = $1")
        .bind(code)
        .fetch_one(pool)
        .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn slug_is_unique_per_language_and_aliases_resolve() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool, "es", "Spanish").await?;
    let level = level_id(&pool, "A1").await?;

    let rule_id: i32 = sqlx::query_scalar(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, title, explanation, source, slug)
         VALUES ($1, $2, 'Ser vs estar: permanent qualities', 'x', 'llm',
                 'ser-vs-estar-permanent-qualities')
         RETURNING id",
    )
    .bind(language_id)
    .bind(level)
    .fetch_one(&pool)
    .await?;

    let duplicate = sqlx::query(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, title, explanation, source, slug)
         VALUES ($1, $2, 'A different title', 'y', 'llm', 'ser-vs-estar-permanent-qualities')",
    )
    .bind(language_id)
    .bind(level)
    .execute(&pool)
    .await;
    assert!(duplicate.is_err(), "slug must be unique per language");

    sqlx::query(
        "INSERT INTO grammar_rule_aliases (language_id, alias_slug, rule_id)
         VALUES ($1, 'pcic-2-3-ser-estar', $2)",
    )
    .bind(language_id)
    .bind(rule_id)
    .execute(&pool)
    .await?;

    let resolved: i32 = sqlx::query_scalar(
        "SELECT rule_id FROM grammar_rule_aliases
         WHERE language_id = $1 AND alias_slug = 'pcic-2-3-ser-estar'",
    )
    .bind(language_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(resolved, rule_id);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_same_slug_may_repeat_across_languages() -> TestResult {
    let pool = reset_pool().await?;
    let spanish = seed_language(&pool, "es", "Spanish").await?;
    let french = seed_language(&pool, "fr", "French").await?;
    let level = level_id(&pool, "A1").await?;

    for language_id in [spanish, french] {
        sqlx::query(
            "INSERT INTO grammar_rules (language_id, cefr_level_id, title, explanation, source, slug)
             VALUES ($1, $2, 'Gendered articles', 'x', 'llm', 'gendered-articles')",
        )
        .bind(language_id)
        .bind(level)
        .execute(&pool)
        .await?;
    }

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM grammar_rules WHERE slug = 'gendered-articles'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(count, 2, "uniqueness is scoped to one language");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn provenance_tiers_accept_llm_and_reference() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool, "fr", "French").await?;
    let level = level_id(&pool, "B1").await?;

    for (slug, source) in [("a", "llm"), ("b", "reference"), ("c", "manual")] {
        sqlx::query(
            "INSERT INTO grammar_rules (language_id, cefr_level_id, title, explanation, source, slug)
             VALUES ($1, $2, $3, 'x', $4, $3)",
        )
        .bind(language_id)
        .bind(level)
        .bind(slug)
        .bind(source)
        .execute(&pool)
        .await?;
    }

    let tiered: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM grammar_rules
         WHERE language_id = $1 AND source IN ('llm', 'reference')",
    )
    .bind(language_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(tiered, 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_empty_slug_is_refused() -> TestResult {
    let pool = reset_pool().await?;
    let language_id = seed_language(&pool, "pl", "Polish").await?;
    let level = level_id(&pool, "A2").await?;

    let blank = sqlx::query(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, title, explanation, source, slug)
         VALUES ($1, $2, 'Blank', 'x', 'llm', '   ')",
    )
    .bind(language_id)
    .bind(level)
    .execute(&pool)
    .await;
    assert!(blank.is_err(), "a whitespace-only slug is not an identity");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn upsert_matches_on_slug_and_preserves_rule_id() -> TestResult {
    use wisecrow::grammar::rules::{NewGrammarRule, RuleRepository, RuleSource};

    let pool = reset_pool().await?;
    let language_id = seed_language(&pool, "pl", "Polish").await?;
    let level = RuleRepository::ensure_cefr_level(&pool, "A2").await?;

    let first = NewGrammarRule {
        slug: "instrumental-case-with-z".to_owned(),
        title: "Instrumental case with z".to_owned(),
        explanation: "First wording".to_owned(),
        source: RuleSource::Llm,
        examples: vec![],
    };
    RuleRepository::upsert_rule(&pool, language_id, level, &first).await?;
    let original_id: i32 =
        sqlx::query_scalar("SELECT id FROM grammar_rules WHERE language_id = $1 AND slug = $2")
            .bind(language_id)
            .bind("instrumental-case-with-z")
            .fetch_one(&pool)
            .await?;

    let reworded = NewGrammarRule {
        title: "The instrumental case after z".to_owned(),
        explanation: "Second wording".to_owned(),
        ..first
    };
    RuleRepository::upsert_rule(&pool, language_id, level, &reworded).await?;

    let rows: Vec<(i32, String, String)> =
        sqlx::query_as("SELECT id, title, explanation FROM grammar_rules WHERE language_id = $1")
            .bind(language_id)
            .fetch_all(&pool)
            .await?;
    assert_eq!(rows.len(), 1, "re-wording must not create a second point");
    assert_eq!(rows[0].0, original_id, "rule id must survive re-wording");
    assert_eq!(rows[0].1, "The instrumental case after z");
    assert_eq!(rows[0].2, "Second wording");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn rules_for_level_returns_syllabus_order() -> TestResult {
    use wisecrow::grammar::rules::{NewGrammarRule, RuleRepository, RuleSource};

    let pool = reset_pool().await?;
    let language_id = seed_language(&pool, "de", "German").await?;
    let level = RuleRepository::ensure_cefr_level(&pool, "A1").await?;

    for (slug, title) in [
        ("articles-definite", "Zzz definite articles"),
        ("adjective-endings", "Aaa adjective endings"),
    ] {
        RuleRepository::upsert_rule(
            &pool,
            language_id,
            level,
            &NewGrammarRule {
                slug: slug.to_owned(),
                title: title.to_owned(),
                explanation: "x".to_owned(),
                source: RuleSource::Llm,
                examples: vec![],
            },
        )
        .await?;
    }

    let rules = RuleRepository::rules_for_level(&pool, language_id, "A1").await?;
    let slugs: Vec<&str> = rules.iter().map(|rule| rule.slug.as_str()).collect();
    assert_eq!(
        slugs,
        vec!["adjective-endings", "articles-definite"],
        "syllabus order is by slug, not by title"
    );
    Ok(())
}

#[test]
fn rule_source_round_trips_all_tiers() {
    use std::str::FromStr;
    use wisecrow::grammar::rules::RuleSource;

    for code in ["manual", "ai", "pdf", "llm", "reference"] {
        let parsed = RuleSource::from_str(code).expect("known source");
        assert_eq!(parsed.as_str(), code);
    }
    assert!(RuleSource::from_str("nonsense").is_err());
}

#[test]
fn slugify_folds_case_punctuation_and_accents_into_hyphens() {
    use wisecrow::grammar::rules::slugify;

    assert_eq!(
        slugify("Ser vs estar: permanent qualities"),
        "ser-vs-estar-permanent-qualities"
    );
    assert_eq!(slugify("  Passé composé!  "), "passe-compose");
    assert_eq!(slugify("¿Qué tal?"), "que-tal");
    assert!(
        !slugify("的字结构").is_empty(),
        "a non-Latin title still yields an identity"
    );
}

/// Counts calls and returns a fixed number of fabricated rules per call, so
/// that a test can assert the model was consulted exactly as often as the
/// gaps in the syllabus required.
struct CountingProvider {
    rules_per_call: usize,
    calls: std::sync::atomic::AtomicUsize,
    explanation: String,
}

impl CountingProvider {
    fn with_rules(rules_per_call: usize) -> Self {
        Self {
            rules_per_call,
            calls: std::sync::atomic::AtomicUsize::new(0),
            explanation: "GENERATED".to_owned(),
        }
    }

    fn saying(explanation: &str) -> Self {
        Self {
            rules_per_call: 1,
            calls: std::sync::atomic::AtomicUsize::new(0),
            explanation: explanation.to_owned(),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl wisecrow::llm::LlmProvider for CountingProvider {
    async fn generate(
        &self,
        prompt: &str,
        _max_tokens: u32,
    ) -> Result<String, wisecrow::errors::WisecrowError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        // The prompt names the level; vary titles by it so slugs stay distinct.
        let level = ["A1", "A2", "B1", "B2", "C1", "C2"]
            .into_iter()
            .find(|code| prompt.contains(*code))
            .unwrap_or("A1");
        let rules: Vec<String> = (0..self.rules_per_call)
            .map(|index| {
                format!(
                    r#"{{"title": "{level} point {index}", "explanation": "{}", "examples": []}}"#,
                    self.explanation
                )
            })
            .collect();
        Ok(format!("[{}]", rules.join(",")))
    }

    fn name(&self) -> &str {
        "counting"
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ensure_syllabus_fills_gaps_and_is_idempotent() -> TestResult {
    use wisecrow::grammar::syllabus;

    let pool = reset_pool().await?;
    let provider = CountingProvider::with_rules(3);

    let first = syllabus::ensure_syllabus(&pool, &provider, "pl").await?;
    assert_eq!(first.levels_filled, 6);
    assert_eq!(first.points_added, 18, "three points across six levels");
    assert_eq!(provider.calls(), 6, "one call per empty level");

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM grammar_rules")
        .fetch_one(&pool)
        .await?;
    assert_eq!(total, 18);

    let second = syllabus::ensure_syllabus(&pool, &provider, "pl").await?;
    assert_eq!(second.points_added, 0, "second run adds nothing");
    assert_eq!(second.levels_skipped, 6);
    assert_eq!(
        provider.calls(),
        6,
        "second run consults the model not at all"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ensure_syllabus_leaves_existing_prose_untouched() -> TestResult {
    use wisecrow::grammar::syllabus;

    let pool = reset_pool().await?;
    let language_id = seed_language(&pool, "pl", "Polish").await?;
    let level = level_id(&pool, "A1").await?;
    sqlx::query(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         VALUES ($1, $2, 'hand-written', 'Hand written', 'ORIGINAL', 'manual')",
    )
    .bind(language_id)
    .bind(level)
    .execute(&pool)
    .await?;

    let provider = CountingProvider::saying("REWRITTEN");
    syllabus::ensure_syllabus(&pool, &provider, "pl").await?;

    let explanation: String =
        sqlx::query_scalar("SELECT explanation FROM grammar_rules WHERE slug = 'hand-written'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(explanation, "ORIGINAL", "a filled level is left alone");
    assert_eq!(
        provider.calls(),
        5,
        "A1 already had a point, so five gaps remain"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ensure_syllabus_marks_generated_points_as_llm_provenance() -> TestResult {
    use wisecrow::grammar::syllabus;

    let pool = reset_pool().await?;
    let provider = CountingProvider::with_rules(1);
    syllabus::ensure_syllabus(&pool, &provider, "pl").await?;

    let non_llm: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM grammar_rules WHERE source <> 'llm'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        non_llm, 0,
        "seeded points declare themselves machine-generated"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn refresh_rewrites_llm_prose_and_skips_reference_points() -> TestResult {
    use wisecrow::grammar::syllabus;

    let pool = reset_pool().await?;
    let language_id = seed_language(&pool, "pl", "Polish").await?;
    let level = level_id(&pool, "A1").await?;
    for (slug, title, source) in [
        ("a1-point-0", "A1 point 0", "llm"),
        ("curated-point", "Curated point", "reference"),
    ] {
        sqlx::query(
            "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
             VALUES ($1, $2, $3, $4, 'ORIGINAL', $5)",
        )
        .bind(language_id)
        .bind(level)
        .bind(slug)
        .bind(title)
        .bind(source)
        .execute(&pool)
        .await?;
    }

    let provider = CountingProvider::saying("REWRITTEN");
    let refreshed = syllabus::refresh_syllabus(&pool, &provider, "pl").await?;
    assert_eq!(
        refreshed, 1,
        "only the machine-generated point is rewritten"
    );

    let llm_prose: String =
        sqlx::query_scalar("SELECT explanation FROM grammar_rules WHERE slug = 'a1-point-0'")
            .fetch_one(&pool)
            .await?;
    let reference_prose: String =
        sqlx::query_scalar("SELECT explanation FROM grammar_rules WHERE slug = 'curated-point'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(llm_prose, "REWRITTEN");
    assert_eq!(
        reference_prose, "ORIGINAL",
        "curated prose is never overwritten"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn reference_import_adopts_existing_point_through_alias() -> TestResult {
    use wisecrow::grammar::syllabus;

    let pool = reset_pool().await?;
    let language_id = seed_language(&pool, "es", "Spanish").await?;
    let level = level_id(&pool, "A1").await?;
    let existing_id: i32 = sqlx::query_scalar(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         VALUES ($1, $2, 'ser-vs-estar-permanent-qualities', 'Ser vs estar', 'x', 'llm')
         RETURNING id",
    )
    .bind(language_id)
    .bind(level)
    .fetch_one(&pool)
    .await?;

    let document = r#"[
        {"slug": "pcic-2-3-ser-estar", "level": "A1", "title": "Ser y estar",
         "explanation": "Curated wording", "adopts": "ser-vs-estar-permanent-qualities"},
        {"slug": "pcic-3-1-preterito", "level": "A2", "title": "Pretérito indefinido",
         "explanation": "Curated wording"}
    ]"#;
    let imported = syllabus::import_syllabus(&pool, "es", document).await?;
    assert_eq!(imported, 2);

    let rows: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT id, slug, source FROM grammar_rules WHERE language_id = $1 ORDER BY slug",
    )
    .bind(language_id)
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        rows.len(),
        2,
        "adoption must not duplicate the adopted point"
    );

    let adopted = rows
        .iter()
        .find(|row| row.0 == existing_id)
        .expect("the original rule survives adoption");
    assert_eq!(
        adopted.1, "ser-vs-estar-permanent-qualities",
        "identity is preserved"
    );
    assert_eq!(adopted.2, "reference", "provenance is upgraded in place");

    let alias_target: i32 = sqlx::query_scalar(
        "SELECT rule_id FROM grammar_rule_aliases
         WHERE language_id = $1 AND alias_slug = 'pcic-2-3-ser-estar'",
    )
    .bind(language_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(alias_target, existing_id);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn export_round_trips_through_import() -> TestResult {
    use wisecrow::grammar::syllabus;

    let pool = reset_pool().await?;
    let provider = CountingProvider::with_rules(2);
    syllabus::ensure_syllabus(&pool, &provider, "pl").await?;

    let exported = syllabus::export_syllabus(&pool, "pl").await?;
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM grammar_rules")
        .fetch_one(&pool)
        .await?;

    let reimported = syllabus::import_syllabus(&pool, "pl", &exported).await?;
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM grammar_rules")
        .fetch_one(&pool)
        .await?;

    assert_eq!(usize::try_from(before).unwrap_or(0), reimported);
    assert_eq!(before, after, "re-importing an export changes nothing");
    Ok(())
}
