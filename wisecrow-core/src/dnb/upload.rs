use std::collections::BTreeSet;

use num_traits::ToPrimitive;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use wisecrow_dto::{
    DnbSessionResultsDto, NbackModeDto, NbackSessionUploadDto, NbackUploadAckDto,
    NbackUploadStatusDto,
};

use crate::dnb::feedback::review_events;
use crate::dnb::scoring::{channel_accuracy, AdaptationState, Channel};
use crate::dnb::session::{DnbSessionCompletion, DnbSessionRepository};
use crate::dnb::{CompletedTrial, DnbConfig, DnbEngine, DnbMode, DnbVocab, TrialResponse};
use crate::dto_convert::dnb_results_to_dto;
use crate::errors::WisecrowError;
use crate::srs::reviews::{ReviewLedger, ReviewSource};

const MAX_NBACK_BATCH: usize = 20;
const MAX_SESSION_RESPONSES: usize = 1_000;
const MAX_VOCABULARY: usize = 500;
const MIN_N_LEVEL: u8 = 1;
const MAX_N_LEVEL: u8 = 9;
const MIN_INTERVAL_MS: u32 = 1_500;
const MAX_INTERVAL_MS: u32 = 5_000;

pub struct NbackUploadService<'pool> {
    pool: &'pool PgPool,
}

impl<'pool> NbackUploadService<'pool> {
    #[must_use]
    pub const fn new(pool: &'pool PgPool) -> Self {
        Self { pool }
    }

    /// Applies completed sessions independently while preserving UUID idempotency.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid batch or device, UUID collision, or storage
    /// failure. Invalid individual sessions are acknowledged as rejected.
    pub async fn apply_batch(
        &self,
        user_id: i32,
        device_id: Uuid,
        sessions: &[NbackSessionUploadDto],
    ) -> Result<Vec<NbackUploadAckDto>, WisecrowError> {
        validate_batch(sessions)?;
        validate_device(self.pool, user_id, device_id).await?;
        let mut acknowledgements = Vec::with_capacity(sessions.len());
        for session in sessions {
            acknowledgements.push(self.apply_one(user_id, device_id, session).await?);
        }
        Ok(acknowledgements)
    }

    async fn apply_one(
        &self,
        user_id: i32,
        device_id: Uuid,
        session: &NbackSessionUploadDto,
    ) -> Result<NbackUploadAckDto, WisecrowError> {
        let prepared = match prepare(self.pool, session).await {
            Ok(prepared) => prepared,
            Err(WisecrowError::InvalidInput(reason)) => {
                return Ok(rejected(session.client_session_id, reason));
            }
            Err(error) => return Err(error),
        };
        match persist(self.pool, user_id, device_id, session, &prepared).await {
            Ok(acknowledgement) => Ok(acknowledgement),
            Err(WisecrowError::InvalidInput(reason)) => {
                Ok(rejected(session.client_session_id, reason))
            }
            Err(error) => Err(error),
        }
    }
}

struct PreparedSession {
    payload_sha256: [u8; 32],
    mode: DnbMode,
    initial_state: AdaptationState,
    final_state: AdaptationState,
    trials: Vec<CompletedTrial>,
    accuracy_audio: f32,
    accuracy_visual: f32,
}

async fn prepare(
    pool: &PgPool,
    session: &NbackSessionUploadDto,
) -> Result<PreparedSession, WisecrowError> {
    validate_session_shape(session)?;
    let vocabulary = load_vocabulary(pool, session).await?;
    let mode = mode_from_dto(session.mode);
    let initial_state = AdaptationState::new(session.n_level, session.interval_ms);
    let config = DnbConfig::new(mode, session.n_level, session.interval_ms);
    let mut engine = DnbEngine::new(vocabulary, &config, session.seed)
        .map_err(|error| WisecrowError::InvalidInput(error.to_string()))?;
    replay_responses(&mut engine, session)?;
    let accuracy_audio = accuracy(engine.completed_trials(), Channel::Audio)?;
    let accuracy_visual = accuracy(engine.completed_trials(), Channel::Visual)?;
    let final_state = *engine.state();
    Ok(PreparedSession {
        payload_sha256: payload_hash(session)?,
        mode,
        initial_state,
        final_state,
        accuracy_audio,
        accuracy_visual,
        trials: engine.into_completed_trials(),
    })
}

fn validate_batch(sessions: &[NbackSessionUploadDto]) -> Result<(), WisecrowError> {
    if sessions.len() > MAX_NBACK_BATCH {
        return Err(WisecrowError::InvalidInput(format!(
            "n-back batches may contain at most {MAX_NBACK_BATCH} sessions"
        )));
    }
    Ok(())
}

fn validate_session_shape(session: &NbackSessionUploadDto) -> Result<(), WisecrowError> {
    validate_pair(session)?;
    if !(MIN_N_LEVEL..=MAX_N_LEVEL).contains(&session.n_level) {
        return invalid("n-back level must be in 1..=9");
    }
    if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&session.interval_ms) {
        return invalid("n-back interval must be in 1500..=5000 milliseconds");
    }
    if session.started_at > session.completed_at
        || session.completed_at > chrono::Utc::now() + chrono::Duration::minutes(5)
    {
        return invalid("n-back session timestamps are invalid");
    }
    validate_vocabulary_ids(&session.vocabulary_translation_ids)?;
    validate_response_order(session)
}

fn validate_pair(session: &NbackSessionUploadDto) -> Result<(), WisecrowError> {
    let pair = &session.pair;
    if !crate::lang::is_valid_code(&pair.native_lang)
        || !crate::lang::is_valid_code(&pair.foreign_lang)
        || pair.native_lang == pair.foreign_lang
    {
        return invalid("n-back language pair is invalid");
    }
    Ok(())
}

fn validate_vocabulary_ids(ids: &[i32]) -> Result<(), WisecrowError> {
    if !(8..=MAX_VOCABULARY).contains(&ids.len()) {
        return invalid("n-back vocabulary must contain 8..=500 translations");
    }
    if ids.iter().any(|id| *id <= 0) || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return invalid("n-back vocabulary translation IDs must be positive and unique");
    }
    Ok(())
}

fn validate_response_order(session: &NbackSessionUploadDto) -> Result<(), WisecrowError> {
    if !(1..=MAX_SESSION_RESPONSES).contains(&session.responses.len()) {
        return invalid("n-back response count must be in 1..=1000");
    }
    for (index, response) in session.responses.iter().enumerate() {
        let expected = u32::try_from(index)
            .map_err(|_| WisecrowError::InvalidInput("n-back response index overflow".into()))?
            .saturating_add(1);
        if response.trial_number != expected || response.response_time_ms > MAX_INTERVAL_MS {
            return invalid("n-back responses must be ordered and within timing bounds");
        }
    }
    Ok(())
}

async fn load_vocabulary(
    pool: &PgPool,
    session: &NbackSessionUploadDto,
) -> Result<Vec<DnbVocab>, WisecrowError> {
    let rows = sqlx::query_as::<_, (i32, String, String)>(
        "SELECT requested.translation_id, translation.from_phrase, translation.to_phrase
         FROM unnest($1::integer[]) WITH ORDINALITY AS requested(translation_id, position)
         JOIN translations AS translation ON translation.id = requested.translation_id
         JOIN languages AS source ON source.id = translation.from_language_id
         JOIN languages AS target ON target.id = translation.to_language_id
         WHERE source.code = $2 AND target.code = $3
         ORDER BY requested.position",
    )
    .bind(&session.vocabulary_translation_ids)
    .bind(&session.pair.foreign_lang)
    .bind(&session.pair.native_lang)
    .fetch_all(pool)
    .await?;
    if rows.len() != session.vocabulary_translation_ids.len() {
        return invalid("n-back vocabulary is not owned by the requested language pair");
    }
    Ok(rows
        .into_iter()
        .map(|(translation_id, from_phrase, to_phrase)| DnbVocab {
            translation_id,
            from_phrase,
            to_phrase,
        })
        .collect())
}

fn replay_responses(
    engine: &mut DnbEngine,
    session: &NbackSessionUploadDto,
) -> Result<(), WisecrowError> {
    for response in &session.responses {
        if engine.should_terminate() {
            return invalid("n-back response count exceeds the server termination boundary");
        }
        let trial = engine
            .next_trial()
            .map_err(|error| WisecrowError::InvalidInput(error.to_string()))?;
        if response.response_time_ms > trial.interval_ms {
            return invalid("n-back response time exceeds its generated trial interval");
        }
        engine
            .record_response(
                &trial,
                TrialResponse {
                    audio_response: response.audio_response,
                    visual_response: response.visual_response,
                    response_time_ms: Some(response.response_time_ms),
                },
            )
            .map_err(|error| WisecrowError::InvalidInput(error.to_string()))?;
    }
    Ok(())
}

fn payload_hash(session: &NbackSessionUploadDto) -> Result<[u8; 32], WisecrowError> {
    let encoded = serde_json::to_vec(session)
        .map_err(|error| WisecrowError::SyncError(format!("n-back payload encoding: {error}")))?;
    Ok(Sha256::digest(encoded).into())
}

fn accuracy(trials: &[CompletedTrial], channel: Channel) -> Result<f32, WisecrowError> {
    channel_accuracy(trials, channel, trials.len())
        .to_f32()
        .ok_or_else(|| WisecrowError::SyncError("n-back accuracy conversion failed".into()))
}

async fn validate_device(
    pool: &PgPool,
    user_id: i32,
    device_id: Uuid,
) -> Result<(), WisecrowError> {
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM mobile_devices
             WHERE user_id = $1 AND id = $2 AND revoked_at IS NULL
         )",
    )
    .bind(user_id)
    .bind(device_id)
    .fetch_one(pool)
    .await?;
    active.then_some(()).ok_or(WisecrowError::Unauthorized)
}

async fn persist(
    pool: &PgPool,
    user_id: i32,
    device_id: Uuid,
    session: &NbackSessionUploadDto,
    prepared: &PreparedSession,
) -> Result<NbackUploadAckDto, WisecrowError> {
    let mut transaction = pool.begin().await?;
    let inserted = reserve_upload(&mut transaction, user_id, device_id, session, prepared).await?;
    let acknowledgement = if inserted {
        persist_new(
            pool,
            &mut transaction,
            user_id,
            device_id,
            session,
            prepared,
        )
        .await?
    } else {
        load_existing(&mut transaction, user_id, device_id, session, prepared).await?
    };
    transaction.commit().await?;
    Ok(acknowledgement)
}

async fn reserve_upload(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    device_id: Uuid,
    session: &NbackSessionUploadDto,
    prepared: &PreparedSession,
) -> Result<bool, WisecrowError> {
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO mobile_nback_uploads (
             user_id, client_session_id, device_id, payload_sha256
         ) VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, client_session_id) DO NOTHING
         RETURNING client_session_id",
    )
    .bind(user_id)
    .bind(session.client_session_id)
    .bind(device_id)
    .bind(&prepared.payload_sha256[..])
    .fetch_optional(&mut **transaction)
    .await?;
    Ok(inserted.is_some())
}

async fn persist_new(
    pool: &PgPool,
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    device_id: Uuid,
    session: &NbackSessionUploadDto,
    prepared: &PreparedSession,
) -> Result<NbackUploadAckDto, WisecrowError> {
    let server_session_id = create_session(transaction, user_id, session, prepared).await?;
    for trial in &prepared.trials {
        DnbSessionRepository::save_trial_in(transaction, server_session_id, user_id, trial).await?;
    }
    complete_session(transaction, user_id, server_session_id, session, prepared).await?;
    let feedback = review_events(
        session.client_session_id,
        session.completed_at,
        &prepared.trials,
    );
    ReviewLedger::new(pool)
        .apply_in_transaction(
            transaction,
            user_id,
            Some(device_id),
            ReviewSource::NBack,
            &feedback,
        )
        .await?;
    attach_server_session(
        transaction,
        user_id,
        session.client_session_id,
        server_session_id,
    )
    .await?;
    applied_ack(session, prepared, server_session_id)
}

async fn create_session(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    session: &NbackSessionUploadDto,
    prepared: &PreparedSession,
) -> Result<i32, WisecrowError> {
    DnbSessionRepository::create_session_in(
        transaction,
        user_id,
        &session.pair.native_lang,
        &session.pair.foreign_lang,
        prepared.mode,
        &prepared.initial_state,
        session.started_at,
    )
    .await
}

async fn complete_session(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    server_session_id: i32,
    session: &NbackSessionUploadDto,
    prepared: &PreparedSession,
) -> Result<(), WisecrowError> {
    let count = u32::try_from(prepared.trials.len())
        .map_err(|_| WisecrowError::InvalidInput("n-back trial count overflow".into()))?;
    let completion = DnbSessionCompletion {
        state: &prepared.final_state,
        trials_completed: count,
        accuracy_audio: Some(prepared.accuracy_audio),
        accuracy_visual: Some(prepared.accuracy_visual),
        completed_at: session.completed_at,
    };
    DnbSessionRepository::complete_session_in(transaction, server_session_id, user_id, &completion)
        .await
}

async fn attach_server_session(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    client_session_id: Uuid,
    server_session_id: i32,
) -> Result<(), WisecrowError> {
    sqlx::query(
        "UPDATE mobile_nback_uploads SET server_session_id = $1
         WHERE user_id = $2 AND client_session_id = $3",
    )
    .bind(server_session_id)
    .bind(user_id)
    .bind(client_session_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn load_existing(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    device_id: Uuid,
    session: &NbackSessionUploadDto,
    prepared: &PreparedSession,
) -> Result<NbackUploadAckDto, WisecrowError> {
    let (stored_device, stored_hash, server_session_id): (Uuid, Vec<u8>, Option<i32>) =
        sqlx::query_as(
            "SELECT device_id, payload_sha256, server_session_id
             FROM mobile_nback_uploads
             WHERE user_id = $1 AND client_session_id = $2
             FOR UPDATE",
        )
        .bind(user_id)
        .bind(session.client_session_id)
        .fetch_one(&mut **transaction)
        .await?;
    if stored_device != device_id || stored_hash.as_slice() != prepared.payload_sha256 {
        return Err(WisecrowError::Conflict(
            "n-back session UUID is already associated with another payload".into(),
        ));
    }
    let server_session_id = server_session_id.ok_or_else(|| {
        WisecrowError::SyncError("stored n-back upload has no server session".into())
    })?;
    Ok(NbackUploadAckDto {
        client_session_id: session.client_session_id,
        status: NbackUploadStatusDto::AlreadyApplied,
        result: Some(load_result(transaction, server_session_id, user_id).await?),
    })
}

async fn load_result(
    transaction: &mut Transaction<'_, Postgres>,
    session_id: i32,
    user_id: i32,
) -> Result<DnbSessionResultsDto, WisecrowError> {
    let row: (
        String,
        i16,
        i16,
        i16,
        i32,
        Option<f32>,
        Option<f32>,
        i32,
        i32,
    ) = sqlx::query_as(
        "SELECT mode, n_level_start, n_level_peak, n_level_end, trials_completed,
                    accuracy_audio, accuracy_visual, interval_ms_start, interval_ms_end
             FROM dnb_sessions WHERE id = $1 AND user_id = $2",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await?;
    result_from_row(session_id, row)
}

fn result_from_row(
    session_id: i32,
    row: (
        String,
        i16,
        i16,
        i16,
        i32,
        Option<f32>,
        Option<f32>,
        i32,
        i32,
    ),
) -> Result<DnbSessionResultsDto, WisecrowError> {
    let mode = row
        .0
        .parse()
        .map_err(|error: wisecrow_learning::LearningError| {
            WisecrowError::SyncError(error.to_string())
        })?;
    let state = AdaptationState {
        n_level_start: db_u8(row.1, "starting level")?,
        n_level_peak: db_u8(row.2, "peak level")?,
        n_level: db_u8(row.3, "ending level")?,
        interval_ms_start: db_u32(row.7, "starting interval")?,
        interval_ms: db_u32(row.8, "ending interval")?,
        consecutive_below_start: 0,
    };
    Ok(dnb_results_to_dto(
        session_id,
        mode,
        &state,
        db_u32(row.4, "trial count")?,
        row.5,
        row.6,
    ))
}

fn applied_ack(
    session: &NbackSessionUploadDto,
    prepared: &PreparedSession,
    server_session_id: i32,
) -> Result<NbackUploadAckDto, WisecrowError> {
    let count = u32::try_from(prepared.trials.len())
        .map_err(|_| WisecrowError::InvalidInput("n-back trial count overflow".into()))?;
    Ok(NbackUploadAckDto {
        client_session_id: session.client_session_id,
        status: NbackUploadStatusDto::Applied,
        result: Some(dnb_results_to_dto(
            server_session_id,
            prepared.mode,
            &prepared.final_state,
            count,
            Some(prepared.accuracy_audio),
            Some(prepared.accuracy_visual),
        )),
    })
}

fn rejected(client_session_id: Uuid, reason: String) -> NbackUploadAckDto {
    NbackUploadAckDto {
        client_session_id,
        status: NbackUploadStatusDto::Rejected { reason },
        result: None,
    }
}

fn db_u8(value: i16, field: &str) -> Result<u8, WisecrowError> {
    u8::try_from(value)
        .map_err(|_| WisecrowError::SyncError(format!("stored n-back {field} is invalid")))
}

fn db_u32(value: i32, field: &str) -> Result<u32, WisecrowError> {
    u32::try_from(value)
        .map_err(|_| WisecrowError::SyncError(format!("stored n-back {field} is invalid")))
}

fn invalid<T>(reason: &str) -> Result<T, WisecrowError> {
    Err(WisecrowError::InvalidInput(String::from(reason)))
}

const fn mode_from_dto(mode: NbackModeDto) -> DnbMode {
    match mode {
        NbackModeDto::AudioWritten => DnbMode::AudioWritten,
        NbackModeDto::WordTranslation => DnbMode::WordTranslation,
        NbackModeDto::AudioImage => DnbMode::AudioImage,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use proptest::prelude::*;
    use wisecrow_dto::{LanguagePairDto, NbackTrialResponseDto};

    use super::*;

    fn session(response_count: usize) -> NbackSessionUploadDto {
        let completed_at = Utc::now();
        NbackSessionUploadDto {
            client_session_id: Uuid::from_u128(1),
            pair: LanguagePairDto {
                native_lang: String::from("en"),
                foreign_lang: String::from("de"),
            },
            mode: NbackModeDto::AudioWritten,
            n_level: 2,
            interval_ms: 4_000,
            seed: 42,
            vocabulary_translation_ids: (1..=8).collect(),
            responses: (1..=response_count)
                .map(|trial_number| NbackTrialResponseDto {
                    trial_number: u32::try_from(trial_number).expect("generated bound"),
                    audio_response: Some(false),
                    visual_response: Some(false),
                    response_time_ms: 100,
                })
                .collect(),
            started_at: completed_at,
            completed_at,
        }
    }

    proptest! {
        #[test]
        fn response_order_requires_exact_one_based_indexes(count in 1usize..100) {
            let valid = session(count);
            prop_assert!(validate_response_order(&valid).is_ok());

            let mut invalid = valid;
            invalid.responses[0].trial_number = 2;
            prop_assert!(validate_response_order(&invalid).is_err());
        }
    }
}
