use dioxus::prelude::*;
use wisecrow_dto::{CardDto, FastDeckDto, LanguageInfo, ReviewRatingDto, SessionDto};

/// Lists supported learning languages for an authenticated user.
///
/// # Errors
///
/// Returns unauthorized when no valid session is present.
#[post("/api/learn/languages")]
pub async fn list_languages() -> Result<Vec<LanguageInfo>, ServerFnError> {
    crate::server::auth::current_user().await?;
    Ok(wisecrow::cli::SUPPORTED_LANGUAGE_INFO
        .iter()
        .map(|(code, name)| wisecrow::dto_convert::language_info(code, name))
        .collect())
}

/// Creates an authenticated learning session.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/learn/session/create")]
pub async fn create_session(
    native: String,
    foreign: String,
    deck_size: u32,
    speed_ms: u32,
) -> Result<SessionDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    let session = wisecrow::srs::session::SessionManager::create(
        crate::server::pool()?,
        user.id,
        &native,
        &foreign,
        deck_size,
        speed_ms,
    )
    .await
    .map_err(|error| crate::server::internal_error("learning session creation", &error))?;
    Ok(SessionDto::from(&session))
}

/// Restores an unfinished authenticated learning session.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/learn/session/resume")]
pub async fn resume_session(
    native: String,
    foreign: String,
) -> Result<Option<SessionDto>, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    let session = wisecrow::srs::session::SessionManager::resume(
        crate::server::pool()?,
        user.id,
        &native,
        &foreign,
    )
    .await
    .map_err(|error| crate::server::internal_error("learning session resume", &error))?;
    Ok(session.as_ref().map(SessionDto::from))
}

/// Records one card answer in an authenticated learning session.
///
/// # Errors
///
/// Returns authentication or sanitized storage errors.
#[post("/api/learn/card/answer")]
pub async fn answer_card(
    session_id: i32,
    card_id: i32,
    rating: ReviewRatingDto,
) -> Result<CardDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    let db = crate::server::pool()?;
    let card = wisecrow::srs::scheduler::CardManager::get_card_by_id(db, card_id)
        .await
        .map_err(|error| crate::server::internal_error("learning card lookup", &error))?;
    let updated = wisecrow::srs::session::SessionManager::answer_card(
        db,
        session_id,
        user.id,
        &card,
        wisecrow::srs::scheduler::ReviewRating::from(rating),
    )
    .await
    .map_err(|error| crate::server::internal_error("learning card answer", &error))?;
    Ok(CardDto::from(&updated))
}

/// Pauses an authenticated learning session.
///
/// # Errors
///
/// Returns authentication or sanitized storage errors.
#[post("/api/learn/session/pause")]
pub async fn pause_session(session_id: i32) -> Result<(), ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    wisecrow::srs::session::SessionManager::pause(crate::server::pool()?, session_id, user.id)
        .await
        .map_err(|error| crate::server::internal_error("learning session pause", &error))?;
    Ok(())
}

/// Completes an authenticated learning session.
///
/// # Errors
///
/// Returns authentication or sanitized storage errors.
#[post("/api/learn/session/complete")]
pub async fn complete_session(session_id: i32) -> Result<(), ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    wisecrow::srs::session::SessionManager::complete(crate::server::pool()?, session_id, user.id)
        .await
        .map_err(|error| crate::server::internal_error("learning session completion", &error))?;
    Ok(())
}

/// Returns the top-ranked translations for a passive fast-mode run.
///
/// Writes nothing: fast mode has no session row and no SRS state. The deck
/// is an 80/20 word/phrase interleave; with no phrases promoted yet it is
/// simply the top words at full size.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/learn/fast-deck")]
pub async fn create_fast_deck(
    native: String,
    foreign: String,
    size: u32,
) -> Result<FastDeckDto, ServerFnError> {
    use wisecrow::vocabulary::{interleave_deck, IncludeCarded, PhraseFilter, VocabularyQuery};
    use wisecrow_dto::FastCardDto;

    crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    if size == 0 {
        return Err(crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            "Deck size must be positive",
        ));
    }
    let size = size.min(500);
    let pool = crate::server::pool()?;

    let words = VocabularyQuery::ranked_candidates(
        pool,
        &native,
        &foreign,
        size,
        IncludeCarded::Yes,
        PhraseFilter::Exclude,
    )
    .await
    .map_err(|error| crate::server::internal_error("fast deck words", &error))?;
    let phrases = VocabularyQuery::ranked_candidates(
        pool,
        &native,
        &foreign,
        size / 5,
        IncludeCarded::Yes,
        PhraseFilter::Only,
    )
    .await
    .map_err(|error| crate::server::internal_error("fast deck phrases", &error))?;

    let to_dto = |image_allowed: bool| {
        move |entry: wisecrow::vocabulary::VocabularyEntry| FastCardDto {
            translation_id: entry.translation_id,
            from_phrase: entry.from_phrase,
            to_phrase: entry.to_phrase,
            frequency: entry.frequency,
            image_allowed,
        }
    };
    let words: Vec<FastCardDto> = words.into_iter().map(to_dto(true)).collect();
    let phrases: Vec<FastCardDto> = phrases.into_iter().map(to_dto(false)).collect();
    let deck_size = usize::try_from(size).unwrap_or(usize::MAX);
    Ok(FastDeckDto {
        cards: interleave_deck(words, phrases, deck_size),
    })
}
