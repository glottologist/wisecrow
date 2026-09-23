//! Choosing what a grammar session serves next.
//!
//! Points fall into three groups, in this order: those whose schedule says
//! they are due, those never attempted, and the rest. The first group is
//! ordered by due date so that the most overdue is met first, the second in
//! syllabus order so that a learner works forwards through a level, and the
//! third by ascending accuracy so that the points they keep failing surface
//! ahead of the points they merely have not seen lately.
//!
//! Within a point, the item served is the active one least recently shown to
//! that learner, which spreads exposure across the bank rather than drilling
//! one sentence.

use sqlx::PgPool;

use crate::errors::WisecrowError;

/// One item, with the grammar point it teaches.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PracticeItem {
    pub rule_id: i32,
    pub slug: String,
    pub rule_title: String,
    pub explanation: String,
    pub level: String,
    pub item_id: i32,
    pub revision: i32,
    pub kind: String,
    pub prompt: String,
    pub answer: Option<String>,
    pub accepted: serde_json::Value,
    pub options: Option<serde_json::Value>,
    pub correct_option: Option<String>,
    pub hint: Option<String>,
}

/// Groups a point, then orders within its group.
///
/// The bucket has to be decided before ordering rather than folded into it: a
/// single `due ASC` term would sort a point due next week ahead of one never
/// attempted, because an unattempted point has no due date at all.
const SELECTION_ORDER: &str = "
    CASE
        WHEN gm.due IS NOT NULL AND gm.due <= CURRENT_TIMESTAMP THEN 0
        WHEN COALESCE(gm.attempts, 0) = 0 THEN 1
        ELSE 2
    END,
    CASE WHEN gm.due <= CURRENT_TIMESTAMP THEN gm.due END ASC,
    gm.accuracy ASC NULLS LAST,
    cl.sort_order,
    gr.slug";

/// The active item of each point, least recently served first.
const LEAST_RECENTLY_SERVED: &str = "
    JOIN LATERAL (
        SELECT qi.id, qi.revision, qi.kind, qi.prompt, qi.answer, qi.accepted,
               qi.options, qi.correct_option, qi.hint
        FROM quiz_items qi
        LEFT JOIN quiz_item_exposures qe ON qe.item_id = qi.id AND qe.user_id = $1
        WHERE qi.rule_id = gr.id AND qi.status = 'active'
        ORDER BY qe.served_at ASC NULLS FIRST, qi.id
        LIMIT 1
    ) item ON TRUE";

/// Picks the next items for a practice session.
///
/// A point with no active item is absent rather than empty: there is nothing
/// to ask about it yet, and a session must not stall on a bank that has not
/// been promoted.
///
/// # Errors
///
/// Returns an error when the query fails.
pub async fn select_practice(
    pool: &PgPool,
    user_id: i32,
    lang_code: &str,
    level_code: Option<&str>,
    limit: i64,
) -> Result<Vec<PracticeItem>, WisecrowError> {
    let items = sqlx::query_as::<_, PracticeItem>(&format!(
        "SELECT gr.id AS rule_id, gr.slug, gr.title AS rule_title, gr.explanation,
                cl.code AS level, item.id AS item_id, item.revision, item.kind,
                item.prompt, item.answer, item.accepted, item.options,
                item.correct_option, item.hint
         FROM grammar_rules gr
         JOIN languages l ON l.id = gr.language_id
         JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
         LEFT JOIN grammar_mastery gm ON gm.rule_id = gr.id AND gm.user_id = $1
         {LEAST_RECENTLY_SERVED}
         WHERE l.code = $2 AND ($3::TEXT IS NULL OR cl.code = $3)
         ORDER BY {SELECTION_ORDER}
         LIMIT $4"
    ))
    .bind(user_id)
    .bind(lang_code)
    .bind(level_code)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(items)
}

/// Notes that an item has been shown, so the next session picks another.
///
/// # Errors
///
/// Returns an error when the write fails.
pub async fn record_exposure(
    pool: &PgPool,
    user_id: i32,
    item_id: i32,
) -> Result<(), WisecrowError> {
    sqlx::query(
        "INSERT INTO quiz_item_exposures (user_id, item_id) VALUES ($1, $2)
         ON CONFLICT (user_id, item_id) DO UPDATE SET served_at = CURRENT_TIMESTAMP",
    )
    .bind(user_id)
    .bind(item_id)
    .execute(pool)
    .await?;
    Ok(())
}
