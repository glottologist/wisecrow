use dioxus::prelude::*;

use wisecrow::dto_convert::quizzes_to_dto;
use wisecrow::grammar::quiz::{shuffle_options, QuizGenerator};
use wisecrow_dto::QuizItemDto;

#[server]
pub async fn generate_quiz(
    pdf_bytes: Vec<u8>,
    num_questions: u32,
) -> Result<Vec<QuizItemDto>, ServerFnError> {
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!("wisecrow-quiz-{}.pdf", std::process::id()));

    tokio::fs::write(&tmp_path, &pdf_bytes)
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to write temp PDF: {e}")))?;

    let content = wisecrow::grammar::pdf::extract(&tmp_path)
        .map_err(|e| ServerFnError::new(format!("PDF extraction failed: {e}")))?;

    let _ = tokio::fs::remove_file(&tmp_path).await;

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
