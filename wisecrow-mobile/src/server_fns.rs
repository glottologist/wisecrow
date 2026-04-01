use dioxus::prelude::*;
use wisecrow_dto::{
    CardDto, DnbAdaptationDto, DnbConfigDto, DnbSessionResultsDto, DnbTrialDto, DnbTrialResultDto,
    LanguageInfo, QuizItemDto, ReviewRatingDto, SessionDto, UserDto,
};

#[server]
pub async fn list_languages() -> Result<Vec<LanguageInfo>, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn create_session(
    user_id: i32,
    native: String,
    foreign: String,
    deck_size: u32,
    speed_ms: u32,
) -> Result<SessionDto, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn resume_session(
    user_id: i32,
    native: String,
    foreign: String,
) -> Result<Option<SessionDto>, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn answer_card(
    session_id: i32,
    card_id: i32,
    rating: ReviewRatingDto,
) -> Result<CardDto, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn pause_session(session_id: i32) -> Result<(), ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn complete_session(session_id: i32) -> Result<(), ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn list_users() -> Result<Vec<UserDto>, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn create_user(display_name: String) -> Result<UserDto, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn start_nback_session(
    config: DnbConfigDto,
) -> Result<(i32, Vec<DnbTrialDto>), ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn submit_nback_trial(
    session_id: i32,
    trial_result: DnbTrialResultDto,
    trial_dto: DnbTrialDto,
) -> Result<DnbAdaptationDto, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn complete_nback_session(
    session_id: i32,
    n_level: u8,
    interval_ms: u32,
    n_level_peak: u8,
    trials_completed: u32,
    accuracy_audio: f32,
    accuracy_visual: f32,
) -> Result<DnbSessionResultsDto, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn generate_quiz(
    pdf_bytes: Vec<u8>,
    num_questions: u32,
) -> Result<Vec<QuizItemDto>, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}

#[server]
pub async fn generate_rule_quiz(
    lang: String,
    level: String,
    num_questions: u32,
) -> Result<Vec<QuizItemDto>, ServerFnError> {
    Err(ServerFnError::new("client-side stub"))
}
