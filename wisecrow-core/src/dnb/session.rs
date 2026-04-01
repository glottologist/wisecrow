use sqlx::PgPool;

use crate::errors::WisecrowError;

use super::scoring::AdaptationState;
use super::{CompletedTrial, DnbMode, DnbVocab};

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

    /// Saves a completed trial to the database.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub async fn save_trial(
        pool: &PgPool,
        session_id: i32,
        trial: &CompletedTrial,
    ) -> Result<(), WisecrowError> {
        sqlx::query(
            "INSERT INTO dnb_trials \
                (session_id, trial_number, n_level, \
                 audio_translation_id, visual_translation_id, \
                 audio_match, visual_match, \
                 audio_response, visual_response, \
                 response_time_ms, interval_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
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
        .execute(pool)
        .await?;

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
        state: &AdaptationState,
        trials_completed: u32,
        accuracy_audio: Option<f32>,
        accuracy_visual: Option<f32>,
    ) -> Result<(), WisecrowError> {
        sqlx::query(
            "UPDATE dnb_sessions SET \
                n_level_peak = $2, \
                n_level_end = $3, \
                trials_completed = $4, \
                accuracy_audio = $5, \
                accuracy_visual = $6, \
                interval_ms_end = $7, \
                completed_at = NOW() \
             WHERE id = $1",
        )
        .bind(session_id)
        .bind(i16::from(state.n_level_peak))
        .bind(i16::from(state.n_level))
        .bind(i32::try_from(trials_completed).unwrap_or(i32::MAX))
        .bind(accuracy_audio)
        .bind(accuracy_visual)
        .bind(i32::try_from(state.interval_ms).unwrap_or(i32::MAX))
        .execute(pool)
        .await?;

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
             LEFT JOIN cards c ON c.translation_id = t.id \
             ORDER BY COALESCE(c.stability, 0) DESC, t.frequency DESC \
             LIMIT $3",
        )
        .bind(foreign_lang)
        .bind(native_lang)
        .bind(limit_i64)
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
}
