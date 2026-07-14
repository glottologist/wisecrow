use dioxus::prelude::*;
use wisecrow_dto::{
    DnbAdaptationDto, DnbConfigDto, DnbModeDto, DnbSessionResultsDto, DnbTrialDto,
    DnbTrialResultDto,
};

use super::auth::current_user;
use super::pool;

#[server]
pub async fn start_nback_session(
    config: DnbConfigDto,
) -> Result<(i32, Vec<DnbTrialDto>), ServerFnError> {
    use wisecrow::dnb::scoring::AdaptationState;
    use wisecrow::dnb::session::DnbSessionRepository;
    use wisecrow::dnb::DnbMode;

    let user = current_user().await?;
    let pool = pool()?;
    let mode: DnbMode = DnbModeDto::into(config.mode);
    let dnb_config = wisecrow::dnb::DnbConfig {
        mode,
        n_level: config.n_level,
        interval_ms: config.interval_ms,
    };

    let vocab = DnbSessionRepository::load_vocab(
        pool,
        user.id,
        &config.native_lang,
        &config.foreign_lang,
        100,
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    if vocab.len() < 8 {
        return Err(ServerFnError::new(format!(
            "Not enough vocabulary ({} items, need 8+)",
            vocab.len()
        )));
    }

    let state = AdaptationState::new(config.n_level, config.interval_ms);
    let session_id = DnbSessionRepository::create_session(
        pool,
        user.id,
        &config.native_lang,
        &config.foreign_lang,
        mode,
        &state,
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    let seed = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            % u128::from(u64::MAX),
    )
    .unwrap_or(42);

    let mut engine = wisecrow::dnb::DnbEngine::new(vocab, &dnb_config, seed)
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let mut domain_trials = Vec::with_capacity(5);
    for _ in 0..5 {
        domain_trials.push(engine.next_trial());
    }

    // Persist the generated trials (with their match flags) up front, so the
    // server owns the authoritative outcome and submit only records responses.
    DnbSessionRepository::insert_generated_trials(pool, session_id, user.id, &domain_trials)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let trials = domain_trials.iter().map(DnbTrialDto::from).collect();

    Ok((session_id, trials))
}

#[server]
pub async fn submit_nback_trial(
    session_id: i32,
    trial_result: DnbTrialResultDto,
) -> Result<DnbAdaptationDto, ServerFnError> {
    use wisecrow::dnb::scoring::{apply_adaptation, should_terminate};
    use wisecrow::dnb::session::DnbSessionRepository;
    use wisecrow::dnb::TrialResponse;

    let user = current_user().await?;
    let pool = pool()?;

    // Record only the response; the match flags stay as the server generated them
    // at session start, so the client cannot forge the trial outcome.
    let response = TrialResponse {
        audio_response: trial_result.audio_response,
        visual_response: trial_result.visual_response,
        response_time_ms: trial_result.response_time_ms,
    };
    DnbSessionRepository::record_trial_response(
        pool,
        session_id,
        user.id,
        trial_result.trial_number,
        &response,
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    // Recompute adaptation + accuracy server-side from the answered trials.
    let trials = DnbSessionRepository::load_answered_trials(pool, session_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let mut state = DnbSessionRepository::load_state(pool, session_id, user.id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    apply_adaptation(&mut state, &trials);
    let terminate = should_terminate(&state, &trials);
    DnbSessionRepository::update_state(pool, session_id, user.id, &state)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(wisecrow::dto_convert::adaptation_to_dto(
        &state, &trials, terminate,
    ))
}

#[server]
pub async fn complete_nback_session(
    session_id: i32,
) -> Result<DnbSessionResultsDto, ServerFnError> {
    use wisecrow::dnb::scoring::{channel_accuracy, Channel};
    use wisecrow::dnb::session::DnbSessionRepository;

    let user = current_user().await?;
    let pool = pool()?;

    // Everything comes from server-owned state: the adaptation counter evolved by
    // submit, the session's real mode, and accuracy recomputed from the answered
    // trials. No client-supplied statistics are trusted.
    let state = DnbSessionRepository::load_state(pool, session_id, user.id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let mode = DnbSessionRepository::load_mode(pool, session_id, user.id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;
    let trials = DnbSessionRepository::load_answered_trials(pool, session_id)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    let trials_completed = u32::try_from(trials.len()).unwrap_or(u32::MAX);
    #[expect(clippy::cast_possible_truncation)]
    let accuracy_audio = channel_accuracy(&trials, Channel::Audio, trials.len()) as f32;
    #[expect(clippy::cast_possible_truncation)]
    let accuracy_visual = channel_accuracy(&trials, Channel::Visual, trials.len()) as f32;

    DnbSessionRepository::complete_session(
        pool,
        session_id,
        user.id,
        &state,
        trials_completed,
        Some(accuracy_audio),
        Some(accuracy_visual),
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(wisecrow::dto_convert::dnb_results_to_dto(
        session_id,
        mode,
        &state,
        trials_completed,
        Some(accuracy_audio),
        Some(accuracy_visual),
    ))
}
