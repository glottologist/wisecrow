use dioxus::prelude::*;
use wisecrow_dto::QuizItemDto;

/// Generates quiz items from an uploaded PDF.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized processing errors.
#[post("/api/quiz/pdf")]
pub async fn generate_quiz(
    pdf_bytes: Vec<u8>,
    num_questions: u32,
) -> Result<Vec<QuizItemDto>, ServerFnError> {
    implementation::pdf_quiz(pdf_bytes, num_questions).await
}

/// Generates CEFR rule exercises through the configured LLM provider.
///
/// # Errors
///
/// Returns validation, authentication, quota, or sanitized provider errors.
#[post("/api/quiz/rule")]
pub async fn generate_rule_quiz(
    lang: String,
    level: String,
    num_questions: u32,
) -> Result<Vec<QuizItemDto>, ServerFnError> {
    implementation::rule_quiz(&lang, &level, num_questions).await
}

#[cfg(feature = "server")]
mod implementation {
    use axum::http::StatusCode;
    use wisecrow::grammar::quiz::{ClozeQuiz, MultipleChoiceQuiz};
    use wisecrow_dto::{QuizItemDto, RuleContextDto};

    use super::ServerFnError;

    const MAX_PDF_BYTES: usize = 10 * 1024 * 1024;
    const MAX_QUESTIONS: u32 = 100;
    const PDF_HEADER: &[u8] = b"%PDF-";

    pub(super) async fn pdf_quiz(
        pdf_bytes: Vec<u8>,
        num_questions: u32,
    ) -> Result<Vec<QuizItemDto>, ServerFnError> {
        crate::server::auth::current_user().await?;
        validate_question_count(num_questions)?;
        validate_pdf(&pdf_bytes)?;
        let tmp_file = tempfile::Builder::new()
            .prefix("wisecrow-quiz-")
            .suffix(".pdf")
            .tempfile()
            .map_err(|error| crate::server::internal_error("quiz temp file creation", &error))?;
        tokio::fs::write(tmp_file.path(), &pdf_bytes)
            .await
            .map_err(|error| crate::server::internal_error("quiz PDF write", &error))?;
        let content = wisecrow::grammar::pdf::extract(tmp_file.path())
            .map_err(|error| crate::server::internal_error("quiz PDF extraction", &error))?;
        let (cloze, multiple_choice) = wisecrow::grammar::quiz::assemble_from_content(&content);
        let mut items = wisecrow::dto_convert::quizzes_to_dto(&cloze, &multiple_choice);
        items.truncate(usize::try_from(num_questions).unwrap_or(usize::MAX));
        require_items(items)
    }

    fn validate_pdf(pdf_bytes: &[u8]) -> Result<(), ServerFnError> {
        let header_present = pdf_bytes
            .windows(PDF_HEADER.len())
            .take(1024)
            .any(|window| window == PDF_HEADER);
        if pdf_bytes.len() > MAX_PDF_BYTES {
            return Err(crate::server::client_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "PDF exceeds maximum size of 10 MB",
            ));
        }
        if !header_present {
            return Err(crate::server::client_error(
                StatusCode::BAD_REQUEST,
                "Uploaded file is not a PDF",
            ));
        }
        Ok(())
    }

    fn validate_question_count(num_questions: u32) -> Result<(), ServerFnError> {
        if !(1..=MAX_QUESTIONS).contains(&num_questions) {
            return Err(crate::server::client_error(
                StatusCode::BAD_REQUEST,
                "Question count must be between 1 and 100",
            ));
        }
        Ok(())
    }

    fn require_items(items: Vec<QuizItemDto>) -> Result<Vec<QuizItemDto>, ServerFnError> {
        if items.is_empty() {
            return Err(crate::server::client_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "No quiz questions could be generated",
            ));
        }
        Ok(items)
    }

    pub(super) async fn rule_quiz(
        lang: &str,
        level: &str,
        num_questions: u32,
    ) -> Result<Vec<QuizItemDto>, ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        crate::server::validate_lang(lang)?;
        validate_level(level)?;
        validate_question_count(num_questions)?;
        crate::server::ratelimit::check_llm_quota(user.id)?;
        let settings = config::Config::builder()
            .add_source(config::Environment::with_prefix("WISECROW").separator("__"))
            .build()
            .map_err(|error| crate::server::internal_error("quiz configuration", &error))?;
        let cfg: wisecrow::config::Config = settings
            .try_deserialize()
            .map_err(|error| crate::server::internal_error("quiz configuration", &error))?;
        let provider = wisecrow::llm::create_provider(&cfg)
            .map_err(|error| crate::server::internal_error("quiz LLM provider", &error))?;
        let (cloze, multiple_choice) = wisecrow::grammar::ai_exercises::generate_exercises(
            crate::server::pool()?,
            provider.as_ref(),
            lang,
            level,
            num_questions,
        )
        .await
        .map_err(|error| crate::server::internal_error("rule quiz generation", &error))?;
        assemble_rule_items(&cloze, &multiple_choice).await
    }

    fn validate_level(level: &str) -> Result<(), ServerFnError> {
        if !matches!(level, "A1" | "A2" | "B1" | "B2" | "C1" | "C2") {
            return Err(crate::server::client_error(
                StatusCode::BAD_REQUEST,
                "Invalid CEFR level",
            ));
        }
        Ok(())
    }

    async fn assemble_rule_items(
        cloze: &[ClozeQuiz],
        multiple_choice: &[MultipleChoiceQuiz],
    ) -> Result<Vec<QuizItemDto>, ServerFnError> {
        let shuffled: Vec<_> = multiple_choice
            .iter()
            .enumerate()
            .map(|(index, quiz)| wisecrow::grammar::quiz::shuffle_options(quiz, index))
            .collect();
        let mut items = wisecrow::dto_convert::quizzes_to_dto(cloze, &shuffled);
        attach_rule_contexts(crate::server::pool()?, &mut items, cloze, &shuffled).await?;
        require_items(items)
    }

    async fn attach_rule_contexts(
        db: &sqlx::PgPool,
        items: &mut [QuizItemDto],
        cloze: &[ClozeQuiz],
        multiple_choice: &[MultipleChoiceQuiz],
    ) -> Result<(), ServerFnError> {
        for item in items {
            let Some(rule_id) = rule_id(item, cloze, multiple_choice) else {
                continue;
            };
            let Some(context) = load_rule_context(db, rule_id).await? else {
                continue;
            };
            match item {
                QuizItemDto::Cloze(quiz) => quiz.rule_context = Some(context),
                QuizItemDto::MultipleChoice(quiz) => quiz.rule_context = Some(context),
            }
        }
        Ok(())
    }

    fn rule_id(
        item: &QuizItemDto,
        cloze: &[ClozeQuiz],
        multiple_choice: &[MultipleChoiceQuiz],
    ) -> Option<i32> {
        match item {
            QuizItemDto::Cloze(quiz) => cloze
                .iter()
                .find(|candidate| candidate.sentence_with_blank == quiz.sentence_with_blank)
                .and_then(|candidate| candidate.rule_id),
            QuizItemDto::MultipleChoice(quiz) => multiple_choice
                .iter()
                .find(|candidate| candidate.question == quiz.question)
                .and_then(|candidate| candidate.rule_id),
        }
    }

    async fn load_rule_context(
        db: &sqlx::PgPool,
        rule_id: i32,
    ) -> Result<Option<RuleContextDto>, ServerFnError> {
        let row = sqlx::query_as::<_, (String, String, String)>(
            "SELECT gr.title, gr.explanation, cl.code
             FROM grammar_rules gr
             JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
             WHERE gr.id = $1",
        )
        .bind(rule_id)
        .fetch_optional(db)
        .await
        .map_err(|error| crate::server::internal_error("quiz rule context load", &error))?;
        let Some((title, explanation, cefr_level)) = row else {
            return Ok(None);
        };
        let extra_examples = sqlx::query_scalar::<_, String>(
            "SELECT sentence FROM rule_examples
             WHERE rule_id = $1 AND is_correct = TRUE LIMIT 3",
        )
        .bind(rule_id)
        .fetch_all(db)
        .await
        .map_err(|error| crate::server::internal_error("quiz rule examples load", &error))?;
        Ok(Some(RuleContextDto {
            rule_title: title,
            rule_explanation: explanation,
            cefr_level,
            extra_examples,
        }))
    }
}
