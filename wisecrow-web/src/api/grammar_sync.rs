//! Version-2 grammar endpoints a device syncs against.
//!
//! The bank and the mastery feed are read through the visible-cursor rule in
//! `wisecrow::grammar::sync`, so a device that stores the cursor it was given
//! can never step over a change committed late. Uploaded answers are regraded
//! here against the stored revision: the device's own verdict, shown offline
//! from the cached answer, is provisional and never recorded.

use dioxus::prelude::*;
use wisecrow_dto::{
    GrammarAttemptBatchRequestDto, GrammarAttemptBatchResponseDto, GrammarBankChangePageDto,
    GrammarBankChangeRequestDto, GrammarMasteryChangePageDto, GrammarMasteryChangeRequestDto,
};

/// Largest page either feed will serve.
#[cfg(feature = "server")]
const MAX_PAGE: u16 = 500;

/// Most answers one upload may carry.
#[cfg(feature = "server")]
const MAX_BATCH: usize = 500;

/// The item-bank changes for one language since a cursor.
///
/// # Errors
///
/// Returns protocol, validation, authentication or sanitized storage errors.
#[post("/api/mobile/v2/grammar/bank/changes")]
pub async fn mobile_grammar_bank_changes(
    request: GrammarBankChangeRequestDto,
) -> Result<GrammarBankChangePageDto, ServerFnError> {
    implementation::bank_changes(request).await
}

/// The learner's mastery changes since a cursor.
///
/// # Errors
///
/// Returns protocol, validation, authentication or sanitized storage errors.
#[post("/api/mobile/v2/grammar/mastery/changes")]
pub async fn mobile_grammar_mastery_changes(
    request: GrammarMasteryChangeRequestDto,
) -> Result<GrammarMasteryChangePageDto, ServerFnError> {
    implementation::mastery_changes(request).await
}

/// Records answers a device took while it was offline.
///
/// # Errors
///
/// Returns protocol, validation, authentication or sanitized storage errors.
#[post("/api/mobile/v2/grammar/attempts/upload")]
pub async fn mobile_grammar_attempt_upload(
    request: GrammarAttemptBatchRequestDto,
) -> Result<GrammarAttemptBatchResponseDto, ServerFnError> {
    implementation::upload_attempts(request).await
}

#[cfg(feature = "server")]
mod implementation {
    use super::{MAX_BATCH, MAX_PAGE};

    use axum::http::StatusCode;
    use dioxus::prelude::*;
    use sqlx::PgPool;
    use std::collections::HashMap;
    use uuid::Uuid;
    use wisecrow::grammar::mastery::AttemptSource;
    use wisecrow::grammar::session::{GrammarSessionManager, SessionKind, SubmissionContext};
    use wisecrow::grammar::sync::{
        bank_items, mastery_states, visible_grammar_changes, visible_item_changes, ChangeOperation,
    };
    use wisecrow_dto::{
        GrammarAttemptBatchRequestDto, GrammarAttemptBatchResponseDto, GrammarBankChangeDto,
        GrammarBankChangePageDto, GrammarBankChangeRequestDto, GrammarChangeOperationDto,
        GrammarMasteryChangeDto, GrammarMasteryChangePageDto, GrammarMasteryChangeRequestDto,
        OfflineAttemptDto, OfflineAttemptStatusDto, VerdictDto, MOBILE_PROTOCOL_VERSION_V2,
    };
    use wisecrow_learning::grading::{Answer, Submission};

    pub(super) async fn bank_changes(
        request: GrammarBankChangeRequestDto,
    ) -> Result<GrammarBankChangePageDto, ServerFnError> {
        crate::server::auth::current_user().await?;
        validate_protocol(request.protocol_version)?;
        validate_cursor(request.cursor)?;
        validate_limit(request.limit)?;
        crate::server::validate_lang(&request.language)?;

        let db = crate::server::pool()?;
        let page = visible_item_changes(
            db,
            &request.language,
            request.cursor,
            i64::from(request.limit),
        )
        .await
        .map_err(|error| crate::server::internal_error("grammar bank changes", &error))?;

        let wanted: Vec<i32> = page
            .changes
            .iter()
            .filter(|change| change.operation == ChangeOperation::Upsert)
            .map(|change| change.item_id)
            .collect();
        let items = bank_items(db, &wanted)
            .await
            .map_err(|error| crate::server::internal_error("grammar bank hydration", &error))?;
        let mut held: HashMap<i32, _> =
            items.into_iter().map(|item| (item.item_id, item)).collect();

        let changes = page
            .changes
            .iter()
            .map(|change| GrammarBankChangeDto {
                sequence: change.sequence,
                item_id: change.item_id,
                operation: operation(change.operation),
                item: held
                    .remove(&change.item_id)
                    .map(|item| wisecrow::dto_convert::offline_grammar_item(&item)),
            })
            .collect();

        Ok(GrammarBankChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION_V2,
            language: request.language,
            changes,
            next_cursor: page.next_cursor,
            has_more: page.has_more,
        })
    }

    pub(super) async fn mastery_changes(
        request: GrammarMasteryChangeRequestDto,
    ) -> Result<GrammarMasteryChangePageDto, ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        validate_protocol(request.protocol_version)?;
        validate_cursor(request.cursor)?;
        validate_limit(request.limit)?;

        let db = crate::server::pool()?;
        let page = visible_grammar_changes(db, user.id, request.cursor, i64::from(request.limit))
            .await
            .map_err(|error| crate::server::internal_error("grammar mastery changes", &error))?;

        let wanted: Vec<i32> = page
            .changes
            .iter()
            .filter(|change| change.operation == ChangeOperation::Upsert)
            .map(|change| change.rule_id)
            .collect();
        let states = mastery_states(db, user.id, &wanted)
            .await
            .map_err(|error| crate::server::internal_error("grammar mastery hydration", &error))?;
        let mut held: HashMap<i32, _> = states
            .into_iter()
            .map(|state| (state.rule_id, state))
            .collect();

        let changes = page
            .changes
            .iter()
            .map(|change| GrammarMasteryChangeDto {
                sequence: change.sequence,
                rule_id: change.rule_id,
                operation: operation(change.operation),
                mastery: held
                    .remove(&change.rule_id)
                    .map(|state| wisecrow::dto_convert::grammar_mastery_state(&state)),
            })
            .collect();

        Ok(GrammarMasteryChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION_V2,
            changes,
            next_cursor: page.next_cursor,
            has_more: page.has_more,
        })
    }

    pub(super) async fn upload_attempts(
        request: GrammarAttemptBatchRequestDto,
    ) -> Result<GrammarAttemptBatchResponseDto, ServerFnError> {
        let user = crate::server::auth::current_user().await?;
        validate_protocol(request.protocol_version)?;
        if request.attempts.len() > MAX_BATCH {
            return Err(bad_request(
                "Grammar batches may contain at most 500 answers",
            ));
        }
        let db = crate::server::pool()?;
        ensure_active_device(db, user.id, request.device_id).await?;

        let mut results = Vec::with_capacity(request.attempts.len());
        for attempt in &request.attempts {
            results.push(apply_attempt(db, user.id, request.device_id, attempt).await?);
        }

        Ok(GrammarAttemptBatchResponseDto {
            protocol_version: MOBILE_PROTOCOL_VERSION_V2,
            results,
        })
    }

    /// Records one answer, or reports why it was not recorded.
    ///
    /// A batch is applied answer by answer rather than all or nothing: one
    /// answer against a retired item must not cost a device the other
    /// eleven, which it would have no way to resend without resending them
    /// all.
    async fn apply_attempt(
        db: &PgPool,
        user_id: i32,
        device_id: Uuid,
        attempt: &OfflineAttemptDto,
    ) -> Result<OfflineAttemptStatusDto, ServerFnError> {
        if let Some(correct) = stored_verdict(db, user_id, attempt.event_id).await? {
            return Ok(OfflineAttemptStatusDto::Duplicate(VerdictDto {
                event_id: attempt.event_id,
                correct,
            }));
        }

        match record_attempt(db, user_id, device_id, attempt).await {
            Ok(correct) => Ok(OfflineAttemptStatusDto::Accepted(VerdictDto {
                event_id: attempt.event_id,
                correct,
            })),
            Err(reason) => Ok(OfflineAttemptStatusDto::Rejected {
                event_id: attempt.event_id,
                reason,
            }),
        }
    }

    /// The error is returned as a string because it is reported per answer
    /// rather than failing the request, and a device can only log it.
    async fn record_attempt(
        db: &PgPool,
        user_id: i32,
        device_id: Uuid,
        attempt: &OfflineAttemptDto,
    ) -> Result<bool, String> {
        crate::server::validate_lang(&attempt.language)
            .map_err(|_| "Unknown language".to_owned())?;

        // Resolved before the transaction opens: a query issued on the pool
        // while a transaction is held takes a second connection for no reason,
        // and under a small pool that is how a request comes to wait on itself.
        let ids = wisecrow::grammar::session::session_identity(
            db,
            &attempt.language,
            attempt.level.as_deref(),
        )
        .await
        .map_err(|error| error.to_string())?;

        let mut transaction = db.begin().await.map_err(|error| error.to_string())?;
        GrammarSessionManager::adopt(
            &mut transaction,
            user_id,
            attempt.session_id,
            ids.0,
            ids.1,
            SessionKind::Practice,
        )
        .await
        .map_err(|error| error.to_string())?;

        let submission = Submission {
            item_id: attempt.item_id,
            revision: attempt.revision,
            session_id: attempt.session_id,
            event_id: attempt.event_id,
            answer: if attempt.chose_option {
                Answer::Option(attempt.answer.clone()) // clone: the submission owns its answer
            } else {
                Answer::Text(attempt.answer.clone()) // clone: the submission owns its answer
            },
            hint_shown: attempt.hint_shown,
            ordinal: attempt.ordinal,
        };
        let context = SubmissionContext {
            occurred_at: attempt.occurred_at,
            device_id: Some(device_id),
            source: AttemptSource::Mobile,
        };

        let verdict = GrammarSessionManager::submit_in_transaction(
            &mut transaction,
            user_id,
            &submission,
            context,
        )
        .await
        .map_err(|error| error.to_string())?;
        transaction
            .commit()
            .await
            .map_err(|error| error.to_string())?;
        Ok(verdict.correct)
    }

    async fn stored_verdict(
        db: &PgPool,
        user_id: i32,
        event_id: Uuid,
    ) -> Result<Option<bool>, ServerFnError> {
        sqlx::query_scalar(
            "SELECT correct FROM grammar_attempts WHERE user_id = $1 AND event_id = $2",
        )
        .bind(user_id)
        .bind(event_id)
        .fetch_optional(db)
        .await
        .map_err(|error| crate::server::internal_error("grammar attempt lookup", &error))
    }

    const fn operation(operation: ChangeOperation) -> GrammarChangeOperationDto {
        match operation {
            ChangeOperation::Upsert => GrammarChangeOperationDto::Upsert,
            ChangeOperation::Delete => GrammarChangeOperationDto::Delete,
        }
    }

    fn bad_request(message: &str) -> ServerFnError {
        crate::server::client_error(StatusCode::BAD_REQUEST, message)
    }

    fn validate_protocol(protocol_version: u16) -> Result<(), ServerFnError> {
        if protocol_version != MOBILE_PROTOCOL_VERSION_V2 {
            return Err(crate::server::client_error(
                StatusCode::CONFLICT,
                "Unsupported mobile protocol version",
            ));
        }
        Ok(())
    }

    fn validate_cursor(cursor: i64) -> Result<(), ServerFnError> {
        if cursor < 0 {
            return Err(bad_request("Cursor must not be negative"));
        }
        Ok(())
    }

    fn validate_limit(limit: u16) -> Result<(), ServerFnError> {
        if !(1..=MAX_PAGE).contains(&limit) {
            return Err(bad_request("Page limit must be between 1 and 500"));
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
}
