use dioxus::prelude::*;

use wisecrow::dto_convert::quizzes_to_dto;
use wisecrow::grammar::quiz::{shuffle_options, QuizGenerator};
use wisecrow_dto::QuizItemDto;

const MAX_PDF_BYTES: usize = 10 * 1024 * 1024;

#[server]
pub async fn generate_quiz(
    pdf_bytes: Vec<u8>,
    num_questions: u32,
) -> Result<Vec<QuizItemDto>, ServerFnError> {
    if pdf_bytes.len() > MAX_PDF_BYTES {
        return Err(ServerFnError::new(format!(
            "PDF exceeds maximum size of {} MB",
            MAX_PDF_BYTES / (1024 * 1024)
        )));
    }

    let tmp_file = tempfile::Builder::new()
        .prefix("wisecrow-quiz-")
        .suffix(".pdf")
        .tempfile()
        .map_err(|e| ServerFnError::new(format!("Failed to create temp file: {e}")))?;
    let tmp_path = tmp_file.path().to_owned();

    tokio::fs::write(&tmp_path, &pdf_bytes)
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to write temp PDF: {e}")))?;

    let content = wisecrow::grammar::pdf::extract(&tmp_path)
        .map_err(|e| ServerFnError::new(format!("PDF extraction failed: {e}")))?;

    drop(tmp_file);

    let cloze_quizzes = QuizGenerator::cloze_from_examples(
        &content
            .sections
            .iter()
            .flat_map(|s| s.examples.iter().cloned())
            .collect::<Vec<_>>(),
    );

    let mc_quizzes =
        QuizGenerator::multiple_choice_from_rules(&content.sections).unwrap_or_default();

    let shuffled_mc: Vec<_> = mc_quizzes
        .iter()
        .enumerate()
        .map(|(i, mc)| shuffle_options(mc, i))
        .collect();

    let mut items = quizzes_to_dto(&cloze_quizzes, &shuffled_mc);

    let limit = usize::try_from(num_questions).unwrap_or(usize::MAX);
    items.truncate(limit);

    if items.is_empty() {
        return Err(ServerFnError::new(
            "No quiz questions could be generated from the PDF content",
        ));
    }

    Ok(items)
}

#[server]
pub async fn generate_rule_quiz(
    lang: String,
    level: String,
    num_questions: u32,
) -> Result<Vec<QuizItemDto>, ServerFnError> {
    use wisecrow::grammar::ai_exercises::generate_exercises;
    use wisecrow::grammar::quiz::shuffle_options;
    use wisecrow::llm::create_provider;
    use wisecrow_dto::RuleContextDto;

    let db = super::pool()?;

    let settings = config::Config::builder()
        .add_source(config::Environment::with_prefix("WISECROW").separator("__"))
        .build()
        .map_err(|e| ServerFnError::new(format!("Config error: {e}")))?;
    let cfg: wisecrow::config::Config = settings
        .try_deserialize()
        .map_err(|e| ServerFnError::new(format!("Config error: {e}")))?;

    let provider = create_provider(&cfg)
        .map_err(|e| ServerFnError::new(format!("LLM provider error: {e}")))?;

    let (cloze, mc) = generate_exercises(db, provider.as_ref(), &lang, &level, num_questions)
        .await
        .map_err(|e| ServerFnError::new(format!("Exercise generation failed: {e}")))?;

    let shuffled_mc: Vec<_> = mc
        .iter()
        .enumerate()
        .map(|(i, q)| shuffle_options(q, i))
        .collect();

    let mut items = quizzes_to_dto(&cloze, &shuffled_mc);

    for item in &mut items {
        let rule_id = match item {
            QuizItemDto::Cloze(q) => q
                .rule_context
                .is_none()
                .then(|| {
                    cloze
                        .iter()
                        .find(|c| c.sentence_with_blank == q.sentence_with_blank)
                        .and_then(|c| c.rule_id)
                })
                .flatten(),
            QuizItemDto::MultipleChoice(q) => q
                .rule_context
                .is_none()
                .then(|| {
                    shuffled_mc
                        .iter()
                        .find(|m| m.question == q.question)
                        .and_then(|m| m.rule_id)
                })
                .flatten(),
        };

        if let Some(rid) = rule_id {
            if let Ok(Some((title, explanation, cefr_code))) =
                sqlx::query_as::<_, (String, String, String)>(
                    "SELECT gr.title, gr.explanation, cl.code
                     FROM grammar_rules gr
                     JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
                     WHERE gr.id = $1",
                )
                .bind(rid)
                .fetch_optional(db)
                .await
            {
                let examples = sqlx::query_scalar::<_, String>(
                    "SELECT sentence FROM rule_examples WHERE rule_id = $1 AND is_correct = TRUE LIMIT 3",
                )
                .bind(rid)
                .fetch_all(db)
                .await
                .unwrap_or_default();

                let ctx = RuleContextDto {
                    rule_title: title,
                    rule_explanation: explanation,
                    cefr_level: cefr_code,
                    extra_examples: examples,
                };

                match item {
                    QuizItemDto::Cloze(q) => q.rule_context = Some(ctx),
                    QuizItemDto::MultipleChoice(q) => q.rule_context = Some(ctx),
                }
            }
        }
    }

    if items.is_empty() {
        return Err(ServerFnError::new("No exercises could be generated"));
    }

    Ok(items)
}
