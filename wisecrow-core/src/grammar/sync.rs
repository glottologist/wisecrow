//! Reading the grammar change feeds safely.
//!
//! A feed row carries the identifier of the transaction that wrote it, and a
//! reader serves only rows written by transactions that finished before every
//! transaction still in flight. That is what makes the sequence number safe to
//! use as a cursor: nothing can appear beneath a number already served, so a
//! client that stores the highest number it has seen never needs to look back.
//!
//! The comparison is against `pg_snapshot_xmin(pg_current_snapshot())`, the
//! lowest transaction identifier still running. A row written by a transaction
//! at or above that mark is withheld until the transactions below it finish.

use sqlx::PgPool;

use crate::errors::WisecrowError;

/// Whether a change wrote a row or removed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeOperation {
    Upsert,
    Delete,
}

impl ChangeOperation {
    /// The feeds store one character, which is what the check constraint
    /// allows; anything else means the schema and this code have diverged.
    fn parse(raw: &str) -> Result<Self, WisecrowError> {
        match raw {
            "U" => Ok(Self::Upsert),
            "D" => Ok(Self::Delete),
            other => Err(WisecrowError::SyncError(format!(
                "unknown change operation {other}"
            ))),
        }
    }
}

/// One learner's mastery moving for one grammar point.
#[derive(Debug, Clone)]
pub struct GrammarChange {
    pub sequence: i64,
    pub rule_id: i32,
    pub operation: ChangeOperation,
}

/// One item entering, changing status in, or leaving the bank.
#[derive(Debug, Clone)]
pub struct QuizItemChange {
    pub sequence: i64,
    pub item_id: i32,
    pub language_code: String,
    pub operation: ChangeOperation,
}

/// A page of changes and where to resume.
///
/// `next_cursor` never runs ahead of what the page served, so an empty page
/// leaves the caller exactly where it was rather than skipping the rows that
/// are still waiting on an open transaction.
#[derive(Debug, Clone)]
pub struct ChangePage<T> {
    pub changes: Vec<T>,
    pub next_cursor: i64,
    pub has_more: bool,
}

#[derive(sqlx::FromRow)]
struct GrammarChangeRow {
    sequence: i64,
    rule_id: i32,
    operation: String,
}

#[derive(sqlx::FromRow)]
struct QuizItemChangeRow {
    sequence: i64,
    item_id: i32,
    language_code: String,
    operation: String,
}

/// Rows written by a transaction that every current snapshot can already see.
const VISIBLE: &str = "xact_id < pg_snapshot_xmin(pg_current_snapshot())::TEXT::BIGINT";

/// One row beyond the limit is fetched so the page can say whether more is
/// waiting without a second count.
fn probe_limit(limit: i64) -> i64 {
    limit.saturating_add(1)
}

fn page_from<R, T>(
    mut rows: Vec<R>,
    limit: i64,
    cursor: i64,
    convert: impl Fn(R) -> Result<T, WisecrowError>,
    sequence_of: impl Fn(&T) -> i64,
) -> Result<ChangePage<T>, WisecrowError> {
    let has_more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
    if has_more {
        rows.pop();
    }
    let changes = rows
        .into_iter()
        .map(convert)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = changes.last().map_or(cursor, &sequence_of);
    Ok(ChangePage {
        changes,
        next_cursor,
        has_more,
    })
}

/// The mastery changes one learner may safely be told about.
///
/// # Errors
///
/// Returns a storage error if the query fails, or a sync error if the feed
/// holds an operation this code does not know.
pub async fn visible_grammar_changes(
    pool: &PgPool,
    user_id: i32,
    cursor: i64,
    limit: i64,
) -> Result<ChangePage<GrammarChange>, WisecrowError> {
    let rows: Vec<GrammarChangeRow> = sqlx::query_as(&format!(
        "SELECT sequence, rule_id, operation
         FROM grammar_changes
         WHERE user_id = $1 AND sequence > $2 AND {VISIBLE}
         ORDER BY sequence
         LIMIT $3"
    ))
    .bind(user_id)
    .bind(cursor)
    .bind(probe_limit(limit))
    .fetch_all(pool)
    .await?;

    page_from(
        rows,
        limit,
        cursor,
        |row| {
            Ok(GrammarChange {
                sequence: row.sequence,
                rule_id: row.rule_id,
                operation: ChangeOperation::parse(&row.operation)?,
            })
        },
        |change| change.sequence,
    )
}

/// The item-bank changes for one language.
///
/// # Errors
///
/// Returns a storage error if the query fails, or a sync error if the feed
/// holds an operation this code does not know.
pub async fn visible_item_changes(
    pool: &PgPool,
    language_code: &str,
    cursor: i64,
    limit: i64,
) -> Result<ChangePage<QuizItemChange>, WisecrowError> {
    let rows: Vec<QuizItemChangeRow> = sqlx::query_as(&format!(
        "SELECT sequence, item_id, language_code, operation
         FROM quiz_item_changes
         WHERE language_code = $1 AND sequence > $2 AND {VISIBLE}
         ORDER BY sequence
         LIMIT $3"
    ))
    .bind(language_code)
    .bind(cursor)
    .bind(probe_limit(limit))
    .fetch_all(pool)
    .await?;

    page_from(
        rows,
        limit,
        cursor,
        |row| {
            Ok(QuizItemChange {
                sequence: row.sequence,
                item_id: row.item_id,
                language_code: row.language_code,
                operation: ChangeOperation::parse(&row.operation)?,
            })
        },
        |change| change.sequence,
    )
}

/// An item with everything a device needs to pose and mark it offline.
///
/// The answer travels, unlike the served presentation: a device out of contact
/// still has to tell the learner whether they were right. Its verdict is
/// provisional, and the server regrades the attempt when it arrives.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BankItem {
    pub item_id: i32,
    pub revision: i32,
    pub rule_id: i32,
    pub rule_slug: String,
    pub rule_title: String,
    pub rule_explanation: String,
    pub level: String,
    pub language: String,
    pub kind: String,
    pub prompt: String,
    pub hint: Option<String>,
    pub answer: Option<String>,
    pub accepted: serde_json::Value,
    pub options: Option<serde_json::Value>,
    pub correct_option: Option<String>,
}

/// One learner's projection for one point, as a device mirrors it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MasteryState {
    pub rule_id: i32,
    pub rule_slug: String,
    pub stability: f64,
    pub difficulty: f64,
    pub elapsed_days: i32,
    pub scheduled_days: i32,
    pub reps: i32,
    pub lapses: i32,
    pub state: i16,
    pub accuracy: Option<f64>,
    pub attempts: i32,
    pub last_review: Option<chrono::DateTime<chrono::Utc>>,
    pub due: chrono::DateTime<chrono::Utc>,
}

/// The active items among those named.
///
/// An item that has been retired or rejected is simply absent, which is how a
/// device learns to drop it: the feed reports that the row changed, and the
/// hydration says there is no longer anything to hold.
///
/// # Errors
///
/// Returns a storage error if the query fails.
pub async fn bank_items(pool: &PgPool, item_ids: &[i32]) -> Result<Vec<BankItem>, WisecrowError> {
    if item_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as(
        "SELECT qi.id AS item_id, qi.revision, qi.rule_id, gr.slug AS rule_slug,
                gr.title AS rule_title, gr.explanation AS rule_explanation,
                cl.code AS level, l.code AS language, qi.kind, qi.prompt, qi.hint,
                qi.answer, qi.accepted, qi.options, qi.correct_option
         FROM quiz_items qi
         JOIN grammar_rules gr ON gr.id = qi.rule_id
         JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
         JOIN languages l ON l.id = gr.language_id
         WHERE qi.id = ANY($1) AND qi.status = 'active'",
    )
    .bind(item_ids)
    .fetch_all(pool)
    .await?)
}

/// The learner's mastery for the points named.
///
/// # Errors
///
/// Returns a storage error if the query fails.
pub async fn mastery_states(
    pool: &PgPool,
    user_id: i32,
    rule_ids: &[i32],
) -> Result<Vec<MasteryState>, WisecrowError> {
    if rule_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(sqlx::query_as(
        "SELECT gm.rule_id, gr.slug AS rule_slug, gm.stability, gm.difficulty,
                gm.elapsed_days, gm.scheduled_days, gm.reps, gm.lapses, gm.state,
                gm.accuracy, gm.attempts, gm.last_review, gm.due
         FROM grammar_mastery gm
         JOIN grammar_rules gr ON gr.id = gm.rule_id
         WHERE gm.user_id = $1 AND gm.rule_id = ANY($2)",
    )
    .bind(user_id)
    .bind(rule_ids)
    .fetch_all(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_operation_is_refused_rather_than_guessed() {
        assert_eq!(
            ChangeOperation::parse("U").expect("upsert"),
            ChangeOperation::Upsert
        );
        assert_eq!(
            ChangeOperation::parse("D").expect("delete"),
            ChangeOperation::Delete
        );
        assert!(ChangeOperation::parse("X").is_err());
    }

    #[test]
    fn an_empty_page_leaves_the_cursor_alone() {
        let page = page_from::<GrammarChangeRow, GrammarChange>(
            Vec::new(),
            10,
            42,
            |_| unreachable!("no rows to convert"),
            |change| change.sequence,
        )
        .expect("an empty page cannot fail");
        assert_eq!(page.next_cursor, 42);
        assert!(!page.has_more);
    }
}
