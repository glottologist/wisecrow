use dioxus::prelude::*;
use uuid::Uuid;
use wisecrow_dto::{BrainmapDto, GrammarSessionDto, PlacementStateDto, SubmissionDto, VerdictDto};

/// Builds the transport facts a submission carries but a learner does not.
///
/// The browser answers in the moment, so the client's clock is not consulted:
/// `occurred_at` is when the server heard it. A mobile upload, which may be
/// days old, supplies its own through the sync routes instead.
#[cfg(feature = "server")]
fn web_context() -> wisecrow::grammar::session::SubmissionContext {
    wisecrow::grammar::session::SubmissionContext {
        occurred_at: chrono::Utc::now(),
        device_id: None,
        source: wisecrow::grammar::mastery::AttemptSource::Web,
    }
}

/// Opens a grammar practice session for an authenticated learner.
///
/// Returns `None` when the language has no promoted items, which is the
/// ordinary state of a freshly ingested language rather than a fault.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/grammar/session/start")]
pub async fn start_grammar_session(
    native: String,
    foreign: String,
    level: Option<String>,
) -> Result<Option<GrammarSessionDto>, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;

    let session = wisecrow::grammar::session::GrammarSessionManager::create(
        crate::server::pool()?,
        user.id,
        &foreign,
        level.as_deref(),
        wisecrow::grammar::session::SessionKind::Practice,
        wisecrow::grammar::session::SESSION_LENGTH,
    )
    .await
    .map_err(|error| crate::server::internal_error("grammar session creation", &error))?;

    Ok(session.map(|session| GrammarSessionDto {
        session_id: session.id,
        items: session
            .items
            .iter()
            .map(wisecrow::dto_convert::grammar_item)
            .collect(),
    }))
}

/// Records one answer and returns the server's own verdict on it.
///
/// # Errors
///
/// Returns authentication, conflict, or sanitized storage errors.
#[post("/api/grammar/session/submit")]
pub async fn submit_grammar_answer(submission: SubmissionDto) -> Result<VerdictDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    let event_id = submission.event_id;
    let verdict = wisecrow::grammar::session::GrammarSessionManager::submit(
        crate::server::pool()?,
        user.id,
        &wisecrow::dto_convert::submission(&submission),
        web_context(),
    )
    .await
    .map_err(|error| crate::server::internal_error("grammar answer", &error))?;

    Ok(VerdictDto {
        event_id,
        correct: verdict.correct,
    })
}

/// Closes a grammar session.
///
/// # Errors
///
/// Returns authentication or sanitized storage errors.
#[post("/api/grammar/session/complete")]
pub async fn complete_grammar_session(session_id: Uuid) -> Result<(), ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    wisecrow::grammar::session::GrammarSessionManager::complete(
        crate::server::pool()?,
        user.id,
        session_id,
    )
    .await
    .map_err(|error| crate::server::internal_error("grammar session completion", &error))
}

/// The name a learner recognises, falling back to the code.
///
/// `validate_lang` has already accepted the code, so the fallback covers only
/// a code the supported table has yet to name rather than bad input.
#[cfg(feature = "server")]
fn language_name(code: &str) -> String {
    wisecrow::cli::SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map_or_else(|| code.to_owned(), |(_, name)| (*name).to_owned())
}

/// Reports every syllabus point of a language with the learner's mastery.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/grammar/brainmap")]
pub async fn grammar_brainmap(
    native: String,
    foreign: String,
) -> Result<BrainmapDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;

    let rows = wisecrow::grammar::mastery::MasteryRepository::mastery_for_language(
        crate::server::pool()?,
        user.id,
        &foreign,
    )
    .await
    .map_err(|error| crate::server::internal_error("grammar brainmap", &error))?;

    Ok(BrainmapDto {
        language: language_name(&foreign),
        cells: rows
            .iter()
            .map(wisecrow::dto_convert::brainmap_cell)
            .collect(),
    })
}

/// Opens a placement run and draws its first level.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/grammar/placement/start")]
pub async fn start_placement(
    native: String,
    foreign: String,
) -> Result<PlacementStateDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;

    let state = wisecrow::grammar::placement::start(crate::server::pool()?, user.id, &foreign)
        .await
        .map_err(|error| crate::server::internal_error("placement start", &error))?;
    Ok(wisecrow::dto_convert::placement_state(&state))
}

/// Records a level's answers and returns the next level, or the verdict.
///
/// # Errors
///
/// Returns authentication, conflict, or sanitized storage errors.
#[post("/api/grammar/placement/submit")]
pub async fn submit_placement(
    attempt_id: Uuid,
    answers: Vec<SubmissionDto>,
) -> Result<PlacementStateDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    let submissions: Vec<_> = answers
        .iter()
        .map(wisecrow::dto_convert::submission)
        .collect();

    let state = wisecrow::grammar::placement::submit(
        crate::server::pool()?,
        user.id,
        attempt_id,
        &submissions,
        web_context(),
    )
    .await
    .map_err(|error| crate::server::internal_error("placement answers", &error))?;
    Ok(wisecrow::dto_convert::placement_state(&state))
}
