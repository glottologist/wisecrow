use dioxus::prelude::*;
#[cfg(feature = "server")]
use wisecrow_dto::FastCardDto;
use wisecrow_dto::{CardDto, FastDeckDto, LanguageInfo, ReviewRatingDto, SessionDto};

#[cfg(feature = "server")]
fn validate_requested_deck_size(size: u32) -> Result<usize, ServerFnError> {
    wisecrow::srs::session::validate_deck_size(size).map_err(|_| {
        crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            wisecrow::srs::session::DECK_SIZE_ERROR,
        )
    })?;
    usize::try_from(size).map_err(|_| {
        crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            wisecrow::srs::session::DECK_SIZE_ERROR,
        )
    })
}

#[cfg(feature = "server")]
fn fast_card(entry: wisecrow::vocabulary::VocabularyEntry, image_allowed: bool) -> FastCardDto {
    FastCardDto {
        translation_id: entry.translation_id,
        from_phrase: entry.from_phrase,
        to_phrase: entry.to_phrase,
        frequency: entry.frequency,
        image_allowed,
    }
}

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
    area: Option<String>,
    deck_id: Option<i32>,
) -> Result<SessionDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    validate_requested_deck_size(deck_size)?;
    let area = practice_area(area.as_deref())?;
    let session = wisecrow::srs::session::SessionManager::create(
        crate::server::pool()?,
        user.id,
        &native,
        &foreign,
        deck_size,
        speed_ms,
        wisecrow::srs::session::SessionChoice { area, deck_id },
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
    deck_id: Option<i32>,
) -> Result<Option<SessionDto>, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    let session = wisecrow::srs::session::SessionManager::resume(
        crate::server::pool()?,
        user.id,
        &native,
        &foreign,
        deck_id,
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

/// How many distinct words a fast deck can draw for this pair.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/learn/fast-available")]
pub async fn fast_available_words(
    native: String,
    foreign: String,
    area: Option<String>,
) -> Result<u32, ServerFnError> {
    use wisecrow::grammar::areas::Area;
    use wisecrow::vocabulary::VocabularyQuery;

    crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    let area = practice_area(area.as_deref())?;
    let pool = crate::server::pool()?;
    let count = match area {
        Some(Area::Verb) => verb_form_count(pool, &native, &foreign).await,
        Some(area) => {
            VocabularyQuery::available_area_count(pool, &native, &foreign, class_filter(area)?)
                .await
        }
        None => VocabularyQuery::available_word_count(pool, &native, &foreign).await,
    };
    count.map_err(|error| crate::server::internal_error("fast deck word count", &error))
}

/// Returns the top-ranked translations for a passive fast-mode run.
///
/// Writes nothing: fast mode has no session row and no SRS state. `size` is
/// a word count, from 1 up to every available word in the language. Phrases
/// are mixed in on top of those words and do not replace them. With no
/// phrases promoted yet the deck is simply the top words.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/learn/fast-deck")]
pub async fn create_fast_deck(
    native: String,
    foreign: String,
    size: u32,
    area: Option<String>,
) -> Result<FastDeckDto, ServerFnError> {
    use wisecrow::vocabulary::{interleave_deck, IncludeCarded, PhraseFilter, VocabularyQuery};

    crate::server::auth::current_user().await?;
    crate::server::validate_lang(&native)?;
    crate::server::validate_lang(&foreign)?;
    let area = practice_area(area.as_deref())?;
    if size == 0 {
        return Err(crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            "Choose between 1 and the number of available words",
        ));
    }
    let pool = crate::server::pool()?;
    if let Some(area) = area {
        return fast_area_deck(pool, &native, &foreign, size, area).await;
    }
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
    // A filled page is enough evidence that `size` words exist. Counting the
    // once-only tail here scanned the whole corpus and cancelled the start.
    let requested = usize::try_from(size).map_err(|_| {
        crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            "Choose between 1 and the number of available words",
        )
    })?;
    if words.len() < requested {
        return Err(crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            "Choose between 1 and the number of available words",
        ));
    }
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

    let words: Vec<FastCardDto> = words
        .into_iter()
        .map(|entry| fast_card(entry, true))
        .collect();
    let phrases: Vec<FastCardDto> = phrases
        .into_iter()
        .map(|entry| fast_card(entry, false))
        .collect();
    let deck_len = words.len().saturating_add(phrases.len());
    Ok(FastDeckDto {
        cards: interleave_deck(words, phrases, deck_len),
    })
}

#[cfg(feature = "server")]
fn practice_area(
    value: Option<&str>,
) -> Result<Option<wisecrow::grammar::areas::Area>, ServerFnError> {
    match wisecrow::grammar::areas::parse_practice_area(value) {
        Ok(area) => Ok(area),
        Err(wisecrow::errors::WisecrowError::InvalidInput(message)) => Err(
            crate::server::client_error(axum::http::StatusCode::BAD_REQUEST, &message),
        ),
        Err(error) => Err(crate::server::internal_error("practice area", &error)),
    }
}

#[cfg(feature = "server")]
fn class_filter(
    area: wisecrow::grammar::areas::Area,
) -> Result<wisecrow::vocabulary::AreaFilter, ServerFnError> {
    use wisecrow::grammar::areas::Area;
    use wisecrow::vocabulary::AreaFilter;
    match area {
        Area::Preposition => Ok(AreaFilter::Preposition),
        Area::Pronoun => Ok(AreaFilter::Pronoun),
        Area::Adjective => Ok(AreaFilter::Adjective),
        Area::Adverb => Ok(AreaFilter::Adverb),
        Area::Verb | Area::Other => Err(crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            "unknown area",
        )),
    }
}

#[cfg(feature = "server")]
async fn verb_form_count(
    pool: &sqlx::PgPool,
    native: &str,
    foreign: &str,
) -> Result<u32, wisecrow::errors::WisecrowError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM verb_form_translations vft
         JOIN verb_forms vf ON vf.id = vft.verb_form_id
         JOIN languages nl ON nl.id = vft.native_language_id
         JOIN languages fl ON fl.id = vf.language_id
         WHERE nl.code = $1 AND fl.code = $2",
    )
    .bind(native)
    .bind(foreign)
    .fetch_one(pool)
    .await?;
    u32::try_from(count).map_err(|_| {
        wisecrow::errors::WisecrowError::InvalidInput("verb form count exceeds bounds".into())
    })
}

#[cfg(feature = "server")]
async fn fast_area_deck(
    pool: &sqlx::PgPool,
    native: &str,
    foreign: &str,
    size: u32,
    area: wisecrow::grammar::areas::Area,
) -> Result<FastDeckDto, ServerFnError> {
    use wisecrow::grammar::areas::Area;
    use wisecrow::vocabulary::{IncludeCarded, PhraseFilter, VocabularyQuery};

    let entries = match area {
        Area::Verb => {
            let slots =
                wisecrow::grammar::paradigms::slots_for(foreign).map_err(|error| match error {
                    wisecrow::errors::WisecrowError::InvalidInput(message) => {
                        crate::server::client_error(axum::http::StatusCode::BAD_REQUEST, &message)
                    }
                    other => crate::server::internal_error("verb slots", &other),
                })?;
            let order: Vec<&str> = slots.iter().map(|slot| slot.id).collect();
            VocabularyQuery::verb_form_candidates(
                pool,
                native,
                foreign,
                size,
                IncludeCarded::Yes,
                &order,
            )
            .await
            .map_err(|error| crate::server::internal_error("fast verb deck", &error))?
        }
        other => VocabularyQuery::ranked_area_candidates(
            pool,
            native,
            foreign,
            size,
            IncludeCarded::Yes,
            PhraseFilter::Exclude,
            class_filter(other)?,
        )
        .await
        .map_err(|error| crate::server::internal_error("fast area deck", &error))?,
    };
    let requested = usize::try_from(size).map_err(|_| {
        crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            "Choose between 1 and the number of available words",
        )
    })?;
    if entries.len() < requested {
        return Err(crate::server::client_error(
            axum::http::StatusCode::BAD_REQUEST,
            "Choose between 1 and the number of available words",
        ));
    }
    Ok(FastDeckDto {
        cards: entries
            .into_iter()
            .map(|entry| fast_card(entry, true))
            .collect(),
    })
}
