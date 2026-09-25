use serde::Deserialize;
use sqlx::PgPool;
use tracing::info;

use super::rules::{NewGrammarRule, NewRuleExample, RulePlacement, RuleRepository, RuleSource};
use crate::errors::WisecrowError;
use crate::ingesting::persisting::DatabasePersister;
use crate::llm::prompts::grammar_seed_prompt;
use crate::llm::LlmProvider;

const RULES_PER_LEVEL: u32 = 15;
/// Fifteen rules, each with prose and two examples, is a long answer. Measured
/// on Scottish Gaelic against `claude-sonnet-5`: B2 came back at 4062 output
/// tokens and C1 at 3821, so the former 4096 ceiling left tens of tokens of
/// headroom and cut whichever level ran slightly longer. Double it.
const MAX_LLM_TOKENS: u32 = 8192;

/// LLM response shape for a single grammar rule.
///
/// Separate from `GrammarRuleImport` because the LLM prompt does not
/// include `cefr_level` -- the caller already knows the level.
#[derive(Debug, Deserialize)]
struct LlmGrammarRule {
    title: String,
    explanation: String,
    examples: Vec<LlmRuleExample>,
}

#[derive(Debug, Deserialize)]
struct LlmRuleExample {
    sentence: String,
    translation: Option<String>,
    #[serde(default = "default_true")]
    is_correct: bool,
}

const fn default_true() -> bool {
    true
}

/// Seeds grammar rules for a language and set of CEFR levels using an LLM.
///
/// # Errors
///
/// Returns an error if the LLM call or database persistence fails.
pub async fn seed_grammar(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    lang_code: &str,
    lang_name: &str,
    levels: &[&str],
) -> Result<usize, WisecrowError> {
    let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based
    let language_id = persister.ensure_language(lang_code, lang_name).await?;
    let mut total = 0usize;

    for level_code in levels {
        info!(
            "Generating {RULES_PER_LEVEL} rules for {lang_name} {level_code} via {}",
            provider.name()
        );

        let prompt = grammar_seed_prompt(lang_name, level_code, RULES_PER_LEVEL);
        let response = provider.generate(&prompt, MAX_LLM_TOKENS).await?;

        let imported: Vec<LlmGrammarRule> = parse_llm_json(&response)?;
        let cefr_level_id = RuleRepository::ensure_cefr_level(pool, level_code).await?;
        let mut held = 0usize;

        for rule_import in &imported {
            let new_rule = NewGrammarRule {
                slug: crate::grammar::rules::slugify(&rule_import.title),
                title: rule_import.title.clone(), // clone: building owned struct from borrowed import
                explanation: rule_import.explanation.clone(), // clone: building owned struct from borrowed import
                source: RuleSource::Llm,
                examples: rule_import
                    .examples
                    .iter()
                    .map(|ex| NewRuleExample {
                        sentence: ex.sentence.clone(), // clone: building owned struct from borrowed import
                        translation: ex.translation.clone(), // clone: building owned struct from borrowed import
                        is_correct: ex.is_correct,
                    })
                    .collect(),
            };

            match RuleRepository::place_rule(pool, language_id, cefr_level_id, &new_rule).await? {
                RulePlacement::Placed(_) => total = total.saturating_add(1),
                RulePlacement::HeldAtAnotherLevel(_) => {
                    held = held.saturating_add(1);
                }
            }
        }

        let placed = imported.len().saturating_sub(held);
        if held > 0 {
            info!(
                "Persisted {placed} rules for {lang_name} {level_code}; \
                 {held} were points the model had already placed at another level"
            );
        } else {
            info!("Persisted {placed} rules for {lang_name} {level_code}");
        }
    }

    Ok(total)
}

/// Generates one level's worth of rules without persisting them.
///
/// [`crate::grammar::syllabus::refresh_syllabus`] needs the model's wording
/// for points that already exist, which is the same request seeding makes but
/// a different use of the answer.
///
/// # Errors
///
/// Returns an error when the model call fails or its answer will not parse.
pub(crate) async fn generate_level_rules(
    provider: &dyn LlmProvider,
    lang_name: &str,
    level_code: &str,
) -> Result<Vec<(String, String)>, WisecrowError> {
    let prompt = grammar_seed_prompt(lang_name, level_code, RULES_PER_LEVEL);
    let response = provider.generate(&prompt, MAX_LLM_TOKENS).await?;
    Ok(parse_llm_json(&response)?
        .into_iter()
        .map(|rule| (rule.title, rule.explanation))
        .collect())
}

/// Parses JSON from an LLM response, tolerating markdown code fences.
fn parse_llm_json(response: &str) -> Result<Vec<LlmGrammarRule>, WisecrowError> {
    crate::llm::parse_fenced_json(response, "LLM response as JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_llm_json_with_code_fence() {
        let input = r#"```json
[{"title":"Test","explanation":"Explain","examples":[{"sentence":"Hello","is_correct":true}]}]
```"#;
        let result = parse_llm_json(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].title, "Test");
    }

    #[test]
    fn parse_llm_json_without_fence() {
        let input = r#"[{"title":"Test","explanation":"Explain","examples":[]}]"#;
        let result = parse_llm_json(input).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn parse_llm_json_invalid_returns_error() {
        assert!(parse_llm_json("not json").is_err());
    }
}
