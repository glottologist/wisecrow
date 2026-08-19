use chrono::{DateTime, Utc};
use num_traits::ToPrimitive;
use sqlx::PgPool;
use wisecrow_learning::srs::{
    CardState as LearningCardState, CardStatus as LearningCardStatus, FsrsScheduler,
    ReviewRating as LearningReviewRating, Scheduler,
};

use crate::errors::WisecrowError;

pub(crate) const CARD_SELECT_COLUMNS: &str =
    "c.id, c.translation_id, t.from_phrase, t.to_phrase, t.frequency, \
     c.stability, c.difficulty, c.elapsed_days, c.scheduled_days, c.state, \
     c.last_review, c.due, c.reps, c.lapses, \
     EXISTS (SELECT 1 FROM phrase_translations pt WHERE pt.translation_id = t.id)";

pub(crate) type CardRow = (
    i32,
    i32,
    String,
    String,
    i32,
    f32,
    f32,
    i32,
    i32,
    i16,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    i32,
    i32,
    bool,
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
    pub elapsed_days: i32,
    pub scheduled_days: i32,
    pub state: CardStatus,
    pub last_review: Option<DateTime<Utc>>,
    pub due: DateTime<Utc>,
    pub reps: i32,
    pub lapses: i32,
    /// A promoted phrase rather than a word; phrase cards skip image
    /// fetches because image queries are word-shaped.
    pub is_phrase: bool,
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
    pub const fn from_db(value: i16) -> Self {
        match value {
            1 => Self::Learning,
            2 => Self::Review,
            3 => Self::Relearning,
            _ => Self::New,
        }
    }

    pub(crate) const fn to_db(self) -> i16 {
        match self {
            Self::New => 0,
            Self::Learning => 1,
            Self::Review => 2,
            Self::Relearning => 3,
        }
    }
}

impl From<LearningCardStatus> for CardStatus {
    fn from(status: LearningCardStatus) -> Self {
        match status {
            LearningCardStatus::New => Self::New,
            LearningCardStatus::Learning => Self::Learning,
            LearningCardStatus::Review => Self::Review,
            LearningCardStatus::Relearning => Self::Relearning,
        }
    }
}

impl From<CardStatus> for LearningCardStatus {
    fn from(status: CardStatus) -> Self {
        match status {
            CardStatus::New => Self::New,
            CardStatus::Learning => Self::Learning,
            CardStatus::Review => Self::Review,
            CardStatus::Relearning => Self::Relearning,
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

impl From<ReviewRating> for LearningReviewRating {
    fn from(rating: ReviewRating) -> Self {
        match rating {
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

pub(crate) fn f64_to_f32(value: f64, field: &str) -> Result<f32, WisecrowError> {
    if !value.is_finite() {
        return Err(WisecrowError::InvalidInput(format!(
            "FSRS {field} must be finite"
        )));
    }
    value.to_f32().ok_or_else(|| {
        WisecrowError::InvalidInput(format!("FSRS {field} exceeds PostgreSQL REAL bounds"))
    })
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
            elapsed_days,
            scheduled_days,
            state,
            last_review,
            due,
            reps,
            lapses,
            is_phrase,
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
            elapsed_days,
            scheduled_days,
            state: CardStatus::from_db(state),
            last_review,
            due,
            reps,
            lapses,
            is_phrase,
        }
    }

    fn learning_state(&self) -> LearningCardState {
        LearningCardState {
            translation_id: self.translation_id,
            baseline_at: self.last_review.unwrap_or(self.due),
            stability: self.stability,
            difficulty: self.difficulty,
            elapsed_days: i64::from(self.elapsed_days),
            scheduled_days: i64::from(self.scheduled_days),
            reps: self.reps,
            lapses: self.lapses,
            status: self.state.into(),
            last_review: self.last_review,
            due: self.due,
        }
    }

    fn with_learning_state(&self, state: LearningCardState) -> Result<Self, WisecrowError> {
        Ok(Self {
            card_id: self.card_id,
            translation_id: self.translation_id,
            from_phrase: self.from_phrase.clone(), // clone: returned card must own borrowed phrase
            to_phrase: self.to_phrase.clone(),     // clone: returned card must own borrowed phrase
            frequency: self.frequency,
            stability: state.stability,
            difficulty: state.difficulty,
            elapsed_days: i32::try_from(state.elapsed_days).map_err(|_| {
                WisecrowError::InvalidInput("FSRS elapsed days exceed database bounds".into())
            })?,
            scheduled_days: i32::try_from(state.scheduled_days).map_err(|_| {
                WisecrowError::InvalidInput("FSRS scheduled days exceed database bounds".into())
            })?,
            state: state.status.into(),
            last_review: state.last_review,
            due: state.due,
            reps: state.reps,
            lapses: state.lapses,
            is_phrase: self.is_phrase,
        })
    }
}

pub struct CardManager;

impl CardManager {
    /// Creates cards for translations that don't already have them, scoped to
    /// the given user.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub async fn ensure_cards(
        pool: &PgPool,
        translation_ids: &[i32],
        user_id: i32,
    ) -> Result<Vec<i32>, WisecrowError> {
        if translation_ids.is_empty() {
            return Ok(Vec::new());
        }

        let ids = sqlx::query_scalar::<_, i32>(
            "INSERT INTO cards (translation_id, user_id)
             SELECT unnest($1::int[]), $2
             ON CONFLICT (translation_id, user_id)
                 DO UPDATE SET translation_id = cards.translation_id
             RETURNING id",
        )
        .bind(translation_ids)
        .bind(user_id)
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
        user_id: i32,
        limit: u32,
    ) -> Result<Vec<CardState>, WisecrowError> {
        let query = format!(
            "SELECT {CARD_SELECT_COLUMNS} \
             FROM cards c \
             JOIN translations t ON c.translation_id = t.id \
             JOIN languages fl ON t.from_language_id = fl.id \
             JOIN languages tl ON t.to_language_id = tl.id \
             WHERE fl.code = $1 AND tl.code = $2 \
               AND c.user_id = $3 AND c.due <= NOW() \
             ORDER BY \
                CASE c.state \
                    WHEN 3 THEN 0 \
                    WHEN 1 THEN 1 \
                    WHEN 0 THEN 2 \
                    WHEN 2 THEN 3 \
                    ELSE 4 \
                END, \
                c.due ASC \
             LIMIT $4"
        );
        let rows = sqlx::query_as::<_, CardRow>(&query)
            .bind(native_lang)
            .bind(foreign_lang)
            .bind(user_id)
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
        let now = Utc::now();
        let scheduled = FsrsScheduler
            .schedule(&card.learning_state(), rating.into(), now)
            .map_err(|error| WisecrowError::InvalidInput(error.to_string()))?;
        let updated = card.with_learning_state(scheduled)?;

        sqlx::query(
            "UPDATE cards SET
                stability = $1, difficulty = $2,
                elapsed_days = $3, scheduled_days = $4,
                reps = $5, lapses = $6, state = $7,
                last_review = $8, due = $9
             WHERE id = $10",
        )
        .bind(f64_to_f32(updated.stability, "stability")?)
        .bind(f64_to_f32(updated.difficulty, "difficulty")?)
        .bind(updated.elapsed_days)
        .bind(updated.scheduled_days)
        .bind(updated.reps)
        .bind(updated.lapses)
        .bind(updated.state.to_db())
        .bind(updated.last_review)
        .bind(updated.due)
        .bind(card.card_id)
        .execute(pool)
        .await?;

        Ok(updated)
    }

    /// Fetches a card by its translation ID, returning `None` if no card exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn card_for_translation(
        pool: &PgPool,
        translation_id: i32,
        user_id: i32,
    ) -> Result<Option<CardState>, WisecrowError> {
        let query = format!(
            "SELECT {CARD_SELECT_COLUMNS} \
             FROM cards c \
             JOIN translations t ON c.translation_id = t.id \
             WHERE c.translation_id = $1 AND c.user_id = $2"
        );
        let row = sqlx::query_as::<_, CardRow>(&query)
            .bind(translation_id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;

        Ok(row.map(CardState::from_row))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

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

        #[test]
        fn f64_to_f32_accepts_only_representable_finite_values(v in proptest::num::f64::ANY) {
            let expected = v.is_finite() && v.to_f32().is_some();
            prop_assert_eq!(f64_to_f32(v, "test").is_ok(), expected);
        }

        #[test]
        fn f64_to_f32_roundtrips_finite_f32(v in proptest::num::f32::NORMAL) {
            let widened = f64::from(v);
            let result = f64_to_f32(widened, "test").expect("finite f32 is representable");
            prop_assert_eq!(result, v);
        }

        #[test]
        fn out_of_range_status_defaults_to_new(v in proptest::num::i16::ANY) {
            if !(0..=3).contains(&v) {
                prop_assert_eq!(CardStatus::from_db(v), CardStatus::New);
            }
        }

        #[test]
        fn out_of_range_rating_returns_none(v in proptest::num::i16::ANY) {
            if !(1..=4).contains(&v) {
                prop_assert!(ReviewRating::from_db(v).is_none());
            }
        }
    }
}
