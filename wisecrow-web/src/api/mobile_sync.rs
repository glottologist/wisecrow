use dioxus::prelude::*;
use wisecrow_dto::{
    CardChangePageDto, CorpusChangePageDto, CorpusPageDto, CorpusSnapshotRequestDto,
    DeviceRegistrationRequestDto, MobileCapabilitiesDto, MobileFeatureDto, NbackBatchRequestDto,
    NbackBatchResponseDto, RegisteredDeviceDto, ReviewBatchRequestDto, ReviewBatchResponseDto,
    MOBILE_PROTOCOL_VERSION,
};

/// Returns the public protocol capabilities of this server build.
///
/// # Errors
///
/// This endpoint currently has no failure path.
#[post("/api/mobile/capabilities")]
pub async fn mobile_capabilities() -> Result<MobileCapabilitiesDto, ServerFnError> {
    Ok(MobileCapabilitiesDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        supported_features: vec![
            MobileFeatureDto::CorpusSync,
            MobileFeatureDto::CardSync,
            MobileFeatureDto::ReviewUpload,
            MobileFeatureDto::NbackUpload,
            MobileFeatureDto::QuizCache,
        ],
        max_snapshot_page: 500,
        max_review_batch: 500,
        max_nback_batch: 20,
        server_version: String::from(env!("CARGO_PKG_VERSION")),
    })
}

/// Registers or refreshes one authenticated mobile installation.
///
/// # Errors
///
/// Returns authentication, protocol, validation, or sanitized storage errors.
#[post("/api/mobile/devices/register")]
pub async fn mobile_register_device(
    request: DeviceRegistrationRequestDto,
) -> Result<RegisteredDeviceDto, ServerFnError> {
    implementation::register_device(request).await
}

/// Returns one resumable page from the requested corpus snapshot.
///
/// # Errors
///
/// Returns authentication, protocol, validation, or sanitized storage errors.
#[post("/api/mobile/corpus/snapshot")]
pub async fn mobile_corpus_snapshot(
    request: CorpusSnapshotRequestDto,
) -> Result<CorpusPageDto, ServerFnError> {
    implementation::corpus_snapshot(request).await
}

/// Returns corpus changes after the supplied durable cursor.
///
/// # Errors
///
/// Returns authentication, protocol, validation, or sanitized storage errors.
#[post("/api/mobile/corpus/changes")]
pub async fn mobile_corpus_changes(
    request: wisecrow_dto::CorpusChangeRequestDto,
) -> Result<CorpusChangePageDto, ServerFnError> {
    implementation::corpus_changes(request).await
}

/// Returns authoritative card changes for the authenticated account.
///
/// # Errors
///
/// Returns authentication, protocol, validation, or sanitized storage errors.
#[post("/api/mobile/cards/changes")]
pub async fn mobile_card_changes(
    request: wisecrow_dto::CardChangeRequestDto,
) -> Result<CardChangePageDto, ServerFnError> {
    implementation::card_changes(request).await
}

/// Applies offline review events and returns authoritative card state.
///
/// # Errors
///
/// Returns authentication, protocol, validation, collision, or sanitized storage errors.
#[post("/api/mobile/reviews/upload")]
pub async fn mobile_upload_reviews(
    request: ReviewBatchRequestDto,
) -> Result<ReviewBatchResponseDto, ServerFnError> {
    implementation::upload_reviews(request).await
}

/// Applies completed offline N-Back sessions using server-generated stimuli.
///
/// # Errors
///
/// Returns authentication, protocol, validation, collision, or sanitized storage errors.
#[post("/api/mobile/nback/upload")]
pub async fn mobile_upload_nback(
    request: NbackBatchRequestDto,
) -> Result<NbackBatchResponseDto, ServerFnError> {
    implementation::upload_nback(request).await
}

#[cfg(feature = "server")]
mod implementation {
    use std::collections::{BTreeMap, BTreeSet};

    use axum::http::StatusCode;
    use sqlx::types::chrono::{DateTime, Utc};
    use sqlx::PgPool;
    use uuid::Uuid;
    use wisecrow::dnb::upload::NbackUploadService;
    use wisecrow::errors::WisecrowError;
    use wisecrow::srs::reviews::{ReviewLedger, ReviewSource};
    use wisecrow::srs::scheduler::{CardState, CardStatus};
    use wisecrow_dto::{
        CardChangeDto, CardChangePageDto, CardChangeRequestDto, CardSnapshotDto, CardStatusDto,
        CorpusChangeDto, CorpusChangeKindDto, CorpusChangePageDto, CorpusChangeRequestDto,
        CorpusPageDto, CorpusSnapshotRequestDto, CorpusTranslationDto,
        DeviceRegistrationRequestDto, LanguagePairDto, NbackBatchRequestDto, NbackBatchResponseDto,
        RegisteredDeviceDto, ReviewBatchRequestDto, ReviewBatchResponseDto, ReviewEventAckDto,
        ReviewEventDto, ReviewEventStatusDto, MOBILE_PROTOCOL_VERSION,
    };

    use super::ServerFnError;

    const MAX_PAGE_SIZE: u16 = 500;

    #[derive(sqlx::FromRow)]
    struct DeviceRow {
        id: Uuid,
        display_name: String,
        created_at: DateTime<Utc>,
        last_seen_at: DateTime<Utc>,
        revoked_at: Option<DateTime<Utc>>,
    }

    #[derive(sqlx::FromRow)]
    struct CorpusRow {
        translation_id: i32,
        from_phrase: String,
        to_phrase: String,
        frequency: i32,
        is_phrase: bool,
    }

    #[derive(sqlx::FromRow)]
    struct CorpusChangeRow {
        sequence: i64,
        translation_id: i32,
        from_phrase: Option<String>,
        to_phrase: Option<String>,
        frequency: Option<i32>,
        is_phrase: Option<bool>,
        operation: String,
        changed_at: DateTime<Utc>,
    }

    #[derive(sqlx::FromRow)]
    struct CardChangeRow {
        sequence: i64,
        translation_id: i32,
        stability: Option<f32>,
        difficulty: Option<f32>,
        elapsed_days: Option<i32>,
        scheduled_days: Option<i32>,
        reps: Option<i32>,
        lapses: Option<i32>,
        state: Option<i16>,
        last_review: Option<DateTime<Utc>>,
        due: Option<DateTime<Utc>>,
    }

    #[derive(sqlx::FromRow)]
    struct ReviewValidationRow {
        translation_id: i32,
        known_translation: bool,
        captured_at: Option<DateTime<Utc>>,
    }

    pub(super) async fn register_device(
        request: DeviceRegistrationRequestDto,
    ) -> Result<RegisteredDeviceDto, ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        validate_protocol(request.protocol_version)?;
        let display_name = request.display_name.trim();
        validate_display_name(display_name)?;
        let row = sqlx::query_as::<_, DeviceRow>(
            "INSERT INTO mobile_devices (user_id, id, display_name)
             VALUES ($1, $2, $3)
             ON CONFLICT (user_id, id) DO UPDATE
             SET display_name = EXCLUDED.display_name, last_seen_at = CURRENT_TIMESTAMP
             WHERE mobile_devices.revoked_at IS NULL
             RETURNING id, display_name, created_at, last_seen_at, revoked_at",
        )
        .bind(user.id)
        .bind(request.device_id)
        .bind(display_name)
        .fetch_optional(crate::server::pool()?)
        .await
        .map_err(|error| crate::server::internal_error("mobile device registration", &error))?
        .ok_or_else(|| {
            crate::server::client_error(StatusCode::FORBIDDEN, "Device registration is revoked")
        })?;
        Ok(registered_device(row))
    }

    pub(super) async fn corpus_snapshot(
        request: CorpusSnapshotRequestDto,
    ) -> Result<CorpusPageDto, ServerFnError> {
        crate::server::auth::current_user().await?;
        validate_snapshot_request(&request)?;
        let db = crate::server::pool()?;
        let watermark =
            snapshot_watermark(db, request.snapshot_watermark, request.after_translation_id)
                .await?;
        let mut rows = load_snapshot_rows(db, &request).await?;
        let has_more = trim_page(&mut rows, request.limit);
        let next_cursor = rows
            .last()
            .map_or(request.after_translation_id, |row| row.translation_id);
        Ok(CorpusPageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            pair: request.pair,
            translations: rows.into_iter().map(corpus_translation).collect(),
            next_cursor,
            has_more,
            snapshot_watermark: watermark,
        })
    }

    pub(super) async fn corpus_changes(
        request: CorpusChangeRequestDto,
    ) -> Result<CorpusChangePageDto, ServerFnError> {
        crate::server::auth::current_user().await?;
        validate_change_request(
            request.protocol_version,
            &request.pair,
            request.cursor,
            request.limit,
        )?;
        let db = crate::server::pool()?;
        let change_watermark = corpus_change_watermark(db, &request.pair).await?;
        validate_cursor_watermark(request.cursor, change_watermark)?;
        let mut rows = load_corpus_changes(db, &request, change_watermark).await?;
        let has_more = trim_page(&mut rows, request.limit);
        let next_cursor = rows.last().map_or(request.cursor, |row| row.sequence);
        let changes = rows
            .into_iter()
            .map(corpus_change)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CorpusChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            pair: request.pair,
            changes,
            next_cursor,
            has_more,
            change_watermark,
        })
    }

    pub(super) async fn card_changes(
        request: CardChangeRequestDto,
    ) -> Result<CardChangePageDto, ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        validate_cursor_request(request.protocol_version, request.cursor, request.limit)?;
        let db = crate::server::pool()?;
        let change_watermark = card_change_watermark(db, user.id).await?;
        validate_cursor_watermark(request.cursor, change_watermark)?;
        let mut rows = load_card_changes(db, user.id, &request, change_watermark).await?;
        let has_more = trim_page(&mut rows, request.limit);
        let next_cursor = rows.last().map_or(request.cursor, |row| row.sequence);
        let changes = rows
            .into_iter()
            .map(card_change)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CardChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            changes,
            next_cursor,
            has_more,
            change_watermark,
        })
    }

    pub(super) async fn upload_reviews(
        request: ReviewBatchRequestDto,
    ) -> Result<ReviewBatchResponseDto, ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        validate_protocol(request.protocol_version)?;
        validate_review_batch_size(request.events.len())?;
        let db = crate::server::pool()?;
        ensure_active_device(db, user.id, request.device_id).await?;
        let reasons = review_rejection_reasons(db, user.id, &request.events).await?;
        let (slots, valid_events) = split_review_events(request.events, reasons);
        let result = ReviewLedger::new(db)
            .apply_batch(
                user.id,
                Some(request.device_id),
                ReviewSource::Mobile,
                &valid_events,
            )
            .await
            .map_err(map_upload_error)?;
        let acknowledgements = merge_review_acknowledgements(slots, result.acknowledgements)?;
        let cards = card_snapshots(db, user.id, result.cards).await?;
        Ok(ReviewBatchResponseDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            acknowledgements,
            cards,
        })
    }

    pub(super) async fn upload_nback(
        request: NbackBatchRequestDto,
    ) -> Result<NbackBatchResponseDto, ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        validate_protocol(request.protocol_version)?;
        let acknowledgements = NbackUploadService::new(crate::server::pool()?)
            .apply_batch(user.id, request.device_id, &request.sessions)
            .await
            .map_err(map_upload_error)?;
        Ok(NbackBatchResponseDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            acknowledgements,
        })
    }

    fn registered_device(row: DeviceRow) -> RegisteredDeviceDto {
        RegisteredDeviceDto {
            device_id: row.id,
            display_name: row.display_name,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
            revoked_at: row.revoked_at,
        }
    }

    fn validate_protocol(protocol_version: u16) -> Result<(), ServerFnError> {
        if protocol_version != MOBILE_PROTOCOL_VERSION {
            return Err(crate::server::client_error(
                StatusCode::CONFLICT,
                "Unsupported mobile protocol version",
            ));
        }
        Ok(())
    }

    fn validate_pair(pair: &LanguagePairDto) -> Result<(), ServerFnError> {
        crate::server::validate_lang(&pair.native_lang)?;
        crate::server::validate_lang(&pair.foreign_lang)?;
        if pair.native_lang == pair.foreign_lang {
            return Err(bad_request("Native and foreign languages must differ"));
        }
        Ok(())
    }

    fn validate_limit(limit: u16) -> Result<(), ServerFnError> {
        if !(1..=MAX_PAGE_SIZE).contains(&limit) {
            return Err(bad_request("Page limit must be between 1 and 500"));
        }
        Ok(())
    }

    fn validate_display_name(display_name: &str) -> Result<(), ServerFnError> {
        if !(1..=128).contains(&display_name.chars().count()) {
            return Err(bad_request(
                "Device display name must contain 1 to 128 characters",
            ));
        }
        Ok(())
    }

    fn validate_review_batch_size(size: usize) -> Result<(), ServerFnError> {
        if size > 500 {
            return Err(bad_request("Review batches may contain at most 500 events"));
        }
        Ok(())
    }

    async fn ensure_active_device(
        db: &PgPool,
        user_id: i32,
        device_id: Uuid,
    ) -> Result<(), ServerFnError> {
        let active: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1 FROM mobile_devices
                 WHERE user_id = $1 AND id = $2 AND revoked_at IS NULL
             )",
        )
        .bind(user_id)
        .bind(device_id)
        .fetch_one(db)
        .await
        .map_err(|error| crate::server::internal_error("mobile device validation", &error))?;
        active
            .then_some(())
            .ok_or_else(|| crate::server::client_error(StatusCode::FORBIDDEN, "Device is invalid"))
    }

    async fn review_rejection_reasons(
        db: &PgPool,
        user_id: i32,
        events: &[ReviewEventDto],
    ) -> Result<Vec<Option<String>>, ServerFnError> {
        let ids = events
            .iter()
            .map(|event| event.translation_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let rows = load_review_validation(db, user_id, &ids).await?;
        let by_id = rows
            .into_iter()
            .map(|row| (row.translation_id, row))
            .collect::<BTreeMap<_, _>>();
        let future_limit = Utc::now() + std::time::Duration::from_secs(300);
        Ok(events
            .iter()
            .map(|event| {
                review_rejection_reason(event, by_id.get(&event.translation_id), future_limit)
            })
            .collect())
    }

    async fn load_review_validation(
        db: &PgPool,
        user_id: i32,
        translation_ids: &[i32],
    ) -> Result<Vec<ReviewValidationRow>, ServerFnError> {
        sqlx::query_as(
            "SELECT requested.translation_id,
                    translation.id IS NOT NULL AS known_translation,
                    baseline.captured_at
             FROM unnest($1::integer[]) AS requested(translation_id)
             LEFT JOIN translations AS translation ON translation.id = requested.translation_id
             LEFT JOIN card_review_baselines AS baseline
               ON baseline.user_id = $2 AND baseline.translation_id = requested.translation_id",
        )
        .bind(translation_ids)
        .bind(user_id)
        .fetch_all(db)
        .await
        .map_err(|error| crate::server::internal_error("review validation", &error))
    }

    fn review_rejection_reason(
        event: &ReviewEventDto,
        row: Option<&ReviewValidationRow>,
        future_limit: DateTime<Utc>,
    ) -> Option<String> {
        if event.occurred_at > future_limit {
            Some(String::from(
                "Review timestamp is more than five minutes in the future",
            ))
        } else if !row.is_some_and(|row| row.known_translation) {
            Some(String::from("Review references an unknown translation"))
        } else if row
            .and_then(|row| row.captured_at)
            .is_some_and(|captured_at| event.occurred_at < captured_at)
        {
            Some(String::from(
                "Review timestamp predates the stored card baseline",
            ))
        } else {
            None
        }
    }

    fn split_review_events(
        events: Vec<ReviewEventDto>,
        reasons: Vec<Option<String>>,
    ) -> (Vec<Option<ReviewEventAckDto>>, Vec<ReviewEventDto>) {
        let mut slots = Vec::with_capacity(events.len());
        let mut valid = Vec::with_capacity(events.len());
        for (event, reason) in events.into_iter().zip(reasons) {
            if let Some(reason) = reason {
                slots.push(Some(ReviewEventAckDto {
                    event_id: event.event_id,
                    status: ReviewEventStatusDto::Rejected { reason },
                }));
            } else {
                slots.push(None);
                valid.push(event);
            }
        }
        (slots, valid)
    }

    fn merge_review_acknowledgements(
        slots: Vec<Option<ReviewEventAckDto>>,
        applied: Vec<ReviewEventAckDto>,
    ) -> Result<Vec<ReviewEventAckDto>, ServerFnError> {
        let mut applied = applied.into_iter();
        slots
            .into_iter()
            .map(|slot| {
                slot.or_else(|| applied.next())
                    .ok_or_else(|| invalid_ledger("review acknowledgement"))
            })
            .collect()
    }

    async fn card_snapshots(
        db: &PgPool,
        user_id: i32,
        cards: Vec<CardState>,
    ) -> Result<Vec<CardSnapshotDto>, ServerFnError> {
        let ids = cards
            .iter()
            .map(|card| card.translation_id)
            .collect::<Vec<_>>();
        let rows: Vec<(i32, i64)> = sqlx::query_as(
            "SELECT translation_id, MAX(sequence) FROM card_changes
             WHERE user_id = $1 AND translation_id = ANY($2)
             GROUP BY translation_id",
        )
        .bind(user_id)
        .bind(&ids)
        .fetch_all(db)
        .await
        .map_err(|error| crate::server::internal_error("review card cursors", &error))?;
        let cursors = rows.into_iter().collect::<BTreeMap<_, _>>();
        cards
            .into_iter()
            .map(|card| card_snapshot(card, &cursors))
            .collect()
    }

    fn card_snapshot(
        card: CardState,
        cursors: &BTreeMap<i32, i64>,
    ) -> Result<CardSnapshotDto, ServerFnError> {
        let server_cursor = cursors
            .get(&card.translation_id)
            .copied()
            .ok_or_else(|| invalid_ledger("review card cursor"))?;
        Ok(CardSnapshotDto {
            translation_id: card.translation_id,
            stability: card.stability,
            difficulty: card.difficulty,
            elapsed_days: i64::from(card.elapsed_days),
            scheduled_days: i64::from(card.scheduled_days),
            reps: card.reps,
            lapses: card.lapses,
            state: match card.state {
                CardStatus::New => CardStatusDto::New,
                CardStatus::Learning => CardStatusDto::Learning,
                CardStatus::Review => CardStatusDto::Review,
                CardStatus::Relearning => CardStatusDto::Relearning,
            },
            last_review: card.last_review,
            due: card.due,
            server_cursor,
        })
    }

    fn map_upload_error(error: WisecrowError) -> ServerFnError {
        match error {
            WisecrowError::Conflict(_) => {
                crate::server::client_error(StatusCode::CONFLICT, "Upload UUID collision")
            }
            WisecrowError::InvalidInput(_) => bad_request("Upload request is invalid"),
            WisecrowError::Unauthorized => {
                crate::server::client_error(StatusCode::FORBIDDEN, "Device is invalid")
            }
            other => crate::server::internal_error("mobile event upload", &other),
        }
    }

    fn validate_snapshot_request(request: &CorpusSnapshotRequestDto) -> Result<(), ServerFnError> {
        validate_protocol(request.protocol_version)?;
        validate_pair(&request.pair)?;
        validate_limit(request.limit)?;
        if request.after_translation_id < 0 {
            return Err(bad_request("Snapshot cursor must not be negative"));
        }
        if request.after_translation_id > 0 && request.snapshot_watermark.is_none() {
            return Err(bad_request(
                "Snapshot watermark is required after the first page",
            ));
        }
        Ok(())
    }

    fn validate_change_request(
        protocol_version: u16,
        pair: &LanguagePairDto,
        cursor: i64,
        limit: u16,
    ) -> Result<(), ServerFnError> {
        validate_pair(pair)?;
        validate_cursor_request(protocol_version, cursor, limit)
    }

    fn validate_cursor_request(
        protocol_version: u16,
        cursor: i64,
        limit: u16,
    ) -> Result<(), ServerFnError> {
        validate_protocol(protocol_version)?;
        validate_limit(limit)?;
        if cursor < 0 {
            return Err(bad_request("Change cursor must not be negative"));
        }
        Ok(())
    }

    fn validate_cursor_watermark(cursor: i64, watermark: i64) -> Result<(), ServerFnError> {
        if cursor > watermark {
            return Err(bad_request(
                "Change cursor is ahead of the server watermark",
            ));
        }
        Ok(())
    }

    fn bad_request(message: &str) -> ServerFnError {
        crate::server::client_error(StatusCode::BAD_REQUEST, message)
    }

    async fn snapshot_watermark(
        db: &PgPool,
        requested: Option<i64>,
        after_translation_id: i32,
    ) -> Result<i64, ServerFnError> {
        let current =
            sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(sequence), 0) FROM corpus_changes")
                .fetch_one(db)
                .await
                .map_err(|error| {
                    crate::server::internal_error("corpus snapshot watermark", &error)
                })?;
        match requested {
            Some(value) if (0..=current).contains(&value) => Ok(value),
            Some(_) => Err(bad_request("Snapshot watermark is invalid")),
            None if after_translation_id == 0 => Ok(current),
            None => Err(bad_request(
                "Snapshot watermark is required after the first page",
            )),
        }
    }

    async fn load_snapshot_rows(
        db: &PgPool,
        request: &CorpusSnapshotRequestDto,
    ) -> Result<Vec<CorpusRow>, ServerFnError> {
        sqlx::query_as::<_, CorpusRow>(
            "SELECT translation.id AS translation_id, translation.from_phrase,
                    translation.to_phrase, translation.frequency,
                    EXISTS (
                        SELECT 1 FROM phrase_translations
                        WHERE translation_id = translation.id
                    ) AS is_phrase
             FROM translations AS translation
             JOIN languages AS native ON native.id = translation.from_language_id
             JOIN languages AS target_language ON target_language.id = translation.to_language_id
             WHERE native.code = $1 AND target_language.code = $2 AND translation.id > $3
             ORDER BY translation.id
             LIMIT $4",
        )
        .bind(&request.pair.native_lang)
        .bind(&request.pair.foreign_lang)
        .bind(request.after_translation_id)
        .bind(i64::from(request.limit) + 1)
        .fetch_all(db)
        .await
        .map_err(|error| crate::server::internal_error("corpus snapshot page", &error))
    }

    fn corpus_translation(row: CorpusRow) -> CorpusTranslationDto {
        CorpusTranslationDto {
            translation_id: row.translation_id,
            from_phrase: row.from_phrase,
            to_phrase: row.to_phrase,
            frequency: row.frequency,
            is_phrase: row.is_phrase,
        }
    }

    async fn corpus_change_watermark(
        db: &PgPool,
        pair: &LanguagePairDto,
    ) -> Result<i64, ServerFnError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(sequence), 0)
             FROM corpus_changes
             WHERE from_language_code = $1 AND to_language_code = $2",
        )
        .bind(&pair.native_lang)
        .bind(&pair.foreign_lang)
        .fetch_one(db)
        .await
        .map_err(|error| crate::server::internal_error("corpus change watermark", &error))
    }

    async fn load_corpus_changes(
        db: &PgPool,
        request: &CorpusChangeRequestDto,
        watermark: i64,
    ) -> Result<Vec<CorpusChangeRow>, ServerFnError> {
        sqlx::query_as::<_, CorpusChangeRow>(
            "SELECT sequence, translation_id, from_phrase, to_phrase, frequency,
                    is_phrase, operation::text, changed_at
             FROM corpus_changes
             WHERE from_language_code = $1 AND to_language_code = $2
               AND sequence > $3 AND sequence <= $4
             ORDER BY sequence
             LIMIT $5",
        )
        .bind(&request.pair.native_lang)
        .bind(&request.pair.foreign_lang)
        .bind(request.cursor)
        .bind(watermark)
        .bind(i64::from(request.limit) + 1)
        .fetch_all(db)
        .await
        .map_err(|error| crate::server::internal_error("corpus change page", &error))
    }

    fn corpus_change(row: CorpusChangeRow) -> Result<CorpusChangeDto, ServerFnError> {
        let (kind, translation) = match row.operation.as_str() {
            "D" => (CorpusChangeKindDto::Delete, None),
            "U" => (
                CorpusChangeKindDto::Upsert,
                Some(CorpusTranslationDto {
                    translation_id: row.translation_id,
                    from_phrase: required(row.from_phrase, "from phrase")?,
                    to_phrase: required(row.to_phrase, "to phrase")?,
                    frequency: required(row.frequency, "frequency")?,
                    is_phrase: required(row.is_phrase, "phrase membership")?,
                }),
            ),
            _ => return Err(invalid_ledger("corpus change operation")),
        };
        Ok(CorpusChangeDto {
            sequence: row.sequence,
            translation_id: row.translation_id,
            kind,
            translation,
            changed_at: row.changed_at,
        })
    }

    async fn card_change_watermark(db: &PgPool, user_id: i32) -> Result<i64, ServerFnError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(sequence), 0) FROM card_changes WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_one(db)
        .await
        .map_err(|error| crate::server::internal_error("card change watermark", &error))
    }

    async fn load_card_changes(
        db: &PgPool,
        user_id: i32,
        request: &CardChangeRequestDto,
        watermark: i64,
    ) -> Result<Vec<CardChangeRow>, ServerFnError> {
        sqlx::query_as::<_, CardChangeRow>(
            "WITH latest AS (
                 SELECT DISTINCT ON (translation_id) translation_id, sequence
                 FROM card_changes
                 WHERE user_id = $1 AND sequence > $2 AND sequence <= $3
                 ORDER BY translation_id, sequence DESC
             )
             SELECT latest.sequence, latest.translation_id, card.stability, card.difficulty,
                    card.elapsed_days, card.scheduled_days, card.reps, card.lapses,
                    card.state, card.last_review, card.due
             FROM latest
             LEFT JOIN cards AS card
               ON card.user_id = $1 AND card.translation_id = latest.translation_id
             ORDER BY latest.sequence
             LIMIT $4",
        )
        .bind(user_id)
        .bind(request.cursor)
        .bind(watermark)
        .bind(i64::from(request.limit) + 1)
        .fetch_all(db)
        .await
        .map_err(|error| crate::server::internal_error("card change page", &error))
    }

    fn card_change(row: CardChangeRow) -> Result<CardChangeDto, ServerFnError> {
        let Some(stability) = row.stability else {
            return Ok(CardChangeDto::Delete {
                sequence: row.sequence,
                translation_id: row.translation_id,
            });
        };
        Ok(CardChangeDto::Upsert {
            sequence: row.sequence,
            card: CardSnapshotDto {
                translation_id: row.translation_id,
                stability: f64::from(stability),
                difficulty: f64::from(required(row.difficulty, "difficulty")?),
                elapsed_days: i64::from(required(row.elapsed_days, "elapsed days")?),
                scheduled_days: i64::from(required(row.scheduled_days, "scheduled days")?),
                reps: required(row.reps, "repetitions")?,
                lapses: required(row.lapses, "lapses")?,
                state: card_status(required(row.state, "state")?)?,
                last_review: row.last_review,
                due: required(row.due, "due date")?,
                server_cursor: row.sequence,
            },
        })
    }

    fn card_status(value: i16) -> Result<CardStatusDto, ServerFnError> {
        match value {
            0 => Ok(CardStatusDto::New),
            1 => Ok(CardStatusDto::Learning),
            2 => Ok(CardStatusDto::Review),
            3 => Ok(CardStatusDto::Relearning),
            _ => Err(invalid_ledger("card state")),
        }
    }

    fn required<T>(value: Option<T>, field: &str) -> Result<T, ServerFnError> {
        value.ok_or_else(|| invalid_ledger(field))
    }

    fn invalid_ledger(field: &str) -> ServerFnError {
        crate::server::internal_error("mobile change ledger decoding", &field)
    }

    fn trim_page<T>(rows: &mut Vec<T>, limit: u16) -> bool {
        let limit = usize::from(limit);
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        has_more
    }
}
