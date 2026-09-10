use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use wisecrow_dto::{ReviewEventDto, ReviewRatingDto};

use crate::errors::WisecrowError;
use crate::srs::reviews::{ReviewLedger, ReviewSource};
use crate::srs::scheduler::{CardManager, CardState, ReviewRating};
use crate::vocabulary::VocabularyQuery;

/// Largest deck accepted by normal and passive learning sessions.
pub const MAX_DECK_SIZE: u32 = 500;
/// Stable validation message shared by core and HTTP boundaries.
pub const DECK_SIZE_ERROR: &str = "Deck size must be between 1 and 500";

/// Validates the shared positive deck-size boundary.
///
/// # Errors
///
/// Returns [`WisecrowError::InvalidInput`] outside `1..=MAX_DECK_SIZE`.
pub fn validate_deck_size(deck_size: u32) -> Result<(), WisecrowError> {
    if !(1..=MAX_DECK_SIZE).contains(&deck_size) {
        return Err(WisecrowError::InvalidInput(DECK_SIZE_ERROR.into()));
    }
    Ok(())
}

#[derive(Debug)]
pub struct Session {
    pub id: i32,
    pub user_id: i32,
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
        user_id: i32,
        native_lang: &str,
        foreign_lang: &str,
        deck_size: u32,
        speed_ms: u32,
    ) -> Result<Session, WisecrowError> {
        validate_deck_size(deck_size)?;
        let cards = Self::build_cards(pool, user_id, native_lang, foreign_lang, deck_size).await?;
        let stored_deck_size = i32::try_from(cards.len())
            .map_err(|_| WisecrowError::InvalidInput("session deck is too large".into()))?;
        let stored_speed = i32::try_from(speed_ms).unwrap_or(i32::MAX);
        let session_id = Self::insert_session(
            pool,
            user_id,
            native_lang,
            foreign_lang,
            stored_deck_size,
            stored_speed,
        )
        .await?;
        Self::insert_session_cards(pool, session_id, &cards).await?;
        Ok(Session {
            id: session_id,
            user_id,
            native_lang: native_lang.to_owned(),
            foreign_lang: foreign_lang.to_owned(),
            deck_size: stored_deck_size,
            speed_ms: stored_speed,
            current_index: 0,
            cards,
        })
    }

    async fn build_cards(
        pool: &PgPool,
        user_id: i32,
        native_lang: &str,
        foreign_lang: &str,
        deck_size: u32,
    ) -> Result<Vec<CardState>, WisecrowError> {
        let mut cards =
            CardManager::due_cards(pool, native_lang, foreign_lang, user_id, deck_size).await?;
        let due_count = u32::try_from(cards.len())
            .map_err(|_| WisecrowError::InvalidInput("too many due cards".into()))?;
        if due_count >= deck_size {
            return Ok(cards);
        }
        let remaining = deck_size - due_count;
        let unlearned = Self::select_unlearned(pool, native_lang, foreign_lang, remaining).await?;
        if unlearned.is_empty() {
            return Ok(cards);
        }
        let translation_ids: Vec<i32> =
            unlearned.iter().map(|entry| entry.translation_id).collect();
        CardManager::ensure_cards(pool, &translation_ids, user_id).await?;
        let new_cards =
            CardManager::cards_for_translation_ids(pool, user_id, &translation_ids).await?;
        cards.extend(new_cards);
        Ok(cards)
    }

    async fn select_unlearned(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
        remaining: u32,
    ) -> Result<Vec<crate::vocabulary::VocabularyEntry>, WisecrowError> {
        let words = VocabularyQuery::ranked_candidates(
            pool,
            native_lang,
            foreign_lang,
            remaining,
            crate::vocabulary::IncludeCarded::No,
            crate::vocabulary::PhraseFilter::Exclude,
        )
        .await?;
        let phrases = VocabularyQuery::ranked_candidates(
            pool,
            native_lang,
            foreign_lang,
            remaining / 5,
            crate::vocabulary::IncludeCarded::No,
            crate::vocabulary::PhraseFilter::Only,
        )
        .await?;
        let size = usize::try_from(remaining)
            .map_err(|_| WisecrowError::InvalidInput("remaining deck is too large".into()))?;
        Ok(crate::vocabulary::interleave_deck(words, phrases, size))
    }

    async fn insert_session(
        pool: &PgPool,
        user_id: i32,
        native_lang: &str,
        foreign_lang: &str,
        deck_size: i32,
        speed_ms: i32,
    ) -> Result<i32, WisecrowError> {
        Ok(sqlx::query_scalar(
            "INSERT INTO sessions (user_id, native_lang, foreign_lang, deck_size, speed_ms)
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(user_id)
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(deck_size)
        .bind(speed_ms)
        .fetch_one(pool)
        .await?)
    }

    async fn insert_session_cards(
        pool: &PgPool,
        session_id: i32,
        cards: &[CardState],
    ) -> Result<(), WisecrowError> {
        if cards.is_empty() {
            return Ok(());
        }
        let card_ids: Vec<i32> = cards.iter().map(|card| card.card_id).collect();
        let card_count = i32::try_from(cards.len())
            .map_err(|_| WisecrowError::InvalidInput("too many session cards".into()))?;
        let positions: Vec<i32> = (0..card_count).collect();
        sqlx::query(
            "INSERT INTO session_cards (session_id, card_id, position)
             SELECT $1, unnest($2::int[]), unnest($3::int[])",
        )
        .bind(session_id)
        .bind(&card_ids)
        .bind(&positions)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn latest_paused_session(
        pool: &PgPool,
        user_id: i32,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<Option<(i32, i32, i32)>, WisecrowError> {
        Ok(sqlx::query_as(
            "SELECT id, deck_size, speed_ms
             FROM sessions
             WHERE native_lang = $1 AND foreign_lang = $2 AND user_id = $3
               AND completed_at IS NULL AND paused_at IS NOT NULL
             ORDER BY paused_at DESC LIMIT 1",
        )
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(user_id)
        .fetch_optional(pool)
        .await?)
    }

    /// Resumes the latest unfinished session, if one is paused.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn resume(
        pool: &PgPool,
        user_id: i32,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<Option<Session>, WisecrowError> {
        let row = Self::latest_paused_session(pool, user_id, native_lang, foreign_lang).await?;

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
            user_id,
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
    pub async fn pause(pool: &PgPool, session_id: i32, user_id: i32) -> Result<(), WisecrowError> {
        let result =
            sqlx::query("UPDATE sessions SET paused_at = NOW() WHERE id = $1 AND user_id = $2")
                .bind(session_id)
                .bind(user_id)
                .execute(pool)
                .await?;
        if result.rows_affected() == 0 {
            return Err(WisecrowError::Unauthorized);
        }
        Ok(())
    }

    /// Marks a session as complete.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn complete(
        pool: &PgPool,
        session_id: i32,
        user_id: i32,
    ) -> Result<(), WisecrowError> {
        let result =
            sqlx::query("UPDATE sessions SET completed_at = NOW() WHERE id = $1 AND user_id = $2")
                .bind(session_id)
                .bind(user_id)
                .execute(pool)
                .await?;
        if result.rows_affected() == 0 {
            return Err(WisecrowError::Unauthorized);
        }
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
        user_id: i32,
        card: &CardState,
        rating: ReviewRating,
    ) -> Result<CardState, WisecrowError> {
        let occurred_at = Utc::now();
        let mut transaction = pool.begin().await?;
        let translation_id = Self::mark_answered(
            &mut transaction,
            session_id,
            user_id,
            card.card_id,
            rating,
            occurred_at,
        )
        .await?;
        let event = ReviewEventDto {
            event_id: Uuid::new_v4(),
            translation_id,
            rating: review_rating_dto(rating),
            occurred_at,
        };
        let result = ReviewLedger::new(pool)
            .apply_in_transaction(&mut transaction, user_id, None, ReviewSource::Web, &[event])
            .await?;
        let updated =
            result.cards.into_iter().next().ok_or_else(|| {
                WisecrowError::InvalidInput("review did not reconcile a card".into())
            })?;
        transaction.commit().await?;
        Ok(updated)
    }

    async fn mark_answered(
        transaction: &mut Transaction<'_, Postgres>,
        session_id: i32,
        user_id: i32,
        card_id: i32,
        rating: ReviewRating,
        occurred_at: DateTime<Utc>,
    ) -> Result<i32, WisecrowError> {
        let translation_id = sqlx::query_scalar(
            "UPDATE session_cards sc
             SET answered = TRUE, rating = $1, answered_at = $2
             FROM sessions s, cards c
             WHERE sc.session_id = $3 AND sc.card_id = $4
               AND s.id = sc.session_id AND s.user_id = $5
               AND c.id = sc.card_id AND c.user_id = $5
             RETURNING c.translation_id",
        )
        .bind(rating.to_db())
        .bind(occurred_at)
        .bind(session_id)
        .bind(card_id)
        .bind(user_id)
        .fetch_optional(&mut **transaction)
        .await?;
        translation_id.ok_or(WisecrowError::Unauthorized)
    }

    async fn load_session_cards(
        pool: &PgPool,
        session_id: i32,
    ) -> Result<Vec<CardState>, WisecrowError> {
        let query = format!(
            "SELECT {} \
             FROM session_cards sc \
             JOIN cards c ON sc.card_id = c.id \
             {} \
             WHERE sc.session_id = $1 \
             ORDER BY sc.position",
            super::scheduler::CARD_SELECT_COLUMNS,
            super::scheduler::CARD_PRESENTATION_JOINS
        );
        let rows = sqlx::query_as::<_, super::scheduler::CardRow>(&query)
            .bind(session_id)
            .fetch_all(pool)
            .await?;

        Ok(rows.into_iter().map(CardState::from_row).collect())
    }
}

const fn review_rating_dto(rating: ReviewRating) -> ReviewRatingDto {
    match rating {
        ReviewRating::Again => ReviewRatingDto::Again,
        ReviewRating::Hard => ReviewRatingDto::Hard,
        ReviewRating::Good => ReviewRatingDto::Good,
        ReviewRating::Easy => ReviewRatingDto::Easy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use sqlx::postgres::PgPoolOptions;

    #[rstest]
    #[case(0, false)]
    #[case(1, true)]
    #[case(500, true)]
    #[case(501, false)]
    fn deck_size_boundaries(#[case] size: u32, #[case] valid: bool) {
        assert_eq!(validate_deck_size(size).is_ok(), valid);
    }

    #[rstest]
    #[case(0)]
    #[case(501)]
    #[tokio::test]
    async fn create_rejects_invalid_size_before_database_access(#[case] size: u32) {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://wisecrow:wisecrow@127.0.0.1:1/unreachable")
            .expect("lazy pool");
        let result = SessionManager::create(&pool, 1, "en", "fr", size, 1_000).await;
        assert!(matches!(result, Err(WisecrowError::InvalidInput(_))));
    }
}
