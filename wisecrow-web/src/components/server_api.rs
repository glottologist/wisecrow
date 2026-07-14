//! Single source of truth for the server-function surface the client components
//! call. Under the `server` feature these names re-export the real implementations
//! from [`crate::server`]; without it (the WASM client build) they are thin stubs
//! that return an error. Consolidating the stub signatures here — rather than
//! repeating them per component — stops them drifting silently from the real
//! functions they mirror.
//!
//! Only the responseless media helpers (`get_audio_data`, `get_image_data`) stay
//! local to the learn view, because they are gated on the `audio`/`images`
//! features rather than on `server`.

#[cfg(feature = "server")]
pub use crate::server::{
    auth::{login, logout},
    learn::{
        answer_card, complete_session, create_session, list_languages, pause_session,
        resume_session,
    },
    nback::{complete_nback_session, start_nback_session, submit_nback_trial},
    quiz::generate_quiz,
};

#[cfg(not(feature = "server"))]
mod stubs {
    use dioxus::prelude::*;
    use wisecrow_dto::{
        CardDto, DnbAdaptationDto, DnbConfigDto, DnbSessionResultsDto, DnbTrialDto,
        DnbTrialResultDto, LanguageInfo, QuizItemDto, ReviewRatingDto, SessionDto, UserDto,
    };

    #[server]
    pub async fn login(email: String, password: String) -> Result<UserDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn logout() -> Result<(), ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn list_languages() -> Result<Vec<LanguageInfo>, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn create_session(
        native: String,
        foreign: String,
        deck_size: u32,
        speed_ms: u32,
    ) -> Result<SessionDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn resume_session(
        native: String,
        foreign: String,
    ) -> Result<Option<SessionDto>, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn answer_card(
        session_id: i32,
        card_id: i32,
        rating: ReviewRatingDto,
    ) -> Result<CardDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn pause_session(session_id: i32) -> Result<(), ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn complete_session(session_id: i32) -> Result<(), ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn start_nback_session(
        config: DnbConfigDto,
    ) -> Result<(i32, Vec<DnbTrialDto>), ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn submit_nback_trial(
        session_id: i32,
        trial_result: DnbTrialResultDto,
    ) -> Result<DnbAdaptationDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn complete_nback_session(
        session_id: i32,
    ) -> Result<DnbSessionResultsDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn generate_quiz(
        pdf_bytes: Vec<u8>,
        num_questions: u32,
    ) -> Result<Vec<QuizItemDto>, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }
}

#[cfg(not(feature = "server"))]
pub use stubs::*;
