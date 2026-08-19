use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use wisecrow_dto::{ReviewEventAckDto, ReviewEventDto, ReviewEventStatusDto, ReviewRatingDto};
use wisecrow_learning::srs::{
    CardState as LearningCardState, CardStatus as LearningCardStatus, FsrsScheduler,
    ReviewEvent as LearningReviewEvent, ReviewRating as LearningReviewRating, Scheduler,
};

use crate::errors::WisecrowError;
use crate::srs::scheduler::{f64_to_f32, CardRow, CardState, CardStatus, CARD_SELECT_COLUMNS};

const MAX_REVIEW_BATCH: usize = 500;
const MAX_FUTURE_MINUTES: i64 = 5;

/// Origin of a persisted review event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewSource {
    /// Review submitted through an existing browser session.
    Web,
    /// Review uploaded by a registered mobile device.
    Mobile,
    /// Review derived from an uploaded N-Back session.
    NBack,
}

impl ReviewSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Mobile => "mobile",
            Self::NBack => "nback",
        }
    }

    const fn requires_device(self) -> bool {
        matches!(self, Self::Mobile)
    }
}

/// Transaction result for one review upload batch.
#[derive(Debug)]
pub struct ReviewBatchResult {
    /// Outcome for each submitted event in request order.
    pub acknowledgements: Vec<ReviewEventAckDto>,
    /// Authoritative cards rebuilt by this batch.
    pub cards: Vec<CardState>,
}

/// Persists immutable reviews and reconciles authoritative card state.
pub struct ReviewLedger<'pool> {
    pool: &'pool PgPool,
}

impl<'pool> ReviewLedger<'pool> {
    /// Creates a ledger backed by a borrowed PostgreSQL pool.
    #[must_use]
    pub const fn new(pool: &'pool PgPool) -> Self {
        Self { pool }
    }

    /// Applies a bounded batch atomically.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid batches, unauthorized devices, event UUID
    /// collisions, replay failures, or database failures.
    pub async fn apply_batch(
        &self,
        user_id: i32,
        device_id: Option<Uuid>,
        source: ReviewSource,
        events: &[ReviewEventDto],
    ) -> Result<ReviewBatchResult, WisecrowError> {
        validate_batch(events)?;
        let mut transaction = self.pool.begin().await?;
        let result = self
            .apply_in_transaction(&mut transaction, user_id, device_id, source, events)
            .await?;
        transaction.commit().await?;
        Ok(result)
    }

    pub(crate) async fn apply_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        user_id: i32,
        device_id: Option<Uuid>,
        source: ReviewSource,
        events: &[ReviewEventDto],
    ) -> Result<ReviewBatchResult, WisecrowError> {
        validate_batch(events)?;
        validate_device(transaction, user_id, device_id, source).await?;
        if events.is_empty() {
            return Ok(ReviewBatchResult {
                acknowledgements: Vec::new(),
                cards: Vec::new(),
            });
        }

        let translation_ids = affected_translation_ids(events);
        ensure_cards(transaction, user_id, &translation_ids).await?;
        ensure_baselines(transaction, user_id, &translation_ids, events).await?;
        lock_cards(transaction, user_id, &translation_ids).await?;
        let baselines = load_baselines(transaction, user_id, &translation_ids).await?;
        validate_event_times(events, &baselines)?;
        let acknowledgements =
            insert_events(transaction, user_id, device_id, source, events).await?;
        let applied = applied_translation_ids(events, &acknowledgements);
        let cards = reconcile_cards(transaction, user_id, &baselines, &applied).await?;
        Ok(ReviewBatchResult {
            acknowledgements,
            cards,
        })
    }
}

fn applied_translation_ids(
    events: &[ReviewEventDto],
    acknowledgements: &[ReviewEventAckDto],
) -> BTreeSet<i32> {
    events
        .iter()
        .zip(acknowledgements)
        .filter_map(|(event, acknowledgement)| {
            matches!(acknowledgement.status, ReviewEventStatusDto::Applied)
                .then_some(event.translation_id)
        })
        .collect()
}

#[derive(sqlx::FromRow)]
struct BaselineRecord {
    translation_id: i32,
    stability: f64,
    difficulty: f64,
    elapsed_days: i32,
    scheduled_days: i32,
    reps: i32,
    lapses: i32,
    state: i16,
    last_review: Option<DateTime<Utc>>,
    due: DateTime<Utc>,
    captured_at: DateTime<Utc>,
}

impl BaselineRecord {
    fn learning_state(&self) -> LearningCardState {
        LearningCardState {
            translation_id: self.translation_id,
            baseline_at: self.captured_at,
            stability: self.stability,
            difficulty: self.difficulty,
            elapsed_days: i64::from(self.elapsed_days),
            scheduled_days: i64::from(self.scheduled_days),
            reps: self.reps,
            lapses: self.lapses,
            status: learning_status(self.state),
            last_review: self.last_review,
            due: self.due,
        }
    }
}

fn validate_batch(events: &[ReviewEventDto]) -> Result<(), WisecrowError> {
    if events.len() > MAX_REVIEW_BATCH {
        return Err(WisecrowError::InvalidInput(format!(
            "review batches may contain at most {MAX_REVIEW_BATCH} events"
        )));
    }
    let future_limit = Utc::now() + Duration::minutes(MAX_FUTURE_MINUTES);
    if events.iter().any(|event| event.occurred_at > future_limit) {
        return Err(WisecrowError::InvalidInput(
            "review timestamp is more than five minutes in the future".into(),
        ));
    }
    Ok(())
}

async fn validate_device(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    device_id: Option<Uuid>,
    source: ReviewSource,
) -> Result<(), WisecrowError> {
    let Some(device_id) = device_id else {
        return if source.requires_device() {
            Err(WisecrowError::InvalidInput(
                "a registered device is required for this review source".into(),
            ))
        } else {
            Ok(())
        };
    };
    let active: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM mobile_devices
             WHERE user_id = $1 AND id = $2 AND revoked_at IS NULL
         )",
    )
    .bind(user_id)
    .bind(device_id)
    .fetch_one(&mut **transaction)
    .await?;
    if active {
        Ok(())
    } else {
        Err(WisecrowError::Unauthorized)
    }
}

fn affected_translation_ids(events: &[ReviewEventDto]) -> Vec<i32> {
    events
        .iter()
        .map(|event| event.translation_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

async fn ensure_cards(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    translation_ids: &[i32],
) -> Result<(), WisecrowError> {
    sqlx::query(
        "INSERT INTO cards (user_id, translation_id)
         SELECT $1, id FROM translations WHERE id = ANY($2)
         ON CONFLICT (translation_id, user_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(translation_ids)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn ensure_baselines(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    translation_ids: &[i32],
    events: &[ReviewEventDto],
) -> Result<(), WisecrowError> {
    let earliest_event = events
        .iter()
        .map(|event| event.occurred_at)
        .min()
        .ok_or_else(|| WisecrowError::InvalidInput("review batch is empty".into()))?;
    sqlx::query(
        "INSERT INTO card_review_baselines (
             user_id, translation_id, stability, difficulty, elapsed_days,
             scheduled_days, reps, lapses, state, last_review, due, captured_at
         )
         SELECT c.user_id, c.translation_id, c.stability, c.difficulty,
                c.elapsed_days, c.scheduled_days, c.reps, c.lapses, c.state,
                c.last_review, c.due,
                CASE WHEN c.reps = 0 AND c.last_review IS NULL
                     THEN LEAST(CURRENT_TIMESTAMP, $3)
                     ELSE CURRENT_TIMESTAMP END
         FROM cards c
         WHERE c.user_id = $1 AND c.translation_id = ANY($2)
         ON CONFLICT (user_id, translation_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(translation_ids)
    .bind(earliest_event)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn lock_cards(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    translation_ids: &[i32],
) -> Result<(), WisecrowError> {
    let locked: Vec<i32> = sqlx::query_scalar(
        "SELECT translation_id FROM cards
         WHERE user_id = $1 AND translation_id = ANY($2)
         ORDER BY translation_id FOR UPDATE",
    )
    .bind(user_id)
    .bind(translation_ids)
    .fetch_all(&mut **transaction)
    .await?;
    if locked == translation_ids {
        Ok(())
    } else {
        Err(WisecrowError::InvalidInput(
            "review references an unknown translation".into(),
        ))
    }
}

async fn load_baselines(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    translation_ids: &[i32],
) -> Result<Vec<BaselineRecord>, WisecrowError> {
    let rows = sqlx::query_as(
        "SELECT translation_id, stability, difficulty, elapsed_days,
                scheduled_days, reps, lapses, state, last_review, due, captured_at
         FROM card_review_baselines
         WHERE user_id = $1 AND translation_id = ANY($2)
         ORDER BY translation_id",
    )
    .bind(user_id)
    .bind(translation_ids)
    .fetch_all(&mut **transaction)
    .await?;
    if rows.len() == translation_ids.len() {
        Ok(rows)
    } else {
        Err(WisecrowError::InvalidInput(
            "review baseline could not be established".into(),
        ))
    }
}

fn validate_event_times(
    events: &[ReviewEventDto],
    baselines: &[BaselineRecord],
) -> Result<(), WisecrowError> {
    for event in events {
        let baseline = baselines
            .iter()
            .find(|baseline| baseline.translation_id == event.translation_id)
            .ok_or_else(|| WisecrowError::InvalidInput("review baseline is missing".into()))?;
        if event.occurred_at < baseline.captured_at {
            return Err(WisecrowError::InvalidInput(
                "review timestamp predates the stored card baseline".into(),
            ));
        }
    }
    Ok(())
}

async fn insert_events(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    device_id: Option<Uuid>,
    source: ReviewSource,
    events: &[ReviewEventDto],
) -> Result<Vec<ReviewEventAckDto>, WisecrowError> {
    let mut acknowledgements = Vec::with_capacity(events.len());
    for event in events {
        let status = insert_event(transaction, user_id, device_id, source, event).await?;
        acknowledgements.push(ReviewEventAckDto {
            event_id: event.event_id,
            status,
        });
    }
    Ok(acknowledgements)
}

async fn insert_event(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    device_id: Option<Uuid>,
    source: ReviewSource,
    event: &ReviewEventDto,
) -> Result<ReviewEventStatusDto, WisecrowError> {
    let rating = rating_to_db(event.rating);
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO review_events (
             user_id, event_id, device_id, translation_id, rating, occurred_at, source
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (user_id, event_id) DO NOTHING
         RETURNING event_id",
    )
    .bind(user_id)
    .bind(event.event_id)
    .bind(device_id)
    .bind(event.translation_id)
    .bind(rating)
    .bind(event.occurred_at)
    .bind(source.as_str())
    .fetch_optional(&mut **transaction)
    .await?;
    if inserted.is_some() {
        return Ok(ReviewEventStatusDto::Applied);
    }
    compare_existing_event(transaction, user_id, device_id, source, event, rating).await
}

async fn compare_existing_event(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    device_id: Option<Uuid>,
    source: ReviewSource,
    event: &ReviewEventDto,
    rating: i16,
) -> Result<ReviewEventStatusDto, WisecrowError> {
    let existing: (Option<Uuid>, i32, i16, DateTime<Utc>, String) = sqlx::query_as(
        "SELECT device_id, translation_id, rating, occurred_at, source
         FROM review_events WHERE user_id = $1 AND event_id = $2",
    )
    .bind(user_id)
    .bind(event.event_id)
    .fetch_one(&mut **transaction)
    .await?;
    let matches = existing.0 == device_id
        && existing.1 == event.translation_id
        && existing.2 == rating
        && existing.3.timestamp_micros() == event.occurred_at.timestamp_micros()
        && existing.4 == source.as_str();
    if matches {
        Ok(ReviewEventStatusDto::AlreadyApplied)
    } else {
        Err(WisecrowError::Conflict(
            "review event UUID is already associated with another payload".into(),
        ))
    }
}

async fn reconcile_cards(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    baselines: &[BaselineRecord],
    applied: &BTreeSet<i32>,
) -> Result<Vec<CardState>, WisecrowError> {
    let mut cards = Vec::with_capacity(baselines.len());
    for baseline in baselines {
        if applied.contains(&baseline.translation_id) {
            replay_and_persist(transaction, user_id, baseline).await?;
        }
        cards.push(load_card(transaction, user_id, baseline.translation_id).await?);
    }
    Ok(cards)
}

async fn replay_and_persist(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    baseline: &BaselineRecord,
) -> Result<(), WisecrowError> {
    let events = load_review_events(
        transaction,
        user_id,
        baseline.translation_id,
        baseline.captured_at,
    )
    .await?;
    let replayed = FsrsScheduler
        .replay(&baseline.learning_state(), &events)
        .map_err(|error| WisecrowError::InvalidInput(error.to_string()))?;
    persist_card(transaction, user_id, &replayed).await
}

async fn load_review_events(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    translation_id: i32,
    baseline_at: DateTime<Utc>,
) -> Result<Vec<LearningReviewEvent>, WisecrowError> {
    let rows: Vec<(Uuid, DateTime<Utc>, i16)> = sqlx::query_as(
        "SELECT event_id, occurred_at, rating FROM review_events
         WHERE user_id = $1 AND translation_id = $2 AND occurred_at >= $3
         ORDER BY occurred_at, event_id",
    )
    .bind(user_id)
    .bind(translation_id)
    .bind(baseline_at)
    .fetch_all(&mut **transaction)
    .await?;
    rows.into_iter()
        .map(|(event_id, occurred_at, rating)| {
            Ok(LearningReviewEvent {
                event_id,
                occurred_at,
                rating: learning_rating_from_db(rating)?,
            })
        })
        .collect()
}

async fn persist_card(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    card: &LearningCardState,
) -> Result<(), WisecrowError> {
    sqlx::query(
        "UPDATE cards SET stability = $1, difficulty = $2, elapsed_days = $3,
             scheduled_days = $4, reps = $5, lapses = $6, state = $7,
             last_review = $8, due = $9
         WHERE user_id = $10 AND translation_id = $11",
    )
    .bind(f64_to_f32(card.stability, "stability")?)
    .bind(f64_to_f32(card.difficulty, "difficulty")?)
    .bind(i32::try_from(card.elapsed_days).map_err(|_| day_overflow("elapsed"))?)
    .bind(i32::try_from(card.scheduled_days).map_err(|_| day_overflow("scheduled"))?)
    .bind(card.reps)
    .bind(card.lapses)
    .bind(CardStatus::from(card.status).to_db())
    .bind(card.last_review)
    .bind(card.due)
    .bind(user_id)
    .bind(card.translation_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn load_card(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    translation_id: i32,
) -> Result<CardState, WisecrowError> {
    let query = format!(
        "SELECT {CARD_SELECT_COLUMNS} FROM cards c
         JOIN translations t ON c.translation_id = t.id
         WHERE c.user_id = $1 AND c.translation_id = $2"
    );
    let row = sqlx::query_as::<_, CardRow>(&query)
        .bind(user_id)
        .bind(translation_id)
        .fetch_one(&mut **transaction)
        .await?;
    Ok(CardState::from_row(row))
}

const fn rating_to_db(rating: ReviewRatingDto) -> i16 {
    match rating {
        ReviewRatingDto::Again => 1,
        ReviewRatingDto::Hard => 2,
        ReviewRatingDto::Good => 3,
        ReviewRatingDto::Easy => 4,
    }
}

fn learning_rating_from_db(rating: i16) -> Result<LearningReviewRating, WisecrowError> {
    match rating {
        1 => Ok(LearningReviewRating::Again),
        2 => Ok(LearningReviewRating::Hard),
        3 => Ok(LearningReviewRating::Good),
        4 => Ok(LearningReviewRating::Easy),
        _ => Err(WisecrowError::InvalidInput(
            "stored review rating is unsupported".into(),
        )),
    }
}

const fn learning_status(state: i16) -> LearningCardStatus {
    match state {
        1 => LearningCardStatus::Learning,
        2 => LearningCardStatus::Review,
        3 => LearningCardStatus::Relearning,
        _ => LearningCardStatus::New,
    }
}

fn day_overflow(field: &str) -> WisecrowError {
    WisecrowError::InvalidInput(format!("FSRS {field} days exceed database bounds"))
}
