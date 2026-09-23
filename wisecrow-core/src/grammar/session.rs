//! The grammar session: what was served, what came back, and when it ended.
//!
//! A session is a first-class entity because the rating rule depends on it.
//! Within one session the first item served for a point opens an interaction,
//! and every submission against it belongs to that interaction; without a
//! session identifier "the first attempt" cannot be told from a retry.
//!
//! Every submission is regraded here against the stored item revision rather
//! than trusting the client's verdict. That is a correctness measure before it
//! is an anti-tampering one: the two clients and the server must agree, and
//! the stored row is the only copy all three can see.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use wisecrow_learning::grading::{grade, Submission, Verdict};

use crate::errors::WisecrowError;
use crate::grammar::items::{StoredItem, ITEM_COLUMNS};
use crate::grammar::mastery::{AttemptRecord, AttemptSource, MasteryRepository};
use crate::grammar::selection::{record_exposure, select_practice, PracticeItem};

/// Interactions a practice session offers by default.
///
/// Comparable to the vocabulary deck default, and short enough to finish.
pub const SESSION_LENGTH: i64 = 12;

/// Why a session exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Practice,
    Placement,
}

impl SessionKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Practice => "practice",
            Self::Placement => "placement",
        }
    }
}

/// A session and the items it opened with.
#[derive(Debug, Clone)]
pub struct GrammarSession {
    pub id: Uuid,
    pub items: Vec<PracticeItem>,
}

/// What the transport knows about a submission but the learner does not.
///
/// `occurred_at` comes from the client because a mobile answer may be days old
/// by the time it is uploaded, and recording it as "now" would corrupt both
/// the schedule and the interaction it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmissionContext {
    pub occurred_at: DateTime<Utc>,
    pub device_id: Option<Uuid>,
    pub source: AttemptSource,
}

/// Creates, feeds and closes grammar sessions.
pub struct GrammarSessionManager;

impl GrammarSessionManager {
    /// Opens a session over the points most worth practising.
    ///
    /// Returns `None` when the language has no promoted items at all, which is
    /// an ordinary state for a freshly ingested language rather than a fault:
    /// the syllabus exists, the bank does not yet.
    ///
    /// # Errors
    ///
    /// Returns an error when the language or level is unknown, or a write
    /// fails.
    pub async fn create(
        pool: &PgPool,
        user_id: i32,
        lang_code: &str,
        level_code: Option<&str>,
        kind: SessionKind,
        limit: i64,
    ) -> Result<Option<GrammarSession>, WisecrowError> {
        let items = select_practice(pool, user_id, lang_code, level_code, limit).await?;
        if items.is_empty() {
            return Ok(None);
        }

        let language_id = language_id(pool, lang_code).await?;
        let level_id = match level_code {
            Some(code) => Some(level_id(pool, code).await?),
            None => None,
        };

        let mut transaction = pool.begin().await?;
        let id = Self::open(&mut transaction, user_id, language_id, level_id, kind).await?;
        transaction.commit().await?;

        for item in &items {
            record_exposure(pool, user_id, item.item_id).await?;
        }

        Ok(Some(GrammarSession { id, items }))
    }

    /// Writes a session row and returns its identifier.
    ///
    /// Placement opens a session of its own rather than through [`Self::create`],
    /// because it draws its items level by level as the learner climbs.
    ///
    /// # Errors
    ///
    /// Returns an error when the write fails.
    pub async fn open(
        transaction: &mut Transaction<'_, Postgres>,
        user_id: i32,
        language_id: i32,
        level_id: Option<i32>,
        kind: SessionKind,
    ) -> Result<Uuid, WisecrowError> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO grammar_sessions (id, user_id, language_id, cefr_level_id, kind)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(id)
        .bind(user_id)
        .bind(language_id)
        .bind(level_id)
        .bind(kind.as_str())
        .execute(&mut **transaction)
        .await?;
        Ok(id)
    }

    /// Adopts a session a device opened while it could not reach the server.
    ///
    /// The identifier is the device's, not the server's, because the answers
    /// were grouped under it offline and the interaction rule depends on that
    /// grouping surviving the journey. Adopting one already present changes
    /// nothing, so a batch retried after a lost acknowledgement is harmless.
    ///
    /// # Errors
    ///
    /// Returns a conflict when the identifier already belongs to another
    /// learner, or a storage error when the write fails.
    pub async fn adopt(
        transaction: &mut Transaction<'_, Postgres>,
        user_id: i32,
        session_id: Uuid,
        language_id: i32,
        level_id: Option<i32>,
        kind: SessionKind,
    ) -> Result<(), WisecrowError> {
        let owner: Option<i32> =
            sqlx::query_scalar("SELECT user_id FROM grammar_sessions WHERE id = $1")
                .bind(session_id)
                .fetch_optional(&mut **transaction)
                .await?;

        match owner {
            Some(existing) if existing == user_id => Ok(()),
            Some(_) => Err(WisecrowError::Conflict(format!(
                "Grammar session {session_id} belongs to another learner"
            ))),
            None => {
                sqlx::query(
                    "INSERT INTO grammar_sessions (id, user_id, language_id, cefr_level_id, kind)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (id) DO NOTHING",
                )
                .bind(session_id)
                .bind(user_id)
                .bind(language_id)
                .bind(level_id)
                .bind(kind.as_str())
                .execute(&mut **transaction)
                .await?;
                Ok(())
            }
        }
    }

    /// Regrades a submission and records it against its session.
    ///
    /// # Errors
    ///
    /// Returns an error when the session is unknown, belongs to another
    /// learner or has been completed; when the item does not match the
    /// revision claimed; or when a write fails.
    pub async fn submit(
        pool: &PgPool,
        user_id: i32,
        submission: &Submission,
        context: SubmissionContext,
    ) -> Result<Verdict, WisecrowError> {
        let mut transaction = pool.begin().await?;
        let verdict =
            Self::submit_in_transaction(&mut transaction, user_id, submission, context).await?;
        transaction.commit().await?;
        Ok(verdict)
    }

    /// Regrades and records inside a transaction the caller controls.
    ///
    /// # Errors
    ///
    /// As [`Self::submit`].
    pub async fn submit_in_transaction(
        transaction: &mut Transaction<'_, Postgres>,
        user_id: i32,
        submission: &Submission,
        context: SubmissionContext,
    ) -> Result<Verdict, WisecrowError> {
        ensure_open(transaction, user_id, submission.session_id).await?;
        let item = load_item(transaction, submission.item_id).await?;
        if item.revision != submission.revision {
            return Err(WisecrowError::Conflict(format!(
                "Quiz item {} is at revision {}, not {}",
                item.id, item.revision, submission.revision
            )));
        }

        let verdict = grade(&item.gradable()?, submission);
        let ordinal = i16::try_from(submission.ordinal).map_err(|_| {
            WisecrowError::InvalidInput("Submission ordinal exceeds an interaction".into())
        })?;

        MasteryRepository::record_attempt_in_transaction(
            transaction,
            user_id,
            &AttemptRecord {
                event_id: submission.event_id,
                session_id: submission.session_id,
                device_id: context.device_id,
                rule_id: item.rule_id,
                item_id: item.id,
                item_revision: item.revision,
                ordinal,
                correct: verdict.correct,
                hint_shown: submission.hint_shown,
                occurred_at: context.occurred_at,
                source: context.source,
            },
        )
        .await?;

        Ok(verdict)
    }

    /// Closes a session. Closing one already closed changes nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when the session is unknown or belongs to another
    /// learner, or when the write fails.
    pub async fn complete(
        pool: &PgPool,
        user_id: i32,
        session_id: Uuid,
    ) -> Result<(), WisecrowError> {
        let updated = sqlx::query(
            "UPDATE grammar_sessions
             SET completed_at = COALESCE(completed_at, CURRENT_TIMESTAMP)
             WHERE id = $1 AND user_id = $2",
        )
        .bind(session_id)
        .bind(user_id)
        .execute(pool)
        .await?;

        if updated.rows_affected() == 0 {
            return Err(WisecrowError::InvalidInput(format!(
                "No grammar session {session_id} for this learner"
            )));
        }
        Ok(())
    }
}

async fn ensure_open(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: i32,
    session_id: Uuid,
) -> Result<(), WisecrowError> {
    let completed: Option<Option<DateTime<Utc>>> = sqlx::query_scalar(
        "SELECT completed_at FROM grammar_sessions WHERE id = $1 AND user_id = $2",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(&mut **transaction)
    .await?;

    match completed {
        None => Err(WisecrowError::InvalidInput(format!(
            "No grammar session {session_id} for this learner"
        ))),
        Some(Some(_)) => Err(WisecrowError::Conflict(format!(
            "Grammar session {session_id} has been completed"
        ))),
        Some(None) => Ok(()),
    }
}

async fn load_item(
    transaction: &mut Transaction<'_, Postgres>,
    item_id: i32,
) -> Result<StoredItem, WisecrowError> {
    sqlx::query_as::<_, StoredItem>(&format!(
        "SELECT {ITEM_COLUMNS} FROM quiz_items WHERE id = $1"
    ))
    .bind(item_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| WisecrowError::InvalidInput(format!("No quiz item with id {item_id}")))
}

/// Resolves the language and level a session ran at.
///
/// An uploaded answer names them by code rather than by identifier, because a
/// device holds the syllabus by code and has no business knowing the server's
/// primary keys.
///
/// # Errors
///
/// Returns an error when the language or level is unknown.
pub async fn session_identity(
    pool: &PgPool,
    lang_code: &str,
    level_code: Option<&str>,
) -> Result<(i32, Option<i32>), WisecrowError> {
    let language = language_id(pool, lang_code).await?;
    let level = match level_code {
        Some(code) => Some(level_id(pool, code).await?),
        None => None,
    };
    Ok((language, level))
}

pub(crate) async fn language_id(pool: &PgPool, lang_code: &str) -> Result<i32, WisecrowError> {
    sqlx::query_scalar("SELECT id FROM languages WHERE code = $1")
        .bind(lang_code)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| WisecrowError::UnsupportedLanguage(lang_code.to_owned()))
}

pub(crate) async fn level_id(pool: &PgPool, level_code: &str) -> Result<i32, WisecrowError> {
    sqlx::query_scalar("SELECT id FROM cefr_levels WHERE code = $1")
        .bind(level_code)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| WisecrowError::InvalidInput(format!("Unknown CEFR level {level_code}")))
}
