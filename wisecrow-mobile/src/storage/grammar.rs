use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, Sqlite, Transaction};
use uuid::Uuid;
use wisecrow_dto::{
    GrammarAttemptBatchResponseDto, GrammarBankChangePageDto, GrammarChangeOperationDto,
    GrammarMasteryChangePageDto, GrammarOptionDto, OfflineAttemptStatusDto,
};

use super::{
    sqlite::{active_scope, StoreScope},
    SqliteStore,
};
use wisecrow_learning::grading::grade;
use wisecrow_learning::mastery::fold_accuracy;

use crate::application::{
    GrammarCursors, GrammarRepository, LocalGrammarItem, LocalGrammarMastery, LocalGrammarRule,
    MobileError, QueuedAttempt,
};

/// Most answers one drain will hand over.
const MAX_OUTBOX_PAGE: u16 = 500;

#[derive(FromRow)]
struct ItemRow {
    item_id: i32,
    rule_id: i32,
    revision: i32,
    language: String,
    prompt: String,
    hint: Option<String>,
    options: String,
    answer: Option<String>,
    accepted: String,
    correct_option: Option<String>,
}

#[derive(FromRow)]
struct RuleRow {
    rule_id: i32,
    language: String,
    slug: String,
    title: String,
    explanation: String,
    level: String,
}

#[derive(FromRow)]
struct MasteryRow {
    rule_id: i32,
    stability: f64,
    difficulty: f64,
    elapsed_days: i64,
    scheduled_days: i64,
    reps: i64,
    lapses: i64,
    state: i64,
    accuracy: Option<f64>,
    attempts: i64,
    last_review: Option<DateTime<Utc>>,
    due: DateTime<Utc>,
}

#[derive(FromRow)]
struct OutboxRow {
    event_id: Uuid,
    session_id: Uuid,
    item_id: i32,
    revision: i32,
    answer: String,
    chose_option: bool,
    hint_shown: bool,
    ordinal: i64,
    occurred_at: DateTime<Utc>,
    language: String,
    level: Option<String>,
}

#[async_trait]
impl GrammarRepository for SqliteStore {
    async fn apply_bank_page(&self, page: &GrammarBankChangePageDto) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;

        for change in &page.changes {
            match (&change.item, change.operation) {
                // An item the server withheld is one the device must stop
                // holding, whether it was deleted or merely unpromoted.
                (None, _) | (_, GrammarChangeOperationDto::Delete) => {
                    sqlx::query(
                        "DELETE FROM grammar_items
                         WHERE profile_id = ?1 AND user_id = ?2 AND item_id = ?3",
                    )
                    .bind(scope.profile_id)
                    .bind(scope.user_id)
                    .bind(change.item_id)
                    .execute(&mut *transaction)
                    .await?;
                }
                (Some(item), GrammarChangeOperationDto::Upsert) => {
                    sqlx::query(
                        "INSERT INTO grammar_rules (
                             profile_id, user_id, rule_id, language, slug, title, explanation, level
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                         ON CONFLICT (profile_id, user_id, rule_id) DO UPDATE SET
                             language = excluded.language, slug = excluded.slug,
                             title = excluded.title, explanation = excluded.explanation,
                             level = excluded.level",
                    )
                    .bind(scope.profile_id)
                    .bind(scope.user_id)
                    .bind(item.rule_id)
                    .bind(&item.language)
                    .bind(&item.rule_slug)
                    .bind(&item.rule_title)
                    .bind(&item.rule_explanation)
                    .bind(&item.level)
                    .execute(&mut *transaction)
                    .await?;

                    let options = serde_json::to_string(&item.options)?;
                    let accepted = serde_json::to_string(&item.accepted)?;
                    sqlx::query(
                        "INSERT INTO grammar_items (
                             profile_id, user_id, item_id, rule_id, revision, language, prompt,
                             hint, options, answer, accepted, correct_option
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                         ON CONFLICT (profile_id, user_id, item_id) DO UPDATE SET
                             rule_id = excluded.rule_id, revision = excluded.revision,
                             language = excluded.language, prompt = excluded.prompt,
                             hint = excluded.hint, options = excluded.options,
                             answer = excluded.answer, accepted = excluded.accepted,
                             correct_option = excluded.correct_option",
                    )
                    .bind(scope.profile_id)
                    .bind(scope.user_id)
                    .bind(item.item_id)
                    .bind(item.rule_id)
                    .bind(item.revision)
                    .bind(&item.language)
                    .bind(&item.prompt)
                    .bind(&item.hint)
                    .bind(options)
                    .bind(&item.answer)
                    .bind(accepted)
                    .bind(&item.correct_option)
                    .execute(&mut *transaction)
                    .await?;
                }
            }
        }

        advance_bank_cursor(&mut transaction, scope, &page.language, page.next_cursor).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn apply_mastery_page(
        &self,
        page: &GrammarMasteryChangePageDto,
    ) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;

        // The mastery feed is not per language, so the cursor is advanced for
        // every language the device follows: one feed, one position in it.
        for change in &page.changes {
            match &change.mastery {
                None => {
                    sqlx::query(
                        "DELETE FROM grammar_mastery
                         WHERE profile_id = ?1 AND user_id = ?2 AND rule_id = ?3",
                    )
                    .bind(scope.profile_id)
                    .bind(scope.user_id)
                    .bind(change.rule_id)
                    .execute(&mut *transaction)
                    .await?;
                }
                Some(state) => {
                    sqlx::query(
                        "INSERT INTO grammar_mastery (
                             profile_id, user_id, rule_id, stability, difficulty, elapsed_days,
                             scheduled_days, reps, lapses, state, accuracy, attempts,
                             last_review, due
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                         ON CONFLICT (profile_id, user_id, rule_id) DO UPDATE SET
                             stability = excluded.stability, difficulty = excluded.difficulty,
                             elapsed_days = excluded.elapsed_days,
                             scheduled_days = excluded.scheduled_days, reps = excluded.reps,
                             lapses = excluded.lapses, state = excluded.state,
                             accuracy = excluded.accuracy, attempts = excluded.attempts,
                             last_review = excluded.last_review, due = excluded.due",
                    )
                    .bind(scope.profile_id)
                    .bind(scope.user_id)
                    .bind(state.rule_id)
                    .bind(f64::from(state.stability))
                    .bind(f64::from(state.difficulty))
                    .bind(i64::from(state.elapsed_days))
                    .bind(i64::from(state.scheduled_days))
                    .bind(i64::from(state.reps))
                    .bind(i64::from(state.lapses))
                    .bind(i64::from(state.state))
                    .bind(state.accuracy.map(f64::from))
                    .bind(i64::from(state.attempts))
                    .bind(state.last_review)
                    .bind(state.due)
                    .execute(&mut *transaction)
                    .await?;
                }
            }
        }

        sqlx::query(
            "UPDATE grammar_sync_state SET mastery_cursor = ?3
             WHERE profile_id = ?1 AND user_id = ?2",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(page.next_cursor)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(())
    }

    async fn grammar_cursors(&self, language: &str) -> Result<GrammarCursors, MobileError> {
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        let row: Option<(i64, i64)> = sqlx::query_as(
            "SELECT bank_cursor, mastery_cursor FROM grammar_sync_state
             WHERE profile_id = ?1 AND user_id = ?2 AND language = ?3",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(language)
        .fetch_optional(&mut *transaction)
        .await?;
        transaction.commit().await?;

        Ok(row.map_or(
            GrammarCursors {
                bank: 0,
                mastery: 0,
            },
            |(bank, mastery)| GrammarCursors { bank, mastery },
        ))
    }

    async fn grammar_items(&self, language: &str) -> Result<Vec<LocalGrammarItem>, MobileError> {
        let scope = super::sqlite::active_scope_from_pool(&self.pool).await?;
        let rows: Vec<ItemRow> = sqlx::query_as(
            "SELECT item_id, rule_id, revision, language, prompt, hint, options, answer,
                    accepted, correct_option
             FROM grammar_items
             WHERE profile_id = ?1 AND user_id = ?2 AND language = ?3
             ORDER BY item_id",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(language)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(local_item).collect()
    }

    async fn grammar_rules(&self, language: &str) -> Result<Vec<LocalGrammarRule>, MobileError> {
        let scope = super::sqlite::active_scope_from_pool(&self.pool).await?;
        let rows: Vec<RuleRow> = sqlx::query_as(
            "SELECT rule_id, language, slug, title, explanation, level
             FROM grammar_rules
             WHERE profile_id = ?1 AND user_id = ?2 AND language = ?3
             ORDER BY level, slug",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(language)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| LocalGrammarRule {
                rule_id: row.rule_id,
                language: row.language,
                slug: row.slug,
                title: row.title,
                explanation: row.explanation,
                level: row.level,
            })
            .collect())
    }

    async fn grammar_mastery(
        &self,
        language: &str,
    ) -> Result<Vec<LocalGrammarMastery>, MobileError> {
        let scope = super::sqlite::active_scope_from_pool(&self.pool).await?;
        let rows: Vec<MasteryRow> = sqlx::query_as(
            "SELECT gm.rule_id, gm.stability, gm.difficulty, gm.elapsed_days, gm.scheduled_days,
                    gm.reps, gm.lapses, gm.state, gm.accuracy, gm.attempts, gm.last_review, gm.due
             FROM grammar_mastery gm
             JOIN grammar_rules gr
               ON gr.profile_id = gm.profile_id AND gr.user_id = gm.user_id
              AND gr.rule_id = gm.rule_id
             WHERE gm.profile_id = ?1 AND gm.user_id = ?2 AND gr.language = ?3
             ORDER BY gm.rule_id",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(language)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(local_mastery).collect())
    }

    async fn queue_attempt(&self, attempt: &QueuedAttempt) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        let ordinal = i64::from(attempt.ordinal);
        let held: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM grammar_attempt_outbox
             WHERE profile_id = ?1 AND user_id = ?2 AND event_id = ?3",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(attempt.event_id)
        .fetch_optional(&mut *transaction)
        .await?;

        sqlx::query(
            "INSERT INTO grammar_attempt_outbox (
                 profile_id, user_id, event_id, session_id, item_id, revision, answer,
                 chose_option, hint_shown, ordinal, occurred_at, language, level, queued_at
             ) VALUES (
                 ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 COALESCE(
                     (SELECT MAX(queued_at) + 1 FROM grammar_attempt_outbox
                      WHERE profile_id = ?1 AND user_id = ?2),
                     1
                 )
             )
             ON CONFLICT (profile_id, user_id, event_id) DO NOTHING",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(attempt.event_id)
        .bind(attempt.session_id)
        .bind(attempt.item_id)
        .bind(attempt.revision)
        .bind(&attempt.answer)
        .bind(attempt.chose_option)
        .bind(attempt.hint_shown)
        .bind(ordinal)
        .bind(attempt.occurred_at)
        .bind(&attempt.language)
        .bind(&attempt.level)
        .execute(&mut *transaction)
        .await?;

        if held.is_none() {
            project_local_mastery(&mut transaction, scope, attempt).await?;
        }

        transaction.commit().await?;
        Ok(())
    }

    async fn pending_attempts(&self, limit: u16) -> Result<Vec<QueuedAttempt>, MobileError> {
        let scope = super::sqlite::active_scope_from_pool(&self.pool).await?;
        let rows: Vec<OutboxRow> = sqlx::query_as(
            "SELECT event_id, session_id, item_id, revision, answer, chose_option, hint_shown,
                    ordinal, occurred_at, language, level
             FROM grammar_attempt_outbox
             WHERE profile_id = ?1 AND user_id = ?2
             ORDER BY queued_at
             LIMIT ?3",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(i64::from(limit.min(MAX_OUTBOX_PAGE)))
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(QueuedAttempt {
                    event_id: row.event_id,
                    session_id: row.session_id,
                    item_id: row.item_id,
                    revision: row.revision,
                    answer: row.answer,
                    chose_option: row.chose_option,
                    hint_shown: row.hint_shown,
                    ordinal: u32::try_from(row.ordinal).map_err(|_| {
                        MobileError::InvalidInput(String::from("stored ordinal is out of range"))
                    })?,
                    occurred_at: row.occurred_at,
                    language: row.language,
                    level: row.level,
                })
            })
            .collect()
    }

    async fn apply_attempt_response(
        &self,
        response: &GrammarAttemptBatchResponseDto,
    ) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;

        for result in &response.results {
            // A rejected answer stays queued only if it might yet succeed, and
            // a rejection is never that: the item is gone or the revision has
            // moved, and resending it forever would block every answer behind
            // it. Accepted and duplicate both mean the server holds it.
            let event_id = match result {
                OfflineAttemptStatusDto::Accepted(verdict)
                | OfflineAttemptStatusDto::Duplicate(verdict) => verdict.event_id,
                OfflineAttemptStatusDto::Rejected { event_id, .. } => *event_id,
            };
            sqlx::query(
                "DELETE FROM grammar_attempt_outbox
                 WHERE profile_id = ?1 AND user_id = ?2 AND event_id = ?3",
            )
            .bind(scope.profile_id)
            .bind(scope.user_id)
            .bind(event_id)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(())
    }
}

/// Colours the brainmap from an answer the server has not heard yet.
///
/// The device holds no attempt history — the outbox is drained, not kept — so
/// the running accuracy is folded forward with the reduction the server uses
/// over the whole stream, which [`wisecrow_learning::mastery::fold_accuracy`]
/// keeps in one place. The schedule is deliberately left alone: FSRS is the
/// server's to run, and a device guessing at a due date would only have it
/// overwritten by the next mastery page. What the learner sees change at once
/// is the colour, which is what they answered for.
async fn project_local_mastery(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    attempt: &QueuedAttempt,
) -> Result<(), MobileError> {
    let row: Option<ItemRow> = sqlx::query_as(
        "SELECT item_id, rule_id, revision, language, prompt, hint, options, answer,
                accepted, correct_option
         FROM grammar_items
         WHERE profile_id = ?1 AND user_id = ?2 AND item_id = ?3",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(attempt.item_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some(row) = row else {
        // An answer against an item the device no longer holds still belongs
        // in the outbox; the server will rule on it. There is simply no cell
        // to colour.
        return Ok(());
    };
    let rule_id = row.rule_id;
    let item = local_item(row)?;
    let correct = grade(&item.gradable(), &attempt.submission()).correct;

    let current: Option<(Option<f64>, i64)> = sqlx::query_as(
        "SELECT accuracy, attempts FROM grammar_mastery
         WHERE profile_id = ?1 AND user_id = ?2 AND rule_id = ?3",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(rule_id)
    .fetch_optional(&mut **transaction)
    .await?;

    let (previous, attempts) = current.unwrap_or((None, 0));
    let folded = fold_accuracy(previous, correct);

    sqlx::query(
        "INSERT INTO grammar_mastery (profile_id, user_id, rule_id, accuracy, attempts, due)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (profile_id, user_id, rule_id) DO UPDATE SET
             accuracy = excluded.accuracy, attempts = excluded.attempts",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(rule_id)
    .bind(folded)
    .bind(attempts.saturating_add(1))
    .bind(attempt.occurred_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn local_item(row: ItemRow) -> Result<LocalGrammarItem, MobileError> {
    Ok(LocalGrammarItem {
        item_id: row.item_id,
        rule_id: row.rule_id,
        revision: row.revision,
        language: row.language,
        prompt: row.prompt,
        hint: row.hint,
        options: serde_json::from_str::<Vec<GrammarOptionDto>>(&row.options)?,
        answer: row.answer,
        accepted: serde_json::from_str::<Vec<String>>(&row.accepted)?,
        correct_option: row.correct_option,
    })
}

fn local_mastery(row: MasteryRow) -> LocalGrammarMastery {
    LocalGrammarMastery {
        rule_id: row.rule_id,
        stability: row.stability as f32,
        difficulty: row.difficulty as f32,
        elapsed_days: i32::try_from(row.elapsed_days).unwrap_or(i32::MAX),
        scheduled_days: i32::try_from(row.scheduled_days).unwrap_or(i32::MAX),
        reps: i32::try_from(row.reps).unwrap_or(i32::MAX),
        lapses: i32::try_from(row.lapses).unwrap_or(i32::MAX),
        state: i16::try_from(row.state).unwrap_or(0),
        accuracy: row.accuracy.map(|value| value as f32),
        attempts: i32::try_from(row.attempts).unwrap_or(i32::MAX),
        last_review: row.last_review,
        due: row.due,
    }
}

async fn advance_bank_cursor(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    language: &str,
    cursor: i64,
) -> Result<(), MobileError> {
    sqlx::query(
        "INSERT INTO grammar_sync_state (profile_id, user_id, language, bank_cursor)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (profile_id, user_id, language)
         DO UPDATE SET bank_cursor = excluded.bank_cursor",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(language)
    .bind(cursor)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}
