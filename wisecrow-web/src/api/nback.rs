use dioxus::prelude::*;
use wisecrow_dto::{
    DnbAdaptationDto, DnbConfigDto, DnbSessionResultsDto, DnbTrialDto, DnbTrialResultDto,
};

/// Starts an authenticated n-back session.
///
/// # Errors
///
/// Returns validation, authentication, or sanitized storage errors.
#[post("/api/nback/start")]
pub async fn start_nback_session(
    config: DnbConfigDto,
) -> Result<(i32, Vec<DnbTrialDto>), ServerFnError> {
    implementation::start(config).await
}

/// Records one n-back response and returns server-owned adaptation state.
///
/// # Errors
///
/// Returns authentication or sanitized storage errors.
#[post("/api/nback/trial")]
pub async fn submit_nback_trial(
    session_id: i32,
    trial_result: DnbTrialResultDto,
) -> Result<DnbAdaptationDto, ServerFnError> {
    implementation::submit(session_id, trial_result).await
}

/// Completes an authenticated n-back session.
///
/// # Errors
///
/// Returns authentication or sanitized storage errors.
#[post("/api/nback/complete")]
pub async fn complete_nback_session(
    session_id: i32,
) -> Result<DnbSessionResultsDto, ServerFnError> {
    implementation::complete(session_id).await
}

#[cfg(feature = "server")]
mod implementation {
    use axum::http::StatusCode;
    use wisecrow::dnb::scoring::{apply_adaptation, should_terminate, AdaptationState, Channel};
    use wisecrow::dnb::session::DnbSessionRepository;
    use wisecrow::dnb::{CompletedTrial, DnbConfig, DnbEngine, DnbMode, DnbVocab, TrialResponse};
    use wisecrow_dto::{
        DnbAdaptationDto, DnbConfigDto, DnbModeDto, DnbSessionResultsDto, DnbTrialDto,
        DnbTrialResultDto,
    };

    use super::ServerFnError;

    const GENERATED_TRIALS: usize = 5;
    const MINIMUM_VOCABULARY: usize = 8;
    const VOCABULARY_LIMIT: u32 = 100;

    pub(super) async fn start(
        config: DnbConfigDto,
    ) -> Result<(i32, Vec<DnbTrialDto>), ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        crate::server::validate_lang(&config.native_lang)?;
        crate::server::validate_lang(&config.foreign_lang)?;
        let (session_id, vocab, engine_config) = prepare_session(user.id, &config).await?;
        let mut engine = DnbEngine::new(vocab, &engine_config, session_seed())
            .map_err(|error| crate::server::internal_error("n-back engine creation", &error))?;
        let trials: Vec<_> = std::iter::repeat_with(|| engine.next_trial())
            .take(GENERATED_TRIALS)
            .collect();
        DnbSessionRepository::insert_generated_trials(
            crate::server::pool()?,
            session_id,
            user.id,
            &trials,
        )
        .await
        .map_err(|error| crate::server::internal_error("n-back trial persistence", &error))?;
        Ok((session_id, trials.iter().map(DnbTrialDto::from).collect()))
    }

    async fn prepare_session(
        user_id: i32,
        config: &DnbConfigDto,
    ) -> Result<(i32, Vec<DnbVocab>, DnbConfig), ServerFnError> {
        let db = crate::server::pool()?;
        let mode = DnbMode::from(config.mode);
        let state = AdaptationState::new(config.n_level, config.interval_ms);
        let vocab = DnbSessionRepository::load_vocab(
            db,
            user_id,
            &config.native_lang,
            &config.foreign_lang,
            VOCABULARY_LIMIT,
        )
        .await
        .map_err(|error| crate::server::internal_error("n-back vocabulary load", &error))?;
        if vocab.len() < MINIMUM_VOCABULARY {
            return Err(crate::server::client_error(
                StatusCode::BAD_REQUEST,
                "Not enough vocabulary to start n-back",
            ));
        }
        let session_id = DnbSessionRepository::create_session(
            db,
            user_id,
            &config.native_lang,
            &config.foreign_lang,
            mode,
            &state,
        )
        .await
        .map_err(|error| crate::server::internal_error("n-back session creation", &error))?;
        let engine_config = DnbConfig {
            mode,
            n_level: state.n_level,
            interval_ms: state.interval_ms,
        };
        Ok((session_id, vocab, engine_config))
    }

    fn session_seed() -> u64 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        u64::try_from(nanos % u128::from(u64::MAX)).unwrap_or(42)
    }

    pub(super) async fn submit(
        session_id: i32,
        trial_result: DnbTrialResultDto,
    ) -> Result<DnbAdaptationDto, ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        let db = crate::server::pool()?;
        let response = TrialResponse {
            audio_response: trial_result.audio_response,
            visual_response: trial_result.visual_response,
            response_time_ms: trial_result.response_time_ms,
        };
        DnbSessionRepository::record_trial_response(
            db,
            session_id,
            user.id,
            trial_result.trial_number,
            &response,
        )
        .await
        .map_err(|error| crate::server::internal_error("n-back response recording", &error))?;
        adapt_session(db, session_id, user.id).await
    }

    async fn adapt_session(
        db: &sqlx::PgPool,
        session_id: i32,
        user_id: i32,
    ) -> Result<DnbAdaptationDto, ServerFnError> {
        let trials = DnbSessionRepository::load_answered_trials(db, session_id)
            .await
            .map_err(|error| crate::server::internal_error("n-back trial load", &error))?;
        let mut state = DnbSessionRepository::load_state(db, session_id, user_id)
            .await
            .map_err(|error| crate::server::internal_error("n-back state load", &error))?;
        apply_adaptation(&mut state, &trials);
        let terminate = should_terminate(&state, &trials);
        DnbSessionRepository::update_state(db, session_id, user_id, &state)
            .await
            .map_err(|error| crate::server::internal_error("n-back state update", &error))?;
        Ok(DnbAdaptationDto {
            new_n_level: state.n_level,
            new_interval_ms: state.interval_ms,
            audio_accuracy: channel_ratio(&trials, Channel::Audio, GENERATED_TRIALS),
            visual_accuracy: channel_ratio(&trials, Channel::Visual, GENERATED_TRIALS),
            should_terminate: terminate,
        })
    }

    fn channel_ratio(trials: &[CompletedTrial], channel: Channel, window: usize) -> f32 {
        let start = trials.len().saturating_sub(window);
        let recent = &trials[start..];
        let correct = recent
            .iter()
            .filter(|trial| match channel {
                Channel::Audio => trial.audio_correct(),
                Channel::Visual => trial.visual_correct(),
            })
            .count();
        wisecrow_dto::channel_ratio(
            u32::try_from(correct).unwrap_or(u32::MAX),
            u32::try_from(recent.len()).unwrap_or(u32::MAX),
        )
    }

    pub(super) async fn complete(session_id: i32) -> Result<DnbSessionResultsDto, ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        let db = crate::server::pool()?;
        let state = DnbSessionRepository::load_state(db, session_id, user.id)
            .await
            .map_err(|error| crate::server::internal_error("n-back state load", &error))?;
        let mode = DnbSessionRepository::load_mode(db, session_id, user.id)
            .await
            .map_err(|error| crate::server::internal_error("n-back mode load", &error))?;
        let trials = DnbSessionRepository::load_answered_trials(db, session_id)
            .await
            .map_err(|error| crate::server::internal_error("n-back trial load", &error))?;
        let trials_completed = u32::try_from(trials.len()).unwrap_or(u32::MAX);
        let audio_accuracy = channel_ratio(&trials, Channel::Audio, trials.len());
        let visual_accuracy = channel_ratio(&trials, Channel::Visual, trials.len());
        DnbSessionRepository::complete_session(
            db,
            session_id,
            user.id,
            &state,
            trials_completed,
            Some(audio_accuracy),
            Some(visual_accuracy),
        )
        .await
        .map_err(|error| crate::server::internal_error("n-back session completion", &error))?;
        Ok(results(
            session_id,
            mode,
            &state,
            trials_completed,
            audio_accuracy,
            visual_accuracy,
        ))
    }

    fn results(
        session_id: i32,
        mode: DnbMode,
        state: &AdaptationState,
        trials_completed: u32,
        audio_accuracy: f32,
        visual_accuracy: f32,
    ) -> DnbSessionResultsDto {
        DnbSessionResultsDto {
            session_id,
            mode: DnbModeDto::from(mode),
            n_level_start: state.n_level_start,
            n_level_peak: state.n_level_peak,
            n_level_end: state.n_level,
            trials_completed,
            accuracy_audio: Some(audio_accuracy),
            accuracy_visual: Some(visual_accuracy),
            interval_ms_start: state.interval_ms_start,
            interval_ms_end: state.interval_ms,
        }
    }
}
