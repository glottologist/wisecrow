use sqlx::PgPool;
use tracing::info;

use crate::errors::WisecrowError;
use crate::grammar::quiz::{ClozeQuiz, MultipleChoiceQuiz};
use crate::grammar::rules::{GrammarRule, RuleRepository};
use crate::llm::LlmProvider;

/// Generates quiz exercises from stored grammar rules using an LLM.
///
/// # Errors
///
/// Returns an error if the LLM call or database query fails.
pub async fn generate_exercises(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    lang_code: &str,
    cefr_level_code: &str,
    count: u32,
) -> Result<(Vec<ClozeQuiz>, Vec<MultipleChoiceQuiz>), WisecrowError> {
    let persister = crate::ingesting::persisting::DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based
    let language_id = persister.ensure_language(lang_code, lang_code).await?;

    let rules = RuleRepository::rules_for_level(pool, language_id, cefr_level_code).await?;

    if rules.is_empty() {
        return Err(WisecrowError::QuizGenerationError(format!(
            "No grammar rules found for {lang_code} at level {cefr_level_code}"
        )));
    }

    info!(
        "Generating {count} exercises from {} rules via {}",
        rules.len(),
        provider.name()
    );

    let prompt = exercise_generation_prompt(&rules, count);
    let response = provider.generate(&prompt, 4096).await?;

    let parsed = parse_exercise_response(&response)?;

    let mut cloze_quizzes = Vec::new();
    let mut mc_quizzes = Vec::new();

    for item in parsed {
        match item.exercise_type.as_str() {
            "cloze" => {
                if let (Some(sentence), Some(answer)) = (&item.sentence_with_blank, &item.answer) {
                    cloze_quizzes.push(ClozeQuiz {
                        sentence_with_blank: sentence.clone(), // clone: building owned from parsed response
                        answer: answer.clone(), // clone: building owned from parsed response
                        hint: item.hint.clone(), // clone: building owned from parsed response
                        rule_id: item.rule_id,
                    });
                }
            }
            "multiple_choice" => {
                if let (Some(question), Some(options), Some(correct_index)) =
                    (&item.question, &item.options, item.correct_index)
                {
                    mc_quizzes.push(MultipleChoiceQuiz {
                        question: question.clone(), // clone: building owned from parsed response
                        options: options.clone(),   // clone: building owned from parsed response
                        correct_index,
                        rule_id: item.rule_id,
                    });
                }
            }
            _ => {}
        }
    }

    let limit = usize::try_from(count).unwrap_or(usize::MAX);
    cloze_quizzes.truncate(limit);
    mc_quizzes.truncate(limit.saturating_sub(cloze_quizzes.len()));

    Ok((cloze_quizzes, mc_quizzes))
}

#[derive(Debug, serde::Deserialize)]
struct ExerciseItem {
    exercise_type: String,
    rule_id: Option<i32>,
    sentence_with_blank: Option<String>,
    answer: Option<String>,
    hint: Option<String>,
    question: Option<String>,
    options: Option<Vec<String>>,
    correct_index: Option<usize>,
}

fn exercise_generation_prompt(rules: &[GrammarRule], count: u32) -> String {
    let rules_json: Vec<String> = rules
        .iter()
        .map(|r| {
            format!(
                r#"{{"id": {}, "title": "{}", "explanation": "{}"}}"#,
                r.id, r.title, r.explanation
            )
        })
        .collect();

    format!(
        r#"Generate exactly {count} language exercises based on these grammar rules:

[{rules_list}]

Return a JSON array. Each element must be one of:

Cloze type:
{{
  "exercise_type": "cloze",
  "rule_id": <rule id from the list>,
  "sentence_with_blank": "The student ____ to school every day",
  "answer": "goes",
  "hint": "Starts with 'g'"
}}

Multiple choice type:
{{
  "exercise_type": "multiple_choice",
  "rule_id": <rule id from the list>,
  "question": "Which sentence uses the correct verb form?",
  "options": ["He goes to school", "He go to school", "He going to school", "He goed to school"],
  "correct_index": 0
}}

Requirements:
- Mix cloze and multiple choice roughly equally
- Each exercise must reference a rule_id from the provided list
- Sentences should be natural and at the appropriate difficulty level
- Multiple choice must have exactly 4 options with one correct answer
- Return ONLY the JSON array"#,
        rules_list = rules_json.join(",\n")
    )
}

fn parse_exercise_response(response: &str) -> Result<Vec<ExerciseItem>, WisecrowError> {
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
        .map_err(|e| WisecrowError::LlmError(format!("Failed to parse exercise response: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cloze_exercise() {
        let input = r#"[{"exercise_type":"cloze","rule_id":1,"sentence_with_blank":"He ____ to school","answer":"goes","hint":"Starts with 'g'"}]"#;
        let result = parse_exercise_response(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].exercise_type, "cloze");
        assert_eq!(result[0].rule_id, Some(1));
    }

    #[test]
    fn parse_mc_exercise() {
        let input = r#"[{"exercise_type":"multiple_choice","rule_id":2,"question":"Which is correct?","options":["A","B","C","D"],"correct_index":0}]"#;
        let result = parse_exercise_response(input).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].exercise_type, "multiple_choice");
        assert_eq!(result[0].correct_index, Some(0));
    }

    #[test]
    fn parse_exercise_with_code_fence() {
        let input = "```json\n[{\"exercise_type\":\"cloze\",\"rule_id\":1,\"sentence_with_blank\":\"test\",\"answer\":\"a\"}]\n```";
        let result = parse_exercise_response(input).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn parse_invalid_exercise_returns_error() {
        assert!(parse_exercise_response("not json").is_err());
    }
}
