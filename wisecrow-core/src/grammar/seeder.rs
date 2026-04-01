use serde::Deserialize;
use sqlx::PgPool;
use tracing::info;

use super::rules::{NewGrammarRule, NewRuleExample, RuleRepository, RuleSource};
use crate::errors::WisecrowError;
use crate::ingesting::persisting::DatabasePersister;
use crate::llm::prompts::grammar_seed_prompt;
use crate::llm::LlmProvider;

const RULES_PER_LEVEL: u32 = 15;
const MAX_LLM_TOKENS: u32 = 4096;

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

fn default_true() -> bool {
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

        for rule_import in &imported {
            let new_rule = NewGrammarRule {
                title: rule_import.title.clone(), // clone: building owned struct from borrowed import
                explanation: rule_import.explanation.clone(), // clone: building owned struct from borrowed import
                source: RuleSource::Ai,
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

            RuleRepository::upsert_rule(pool, language_id, cefr_level_id, &new_rule).await?;
            total = total.saturating_add(1);
        }

        info!(
            "Persisted {} rules for {lang_name} {level_code}",
            imported.len()
        );
    }

    Ok(total)
}

/// Parses JSON from an LLM response, tolerating markdown code fences.
fn parse_llm_json(response: &str) -> Result<Vec<LlmGrammarRule>, WisecrowError> {
    let trimmed = response.trim();
    let json_str = if trimmed.starts_with("```") {
        trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
    } else {
        trimmed
    };

    serde_json::from_str(json_str)
        .map_err(|e| WisecrowError::LlmError(format!("Failed to parse LLM response as JSON: {e}")))
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
