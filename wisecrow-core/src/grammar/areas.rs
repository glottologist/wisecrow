//! Area labels for grammar rules.

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::llm::cache::CachedLlm;
use crate::llm::prompts::{rule_area_prompt, RuleAreaInput};

/// How many untagged rules one call sends.
pub const RULE_AREA_BATCH: i64 = 50;
const RULE_AREA_TOKENS: u32 = 2048;

/// The area a rule teaches. Orthogonal to CEFR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Verb,
    Preposition,
    Pronoun,
    Adjective,
    Adverb,
    Other,
}

impl Area {
    /// The value stored in `grammar_rules.area`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verb => "verb",
            Self::Preposition => "preposition",
            Self::Pronoun => "pronoun",
            Self::Adjective => "adjective",
            Self::Adverb => "adverb",
            Self::Other => "other",
        }
    }

    /// Parses a label. An unknown label is `None`, and the caller stores `other`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "verb" => Self::Verb,
            "preposition" => Self::Preposition,
            "pronoun" => Self::Pronoun,
            "adjective" => Self::Adjective,
            "adverb" => Self::Adverb,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// Parses a practice-area request. `all`, blank and omitted mean no filter.
///
/// `other` is a stored rule label, not a deck a learner can ask for.
///
/// # Errors
///
/// Returns [`WisecrowError::InvalidInput`] for any other label, including `other`.
pub fn parse_practice_area(value: Option<&str>) -> Result<Option<Area>, WisecrowError> {
    let Some(value) = value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "all")
    else {
        return Ok(None);
    };
    match value {
        "verb" => Ok(Some(Area::Verb)),
        "preposition" => Ok(Some(Area::Preposition)),
        "pronoun" => Ok(Some(Area::Pronoun)),
        "adjective" => Ok(Some(Area::Adjective)),
        "adverb" => Ok(Some(Area::Adverb)),
        _ => Err(WisecrowError::InvalidInput(format!(
            "unknown area: {value}"
        ))),
    }
}

#[derive(Debug, serde::Deserialize)]
struct AreaBatch {
    areas: Vec<AreaLabel>,
}

#[derive(Debug, serde::Deserialize)]
struct AreaLabel {
    id: i32,
    area: String,
}

/// Classifies one batch of untagged rules.
///
/// Returns how many rows this call wrote. A second call is what classifies
/// the next batch. Already-tagged rules are left as they are.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmUnavailable`] when the model cannot be asked,
/// and a database error when a read or write fails. A rule that the model
/// omits or mislabels is stored as `other` and is not an error.
pub async fn classify_rules(
    pool: &PgPool,
    llm: &CachedLlm<'_>,
    language: &str,
) -> Result<usize, WisecrowError> {
    if !crate::lang::is_valid_code(language) {
        return Err(WisecrowError::InvalidInput(format!(
            "Invalid language code: {language}"
        )));
    }
    let language_name: Option<String> =
        sqlx::query_scalar("SELECT name FROM languages WHERE code = $1")
            .bind(language)
            .fetch_optional(pool)
            .await?;
    let Some(language_name) = language_name else {
        return Err(WisecrowError::InvalidInput(format!(
            "Unknown language: {language}"
        )));
    };
    let rows: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT gr.id, gr.title, gr.explanation
         FROM grammar_rules gr
         JOIN languages l ON l.id = gr.language_id
         WHERE l.code = $1 AND gr.area IS NULL
         ORDER BY gr.id
         LIMIT $2",
    )
    .bind(language)
    .bind(RULE_AREA_BATCH)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let inputs: Vec<RuleAreaInput<'_>> = rows
        .iter()
        .map(|(id, title, explanation)| RuleAreaInput {
            id: *id,
            title,
            explanation,
        })
        .collect();
    let prompt = rule_area_prompt(&language_name, &inputs)?;
    let answer = match llm
        .generate(&prompt, RULE_AREA_TOKENS, parse_areas, |_| true)
        .await
    {
        Ok(answer) => answer,
        Err(error) => {
            let error = WisecrowError::from(error);
            if matches!(error, WisecrowError::LlmUnavailable(_)) {
                return Err(error);
            }
            let quoted = format!(
                "{prompt}\n\nThe previous answer was rejected: {error}\n\
                 Return only the JSON object."
            );
            match llm
                .generate(&quoted, RULE_AREA_TOKENS, parse_areas, |_| true)
                .await
            {
                Ok(answer) => answer,
                Err(error) => {
                    let error = WisecrowError::from(error);
                    if matches!(error, WisecrowError::LlmUnavailable(_)) {
                        return Err(error);
                    }
                    AreaBatch { areas: Vec::new() }
                }
            }
        }
    };
    let mut labels = std::collections::HashMap::<i32, Area>::new();
    for label in answer.areas {
        if let Some(area) = Area::parse(&label.area) {
            labels.insert(label.id, area);
        }
    }
    let mut ids = Vec::with_capacity(rows.len());
    let mut areas = Vec::with_capacity(rows.len());
    for (id, _, _) in &rows {
        ids.push(*id);
        areas.push(labels.get(id).copied().unwrap_or(Area::Other).as_str());
    }
    let written = sqlx::query(
        "UPDATE grammar_rules AS gr
         SET area = batch.area, updated_at = CURRENT_TIMESTAMP
         FROM unnest($1::int[], $2::text[]) AS batch(id, area)
         WHERE gr.id = batch.id AND gr.area IS NULL",
    )
    .bind(&ids)
    .bind(&areas)
    .execute(pool)
    .await?;
    usize::try_from(written.rows_affected())
        .map_err(|_| WisecrowError::InvalidInput("area update count does not fit".into()))
}

fn parse_areas(response: &str) -> Result<AreaBatch, WisecrowError> {
    crate::llm::parse_fenced_json(response, "rule areas")
}
