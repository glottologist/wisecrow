use sqlx::PgPool;

use crate::errors::WisecrowError;

use super::scoring::AdaptationState;
use super::{CompletedTrial, DnbMode, DnbVocab, Trial, TrialResponse};

pub struct DnbSessionRepository;

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
        let session_id = sqlx::query_scalar::<_, i32>(
            "INSERT INTO dnb_sessions \
                (user_id, native_lang, foreign_lang, mode, \
                 n_level_start, n_level_peak, n_level_end, \
                 interval_ms_start, interval_ms_end) \
             VALUES ($1, $2, $3, $4, $5, $5, $5, $6, $6) \
             RETURNING id",
        )
        .bind(user_id)
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(mode.as_str())
        .bind(i16::from(state.n_level))
        .bind(i32::try_from(state.interval_ms).unwrap_or(i32::MAX))
        .fetch_one(pool)
        .await?;

        Ok(session_id)
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
        // Fail closed: the row is inserted only when the session belongs to the
        // caller. Zero affected rows means the caller does not own the session.
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
        .bind(i32::try_from(trial.trial.trial_number).unwrap_or(i32::MAX))
        .bind(i16::from(trial.trial.n_level))
        .bind(trial.trial.audio_vocab.translation_id)
        .bind(trial.trial.visual_vocab.translation_id)
        .bind(trial.trial.audio_match)
        .bind(trial.trial.visual_match)
        .bind(trial.response.audio_response)
        .bind(trial.response.visual_response)
        .bind(
            trial
                .response
                .response_time_ms
                .map(|ms| i32::try_from(ms).unwrap_or(i32::MAX)),
        )
        .bind(i32::try_from(trial.trial.interval_ms).unwrap_or(i32::MAX))
        .bind(user_id)
        .execute(pool)
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
        let result = sqlx::query(
            "UPDATE dnb_sessions SET \
                n_level_peak = $2, \
                n_level_end = $3, \
                trials_completed = $4, \
                accuracy_audio = $5, \
                accuracy_visual = $6, \
                interval_ms_end = $7, \
                completed_at = NOW() \
             WHERE id = $1 AND user_id = $8",
        )
        .bind(session_id)
        .bind(i16::from(state.n_level_peak))
        .bind(i16::from(state.n_level))
        .bind(i32::try_from(trials_completed).unwrap_or(i32::MAX))
        .bind(accuracy_audio)
        .bind(accuracy_visual)
        .bind(i32::try_from(state.interval_ms).unwrap_or(i32::MAX))
        .bind(user_id)
        .execute(pool)
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
        let rows = sqlx::query_as::<
            _,
            (
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
            ),
        >(
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

        Ok(rows
            .into_iter()
            .map(
                |(tn, nl, aid, vid, am, vm, ar, vr, rt, im)| CompletedTrial {
                    trial: Trial {
                        trial_number: u32::try_from(tn).unwrap_or(0),
                        n_level: u8::try_from(nl).unwrap_or(0),
                        audio_vocab: DnbVocab {
                            translation_id: aid,
                            from_phrase: String::new(),
                            to_phrase: String::new(),
                        },
                        visual_vocab: DnbVocab {
                            translation_id: vid,
                            from_phrase: String::new(),
                            to_phrase: String::new(),
                        },
                        audio_match: am,
                        visual_match: vm,
                        interval_ms: u32::try_from(im).unwrap_or(0),
                    },
                    response: TrialResponse {
                        audio_response: ar,
                        visual_response: vr,
                        response_time_ms: rt.map(|r| u32::try_from(r).unwrap_or(0)),
                    },
                },
            )
            .collect())
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
