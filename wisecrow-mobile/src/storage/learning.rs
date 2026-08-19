use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, Sqlite, Transaction};
use uuid::Uuid;
use wisecrow_dto::{
    CardSnapshotDto, CardStatusDto, CorpusTranslationDto, ReviewBatchResponseDto, ReviewEventDto,
    ReviewEventStatusDto, ReviewRatingDto, MOBILE_PROTOCOL_VERSION,
};
use wisecrow_learning::srs::{CardState, CardStatus, FsrsScheduler, ReviewRating, Scheduler};

use super::{
    models::{
        LocalAnswer, LocalSession, LocalSessionCard, LocalSessionRequest, LocalSessionStatus,
    },
    sqlite::{active_scope, interleave_ranked, StoreScope},
    SqliteStore,
};
use crate::application::{LearningRepository, MobileError};

const MAX_DECK_SIZE: u16 = 500;
const MIN_SPEED_MS: u32 = 250;
const MAX_SPEED_MS: u32 = 60_000;

#[derive(FromRow)]
struct SessionRow {
    id: Uuid,
    native_lang: String,
    foreign_lang: String,
    deck_size: i64,
    speed_ms: i64,
    current_index: i64,
    status: String,
    started_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct SessionCardRow {
    translation_id: i32,
    from_phrase: String,
    to_phrase: String,
    frequency: i32,
    is_phrase: bool,
    stability: Option<f64>,
    difficulty: Option<f64>,
    elapsed_days: Option<i64>,
    scheduled_days: Option<i64>,
    reps: Option<i32>,
    lapses: Option<i32>,
    card_state: Option<i64>,
    last_review: Option<DateTime<Utc>>,
    due: Option<DateTime<Utc>>,
    server_cursor: Option<i64>,
    answered: bool,
    rating: Option<i64>,
    answered_at: Option<DateTime<Utc>>,
}

#[derive(FromRow)]
struct ExistingEventRow {
    device_id: Uuid,
    translation_id: i32,
    rating: i64,
    occurred_at: DateTime<Utc>,
}

#[async_trait]
impl LearningRepository for SqliteStore {
    async fn create_session(
        &self,
        request: &LocalSessionRequest,
    ) -> Result<LocalSession, MobileError> {
        validate_session_request(request)?;
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        if let Some(session) = existing_session(&mut transaction, scope, request).await? {
            transaction.commit().await?;
            return Ok(session);
        }
        require_ready_pair(&mut transaction, scope, request).await?;
        let translation_ids = select_session_ids(&mut transaction, scope, request).await?;
        insert_session(&mut transaction, scope, request, &translation_ids).await?;
        let session = load_session(&mut transaction, scope, request.id).await?;
        transaction.commit().await?;
        Ok(session)
    }

    async fn answer(&self, answer: &LocalAnswer) -> Result<CardSnapshotDto, MobileError> {
        validate_answer(answer)?;
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        if let Some(card) = repeated_answer(&mut transaction, scope, answer).await? {
            transaction.commit().await?;
            return Ok(card);
        }
        let card = current_card(&mut transaction, scope, answer).await?;
        let next = schedule_card(card, answer)?;
        persist_answer(&mut transaction, scope, answer, &next).await?;
        transaction.commit().await?;
        Ok(next)
    }

    async fn pending_reviews(&self, limit: u16) -> Result<Vec<ReviewEventDto>, MobileError> {
        validate_limit(limit)?;
        let scope = super::sqlite::active_scope_from_pool(&self.pool).await?;
        let rows = sqlx::query_as::<_, (Uuid, i32, i64, DateTime<Utc>)>(
            "SELECT event_id, translation_id, rating, occurred_at FROM review_outbox
             WHERE profile_id = ? AND user_id = ? AND status = 'pending'
             ORDER BY occurred_at, event_id LIMIT ?",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(review_event).collect()
    }

    async fn apply_review_response(
        &self,
        response: &ReviewBatchResponseDto,
    ) -> Result<(), MobileError> {
        validate_review_response(response)?;
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        for acknowledgement in &response.acknowledgements {
            apply_acknowledgement(&mut transaction, scope, acknowledgement).await?;
        }
        for card in &response.cards {
            validate_snapshot(card)?;
            upsert_card(&mut transaction, scope, card).await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

async fn existing_session(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    request: &LocalSessionRequest,
) -> Result<Option<LocalSession>, MobileError> {
    let row = session_row(transaction, scope, request.id).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.native_lang != request.pair.native_lang
        || row.foreign_lang != request.pair.foreign_lang
        || row.deck_size != i64::from(request.deck_size)
        || row.speed_ms != i64::from(request.speed_ms)
        || row.started_at != request.started_at
    {
        return Err(conflict("the session identifier has different parameters"));
    }
    Ok(Some(build_session(transaction, scope, row).await?))
}

async fn require_ready_pair(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    request: &LocalSessionRequest,
) -> Result<(), MobileError> {
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM language_pairs
         WHERE profile_id = ? AND user_id = ? AND native = ? AND \"foreign\" = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&request.pair.native_lang)
    .bind(&request.pair.foreign_lang)
    .fetch_optional(&mut **transaction)
    .await?;
    match status.as_deref() {
        Some("ready") => Ok(()),
        _ => Err(conflict("the language pair is not ready offline")),
    }
}

async fn select_session_ids(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    request: &LocalSessionRequest,
) -> Result<Vec<i32>, MobileError> {
    let mut due = due_translation_ids(transaction, scope, request).await?;
    let remaining = usize::from(request.deck_size).saturating_sub(due.len());
    if remaining == 0 {
        return Ok(due);
    }
    let words = uncarded_translation_ids(transaction, scope, request, false).await?;
    let phrases = uncarded_translation_ids(transaction, scope, request, true).await?;
    due.extend(interleave_ranked(words, phrases, remaining));
    due.truncate(usize::from(request.deck_size));
    if due.is_empty() {
        return Err(conflict("the language pair has no available translations"));
    }
    Ok(due)
}

async fn due_translation_ids(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    request: &LocalSessionRequest,
) -> Result<Vec<i32>, MobileError> {
    Ok(sqlx::query_scalar(
        "SELECT t.translation_id FROM translations t
         JOIN cards c ON c.profile_id = t.profile_id AND c.user_id = t.user_id
                     AND c.translation_id = t.translation_id
         WHERE t.profile_id = ? AND t.user_id = ? AND t.native = ? AND t.\"foreign\" = ?
           AND c.due <= ?
         ORDER BY CASE c.state WHEN 3 THEN 0 WHEN 1 THEN 1 WHEN 0 THEN 2 ELSE 3 END,
                  c.due, t.translation_id LIMIT ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&request.pair.native_lang)
    .bind(&request.pair.foreign_lang)
    .bind(request.started_at)
    .bind(i64::from(request.deck_size))
    .fetch_all(&mut **transaction)
    .await?)
}

async fn uncarded_translation_ids(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    request: &LocalSessionRequest,
    is_phrase: bool,
) -> Result<Vec<i32>, MobileError> {
    Ok(sqlx::query_scalar(
        "SELECT t.translation_id FROM translations t
         WHERE t.profile_id = ? AND t.user_id = ? AND t.native = ? AND t.\"foreign\" = ?
           AND t.is_phrase = ? AND NOT EXISTS (
               SELECT 1 FROM cards c WHERE c.profile_id = t.profile_id
                 AND c.user_id = t.user_id AND c.translation_id = t.translation_id
           )
         ORDER BY t.frequency DESC, t.translation_id LIMIT ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&request.pair.native_lang)
    .bind(&request.pair.foreign_lang)
    .bind(is_phrase)
    .bind(i64::from(request.deck_size))
    .fetch_all(&mut **transaction)
    .await?)
}

async fn insert_session(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    request: &LocalSessionRequest,
    translation_ids: &[i32],
) -> Result<(), MobileError> {
    sqlx::query(
        "INSERT INTO learn_sessions
             (profile_id, user_id, id, native, \"foreign\", deck_size, speed_ms, status, started_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?)",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(request.id)
    .bind(&request.pair.native_lang)
    .bind(&request.pair.foreign_lang)
    .bind(i64::from(request.deck_size))
    .bind(i64::from(request.speed_ms))
    .bind(request.started_at)
    .execute(&mut **transaction)
    .await?;
    for (position, translation_id) in translation_ids.iter().enumerate() {
        insert_session_card(transaction, scope, request.id, *translation_id, position).await?;
    }
    Ok(())
}

async fn insert_session_card(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    session_id: Uuid,
    translation_id: i32,
    position: usize,
) -> Result<(), MobileError> {
    sqlx::query(
        "INSERT INTO learn_session_cards
             (profile_id, user_id, session_id, translation_id, position)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(session_id)
    .bind(translation_id)
    .bind(i64::try_from(position).map_err(|_| invalid_input("session position is invalid"))?)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn load_session(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    session_id: Uuid,
) -> Result<LocalSession, MobileError> {
    let row = session_row(transaction, scope, session_id)
        .await?
        .ok_or_else(|| conflict("the learning session does not exist"))?;
    build_session(transaction, scope, row).await
}

async fn session_row(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    session_id: Uuid,
) -> Result<Option<SessionRow>, MobileError> {
    Ok(sqlx::query_as::<_, SessionRow>(
        "SELECT id, native AS native_lang, \"foreign\" AS foreign_lang, deck_size, speed_ms,
                current_index, status, started_at FROM learn_sessions
         WHERE profile_id = ? AND user_id = ? AND id = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(session_id)
    .fetch_optional(&mut **transaction)
    .await?)
}

async fn build_session(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    row: SessionRow,
) -> Result<LocalSession, MobileError> {
    let cards = load_session_cards(transaction, scope, row.id, row.started_at).await?;
    Ok(LocalSession {
        id: row.id,
        pair: wisecrow_dto::LanguagePairDto {
            native_lang: row.native_lang,
            foreign_lang: row.foreign_lang,
        },
        speed_ms: u32::try_from(row.speed_ms).map_err(|_| invalid_state())?,
        current_index: u32::try_from(row.current_index).map_err(|_| invalid_state())?,
        status: parse_session_status(&row.status)?,
        started_at: row.started_at,
        cards,
    })
}

async fn load_session_cards(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    session_id: Uuid,
    started_at: DateTime<Utc>,
) -> Result<Vec<LocalSessionCard>, MobileError> {
    let rows = sqlx::query_as::<_, SessionCardRow>(SESSION_CARDS_SQL)
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(session_id)
        .fetch_all(&mut **transaction)
        .await?;
    rows.into_iter()
        .map(|row| session_card(row, started_at))
        .collect()
}

const SESSION_CARDS_SQL: &str =
    "SELECT t.translation_id, t.from_phrase, t.to_phrase, t.frequency, t.is_phrase,
            c.stability, c.difficulty, c.elapsed_days, c.scheduled_days, c.reps, c.lapses,
            c.state AS card_state, c.last_review, c.due, c.server_cursor,
            sc.answered, sc.rating, sc.answered_at
     FROM learn_session_cards sc
     JOIN learn_sessions s ON s.profile_id = sc.profile_id AND s.user_id = sc.user_id
                          AND s.id = sc.session_id
     JOIN translations t ON t.profile_id = s.profile_id AND t.user_id = s.user_id
                        AND t.native = s.native AND t.\"foreign\" = s.\"foreign\"
                        AND t.translation_id = sc.translation_id
     LEFT JOIN cards c ON c.profile_id = sc.profile_id AND c.user_id = sc.user_id
                      AND c.translation_id = sc.translation_id
     WHERE sc.profile_id = ? AND sc.user_id = ? AND sc.session_id = ?
     ORDER BY sc.position";

fn session_card(
    row: SessionCardRow,
    started_at: DateTime<Utc>,
) -> Result<LocalSessionCard, MobileError> {
    let card = snapshot_from_row(&row, started_at)?;
    let translation = CorpusTranslationDto {
        translation_id: row.translation_id,
        from_phrase: row.from_phrase,
        to_phrase: row.to_phrase,
        frequency: row.frequency,
        is_phrase: row.is_phrase,
    };
    Ok(LocalSessionCard {
        translation,
        card,
        answered: row.answered,
        rating: row.rating.map(rating_from_value).transpose()?,
        answered_at: row.answered_at,
    })
}

fn snapshot_from_row(
    row: &SessionCardRow,
    started_at: DateTime<Utc>,
) -> Result<CardSnapshotDto, MobileError> {
    let Some(state) = row.card_state else {
        return Ok(CardSnapshotDto {
            translation_id: row.translation_id,
            stability: 0.0,
            difficulty: 0.0,
            elapsed_days: 0,
            scheduled_days: 0,
            reps: 0,
            lapses: 0,
            state: CardStatusDto::New,
            last_review: None,
            due: started_at,
            server_cursor: 0,
        });
    };
    Ok(CardSnapshotDto {
        translation_id: row.translation_id,
        stability: required(row.stability)?,
        difficulty: required(row.difficulty)?,
        elapsed_days: required(row.elapsed_days)?,
        scheduled_days: required(row.scheduled_days)?,
        reps: required(row.reps)?,
        lapses: required(row.lapses)?,
        state: status_from_value(state)?,
        last_review: row.last_review,
        due: required(row.due)?,
        server_cursor: required(row.server_cursor)?,
    })
}

fn required<T>(value: Option<T>) -> Result<T, MobileError> {
    value.ok_or_else(invalid_state)
}

async fn repeated_answer(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    answer: &LocalAnswer,
) -> Result<Option<CardSnapshotDto>, MobileError> {
    let existing = sqlx::query_as::<_, ExistingEventRow>(
        "SELECT device_id, translation_id, rating, occurred_at FROM review_outbox
         WHERE profile_id = ? AND user_id = ? AND event_id = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(answer.event_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(existing) = existing else {
        return Ok(None);
    };
    if existing.device_id != answer.device_id
        || existing.translation_id != answer.translation_id
        || existing.rating != rating_value(answer.rating)
        || existing.occurred_at != answer.occurred_at
    {
        return Err(conflict(
            "the review event identifier has a different payload",
        ));
    }
    Ok(Some(
        load_card(transaction, scope, answer.translation_id).await?,
    ))
}

async fn current_card(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    answer: &LocalAnswer,
) -> Result<CardState, MobileError> {
    let row = sqlx::query_as::<_, (i32, DateTime<Utc>)>(
        "SELECT sc.translation_id, s.started_at FROM learn_sessions s
         JOIN learn_session_cards sc ON sc.profile_id = s.profile_id AND sc.user_id = s.user_id
              AND sc.session_id = s.id AND sc.position = s.current_index
         WHERE s.profile_id = ? AND s.user_id = ? AND s.id = ?
           AND s.status = 'active' AND sc.answered = 0
           AND EXISTS (
               SELECT 1 FROM profile_users u WHERE u.profile_id = s.profile_id
                 AND u.user_id = s.user_id AND u.device_id = ?
           )",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(answer.session_id)
    .bind(answer.device_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| conflict("the learning session has no unanswered current card"))?;
    if row.0 != answer.translation_id {
        return Err(conflict(
            "the answer does not match the current session card",
        ));
    }
    load_card_state(transaction, scope, row.0, row.1).await
}

async fn load_card_state(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    translation_id: i32,
    baseline_at: DateTime<Utc>,
) -> Result<CardState, MobileError> {
    let card = load_optional_card(transaction, scope, translation_id).await?;
    card.map_or_else(
        || Ok(CardState::new(translation_id, baseline_at, CardStatus::New)),
        |card| state_from_snapshot(&card, baseline_at),
    )
}

fn schedule_card(card: CardState, answer: &LocalAnswer) -> Result<CardSnapshotDto, MobileError> {
    let rating = learning_rating(answer.rating);
    let next = FsrsScheduler.schedule(&card, rating, answer.occurred_at)?;
    Ok(snapshot_from_state(next, 0))
}

async fn persist_answer(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    answer: &LocalAnswer,
    card: &CardSnapshotDto,
) -> Result<(), MobileError> {
    upsert_card(transaction, scope, card).await?;
    insert_review_event(transaction, scope, answer).await?;
    let result = sqlx::query(
        "UPDATE learn_session_cards SET answered = 1, rating = ?, answered_at = ?
         WHERE profile_id = ? AND user_id = ? AND session_id = ?
           AND translation_id = ? AND answered = 0",
    )
    .bind(rating_value(answer.rating))
    .bind(answer.occurred_at)
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(answer.session_id)
    .bind(answer.translation_id)
    .execute(&mut **transaction)
    .await?;
    require_one_row(result.rows_affected())?;
    advance_session(transaction, scope, answer.session_id).await
}

async fn insert_review_event(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    answer: &LocalAnswer,
) -> Result<(), MobileError> {
    sqlx::query(
        "INSERT INTO review_outbox
             (profile_id, user_id, event_id, device_id, translation_id, rating,
              occurred_at, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'pending')",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(answer.event_id)
    .bind(answer.device_id)
    .bind(answer.translation_id)
    .bind(rating_value(answer.rating))
    .bind(answer.occurred_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn advance_session(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    session_id: Uuid,
) -> Result<(), MobileError> {
    let result = sqlx::query(
        "UPDATE learn_sessions
         SET current_index = current_index + 1,
             status = CASE WHEN current_index + 1 >= (
                 SELECT COUNT(*) FROM learn_session_cards sc
                 WHERE sc.profile_id = learn_sessions.profile_id
                   AND sc.user_id = learn_sessions.user_id AND sc.session_id = learn_sessions.id
             ) THEN 'complete' ELSE 'active' END
         WHERE profile_id = ? AND user_id = ? AND id = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(session_id)
    .execute(&mut **transaction)
    .await?;
    require_one_row(result.rows_affected())
}

async fn apply_acknowledgement(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    acknowledgement: &wisecrow_dto::ReviewEventAckDto,
) -> Result<(), MobileError> {
    let (status, reason) = match &acknowledgement.status {
        ReviewEventStatusDto::Applied | ReviewEventStatusDto::AlreadyApplied => ("applied", None),
        ReviewEventStatusDto::Rejected { reason } => {
            validate_rejection_reason(reason)?;
            ("rejected", Some(reason.as_str()))
        }
    };
    let result = sqlx::query(
        "UPDATE review_outbox SET status = ?, rejection_reason = ?
         WHERE profile_id = ? AND user_id = ? AND event_id = ?",
    )
    .bind(status)
    .bind(reason)
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(acknowledgement.event_id)
    .execute(&mut **transaction)
    .await?;
    require_one_row(result.rows_affected())
}

async fn upsert_card(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    card: &CardSnapshotDto,
) -> Result<(), MobileError> {
    sqlx::query(
        "INSERT INTO cards
             (profile_id, user_id, translation_id, stability, difficulty, elapsed_days,
              scheduled_days, reps, lapses, state, last_review, due, server_cursor)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(profile_id, user_id, translation_id) DO UPDATE SET
             stability = excluded.stability, difficulty = excluded.difficulty,
             elapsed_days = excluded.elapsed_days, scheduled_days = excluded.scheduled_days,
             reps = excluded.reps, lapses = excluded.lapses, state = excluded.state,
             last_review = excluded.last_review, due = excluded.due,
             server_cursor = MAX(cards.server_cursor, excluded.server_cursor)",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(card.translation_id)
    .bind(card.stability)
    .bind(card.difficulty)
    .bind(card.elapsed_days)
    .bind(card.scheduled_days)
    .bind(card.reps)
    .bind(card.lapses)
    .bind(status_value(card.state))
    .bind(card.last_review)
    .bind(card.due)
    .bind(card.server_cursor)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn load_optional_card(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    translation_id: i32,
) -> Result<Option<CardSnapshotDto>, MobileError> {
    let row = sqlx::query_as::<
        _,
        (
            i32,
            f64,
            f64,
            i64,
            i64,
            i32,
            i32,
            i64,
            Option<DateTime<Utc>>,
            DateTime<Utc>,
            i64,
        ),
    >(
        "SELECT translation_id, stability, difficulty, elapsed_days, scheduled_days, reps,
                lapses, state, last_review, due, server_cursor FROM cards
         WHERE profile_id = ? AND user_id = ? AND translation_id = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(translation_id)
    .fetch_optional(&mut **transaction)
    .await?;
    row.map(snapshot_from_tuple).transpose()
}

async fn load_card(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    translation_id: i32,
) -> Result<CardSnapshotDto, MobileError> {
    load_optional_card(transaction, scope, translation_id)
        .await?
        .ok_or_else(|| conflict("the reviewed card is missing"))
}

type CardTuple = (
    i32,
    f64,
    f64,
    i64,
    i64,
    i32,
    i32,
    i64,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    i64,
);

fn snapshot_from_tuple(row: CardTuple) -> Result<CardSnapshotDto, MobileError> {
    Ok(CardSnapshotDto {
        translation_id: row.0,
        stability: row.1,
        difficulty: row.2,
        elapsed_days: row.3,
        scheduled_days: row.4,
        reps: row.5,
        lapses: row.6,
        state: status_from_value(row.7)?,
        last_review: row.8,
        due: row.9,
        server_cursor: row.10,
    })
}

fn state_from_snapshot(
    card: &CardSnapshotDto,
    fallback_baseline: DateTime<Utc>,
) -> Result<CardState, MobileError> {
    Ok(CardState {
        translation_id: card.translation_id,
        baseline_at: card.last_review.unwrap_or(fallback_baseline),
        stability: card.stability,
        difficulty: card.difficulty,
        elapsed_days: card.elapsed_days,
        scheduled_days: card.scheduled_days,
        reps: card.reps,
        lapses: card.lapses,
        status: learning_status(card.state),
        last_review: card.last_review,
        due: card.due,
    })
}

fn snapshot_from_state(card: CardState, server_cursor: i64) -> CardSnapshotDto {
    CardSnapshotDto {
        translation_id: card.translation_id,
        stability: card.stability,
        difficulty: card.difficulty,
        elapsed_days: card.elapsed_days,
        scheduled_days: card.scheduled_days,
        reps: card.reps,
        lapses: card.lapses,
        state: dto_status(card.status),
        last_review: card.last_review,
        due: card.due,
        server_cursor,
    }
}

fn review_event(row: (Uuid, i32, i64, DateTime<Utc>)) -> Result<ReviewEventDto, MobileError> {
    Ok(ReviewEventDto {
        event_id: row.0,
        translation_id: row.1,
        rating: rating_from_value(row.2)?,
        occurred_at: row.3,
    })
}

fn validate_session_request(request: &LocalSessionRequest) -> Result<(), MobileError> {
    let valid_pair = valid_language_code(&request.pair.native_lang)
        && valid_language_code(&request.pair.foreign_lang)
        && request.pair.native_lang != request.pair.foreign_lang;
    if !valid_pair
        || request.deck_size == 0
        || request.deck_size > MAX_DECK_SIZE
        || !(MIN_SPEED_MS..=MAX_SPEED_MS).contains(&request.speed_ms)
    {
        return Err(invalid_input("learning session parameters are invalid"));
    }
    Ok(())
}

fn validate_answer(answer: &LocalAnswer) -> Result<(), MobileError> {
    if answer.translation_id <= 0 {
        return Err(invalid_input("review answer is invalid"));
    }
    Ok(())
}

fn validate_review_response(response: &ReviewBatchResponseDto) -> Result<(), MobileError> {
    if response.protocol_version != MOBILE_PROTOCOL_VERSION {
        return Err(MobileError::UnsupportedProtocol {
            required: MOBILE_PROTOCOL_VERSION,
            actual: response.protocol_version,
        });
    }
    Ok(())
}

fn validate_snapshot(card: &CardSnapshotDto) -> Result<(), MobileError> {
    let valid = card.translation_id > 0
        && card.stability.is_finite()
        && card.stability >= 0.0
        && card.difficulty.is_finite()
        && card.difficulty >= 0.0
        && card.elapsed_days >= 0
        && card.scheduled_days >= 0
        && card.reps >= 0
        && card.lapses >= 0
        && card.server_cursor >= 0;
    if valid {
        Ok(())
    } else {
        Err(invalid_input("authoritative card state is invalid"))
    }
}

fn validate_rejection_reason(reason: &str) -> Result<(), MobileError> {
    if reason.is_empty() || reason.len() > 1_024 || reason.chars().any(char::is_control) {
        return Err(invalid_input("review rejection reason is invalid"));
    }
    Ok(())
}

fn validate_limit(limit: u16) -> Result<(), MobileError> {
    if limit == 0 || limit > MAX_DECK_SIZE {
        return Err(invalid_input("review query limit is invalid"));
    }
    Ok(())
}

fn valid_language_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 32
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn parse_session_status(value: &str) -> Result<LocalSessionStatus, MobileError> {
    match value {
        "active" => Ok(LocalSessionStatus::Active),
        "paused" => Ok(LocalSessionStatus::Paused),
        "complete" => Ok(LocalSessionStatus::Complete),
        _ => Err(invalid_state()),
    }
}

fn status_value(status: CardStatusDto) -> i64 {
    match status {
        CardStatusDto::New => 0,
        CardStatusDto::Learning => 1,
        CardStatusDto::Review => 2,
        CardStatusDto::Relearning => 3,
    }
}

fn status_from_value(value: i64) -> Result<CardStatusDto, MobileError> {
    match value {
        0 => Ok(CardStatusDto::New),
        1 => Ok(CardStatusDto::Learning),
        2 => Ok(CardStatusDto::Review),
        3 => Ok(CardStatusDto::Relearning),
        _ => Err(invalid_state()),
    }
}

fn learning_status(status: CardStatusDto) -> CardStatus {
    match status {
        CardStatusDto::New => CardStatus::New,
        CardStatusDto::Learning => CardStatus::Learning,
        CardStatusDto::Review => CardStatus::Review,
        CardStatusDto::Relearning => CardStatus::Relearning,
    }
}

fn dto_status(status: CardStatus) -> CardStatusDto {
    match status {
        CardStatus::New => CardStatusDto::New,
        CardStatus::Learning => CardStatusDto::Learning,
        CardStatus::Review => CardStatusDto::Review,
        CardStatus::Relearning => CardStatusDto::Relearning,
    }
}

fn rating_value(rating: ReviewRatingDto) -> i64 {
    match rating {
        ReviewRatingDto::Again => 1,
        ReviewRatingDto::Hard => 2,
        ReviewRatingDto::Good => 3,
        ReviewRatingDto::Easy => 4,
    }
}

fn rating_from_value(value: i64) -> Result<ReviewRatingDto, MobileError> {
    match value {
        1 => Ok(ReviewRatingDto::Again),
        2 => Ok(ReviewRatingDto::Hard),
        3 => Ok(ReviewRatingDto::Good),
        4 => Ok(ReviewRatingDto::Easy),
        _ => Err(invalid_state()),
    }
}

fn learning_rating(rating: ReviewRatingDto) -> ReviewRating {
    match rating {
        ReviewRatingDto::Again => ReviewRating::Again,
        ReviewRatingDto::Hard => ReviewRating::Hard,
        ReviewRatingDto::Good => ReviewRating::Good,
        ReviewRatingDto::Easy => ReviewRating::Easy,
    }
}

fn require_one_row(rows_affected: u64) -> Result<(), MobileError> {
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(conflict("concurrent local state change"))
    }
}

fn invalid_input(message: &str) -> MobileError {
    MobileError::InvalidInput(String::from(message))
}

fn conflict(message: &str) -> MobileError {
    MobileError::Conflict(String::from(message))
}

fn invalid_state() -> MobileError {
    conflict("stored learning state is invalid")
}
