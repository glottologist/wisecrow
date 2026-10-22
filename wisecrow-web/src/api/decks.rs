//! A learner's named lists of words taken from a book.

use dioxus::prelude::*;
#[cfg(feature = "server")]
use wisecrow::decks::{DeckRepository, DeckSummary};
use wisecrow_dto::DeckDto;

#[cfg(feature = "server")]
fn map_error(operation: &'static str, error: wisecrow::errors::WisecrowError) -> ServerFnError {
    match error {
        wisecrow::errors::WisecrowError::InvalidInput(message) => {
            crate::server::client_error(axum::http::StatusCode::BAD_REQUEST, &message)
        }
        other => crate::server::internal_error(operation, &other),
    }
}

#[cfg(feature = "server")]
fn deck_dto(summary: DeckSummary) -> Result<DeckDto, ServerFnError> {
    let cards = u32::try_from(summary.cards).map_err(|_| {
        crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            "a deck has too many cards",
        )
    })?;
    Ok(DeckDto {
        id: summary.id,
        name: summary.name,
        cards,
    })
}

/// The learner's decks for one pair.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/decks/list")]
pub async fn list_decks(native: String, foreign: String) -> Result<Vec<DeckDto>, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    let rows = DeckRepository::summaries(crate::server::pool()?, user.id, &native, &foreign)
        .await
        .map_err(|error| map_error("deck list", error))?;
    rows.into_iter().map(deck_dto).collect()
}

/// Creates a deck, or returns the one this learner already named.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/decks/create")]
pub async fn create_deck(
    native: String,
    foreign: String,
    name: String,
) -> Result<DeckDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    let pool = crate::server::pool()?;
    let id = DeckRepository::create(pool, user.id, &native, &foreign, &name)
        .await
        .map_err(|error| map_error("deck create", error))?;
    let summary = DeckRepository::summary(pool, user.id, id)
        .await
        .map_err(|error| map_error("deck create", error))?;
    deck_dto(summary)
}

/// Adds a glossed word to a deck the learner owns.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/decks/add")]
pub async fn add_to_deck(deck_id: i32, normalised: String) -> Result<DeckDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    let pool = crate::server::pool()?;
    DeckRepository::add_word(pool, user.id, deck_id, &normalised)
        .await
        .map_err(|error| map_error("deck add", error))?;
    let summary = DeckRepository::summary(pool, user.id, deck_id)
        .await
        .map_err(|error| map_error("deck add", error))?;
    deck_dto(summary)
}
