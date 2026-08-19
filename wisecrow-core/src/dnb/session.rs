use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool};

use crate::errors::WisecrowError;

use super::scoring::AdaptationState;
use super::{CompletedTrial, DnbMode, DnbVocab, Trial, TrialResponse};

pub struct DnbSessionRepository;

pub(crate) struct DnbSessionCompletion<'state> {
    pub state: &'state AdaptationState,
    pub trials_completed: u32,
    pub accuracy_audio: Option<f32>,
    pub accuracy_visual: Option<f32>,
    pub completed_at: DateTime<Utc>,
}

type AnsweredTrialRow = (
    i32,
    i16,
    i32,
    i32,
    bool,
    bool,
    Option<bool>,
    Option<bool>,
    Option<i32>,
    i32,
);

impl DnbSessionRepository {
    /// Creates a new n-back session in the database.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub async fn create_session(
        pool: &PgPool,
        user_id: i32,
        native_lang: &str,
        foreign_lang: &str,
        mode: DnbMode,
        state: &AdaptationState,
    ) -> Result<i32, WisecrowError> {
        let mut connection = pool.acquire().await?;
        Self::create_session_in(
            &mut connection,
            user_id,
            native_lang,
            foreign_lang,
            mode,
            state,
            Utc::now(),
        )
        .await
    }

    pub(crate) async fn create_session_in(
        connection: &mut PgConnection,
        user_id: i32,
        native_lang: &str,
        foreign_lang: &str,
        mode: DnbMode,
        state: &AdaptationState,
        started_at: DateTime<Utc>,
    ) -> Result<i32, WisecrowError> {
        sqlx::query_scalar::<_, i32>(
            "INSERT INTO dnb_sessions \
                (user_id, native_lang, foreign_lang, mode, \
                 n_level_start, n_level_peak, n_level_end, \
                 interval_ms_start, interval_ms_end, started_at) \
             VALUES ($1, $2, $3, $4, $5, $5, $5, $6, $6, $7) \
             RETURNING id",
        )
        .bind(user_id)
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(mode.as_str())
        .bind(i16::from(state.n_level))
        .bind(db_i32(state.interval_ms, "n-back interval")?)
        .bind(started_at)
        .fetch_one(connection)
        .await
        .map_err(Into::into)
    }

    /// Persists freshly generated trials for a session with their responses left
    /// NULL. The server thereby owns the authoritative match flags; the remote
    /// client later fills in only the responses via [`Self::record_trial_response`],
    /// so a modified client cannot forge the match outcome. Called once at session
    /// start (web tier).
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::Unauthorized`] if the session is not owned by the
    /// caller, or a database error on failure.
    pub async fn insert_generated_trials(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
        trials: &[Trial],
    ) -> Result<(), WisecrowError> {
        for trial in trials {
            let result = sqlx::query(
                "INSERT INTO dnb_trials \
                    (session_id, trial_number, n_level, \
                     audio_translation_id, visual_translation_id, \
                     audio_match, visual_match, interval_ms) \
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8 \
                 WHERE EXISTS (SELECT 1 FROM dnb_sessions WHERE id = $1 AND user_id = $9)",
            )
            .bind(session_id)
            .bind(i32::try_from(trial.trial_number).unwrap_or(i32::MAX))
            .bind(i16::from(trial.n_level))
            .bind(trial.audio_vocab.translation_id)
            .bind(trial.visual_vocab.translation_id)
            .bind(trial.audio_match)
            .bind(trial.visual_match)
            .bind(i32::try_from(trial.interval_ms).unwrap_or(i32::MAX))
            .bind(user_id)
            .execute(pool)
            .await?;
            if result.rows_affected() == 0 {
                return Err(WisecrowError::Unauthorized);
            }
        }
        Ok(())
    }

    /// Records the user's response to an already-persisted trial, identified by
    /// its number within the session. Only the response columns are written — the
    /// match flags stay as the server generated them — so scoring is always
    /// computed against server-owned truth rather than client assertion.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::Unauthorized`] if the session is not owned by the
    /// caller or the trial number is unknown, or a database error on failure.
    pub async fn record_trial_response(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
        trial_number: u32,
        response: &TrialResponse,
    ) -> Result<(), WisecrowError> {
        let result = sqlx::query(
            "UPDATE dnb_trials SET \
                audio_response = $3, visual_response = $4, response_time_ms = $5 \
             WHERE session_id = $1 AND trial_number = $2 \
               AND EXISTS (SELECT 1 FROM dnb_sessions WHERE id = $1 AND user_id = $6)",
        )
        .bind(session_id)
        .bind(i32::try_from(trial_number).unwrap_or(i32::MAX))
        .bind(response.audio_response)
        .bind(response.visual_response)
        .bind(
            response
                .response_time_ms
                .map(|ms| i32::try_from(ms).unwrap_or(i32::MAX)),
        )
        .bind(user_id)
        .execute(pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(WisecrowError::Unauthorized);
        }
        Ok(())
    }

    /// Loads the session's mode, checking ownership.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::Unauthorized`] if the session is not owned by the
    /// caller, or a database error on failure.
    pub async fn load_mode(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
    ) -> Result<DnbMode, WisecrowError> {
        sqlx::query_scalar::<_, String>(
            "SELECT mode FROM dnb_sessions WHERE id = $1 AND user_id = $2",
        )
        .bind(session_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .ok_or(WisecrowError::Unauthorized)?
        .parse()
        .map_err(|error: wisecrow_learning::LearningError| {
            WisecrowError::InvalidInput(error.to_string())
        })
    }

    /// Saves a completed trial to the database in a single insert (the local CLI
    /// generates, plays and scores trials in one trusted process, so it may write
    /// the match flags directly).
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub async fn save_trial(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
        trial: &CompletedTrial,
    ) -> Result<(), WisecrowError> {
        let mut connection = pool.acquire().await?;
        Self::save_trial_in(&mut connection, session_id, user_id, trial).await
    }

    pub(crate) async fn save_trial_in(
        connection: &mut PgConnection,
        session_id: i32,
        user_id: i32,
        trial: &CompletedTrial,
    ) -> Result<(), WisecrowError> {
        let result = sqlx::query(
            "INSERT INTO dnb_trials \
                (session_id, trial_number, n_level, \
                 audio_translation_id, visual_translation_id, \
                 audio_match, visual_match, \
                 audio_response, visual_response, \
                 response_time_ms, interval_ms) \
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11 \
             WHERE EXISTS (SELECT 1 FROM dnb_sessions WHERE id = $1 AND user_id = $12)",
        )
        .bind(session_id)
        .bind(db_i32(trial.trial.trial_number, "n-back trial number")?)
        .bind(i16::from(trial.trial.n_level))
        .bind(trial.trial.audio_vocab.translation_id)
        .bind(trial.trial.visual_vocab.translation_id)
        .bind(trial.trial.audio_match)
        .bind(trial.trial.visual_match)
        .bind(trial.response.audio_response)
        .bind(trial.response.visual_response)
        .bind(optional_db_i32(
            trial.response.response_time_ms,
            "n-back response time",
        )?)
        .bind(db_i32(trial.trial.interval_ms, "n-back interval")?)
        .bind(user_id)
        .execute(connection)
        .await?;
        if result.rows_affected() == 0 {
            return Err(WisecrowError::Unauthorized);
        }

        Ok(())
    }

    /// Completes a session, recording final stats.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn complete_session(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
        state: &AdaptationState,
        trials_completed: u32,
        accuracy_audio: Option<f32>,
        accuracy_visual: Option<f32>,
    ) -> Result<(), WisecrowError> {
        let mut connection = pool.acquire().await?;
        let completion = DnbSessionCompletion {
            state,
            trials_completed,
            accuracy_audio,
            accuracy_visual,
            completed_at: Utc::now(),
        };
        Self::complete_session_in(&mut connection, session_id, user_id, &completion).await
    }

    pub(crate) async fn complete_session_in(
        connection: &mut PgConnection,
        session_id: i32,
        user_id: i32,
        completion: &DnbSessionCompletion<'_>,
    ) -> Result<(), WisecrowError> {
        let result = sqlx::query(
            "UPDATE dnb_sessions SET \
                n_level_peak = $2, \
                n_level_end = $3, \
                trials_completed = $4, \
                accuracy_audio = $5, \
                accuracy_visual = $6, \
                interval_ms_end = $7, \
                completed_at = $8 \
             WHERE id = $1 AND user_id = $9",
        )
        .bind(session_id)
        .bind(i16::from(completion.state.n_level_peak))
        .bind(i16::from(completion.state.n_level))
        .bind(db_i32(completion.trials_completed, "n-back trial count")?)
        .bind(completion.accuracy_audio)
        .bind(completion.accuracy_visual)
        .bind(db_i32(completion.state.interval_ms, "n-back interval")?)
        .bind(completion.completed_at)
        .bind(user_id)
        .execute(connection)
        .await?;
        if result.rows_affected() == 0 {
            return Err(WisecrowError::Unauthorized);
        }

        Ok(())
    }

    /// Loads vocabulary for n-back from the translations table.
    /// Returns words sorted by card stability descending (known words first),
    /// falling back to frequency for words without cards.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn load_vocab(
        pool: &PgPool,
        user_id: i32,
        native_lang: &str,
        foreign_lang: &str,
        limit: u32,
    ) -> Result<Vec<DnbVocab>, WisecrowError> {
        let limit_i64 = i64::from(limit);
        let rows = sqlx::query_as::<_, (i32, String, String)>(
            "SELECT t.id, t.from_phrase, t.to_phrase \
             FROM translations t \
             JOIN languages fl ON t.from_language_id = fl.id AND fl.code = $1 \
             JOIN languages tl ON t.to_language_id = tl.id AND tl.code = $2 \
             LEFT JOIN cards c ON c.translation_id = t.id AND c.user_id = $4 \
             ORDER BY COALESCE(c.stability, 0) DESC, t.frequency DESC \
             LIMIT $3",
        )
        .bind(foreign_lang)
        .bind(native_lang)
        .bind(limit_i64)
        .bind(user_id)
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, from_phrase, to_phrase)| DnbVocab {
                translation_id: id,
                from_phrase,
                to_phrase,
            })
            .collect())
    }

    /// Loads the answered trials for a session, ordered by trial number, so the
    /// server can recompute accuracy and adaptation statelessly across requests.
    /// Trials pre-inserted at session start but not yet responded to are excluded,
    /// so they do not count as wrong answers. Phrases are not persisted and come
    /// back empty — scoring only needs the match/response flags.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub async fn load_answered_trials(
        pool: &PgPool,
        session_id: i32,
    ) -> Result<Vec<CompletedTrial>, WisecrowError> {
        let rows = sqlx::query_as::<_, AnsweredTrialRow>(
            "SELECT trial_number, n_level, audio_translation_id, visual_translation_id, \
                    audio_match, visual_match, audio_response, visual_response, \
                    response_time_ms, interval_ms \
             FROM dnb_trials \
             WHERE session_id = $1 \
               AND (audio_response IS NOT NULL OR visual_response IS NOT NULL) \
             ORDER BY trial_number",
        )
        .bind(session_id)
        .fetch_all(pool)
        .await?;
        rows.into_iter().map(completed_trial).collect()
    }

    /// Loads the live adaptation state for a session owned by `user_id`.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::Unauthorized`] if the session is not owned by the
    /// caller, or a database error on failure.
    pub async fn load_state(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
    ) -> Result<AdaptationState, WisecrowError> {
        let (n_end, n_start, n_peak, i_end, i_start, cbs) =
            sqlx::query_as::<_, (i16, i16, i16, i32, i32, i16)>(
                "SELECT n_level_end, n_level_start, n_level_peak, interval_ms_end, \
                        interval_ms_start, consecutive_below_start \
                 FROM dnb_sessions WHERE id = $1 AND user_id = $2",
            )
            .bind(session_id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?
            .ok_or(WisecrowError::Unauthorized)?;

        Ok(AdaptationState {
            n_level: u8::try_from(n_end).unwrap_or(1),
            n_level_start: u8::try_from(n_start).unwrap_or(1),
            n_level_peak: u8::try_from(n_peak).unwrap_or(1),
            interval_ms: u32::try_from(i_end).unwrap_or(3000),
            interval_ms_start: u32::try_from(i_start).unwrap_or(3000),
            consecutive_below_start: u8::try_from(cbs).unwrap_or(0),
        })
    }

    /// Persists the evolved adaptation state for a session owned by `user_id`.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::Unauthorized`] if the session is not owned by the
    /// caller, or a database error on failure.
    pub async fn update_state(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
        state: &AdaptationState,
    ) -> Result<(), WisecrowError> {
        let result = sqlx::query(
            "UPDATE dnb_sessions SET n_level_end = $1, n_level_peak = $2, \
                    interval_ms_end = $3, consecutive_below_start = $4 \
             WHERE id = $5 AND user_id = $6",
        )
        .bind(i16::from(state.n_level))
        .bind(i16::from(state.n_level_peak))
        .bind(i32::try_from(state.interval_ms).unwrap_or(i32::MAX))
        .bind(i16::from(state.consecutive_below_start))
        .bind(session_id)
        .bind(user_id)
        .execute(pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(WisecrowError::Unauthorized);
        }
        Ok(())
    }
}

fn db_i32(value: u32, field: &str) -> Result<i32, WisecrowError> {
    i32::try_from(value)
        .map_err(|_| WisecrowError::InvalidInput(format!("{field} exceeds database bounds")))
}

fn optional_db_i32(value: Option<u32>, field: &str) -> Result<Option<i32>, WisecrowError> {
    value.map(|value| db_i32(value, field)).transpose()
}

fn completed_trial(row: AnsweredTrialRow) -> Result<CompletedTrial, WisecrowError> {
    let (
        trial_number,
        level,
        audio_id,
        visual_id,
        audio_match,
        visual_match,
        audio,
        visual,
        time,
        interval,
    ) = row;
    Ok(CompletedTrial {
        trial: Trial {
            trial_number: stored_u32(trial_number, "trial number")?,
            n_level: u8::try_from(level).map_err(|_| stored_value("level"))?,
            audio_vocab: stored_vocab(audio_id),
            visual_vocab: stored_vocab(visual_id),
            audio_match,
            visual_match,
            interval_ms: stored_u32(interval, "interval")?,
        },
        response: TrialResponse {
            audio_response: audio,
            visual_response: visual,
            response_time_ms: time
                .map(|value| stored_u32(value, "response time"))
                .transpose()?,
        },
    })
}

fn stored_vocab(translation_id: i32) -> DnbVocab {
    DnbVocab {
        translation_id,
        from_phrase: String::new(),
        to_phrase: String::new(),
    }
}

fn stored_u32(value: i32, field: &str) -> Result<u32, WisecrowError> {
    u32::try_from(value).map_err(|_| stored_value(field))
}

fn stored_value(field: &str) -> WisecrowError {
    WisecrowError::SyncError(format!("stored n-back {field} is invalid"))
}
