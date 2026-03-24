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

    let unique_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_dir = std::env::temp_dir();
    let tmp_path = tmp_dir.join(format!("wisecrow-quiz-{unique_id}.pdf"));

    tokio::fs::write(&tmp_path, &pdf_bytes)
        .await
        .map_err(|e| ServerFnError::new(format!("Failed to write temp PDF: {e}")))?;

    let content = wisecrow::grammar::pdf::extract(&tmp_path)
        .map_err(|e| ServerFnError::new(format!("PDF extraction failed: {e}")))?;

    if let Err(e) = tokio::fs::remove_file(&tmp_path).await {
        tracing::debug!("Temp file cleanup failed: {e}");
    }

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
