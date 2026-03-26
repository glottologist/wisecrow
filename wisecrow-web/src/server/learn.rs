use dioxus::prelude::*;

use wisecrow::cli::SUPPORTED_LANGUAGE_INFO;
use wisecrow::dto_convert::language_info;
use wisecrow::srs::scheduler::{CardManager, ReviewRating};
use wisecrow::srs::session::SessionManager;
use wisecrow_dto::{CardDto, LanguageInfo, ReviewRatingDto, SessionDto};

use super::{pool, validate_lang};

#[server]
pub async fn list_languages() -> Result<Vec<LanguageInfo>, ServerFnError> {
    Ok(SUPPORTED_LANGUAGE_INFO
        .iter()
        .map(|(code, name)| language_info(code, name))
        .collect())
}

#[server]
pub async fn create_session(
    native: String,
    foreign: String,
    deck_size: u32,
    speed_ms: u32,
) -> Result<SessionDto, ServerFnError> {
    validate_lang(&native)?;
    validate_lang(&foreign)?;
    let db = pool()?;
    let session = SessionManager::create(db, &native, &foreign, deck_size, speed_ms)
        .await
        .map_err(|e| ServerFnError::new(format!("Session creation failed: {e}")))?;
    Ok(SessionDto::from(&session))
}

#[server]
pub async fn resume_session(
    native: String,
    foreign: String,
) -> Result<Option<SessionDto>, ServerFnError> {
    validate_lang(&native)?;
    validate_lang(&foreign)?;
    let db = pool()?;
    let session = SessionManager::resume(db, &native, &foreign)
        .await
        .map_err(|e| ServerFnError::new(format!("Session resume failed: {e}")))?;
    Ok(session.as_ref().map(SessionDto::from))
}

#[server]
pub async fn answer_card(
    session_id: i32,
    card_id: i32,
    rating: ReviewRatingDto,
) -> Result<CardDto, ServerFnError> {
    let db = pool()?;
    let card = CardManager::get_card_by_id(db, card_id)
        .await
        .map_err(|e| ServerFnError::new(format!("Card lookup failed: {e}")))?;

    let domain_rating = ReviewRating::from(rating);
    let updated = SessionManager::answer_card(db, session_id, &card, domain_rating)
        .await
        .map_err(|e| ServerFnError::new(format!("Answer failed: {e}")))?;

    Ok(CardDto::from(&updated))
}

#[server]
pub async fn pause_session(session_id: i32) -> Result<(), ServerFnError> {
    let db = pool()?;
    SessionManager::pause(db, session_id)
        .await
        .map_err(|e| ServerFnError::new(format!("Pause failed: {e}")))?;
    Ok(())
}

#[server]
pub async fn complete_session(session_id: i32) -> Result<(), ServerFnError> {
    let db = pool()?;
    SessionManager::complete(db, session_id)
        .await
        .map_err(|e| ServerFnError::new(format!("Complete failed: {e}")))?;
    Ok(())
}
