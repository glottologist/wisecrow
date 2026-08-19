use chrono::{DateTime, Utc};
use rs_fsrs::{Card, Rating, State, FSRS};
use std::collections::HashSet;
use uuid::Uuid;

use crate::LearningError;

/// Persistable scheduling state for one translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardState {
    pub translation_id: i32,
    pub baseline_at: DateTime<Utc>,
    pub stability: f64,
    pub difficulty: f64,
    pub elapsed_days: i64,
    pub scheduled_days: i64,
    pub reps: i32,
    pub lapses: i32,
    pub status: CardStatus,
    pub last_review: Option<DateTime<Utc>>,
    pub due: DateTime<Utc>,
}

impl CardState {
    /// Creates an unscheduled card at its replay baseline.
    #[must_use]
    pub const fn new(translation_id: i32, baseline_at: DateTime<Utc>, status: CardStatus) -> Self {
        Self {
            translation_id,
            baseline_at,
            stability: 0.0,
            difficulty: 0.0,
            elapsed_days: 0,
            scheduled_days: 0,
            reps: 0,
            lapses: 0,
            status,
            last_review: None,
            due: baseline_at,
        }
    }
}

/// FSRS lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardStatus {
    New,
    Learning,
    Review,
    Relearning,
}

/// User rating applied to a reviewed card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewRating {
    Again,
    Hard,
    Good,
    Easy,
}

impl TryFrom<u8> for ReviewRating {
    type Error = LearningError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Again),
            2 => Ok(Self::Hard),
            3 => Ok(Self::Good),
            4 => Ok(Self::Easy),
            _ => Err(LearningError::InvalidReviewRating(value)),
        }
    }
}

impl From<ReviewRating> for Rating {
    fn from(rating: ReviewRating) -> Self {
        match rating {
            ReviewRating::Again => Self::Again,
            ReviewRating::Hard => Self::Hard,
            ReviewRating::Good => Self::Good,
            ReviewRating::Easy => Self::Easy,
        }
    }
}

impl From<CardStatus> for State {
    fn from(status: CardStatus) -> Self {
        match status {
            CardStatus::New => Self::New,
            CardStatus::Learning => Self::Learning,
            CardStatus::Review => Self::Review,
            CardStatus::Relearning => Self::Relearning,
        }
    }
}

impl From<State> for CardStatus {
    fn from(status: State) -> Self {
        match status {
            State::New => Self::New,
            State::Learning => Self::Learning,
            State::Review => Self::Review,
            State::Relearning => Self::Relearning,
        }
    }
}

/// Immutable input to deterministic review replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewEvent {
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub rating: ReviewRating,
}

/// Calculates card state without performing persistence.
pub trait Scheduler {
    /// Applies one rating at an explicit timestamp.
    ///
    /// # Errors
    ///
    /// Returns a typed learning error when the review cannot be scheduled.
    fn schedule(
        &self,
        card: &CardState,
        rating: ReviewRating,
        reviewed_at: DateTime<Utc>,
    ) -> Result<CardState, LearningError>;

    /// Rebuilds a card from its baseline and immutable events.
    ///
    /// # Errors
    ///
    /// Returns a typed learning error when an event cannot be replayed.
    fn replay(
        &self,
        baseline: &CardState,
        events: &[ReviewEvent],
    ) -> Result<CardState, LearningError>;
}

/// FSRS-backed scheduler implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct FsrsScheduler;

impl Scheduler for FsrsScheduler {
    fn schedule(
        &self,
        card: &CardState,
        rating: ReviewRating,
        reviewed_at: DateTime<Utc>,
    ) -> Result<CardState, LearningError> {
        if reviewed_at < card.baseline_at {
            return Err(LearningError::ReviewBeforeBaseline);
        }
        validate_finite_fields(card)?;

        let next = FSRS::default().next(fsrs_card(card), reviewed_at, rating.into());
        validate_fsrs_fields(&next.card)?;
        Ok(scheduled_state(card, next.card))
    }

    fn replay(
        &self,
        baseline: &CardState,
        events: &[ReviewEvent],
    ) -> Result<CardState, LearningError> {
        let mut event_ids = HashSet::with_capacity(events.len());
        let mut ordered = Vec::with_capacity(events.len());
        for event in events {
            if !event_ids.insert(event.event_id) {
                return Err(LearningError::DuplicateReview);
            }
            ordered.push(event);
        }
        ordered.sort_unstable_by_key(|event| (event.occurred_at, event.event_id));

        ordered.into_iter().try_fold(*baseline, |card, event| {
            self.schedule(&card, event.rating, event.occurred_at)
        })
    }
}

fn validate_finite_fields(card: &CardState) -> Result<(), LearningError> {
    validate_finite("stability", card.stability)?;
    validate_finite("difficulty", card.difficulty)
}

fn validate_fsrs_fields(card: &Card) -> Result<(), LearningError> {
    validate_finite("stability", card.stability)?;
    validate_finite("difficulty", card.difficulty)
}

fn validate_finite(field: &'static str, value: f64) -> Result<(), LearningError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(LearningError::NonFiniteCardField(field))
    }
}

fn fsrs_card(card: &CardState) -> Card {
    Card {
        due: card.due,
        stability: card.stability,
        difficulty: card.difficulty,
        elapsed_days: card.elapsed_days,
        scheduled_days: card.scheduled_days,
        reps: card.reps,
        lapses: card.lapses,
        state: card.status.into(),
        last_review: card.last_review.unwrap_or(card.baseline_at),
    }
}

fn scheduled_state(baseline: &CardState, card: Card) -> CardState {
    CardState {
        translation_id: baseline.translation_id,
        baseline_at: baseline.baseline_at,
        stability: card.stability,
        difficulty: card.difficulty,
        elapsed_days: card.elapsed_days,
        scheduled_days: card.scheduled_days,
        reps: card.reps,
        lapses: card.lapses,
        status: card.state.into(),
        last_review: Some(card.last_review),
        due: card.due,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use proptest::prelude::*;

    fn baseline() -> CardState {
        CardState::new(
            11,
            Utc.timestamp_opt(1_700_000_000, 0)
                .single()
                .expect("valid timestamp"),
            CardStatus::New,
        )
    }

    proptest! {
        #[test]
        fn every_rating_advances_a_valid_card(rating in 1u8..=4, days in 0i64..=365) {
            let scheduler = FsrsScheduler;
            let card = baseline();
            let reviewed_at = card.baseline_at + Duration::days(days);
            let rating = ReviewRating::try_from(rating).expect("generated rating");
            let next = scheduler.schedule(&card, rating, reviewed_at).expect("schedule");

            prop_assert_eq!(next.translation_id, card.translation_id);
            prop_assert_eq!(next.last_review, Some(reviewed_at));
            prop_assert!(next.due >= reviewed_at);
            prop_assert_eq!(next.reps, card.reps.saturating_add(1));
            prop_assert!(next.stability.is_finite());
            prop_assert!(next.difficulty.is_finite());
        }

        #[test]
        fn replay_is_invariant_to_upload_order(order in prop::collection::vec(0usize..4, 4)) {
            let scheduler = FsrsScheduler;
            let base = baseline();
            let ratings = [
                ReviewRating::Again,
                ReviewRating::Hard,
                ReviewRating::Good,
                ReviewRating::Easy,
            ];
            let canonical: Vec<ReviewEvent> = ratings
                .iter()
                .enumerate()
                .map(|(index, rating)| ReviewEvent {
                    event_id: Uuid::from_u128(u128::try_from(index + 1).expect("small index")),
                    occurred_at: base.baseline_at
                        + Duration::days(i64::try_from(index + 1).expect("small index")),
                    rating: *rating,
                })
                .collect();
            let mut uploaded = Vec::new();
            for index in order {
                let event = canonical[index];
                if !uploaded
                    .iter()
                    .any(|known: &ReviewEvent| known.event_id == event.event_id)
                {
                    uploaded.push(event);
                }
            }
            for event in &canonical {
                if !uploaded
                    .iter()
                    .any(|known| known.event_id == event.event_id)
                {
                    uploaded.push(*event);
                }
            }

            let expected = scheduler
                .replay(&base, &canonical)
                .expect("canonical replay");
            let actual = scheduler
                .replay(&base, &uploaded)
                .expect("permuted replay");
            prop_assert_eq!(actual, expected);
        }

        #[test]
        fn replay_rejects_duplicate_event_ids(rating in 1u8..=4) {
            let scheduler = FsrsScheduler;
            let base = baseline();
            let event = ReviewEvent {
                event_id: Uuid::from_u128(1),
                occurred_at: base.baseline_at + Duration::days(1),
                rating: ReviewRating::try_from(rating).expect("generated rating"),
            };
            prop_assert_eq!(
                scheduler.replay(&base, &[event, event]),
                Err(LearningError::DuplicateReview)
            );
        }
    }
}
