use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::srs::scheduler::{CardManager, CardState, ReviewRating};
use crate::vocabulary::VocabularyQuery;

#[derive(Debug)]
pub struct Session {
    pub id: i32,
    pub native_lang: String,
    pub foreign_lang: String,
    pub deck_size: i32,
    pub speed_ms: i32,
    pub current_index: i32,
    pub cards: Vec<CardState>,
}

pub struct SessionManager;

impl SessionManager {
    /// Creates a new learning session. Selects due cards first, then fills
    /// remaining slots with new unlearned vocabulary ordered by frequency.
    ///
    /// # Errors
    ///
    /// Returns an error if the database operations fail.
    pub async fn create(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
        deck_size: u32,
        speed_ms: u32,
    ) -> Result<Session, WisecrowError> {
        let due = CardManager::due_cards(pool, native_lang, foreign_lang, deck_size).await?;
        let due_count = u32::try_from(due.len()).unwrap_or(u32::MAX);

        let mut all_cards = due;

        if due_count < deck_size {
            let remaining = deck_size.saturating_sub(due_count);
            let unlearned =
                VocabularyQuery::unlearned(pool, native_lang, foreign_lang, remaining).await?;

            if !unlearned.is_empty() {
                let translation_ids: Vec<i32> =
                    unlearned.iter().map(|v| v.translation_id).collect();
                CardManager::ensure_cards(pool, &translation_ids).await?;

                let new_cards =
                    CardManager::due_cards(pool, native_lang, foreign_lang, remaining).await?;
                all_cards.extend(new_cards);
            }
        }

        let deck_size_i32 = i32::try_from(all_cards.len()).unwrap_or(i32::MAX);
        let speed_ms_i32 = i32::try_from(speed_ms).unwrap_or(i32::MAX);

        let session_id = sqlx::query_scalar::<_, i32>(
            "INSERT INTO sessions (native_lang, foreign_lang, deck_size, speed_ms)
             VALUES ($1, $2, $3, $4)
             RETURNING id",
        )
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(deck_size_i32)
        .bind(speed_ms_i32)
        .fetch_one(pool)
        .await?;

        if !all_cards.is_empty() {
            let card_ids: Vec<i32> = all_cards.iter().map(|c| c.card_id).collect();
            let positions: Vec<i32> =
                (0..i32::try_from(all_cards.len()).unwrap_or(i32::MAX)).collect();
            sqlx::query(
                "INSERT INTO session_cards (session_id, card_id, position)
                 SELECT $1, unnest($2::int[]), unnest($3::int[])",
            )
            .bind(session_id)
            .bind(&card_ids)
            .bind(&positions)
            .execute(pool)
            .await?;
        }

        Ok(Session {
            id: session_id,
            native_lang: native_lang.to_owned(),
            foreign_lang: foreign_lang.to_owned(),
            deck_size: deck_size_i32,
            speed_ms: speed_ms_i32,
            current_index: 0,
            cards: all_cards,
        })
    }

    /// Resumes the most recent unfinished session for this language pair.
    /// Returns `None` if no paused session exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn resume(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<Option<Session>, WisecrowError> {
        let row = sqlx::query_as::<_, (i32, i32, i32)>(
            "SELECT id, deck_size, speed_ms
             FROM sessions
             WHERE native_lang = $1 AND foreign_lang = $2
               AND completed_at IS NULL AND paused_at IS NOT NULL
             ORDER BY paused_at DESC
             LIMIT 1",
        )
        .bind(native_lang)
        .bind(foreign_lang)
        .fetch_optional(pool)
        .await?;

        let Some((session_id, deck_size, speed_ms)) = row else {
            return Ok(None);
        };

        let answered_count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM session_cards
             WHERE session_id = $1 AND answered = TRUE",
        )
        .bind(session_id)
        .fetch_one(pool)
        .await?;

        let current_index = i32::try_from(answered_count).unwrap_or(i32::MAX);

        let cards = Self::load_session_cards(pool, session_id).await?;

        sqlx::query("UPDATE sessions SET paused_at = NULL WHERE id = $1")
            .bind(session_id)
            .execute(pool)
            .await?;

        Ok(Some(Session {
            id: session_id,
            native_lang: native_lang.to_owned(),
            foreign_lang: foreign_lang.to_owned(),
            deck_size,
            speed_ms,
            current_index,
            cards,
        }))
    }

    /// Pauses an active session, recording the current position.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn pause(pool: &PgPool, session_id: i32) -> Result<(), WisecrowError> {
        sqlx::query("UPDATE sessions SET paused_at = NOW() WHERE id = $1")
            .bind(session_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Marks a session as complete.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn complete(pool: &PgPool, session_id: i32) -> Result<(), WisecrowError> {
        sqlx::query("UPDATE sessions SET completed_at = NOW() WHERE id = $1")
            .bind(session_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Records a card answer within a session and updates the SRS state.
    ///
    /// # Errors
    ///
    /// Returns an error if the database operations fail.
    pub async fn answer_card(
        pool: &PgPool,
        session_id: i32,
        card: &CardState,
        rating: ReviewRating,
    ) -> Result<CardState, WisecrowError> {
        sqlx::query(
            "UPDATE session_cards SET answered = TRUE, rating = $1, answered_at = NOW()
             WHERE session_id = $2 AND card_id = $3",
        )
        .bind(rating.to_db())
        .bind(session_id)
        .bind(card.card_id)
        .execute(pool)
        .await?;

        CardManager::review(pool, card, rating).await
    }

    async fn load_session_cards(
        pool: &PgPool,
        session_id: i32,
    ) -> Result<Vec<CardState>, WisecrowError> {
        let query = format!(
            "SELECT {} \
             FROM session_cards sc \
             JOIN cards c ON sc.card_id = c.id \
             JOIN translations t ON c.translation_id = t.id \
             WHERE sc.session_id = $1 \
             ORDER BY sc.position",
            super::scheduler::CARD_SELECT_COLUMNS
        );
        let rows = sqlx::query_as::<_, super::scheduler::CardRow>(&query)
            .bind(session_id)
            .fetch_all(pool)
            .await?;

        Ok(rows.into_iter().map(CardState::from_row).collect())
    }
}
