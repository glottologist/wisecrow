use dioxus::prelude::*;
use wisecrow_dto::{
    DnbAdaptationDto, DnbConfigDto, DnbModeDto, DnbSessionResultsDto, DnbTrialDto,
    DnbTrialResultDto,
};

use super::pool;

#[server]
pub async fn start_nback_session(
    config: DnbConfigDto,
) -> Result<(i32, Vec<DnbTrialDto>), ServerFnError> {
    use wisecrow::dnb::scoring::AdaptationState;
    use wisecrow::dnb::session::DnbSessionRepository;
    use wisecrow::dnb::DnbMode;
    use wisecrow::dto_convert::DnbTrialDto as _;

    let pool = pool()?;
    let mode: DnbMode = DnbModeDto::into(config.mode);
    let dnb_config = wisecrow::dnb::DnbConfig {
        mode,
        n_level: config.n_level,
        interval_ms: config.interval_ms,
    };

    let vocab =
        DnbSessionRepository::load_vocab(pool, &config.native_lang, &config.foreign_lang, 100)
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
        config.user_id,
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

    let mut trials = Vec::with_capacity(5);
    for _ in 0..5 {
        let trial = engine.next_trial();
        trials.push(wisecrow_dto::DnbTrialDto::from(&trial));
    }

    Ok((session_id, trials))
}

#[server]
pub async fn submit_nback_trial(
    session_id: i32,
    trial_result: DnbTrialResultDto,
    trial_dto: DnbTrialDto,
) -> Result<DnbAdaptationDto, ServerFnError> {
    use wisecrow::dnb::session::DnbSessionRepository;
    use wisecrow::dnb::{CompletedTrial, DnbVocab, Trial, TrialResponse};

    let pool = pool()?;

    let trial = Trial {
        trial_number: trial_dto.trial_number,
        n_level: trial_dto.n_level,
        audio_vocab: DnbVocab {
            translation_id: 0,
            from_phrase: String::new(),
            to_phrase: trial_dto.audio_phrase.clone(), // clone: reconstructing domain type from DTO
        },
        visual_vocab: DnbVocab {
            translation_id: 0,
            from_phrase: trial_dto.visual_phrase.clone(), // clone: reconstructing domain type from DTO
            to_phrase: String::new(),
        },
        audio_match: trial_dto.audio_match,
        visual_match: trial_dto.visual_match,
        interval_ms: trial_dto.interval_ms,
    };

    let response = TrialResponse {
        audio_response: trial_result.audio_response,
        visual_response: trial_result.visual_response,
        response_time_ms: trial_result.response_time_ms,
    };

    let completed = CompletedTrial { trial, response };

    DnbSessionRepository::save_trial(pool, session_id, &completed)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(DnbAdaptationDto {
        new_n_level: trial_dto.n_level,
        new_interval_ms: trial_dto.interval_ms,
        audio_accuracy: 0.0,
        visual_accuracy: 0.0,
        should_terminate: false,
    })
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
    use wisecrow::dnb::scoring::AdaptationState;
    use wisecrow::dnb::session::DnbSessionRepository;

    let pool = pool()?;

    let mut state = AdaptationState::new(n_level, interval_ms);
    state.n_level_peak = n_level_peak;

    DnbSessionRepository::complete_session(
        pool,
        session_id,
        &state,
        trials_completed,
        Some(accuracy_audio),
        Some(accuracy_visual),
    )
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(DnbSessionResultsDto {
        session_id,
        mode: DnbModeDto::AudioWritten,
        n_level_start: n_level,
        n_level_peak,
        n_level_end: n_level,
        trials_completed,
        accuracy_audio: Some(accuracy_audio),
        accuracy_visual: Some(accuracy_visual),
        interval_ms_start: interval_ms,
        interval_ms_end: interval_ms,
    })
}
