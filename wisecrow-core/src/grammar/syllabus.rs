//! Gap-filling syllabus seeding.
//!
//! A language acquires its syllabus automatically once its corpus has been
//! ingested, which means seeding runs unattended and repeatedly. It must
//! therefore never disturb a point that already exists: rewriting a syllabus
//! underneath a learner's accumulated mastery is exactly the failure the
//! stable slug exists to prevent. Refreshing wording is a separate, explicit
//! act, and lives in [`refresh_syllabus`].

use sqlx::PgPool;
use tracing::{info, warn};

use super::rules::RuleRepository;
use super::seeder::seed_grammar;
use crate::cli::SUPPORTED_LANGUAGE_INFO;
use crate::errors::WisecrowError;
use crate::ingesting::persisting::DatabasePersister;
use crate::llm::LlmProvider;

/// Every CEFR level a syllabus is expected to cover.
pub const ALL_LEVELS: [&str; 6] = ["A1", "A2", "B1", "B2", "C1", "C2"];

/// What one [`ensure_syllabus`] run did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnsureSummary {
    pub levels_filled: usize,
    pub levels_skipped: usize,
    pub points_added: usize,
}

impl EnsureSummary {
    fn absorb(&mut self, other: Self) {
        self.levels_filled = self.levels_filled.saturating_add(other.levels_filled);
        self.levels_skipped = self.levels_skipped.saturating_add(other.levels_skipped);
        self.points_added = self.points_added.saturating_add(other.points_added);
    }
}

/// Fills any CEFR level holding no points for this language.
///
/// Existing points are left exactly as they stand, prose included, so the
/// command is safe to run on a schedule and safe to run twice.
///
/// # Errors
///
/// Returns an error when the language is unsupported, or when a model call or
/// database write fails.
pub async fn ensure_syllabus(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    lang_code: &str,
) -> Result<EnsureSummary, WisecrowError> {
    let lang_name = language_name(lang_code)?;
    let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based
    let language_id = persister.ensure_language(lang_code, lang_name).await?;

    let mut summary = EnsureSummary::default();
    for level in ALL_LEVELS {
        if level_is_populated(pool, language_id, level).await? {
            summary.levels_skipped = summary.levels_skipped.saturating_add(1);
            continue;
        }

        let before = RuleRepository::count_rules(pool, language_id).await?;
        seed_grammar(pool, provider, lang_code, lang_name, &[level]).await?;
        let after = RuleRepository::count_rules(pool, language_id).await?;

        summary.levels_filled = summary.levels_filled.saturating_add(1);
        summary.points_added = summary
            .points_added
            .saturating_add(usize::try_from(after.saturating_sub(before)).unwrap_or(0));
    }

    info!(
        "Syllabus for {lang_code}: {} levels filled, {} already populated, {} points added",
        summary.levels_filled, summary.levels_skipped, summary.points_added
    );
    Ok(summary)
}

/// Fills gaps for every language already present in the corpus.
///
/// One language's failure does not abort the sweep: an unavailable model
/// should leave a gap to be closed by the next run, not halt the rest.
///
/// # Errors
///
/// Returns an error only when the languages themselves cannot be listed.
pub async fn ensure_all_syllabuses(
    pool: &PgPool,
    provider: &dyn LlmProvider,
) -> Result<EnsureSummary, WisecrowError> {
    let codes: Vec<String> = sqlx::query_scalar("SELECT code FROM languages ORDER BY code")
        .fetch_all(pool)
        .await?;

    let mut summary = EnsureSummary::default();
    for code in codes {
        match ensure_syllabus(pool, provider, &code).await {
            Ok(one) => summary.absorb(one),
            Err(error) => warn!("Syllabus for {code} could not be completed: {error}"),
        }
    }
    Ok(summary)
}

async fn level_is_populated(
    pool: &PgPool,
    language_id: i32,
    level_code: &str,
) -> Result<bool, WisecrowError> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM grammar_rules gr
         JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
         WHERE gr.language_id = $1 AND cl.code = $2",
    )
    .bind(language_id)
    .bind(level_code)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

fn language_name(lang_code: &str) -> Result<&'static str, WisecrowError> {
    SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(code, _)| *code == lang_code)
        .map(|(_, name)| *name)
        .ok_or_else(|| WisecrowError::InvalidInput(format!("Unsupported language: {lang_code}")))
}

/// One entry of an exported or imported syllabus document.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SyllabusEntry {
    pub slug: String,
    pub level: String,
    pub title: String,
    pub explanation: String,
    /// Provenance tier; defaults to the curated tier on import, since a
    /// document written by hand is the only reason to import one.
    #[serde(default = "default_reference_source")]
    pub source: String,
    /// Names an existing point this entry describes under a different slug.
    /// Adoption upgrades that point in place rather than duplicating it,
    /// which is what preserves a learner's history across the upgrade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopts: Option<String>,
}

fn default_reference_source() -> String {
    "reference".to_owned()
}

/// Rewrites the prose of machine-generated points, matched by slug.
///
/// Curated points are never touched: a reference inventory is authoritative
/// precisely because a model cannot quietly reword it.
///
/// # Errors
///
/// Returns an error when the language is unsupported, or when a model call or
/// database write fails.
pub async fn refresh_syllabus(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    lang_code: &str,
) -> Result<usize, WisecrowError> {
    let lang_name = language_name(lang_code)?;
    let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based
    let language_id = persister.ensure_language(lang_code, lang_name).await?;

    let mut refreshed = 0usize;
    for level in ALL_LEVELS {
        let generated = super::seeder::generate_level_rules(provider, lang_name, level).await?;
        for (title, explanation) in generated {
            let slug = super::rules::slugify(&title);
            let updated = sqlx::query(
                "UPDATE grammar_rules
                 SET explanation = $3, updated_at = CURRENT_TIMESTAMP
                 WHERE language_id = $1 AND slug = $2 AND source = 'llm'",
            )
            .bind(language_id)
            .bind(&slug)
            .bind(&explanation)
            .execute(pool)
            .await?;
            refreshed =
                refreshed.saturating_add(usize::try_from(updated.rows_affected()).unwrap_or(0));
        }
    }

    info!("Refreshed prose for {refreshed} machine-generated points in {lang_code}");
    Ok(refreshed)
}

/// Serialises a language's syllabus as JSON, for diffing or backup.
///
/// # Errors
///
/// Returns an error when the language is unsupported or the query fails.
pub async fn export_syllabus(pool: &PgPool, lang_code: &str) -> Result<String, WisecrowError> {
    language_name(lang_code)?;
    let rows = sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT gr.slug, cl.code, gr.title, gr.explanation, gr.source
         FROM grammar_rules gr
         JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
         JOIN languages l ON l.id = gr.language_id
         WHERE l.code = $1
         ORDER BY cl.sort_order, gr.slug",
    )
    .bind(lang_code)
    .fetch_all(pool)
    .await?;

    let entries: Vec<SyllabusEntry> = rows
        .into_iter()
        .map(|(slug, level, title, explanation, source)| SyllabusEntry {
            slug,
            level,
            title,
            explanation,
            source,
            adopts: None,
        })
        .collect();

    serde_json::to_string_pretty(&entries)
        .map_err(|error| WisecrowError::InvalidInput(format!("Cannot serialise syllabus: {error}")))
}

/// Applies a curated syllabus document, adopting existing points where the
/// document says so.
///
/// # Errors
///
/// Returns an error when the document will not parse, the language is
/// unsupported, or a database write fails.
pub async fn import_syllabus(
    pool: &PgPool,
    lang_code: &str,
    document: &str,
) -> Result<usize, WisecrowError> {
    let lang_name = language_name(lang_code)?;
    let entries: Vec<SyllabusEntry> = serde_json::from_str(document)
        .map_err(|error| WisecrowError::InvalidInput(format!("Invalid syllabus JSON: {error}")))?;

    let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based
    let language_id = persister.ensure_language(lang_code, lang_name).await?;

    let mut applied = 0usize;
    for entry in &entries {
        let level_id = RuleRepository::ensure_cefr_level(pool, &entry.level).await?;
        let source = entry.source.parse::<super::rules::RuleSource>()?;

        match &entry.adopts {
            Some(adopted_slug) => {
                adopt_existing_point(pool, language_id, level_id, entry, adopted_slug, source)
                    .await?;
            }
            None => {
                RuleRepository::upsert_rule(
                    pool,
                    language_id,
                    level_id,
                    &super::rules::NewGrammarRule {
                        slug: entry.slug.clone(),   // clone: building owned from borrowed entry
                        title: entry.title.clone(), // clone: building owned from borrowed entry
                        explanation: entry.explanation.clone(), // clone: building owned from borrowed entry
                        source,
                        examples: vec![],
                    },
                )
                .await?;
            }
        }
        applied = applied.saturating_add(1);
    }

    Ok(applied)
}

/// Upgrades an existing point in place and records the document's own slug as
/// an alias, so that a learner's mastery survives the change of provenance.
async fn adopt_existing_point(
    pool: &PgPool,
    language_id: i32,
    level_id: i32,
    entry: &SyllabusEntry,
    adopted_slug: &str,
    source: super::rules::RuleSource,
) -> Result<(), WisecrowError> {
    let rule_id = sqlx::query_scalar::<_, i32>(
        "SELECT id FROM grammar_rules WHERE language_id = $1 AND slug = $2",
    )
    .bind(language_id)
    .bind(adopted_slug)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        WisecrowError::InvalidInput(format!(
            "Entry {} adopts {adopted_slug}, which does not exist",
            entry.slug
        ))
    })?;

    sqlx::query(
        "UPDATE grammar_rules
         SET cefr_level_id = $2, title = $3, explanation = $4, source = $5,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = $1",
    )
    .bind(rule_id)
    .bind(level_id)
    .bind(&entry.title)
    .bind(&entry.explanation)
    .bind(source.as_str())
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO grammar_rule_aliases (language_id, alias_slug, rule_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (language_id, alias_slug) DO UPDATE SET rule_id = EXCLUDED.rule_id",
    )
    .bind(language_id)
    .bind(&entry.slug)
    .bind(rule_id)
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_cefr_level_is_covered() {
        assert_eq!(ALL_LEVELS.len(), 6);
        assert!(ALL_LEVELS.contains(&"A1") && ALL_LEVELS.contains(&"C2"));
    }

    #[test]
    fn an_unsupported_language_is_refused_before_any_model_call() {
        assert!(language_name("not-a-language").is_err());
        assert_eq!(language_name("pl").expect("supported"), "Polish");
    }

    #[test]
    fn summaries_accumulate() {
        let mut total = EnsureSummary::default();
        total.absorb(EnsureSummary {
            levels_filled: 2,
            levels_skipped: 4,
            points_added: 30,
        });
        total.absorb(EnsureSummary {
            levels_filled: 1,
            levels_skipped: 5,
            points_added: 15,
        });
        assert_eq!(
            total,
            EnsureSummary {
                levels_filled: 3,
                levels_skipped: 9,
                points_added: 45
            }
        );
    }
}
