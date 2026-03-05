use chrono::{DateTime, Utc};
use rs_fsrs::{Card, Rating, State, FSRS};
use sqlx::PgPool;

use crate::errors::WisecrowError;

pub(crate) const CARD_SELECT_COLUMNS: &str =
    "c.id, c.translation_id, t.from_phrase, t.to_phrase, t.frequency, \
     c.stability, c.difficulty, c.state, c.due, c.reps, c.lapses";

pub(crate) type CardRow = (
    i32,
    i32,
    String,
    String,
    i32,
    f32,
    f32,
    i16,
    DateTime<Utc>,
    i32,
    i32,
);

#[derive(Debug, Clone)]
pub struct CardState {
    pub card_id: i32,
    pub translation_id: i32,
    pub from_phrase: String,
    pub to_phrase: String,
    pub frequency: i32,
    pub stability: f64,
    pub difficulty: f64,
    pub state: CardStatus,
    pub due: DateTime<Utc>,
    pub reps: i32,
    pub lapses: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardStatus {
    New,
    Learning,
    Review,
    Relearning,
}

impl CardStatus {
    #[must_use]
    pub fn from_db(value: i16) -> Self {
        match value {
            1 => Self::Learning,
            2 => Self::Review,
            3 => Self::Relearning,
            _ => Self::New,
        }
    }

    const fn to_db(self) -> i16 {
        match self {
            Self::New => 0,
            Self::Learning => 1,
            Self::Review => 2,
            Self::Relearning => 3,
        }
    }
}

impl From<State> for CardStatus {
    fn from(s: State) -> Self {
        match s {
            State::New => Self::New,
            State::Learning => Self::Learning,
            State::Review => Self::Review,
            State::Relearning => Self::Relearning,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewRating {
    Again,
    Hard,
    Good,
    Easy,
}

impl From<ReviewRating> for Rating {
    fn from(r: ReviewRating) -> Self {
        match r {
            ReviewRating::Again => Self::Again,
            ReviewRating::Hard => Self::Hard,
            ReviewRating::Good => Self::Good,
            ReviewRating::Easy => Self::Easy,
        }
    }
}

impl ReviewRating {
    #[must_use]
    pub const fn from_db(value: i16) -> Option<Self> {
        match value {
            1 => Some(Self::Again),
            2 => Some(Self::Hard),
            3 => Some(Self::Good),
            4 => Some(Self::Easy),
            _ => None,
        }
    }

    pub const fn to_db(self) -> i16 {
        match self {
            Self::Again => 1,
            Self::Hard => 2,
            Self::Good => 3,
            Self::Easy => 4,
        }
    }
}

/// Narrows `f64` to `f32`, clamping to `f32` bounds instead of producing infinity.
/// FSRS uses `f64` internally but PostgreSQL stores as `f32`.
fn f64_to_f32_clamped(v: f64) -> f32 {
    if v.is_nan() {
        0.0
    } else if v > f64::from(f32::MAX) {
        f32::MAX
    } else if v < f64::from(f32::MIN) {
        f32::MIN
    } else {
        // Intentional precision narrowing: FSRS f64 → PostgreSQL REAL (f32)
        #[expect(clippy::cast_possible_truncation)]
        let result = v as f32;
        result
    }
}

impl CardState {
    pub(crate) fn from_row(
        (
            card_id,
            translation_id,
            from_phrase,
            to_phrase,
            frequency,
            stability,
            difficulty,
            state,
            due,
            reps,
            lapses,
        ): CardRow,
    ) -> Self {
        Self {
            card_id,
            translation_id,
            from_phrase,
            to_phrase,
            frequency,
            stability: f64::from(stability),
            difficulty: f64::from(difficulty),
            state: CardStatus::from_db(state),
            due,
            reps,
            lapses,
        }
    }
}

pub struct CardManager;

impl CardManager {
    /// Creates cards for translations that don't already have them.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub async fn ensure_cards(
        pool: &PgPool,
        translation_ids: &[i32],
    ) -> Result<Vec<i32>, WisecrowError> {
        if translation_ids.is_empty() {
            return Ok(Vec::new());
        }

        let ids = sqlx::query_scalar::<_, i32>(
            "INSERT INTO cards (translation_id)
             SELECT unnest($1::int[])
             ON CONFLICT (translation_id) DO UPDATE SET translation_id = cards.translation_id
             RETURNING id",
        )
        .bind(translation_ids)
        .fetch_all(pool)
        .await?;

        Ok(ids)
    }

    /// Fetches a single card by ID, including its translation data.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or the card does not exist.
    pub async fn get_card_by_id(pool: &PgPool, card_id: i32) -> Result<CardState, WisecrowError> {
        let query = format!(
            "SELECT {CARD_SELECT_COLUMNS} \
             FROM cards c \
             JOIN translations t ON c.translation_id = t.id \
             WHERE c.id = $1"
        );
        let row = sqlx::query_as::<_, CardRow>(&query)
            .bind(card_id)
            .fetch_optional(pool)
            .await?;

        row.map(CardState::from_row)
            .ok_or_else(|| WisecrowError::InvalidInput(format!("Card with id {card_id} not found")))
    }

    /// Returns cards due for review, prioritised by state then due date.
    ///
    /// Priority: Relearning > Learning > New > Review. Within each state,
    /// ordered by due date ascending.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn due_cards(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
        limit: u32,
    ) -> Result<Vec<CardState>, WisecrowError> {
        let query = format!(
            "SELECT {CARD_SELECT_COLUMNS} \
             FROM cards c \
             JOIN translations t ON c.translation_id = t.id \
             JOIN languages fl ON t.from_language_id = fl.id \
             JOIN languages tl ON t.to_language_id = tl.id \
             WHERE fl.code = $1 AND tl.code = $2 AND c.due <= NOW() \
             ORDER BY \
                CASE c.state \
                    WHEN 3 THEN 0 \
                    WHEN 1 THEN 1 \
                    WHEN 0 THEN 2 \
                    WHEN 2 THEN 3 \
                    ELSE 4 \
                END, \
                c.due ASC \
             LIMIT $3"
        );
        let rows = sqlx::query_as::<_, CardRow>(&query)
            .bind(native_lang)
            .bind(foreign_lang)
            .bind(i64::from(limit))
            .fetch_all(pool)
            .await?;

        Ok(rows.into_iter().map(CardState::from_row).collect())
    }

    /// Applies a review rating to a card, computes next FSRS state, and
    /// persists the update.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub async fn review(
        pool: &PgPool,
        card: &CardState,
        rating: ReviewRating,
    ) -> Result<CardState, WisecrowError> {
        let fsrs = FSRS::default();
        let now = Utc::now();

        let fsrs_card = Card {
            due: card.due,
            stability: card.stability,
            difficulty: card.difficulty,
            elapsed_days: now.signed_duration_since(card.due).num_days().max(0),
            scheduled_days: 0,
            reps: card.reps,
            lapses: card.lapses,
            state: match card.state {
                CardStatus::New => State::New,
                CardStatus::Learning => State::Learning,
                CardStatus::Review => State::Review,
                CardStatus::Relearning => State::Relearning,
            },
            last_review: now,
        };

        let info = fsrs.next(fsrs_card, now, rating.into());
        let new_card = &info.card;
        let new_state = CardStatus::from(new_card.state);

        sqlx::query(
            "UPDATE cards SET
                stability = $1, difficulty = $2,
                elapsed_days = $3, scheduled_days = $4,
                reps = $5, lapses = $6, state = $7,
                last_review = $8, due = $9
             WHERE id = $10",
        )
        .bind(f64_to_f32_clamped(new_card.stability))
        .bind(f64_to_f32_clamped(new_card.difficulty))
        .bind(i32::try_from(new_card.elapsed_days).unwrap_or(i32::MAX))
        .bind(i32::try_from(new_card.scheduled_days).unwrap_or(i32::MAX))
        .bind(new_card.reps)
        .bind(new_card.lapses)
        .bind(new_state.to_db())
        .bind(now)
        .bind(new_card.due)
        .execute(pool)
        .await?;

        Ok(CardState {
            card_id: card.card_id,
            translation_id: card.translation_id,
            from_phrase: card.from_phrase.clone(), // clone: building new owned struct from borrowed
            to_phrase: card.to_phrase.clone(),     // clone: building new owned struct from borrowed
            frequency: card.frequency,
            stability: new_card.stability,
            difficulty: new_card.difficulty,
            state: new_state,
            due: new_card.due,
            reps: new_card.reps,
            lapses: new_card.lapses,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    proptest! {
        #[test]
        fn card_status_roundtrip(db_val in 0i16..=3) {
            let status = CardStatus::from_db(db_val);
            prop_assert_eq!(status.to_db(), db_val);
        }

        #[test]
        fn review_rating_roundtrip(db_val in 1i16..=4) {
            let rating = ReviewRating::from_db(db_val).unwrap();
            prop_assert_eq!(rating.to_db(), db_val);
        }
    }

    #[test]
    fn unknown_card_status_defaults_to_new() {
        assert_eq!(CardStatus::from_db(99), CardStatus::New);
    }

    #[test]
    fn unknown_rating_returns_none() {
        assert!(ReviewRating::from_db(0).is_none());
        assert!(ReviewRating::from_db(5).is_none());
    }

    #[rstest]
    #[case(ReviewRating::Again, Rating::Again)]
    #[case(ReviewRating::Hard, Rating::Hard)]
    #[case(ReviewRating::Good, Rating::Good)]
    #[case(ReviewRating::Easy, Rating::Easy)]
    fn fsrs_rating_conversion(#[case] input: ReviewRating, #[case] expected: Rating) {
        assert_eq!(Rating::from(input), expected);
    }

    #[rstest]
    #[case(State::New, CardStatus::New)]
    #[case(State::Learning, CardStatus::Learning)]
    #[case(State::Review, CardStatus::Review)]
    #[case(State::Relearning, CardStatus::Relearning)]
    fn fsrs_state_conversion(#[case] input: State, #[case] expected: CardStatus) {
        assert_eq!(CardStatus::from(input), expected);
    }

    #[test]
    fn fsrs_good_rating_produces_future_due() {
        let fsrs = FSRS::default();
        let card = Card::new();
        let now = Utc::now();

        let info = fsrs.next(card, now, Rating::Good);
        assert!(info.card.due >= now);
        assert!(info.card.stability > 0.0);
    }

    #[test]
    fn fsrs_again_rating_keeps_short_interval() {
        let fsrs = FSRS::default();
        let card = Card::new();
        let now = Utc::now();

        let good_info = fsrs.next(card.clone(), now, Rating::Good); // clone: Card is small, need both outcomes
        let again_info = fsrs.next(card, now, Rating::Again);

        assert!(again_info.card.due <= good_info.card.due);
    }

    #[test]
    fn fsrs_repeated_good_increases_stability() {
        let fsrs = FSRS::default();
        let mut card = Card::new();

        let mut last_stability = 0.0f64;
        for _ in 0..5 {
            let info = fsrs.next(card, Utc::now(), Rating::Good);
            assert!(
                info.card.stability >= last_stability,
                "stability should not decrease on Good: {} < {}",
                info.card.stability,
                last_stability
            );
            last_stability = info.card.stability;
            card = info.card;
        }
    }
}
