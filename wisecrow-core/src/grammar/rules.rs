use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::WisecrowError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CefrLevel {
    code: String,
    name: String,
    sort_order: i16,
}

impl CefrLevel {
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "A1" => Some(Self {
                code: "A1".to_owned(),
                name: "Beginner".to_owned(),
                sort_order: 1,
            }),
            "A2" => Some(Self {
                code: "A2".to_owned(),
                name: "Elementary".to_owned(),
                sort_order: 2,
            }),
            "B1" => Some(Self {
                code: "B1".to_owned(),
                name: "Intermediate".to_owned(),
                sort_order: 3,
            }),
            "B2" => Some(Self {
                code: "B2".to_owned(),
                name: "Upper Intermediate".to_owned(),
                sort_order: 4,
            }),
            "C1" => Some(Self {
                code: "C1".to_owned(),
                name: "Advanced".to_owned(),
                sort_order: 5,
            }),
            "C2" => Some(Self {
                code: "C2".to_owned(),
                name: "Proficiency".to_owned(),
                sort_order: 6,
            }),
            _ => None,
        }
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn sort_order(&self) -> i16 {
        self.sort_order
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleSource {
    Manual,
    Ai,
    Pdf,
}

impl RuleSource {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Ai => "ai",
            Self::Pdf => "pdf",
        }
    }
}

impl FromStr for RuleSource {
    type Err = WisecrowError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "manual" => Ok(Self::Manual),
            "ai" => Ok(Self::Ai),
            "pdf" => Ok(Self::Pdf),
            _ => Err(WisecrowError::InvalidInput(format!(
                "Unknown rule source: {s}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GrammarRule {
    pub id: i32,
    pub language_id: i32,
    pub cefr_level_id: i32,
    pub title: String,
    pub explanation: String,
    pub source: RuleSource,
    pub examples: Vec<RuleExample>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RuleExample {
    pub id: i32,
    pub sentence: String,
    pub translation: Option<String>,
    pub is_correct: bool,
}

#[derive(Debug, Clone)]
pub struct NewGrammarRule {
    pub title: String,
    pub explanation: String,
    pub source: RuleSource,
    pub examples: Vec<NewRuleExample>,
}

#[derive(Debug, Clone)]
pub struct NewRuleExample {
    pub sentence: String,
    pub translation: Option<String>,
    pub is_correct: bool,
}

pub struct RuleRepository;

impl RuleRepository {
    /// Inserts a grammar rule and its examples for a given language and CEFR level.
    /// Uses upsert on (language_id, cefr_level_id, title).
    pub async fn upsert_rule(
        pool: &PgPool,
        language_id: i32,
        cefr_level_id: i32,
        rule: &NewGrammarRule,
    ) -> Result<i32, WisecrowError> {
        let row = sqlx::query_scalar::<_, i32>(
            "INSERT INTO grammar_rules (language_id, cefr_level_id, title, explanation, source)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (language_id, cefr_level_id, title)
             DO UPDATE SET explanation = EXCLUDED.explanation,
                           source = EXCLUDED.source,
                           updated_at = CURRENT_TIMESTAMP
             RETURNING id",
        )
        .bind(language_id)
        .bind(cefr_level_id)
        .bind(&rule.title)
        .bind(&rule.explanation)
        .bind(rule.source.as_str())
        .fetch_one(pool)
        .await?;

        sqlx::query("DELETE FROM rule_examples WHERE rule_id = $1")
            .bind(row)
            .execute(pool)
            .await?;

        for example in &rule.examples {
            sqlx::query(
                "INSERT INTO rule_examples (rule_id, sentence, translation, is_correct)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(row)
            .bind(&example.sentence)
            .bind(example.translation.as_deref())
            .bind(example.is_correct)
            .execute(pool)
            .await?;
        }

        Ok(row)
    }

    /// Fetches all grammar rules for a language and CEFR level code.
    pub async fn rules_for_level(
        pool: &PgPool,
        language_id: i32,
        cefr_level_code: &str,
    ) -> Result<Vec<GrammarRule>, WisecrowError> {
        let rows = sqlx::query_as::<
            _,
            (
                i32,
                i32,
                i32,
                String,
                String,
                String,
                DateTime<Utc>,
                DateTime<Utc>,
            ),
        >(
            "SELECT gr.id, gr.language_id, gr.cefr_level_id, gr.title, gr.explanation, gr.source,
                    gr.created_at, gr.updated_at
             FROM grammar_rules gr
             JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
             WHERE gr.language_id = $1 AND cl.code = $2
             ORDER BY gr.title",
        )
        .bind(language_id)
        .bind(cefr_level_code)
        .fetch_all(pool)
        .await?;

        let mut rules = Vec::with_capacity(rows.len());
        for (id, lang_id, level_id, title, explanation, source_str, created, updated) in rows {
            let examples = sqlx::query_as::<_, (i32, String, Option<String>, bool)>(
                "SELECT id, sentence, translation, is_correct
                 FROM rule_examples WHERE rule_id = $1 ORDER BY id",
            )
            .bind(id)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|(eid, sentence, translation, is_correct)| RuleExample {
                id: eid,
                sentence,
                translation,
                is_correct,
            })
            .collect();

            rules.push(GrammarRule {
                id,
                language_id: lang_id,
                cefr_level_id: level_id,
                title,
                explanation,
                source: source_str.parse().unwrap_or(RuleSource::Manual),
                examples,
                created_at: created,
                updated_at: updated,
            });
        }

        Ok(rules)
    }

    /// Resolves a CEFR level by code, returning its DB id.
    pub async fn ensure_cefr_level(pool: &PgPool, code: &str) -> Result<i32, WisecrowError> {
        let level = CefrLevel::from_code(code)
            .ok_or_else(|| WisecrowError::InvalidInput(format!("Invalid CEFR level: {code}")))?;

        let id = sqlx::query_scalar::<_, i32>("SELECT id FROM cefr_levels WHERE code = $1")
            .bind(level.code())
            .fetch_one(pool)
            .await?;

        Ok(id)
    }

    /// Returns the count of grammar rules for a language.
    pub async fn count_rules(pool: &PgPool, language_id: i32) -> Result<i64, WisecrowError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM grammar_rules WHERE language_id = $1",
        )
        .bind(language_id)
        .fetch_one(pool)
        .await?;

        Ok(count)
    }
}

/// Imports grammar rules from a JSON file.
pub async fn import_from_json(
    pool: &PgPool,
    language_id: i32,
    path: &std::path::Path,
) -> Result<usize, WisecrowError> {
    let content = std::fs::read_to_string(path)?;
    let imports: Vec<wisecrow_dto::GrammarRuleImport> = serde_json::from_str(&content)
        .map_err(|e| WisecrowError::InvalidInput(format!("Invalid JSON: {e}")))?;

    let mut count = 0usize;
    for rule_import in &imports {
        let cefr_level_id =
            RuleRepository::ensure_cefr_level(pool, &rule_import.cefr_level).await?;
        let new_rule = NewGrammarRule {
            title: rule_import.title.clone(), // clone: building owned struct from borrowed import
            explanation: rule_import.explanation.clone(), // clone: building owned struct from borrowed import
            source: RuleSource::Manual,
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
        count = count.saturating_add(1);
    }

    Ok(count)
}

/// Imports grammar rules extracted from a PDF file.
pub async fn import_from_pdf(
    pool: &PgPool,
    language_id: i32,
    cefr_level_code: &str,
    pdf_path: &std::path::Path,
) -> Result<usize, WisecrowError> {
    let content = crate::grammar::pdf::extract(pdf_path)?;
    let cefr_level_id = RuleRepository::ensure_cefr_level(pool, cefr_level_code).await?;

    let mut count = 0usize;
    for section in &content.sections {
        let title = section.title.as_deref().unwrap_or("Untitled Rule");

        for rule_text in &section.rules {
            let new_rule = NewGrammarRule {
                title: format!("{title}: {}", truncate(rule_text, 100)),
                explanation: rule_text.clone(), // clone: building owned struct from borrowed extraction
                source: RuleSource::Pdf,
                examples: section
                    .examples
                    .iter()
                    .map(|ex| NewRuleExample {
                        sentence: ex.text.clone(), // clone: building owned struct from borrowed extraction
                        translation: ex.translation.clone(), // clone: building owned struct from borrowed extraction
                        is_correct: true,
                    })
                    .collect(),
            };
            RuleRepository::upsert_rule(pool, language_id, cefr_level_id, &new_rule).await?;
            count = count.saturating_add(1);
        }
    }

    Ok(count)
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        let mut end = max_len;
        while !s.is_char_boundary(end) && end > 0 {
            end = end.saturating_sub(1);
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn cefr_level_roundtrip(code in "(A1|A2|B1|B2|C1|C2)") {
            let level = CefrLevel::from_code(&code).unwrap();
            prop_assert_eq!(level.code(), code.as_str());
        }
    }

    #[test]
    fn cefr_level_invalid_code_returns_none() {
        assert!(CefrLevel::from_code("Z9").is_none());
        assert!(CefrLevel::from_code("").is_none());
    }
}
