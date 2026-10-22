//! The placement ladder.
//!
//! A placement run climbs the CEFR levels from A1, sampling a handful of items
//! at each, and stops once two levels in a row score below threshold. The
//! level reported is the highest one passed.
//!
//! What it deliberately does *not* do matters more than what it does. It seeds
//! mastery only for the points it actually tested; every untested point stays
//! unseen. Inferring that a learner who passed A2 knows all thirty A1 points
//! would fill the brainmap with confidence they never demonstrated, and
//! selection would then skip exactly the material it should be checking.
//!
//! The run is not a separate engine. It opens one ordinary grammar session and
//! submits through the ordinary path, so placement answers are graded, rated
//! and recorded exactly as practice answers are.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use wisecrow_learning::grading::Submission;

use crate::errors::WisecrowError;
use crate::grammar::selection::{record_exposure, select_practice, PracticeItem};
use crate::grammar::session::{GrammarSessionManager, SessionKind, SubmissionContext};

/// Items drawn at each level.
///
/// Enough to separate 0.60 from chance on four-option items without a tedious
/// test.
pub const PLACEMENT_SAMPLE: i64 = 6;

/// Proportion correct at or above which a level is passed.
pub const PLACEMENT_PASS: f64 = 0.60;

/// Fewest items a level must offer to be worth testing at all.
pub const PLACEMENT_MIN_ITEMS: usize = 4;

/// Consecutive failed levels that end the ladder.
pub const PLACEMENT_MAX_FAILURES: usize = 2;

/// One level being put to the learner.
#[derive(Debug, Clone)]
pub struct PlacementStep {
    pub attempt_id: Uuid,
    pub session_id: Uuid,
    pub level: String,
    pub items: Vec<PracticeItem>,
}

/// How one tested level went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelScore {
    pub level: String,
    pub correct: i64,
    pub asked: i64,
    pub passed: bool,
}

/// The verdict of a finished run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementOutcome {
    pub attempt_id: Uuid,
    /// The highest level passed; `None` where even A1 was not.
    pub level_reached: Option<String>,
    pub tested: Vec<LevelScore>,
    /// Levels with too thin a bank to judge. Untested is not failed.
    pub skipped: Vec<String>,
}

/// Where a run stands.
#[derive(Debug, Clone)]
pub enum PlacementState {
    Testing(PlacementStep),
    Finished(PlacementOutcome),
}

/// A finished run, as the dashboard reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedPlacement {
    pub completed_at: DateTime<Utc>,
    pub outcome: PlacementOutcome,
}

/// The verdict of a run over the levels walked so far.
///
/// The live ladder and the replay of a finished run both fold levels through
/// this, so the dashboard reads a past run exactly as the run itself decided.
#[derive(Debug, Default)]
struct Ladder {
    level_reached: Option<String>,
    tested: Vec<LevelScore>,
    skipped: Vec<String>,
    failures: usize,
}

impl Ladder {
    /// Folds in one answered level, reporting whether the ladder ends there.
    fn record(&mut self, score: &LevelScore) -> bool {
        if score.passed {
            self.level_reached = Some(score.level.clone()); // clone: the outcome keeps the level after the loop moves on
            self.failures = 0;
        } else {
            self.failures = self.failures.saturating_add(1);
        }
        self.tested.push(score.clone()); // clone: the scores are read again on the next call
        self.failures >= PLACEMENT_MAX_FAILURES
    }

    fn into_outcome(self, attempt_id: Uuid) -> PlacementOutcome {
        PlacementOutcome {
            attempt_id,
            level_reached: self.level_reached,
            tested: self.tested,
            skipped: self.skipped,
        }
    }
}

/// The learner's most recent finished run for a language, if there is one.
///
/// Only the level reached is stored, so the tested and skipped levels are
/// replayed from the run's answers through the same fold the run used.
///
/// # Errors
///
/// Returns an error when a query fails.
pub async fn latest_completed(
    pool: &PgPool,
    user_id: i32,
    lang_code: &str,
) -> Result<Option<CompletedPlacement>, WisecrowError> {
    let row: Option<(Uuid, Uuid, DateTime<Utc>)> = sqlx::query_as(
        "SELECT pa.id, pa.session_id, pa.completed_at
         FROM placement_attempts pa
         JOIN languages l ON l.id = pa.language_id
         WHERE pa.user_id = $1 AND l.code = $2 AND pa.completed_at IS NOT NULL
         ORDER BY pa.completed_at DESC, pa.started_at DESC
         LIMIT 1",
    )
    .bind(user_id)
    .bind(lang_code)
    .fetch_optional(pool)
    .await?;

    let Some((attempt_id, session_id, completed_at)) = row else {
        return Ok(None);
    };
    let levels = level_codes(pool).await?;
    let answered = level_scores(pool, session_id).await?;
    Ok(Some(CompletedPlacement {
        completed_at,
        outcome: replay_finished(levels, &answered).into_outcome(attempt_id),
    }))
}

/// Opens a run and draws the first level worth testing.
///
/// # Errors
///
/// Returns an error when the language is unknown or a write fails.
pub async fn start(
    pool: &PgPool,
    user_id: i32,
    lang_code: &str,
) -> Result<PlacementState, WisecrowError> {
    let language_id: i32 = sqlx::query_scalar("SELECT id FROM languages WHERE code = $1")
        .bind(lang_code)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| WisecrowError::UnsupportedLanguage(lang_code.to_owned()))?;

    let mut transaction = pool.begin().await?;
    let session_id = GrammarSessionManager::open(
        &mut transaction,
        user_id,
        language_id,
        None,
        SessionKind::Placement,
    )
    .await?;
    let attempt_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO placement_attempts (id, user_id, language_id, session_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(attempt_id)
    .bind(user_id)
    .bind(language_id)
    .bind(session_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    advance(pool, user_id, attempt_id, session_id, lang_code).await
}

/// Records a level's answers and draws the next level, or finishes the run.
///
/// # Errors
///
/// Returns an error when the run is unknown, when a submission names another
/// session, or when grading or a write fails.
pub async fn submit(
    pool: &PgPool,
    user_id: i32,
    attempt_id: Uuid,
    submissions: &[Submission],
    context: SubmissionContext,
) -> Result<PlacementState, WisecrowError> {
    let run = load_run(pool, user_id, attempt_id).await?;
    if run.completed {
        return Err(WisecrowError::Conflict(format!(
            "Placement run {attempt_id} has finished"
        )));
    }
    if submissions
        .iter()
        .any(|submission| submission.session_id != run.session_id)
    {
        return Err(WisecrowError::InvalidInput(
            "A placement answer names another session".into(),
        ));
    }

    let mut transaction = pool.begin().await?;
    for submission in submissions {
        GrammarSessionManager::submit_in_transaction(
            &mut transaction,
            user_id,
            submission,
            context,
        )
        .await?;
    }
    transaction.commit().await?;

    advance(pool, user_id, attempt_id, run.session_id, &run.lang_code).await
}

struct Run {
    session_id: Uuid,
    lang_code: String,
    completed: bool,
}

async fn load_run(pool: &PgPool, user_id: i32, attempt_id: Uuid) -> Result<Run, WisecrowError> {
    let row: Option<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT pa.session_id, l.code, pa.completed_at IS NOT NULL
         FROM placement_attempts pa
         JOIN languages l ON l.id = pa.language_id
         WHERE pa.id = $1 AND pa.user_id = $2",
    )
    .bind(attempt_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    row.map(|(session_id, lang_code, completed)| Run {
        session_id,
        lang_code,
        completed,
    })
    .ok_or_else(|| {
        WisecrowError::InvalidInput(format!("No placement run {attempt_id} for this learner"))
    })
}

/// Walks the ladder from A1 over what has already been answered.
///
/// The whole state of a run lives in its attempt rows and in how many items
/// each level can offer, so this recomputes rather than remembers; a client
/// that loses its place simply asks again.
async fn advance(
    pool: &PgPool,
    user_id: i32,
    attempt_id: Uuid,
    session_id: Uuid,
    lang_code: &str,
) -> Result<PlacementState, WisecrowError> {
    let levels = level_codes(pool).await?;
    let answered = level_scores(pool, session_id).await?;
    let mut ladder = Ladder::default();

    for level in levels {
        if let Some(score) = answered.iter().find(|score| score.level == level) {
            if ladder.record(score) {
                break;
            }
            continue;
        }

        let items =
            select_practice(pool, user_id, lang_code, Some(&level), PLACEMENT_SAMPLE).await?;
        if items.len() < PLACEMENT_MIN_ITEMS {
            ladder.skipped.push(level);
            continue;
        }

        for item in &items {
            record_exposure(pool, user_id, item.item_id).await?;
        }
        return Ok(PlacementState::Testing(PlacementStep {
            attempt_id,
            session_id,
            level,
            items,
        }));
    }

    finish(
        pool,
        user_id,
        attempt_id,
        session_id,
        ladder.level_reached.as_deref(),
    )
    .await?;
    Ok(PlacementState::Finished(ladder.into_outcome(attempt_id)))
}

/// Rebuilds the verdict of a finished run from its answers alone.
///
/// A run finishes only once no level it reached could still be tested, so a
/// level it reached without answering was one whose bank was too thin.
fn replay_finished(levels: Vec<String>, answered: &[LevelScore]) -> Ladder {
    let mut ladder = Ladder::default();
    for level in levels {
        match answered.iter().find(|score| score.level == level) {
            Some(score) => {
                if ladder.record(score) {
                    break;
                }
            }
            None => ladder.skipped.push(level),
        }
    }
    ladder
}

async fn level_codes(pool: &PgPool) -> Result<Vec<String>, WisecrowError> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT code FROM cefr_levels ORDER BY sort_order")
            .fetch_all(pool)
            .await?,
    )
}

async fn level_scores(pool: &PgPool, session_id: Uuid) -> Result<Vec<LevelScore>, WisecrowError> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT cl.code, COUNT(*) AS asked, COUNT(*) FILTER (WHERE ga.correct) AS correct
         FROM grammar_attempts ga
         JOIN grammar_rules gr ON gr.id = ga.rule_id
         JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
         WHERE ga.session_id = $1
         GROUP BY cl.code, cl.sort_order
         ORDER BY cl.sort_order",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(level, asked, correct)| LevelScore {
            level,
            correct,
            asked,
            #[expect(
                clippy::cast_precision_loss,
                reason = "a level is scored over six items"
            )]
            passed: asked > 0 && (correct as f64) / (asked as f64) >= PLACEMENT_PASS,
        })
        .collect())
}

async fn finish(
    pool: &PgPool,
    user_id: i32,
    attempt_id: Uuid,
    session_id: Uuid,
    level_reached: Option<&str>,
) -> Result<(), WisecrowError> {
    sqlx::query(
        "UPDATE placement_attempts
         SET level_reached = $3, completed_at = COALESCE(completed_at, CURRENT_TIMESTAMP)
         WHERE id = $1 AND user_id = $2",
    )
    .bind(attempt_id)
    .bind(user_id)
    .bind(level_reached)
    .execute(pool)
    .await?;
    GrammarSessionManager::complete(pool, user_id, session_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(level: &str, passed: bool) -> LevelScore {
        LevelScore {
            level: level.to_owned(),
            correct: if passed { 6 } else { 0 },
            asked: 6,
            passed,
        }
    }

    fn levels() -> Vec<String> {
        Vec::from(["A1", "A2", "B1", "B2", "C1", "C2"].map(String::from))
    }

    #[test]
    fn a_finished_run_that_passed_nothing_keeps_its_tested_and_skipped_levels() {
        let outcome = replay_finished(levels(), &[score("A1", false), score("B1", false)])
            .into_outcome(Uuid::nil());

        assert_eq!(outcome.level_reached, None);
        assert_eq!(outcome.tested, vec![score("A1", false), score("B1", false)]);
        assert_eq!(
            outcome.skipped,
            vec![String::from("A2")],
            "the ladder stopped at B1, so nothing above it was skipped"
        );
    }

    #[test]
    fn levels_reached_after_the_last_answer_were_skipped_as_thin() {
        let outcome = replay_finished(levels(), &[score("A1", true), score("A2", true)])
            .into_outcome(Uuid::nil());

        assert_eq!(outcome.level_reached.as_deref(), Some("A2"));
        assert_eq!(outcome.skipped, ["B1", "B2", "C1", "C2"]);
    }

    #[test]
    fn a_pass_resets_the_run_of_failures() {
        let outcome = replay_finished(
            levels(),
            &[
                score("A1", false),
                score("A2", true),
                score("B1", false),
                score("B2", false),
                score("C1", true),
            ],
        )
        .into_outcome(Uuid::nil());

        assert_eq!(outcome.level_reached.as_deref(), Some("A2"));
        assert_eq!(
            outcome.tested.len(),
            4,
            "two failures after A2 end the ladder before C1"
        );
        assert!(outcome.skipped.is_empty());
    }
}
