//! Mastery as two projections of one attempt stream.
//!
//! FSRS decides when a grammar point is due; an exponentially weighted
//! accuracy decides how the brainmap colours it. Retrievability answers "how
//! likely is recall now", which is right for scheduling and wrong for a
//! brainmap, where a red cell means "mistakes you keep making" — a statement
//! about repeated error rather than about decay. Holding two numbers risks
//! them drifting, so both are derived here from the same immutable stream and
//! neither is ever the primary record.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::srs::{CardState, FsrsScheduler, ReviewEvent, ReviewRating, Scheduler};
use crate::LearningError;

/// Weight given to the newest outcome.
///
/// Roughly a five-attempt memory: recent error dominates without one slip
/// erasing a good history.
pub const ALPHA: f64 = 0.3;

/// Accuracy at or above which a point reads green.
const GREEN_THRESHOLD: f64 = 0.85;

/// Accuracy at or above which a point reads amber rather than red.
const AMBER_THRESHOLD: f64 = 0.60;

/// Attempts below which a band is provisional rather than settled.
const SETTLED_ATTEMPTS: usize = 3;

/// How confidently a grammar point is known, as the brainmap paints it.
///
/// The provisional variants exist because one unlucky answer should not
/// present as settled knowledge of failure; they render outlined rather than
/// filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Band {
    Unseen,
    Red,
    Amber,
    Green,
    ProvisionalRed,
    ProvisionalAmber,
    ProvisionalGreen,
}

/// Exponentially weighted accuracy over outcomes in chronological order.
///
/// The series starts at the first outcome rather than at one half, because
/// seeding at one half would paint every freshly attempted point amber
/// whatever the learner answered. A point with no attempts has no accuracy;
/// zero is returned for want of a number, and [`band`] reports it as
/// [`Band::Unseen`] rather than as failure.
#[must_use]
pub fn accuracy(outcomes: &[bool]) -> f64 {
    let mut outcomes = outcomes.iter();
    let Some(&first) = outcomes.next() else {
        return 0.0;
    };

    outcomes.fold(score(first), |weighted, &outcome| {
        ALPHA.mul_add(score(outcome), (1.0 - ALPHA) * weighted)
    })
}

/// Folds one further outcome into a weighted accuracy already computed.
///
/// A client that holds the running figure rather than the whole history — a
/// device colouring its brainmap the moment an answer is given, before the
/// server has heard of it — needs the same arithmetic [`accuracy`] applies, or
/// its colours would disagree with the server's over the same answers.
/// `previous` is `None` for the first outcome, which seeds the average rather
/// than being averaged into zero.
#[must_use]
pub fn fold_accuracy(previous: Option<f64>, outcome: bool) -> f64 {
    match previous {
        None => score(outcome),
        Some(weighted) => ALPHA.mul_add(score(outcome), (1.0 - ALPHA) * weighted),
    }
}

/// Colours an outcome history by the policy table.
#[must_use]
pub fn band(outcomes: &[bool]) -> Band {
    if outcomes.is_empty() {
        return Band::Unseen;
    }
    band_from(accuracy(outcomes), outcomes.len())
}

/// Colours a stored projection, which keeps the weighted figure and the count
/// but not the outcomes that produced them.
///
/// Zero attempts is unseen whatever accuracy accompanies it, because a point
/// never attempted must never be coloured as known — or as failed.
#[must_use]
pub fn band_from(weighted: f64, attempts: usize) -> Band {
    if attempts == 0 {
        return Band::Unseen;
    }

    let settled = attempts >= SETTLED_ATTEMPTS;
    if weighted >= GREEN_THRESHOLD {
        if settled {
            Band::Green
        } else {
            Band::ProvisionalGreen
        }
    } else if weighted >= AMBER_THRESHOLD {
        if settled {
            Band::Amber
        } else {
            Band::ProvisionalAmber
        }
    } else if settled {
        Band::Red
    } else {
        Band::ProvisionalRed
    }
}

const fn score(outcome: bool) -> f64 {
    if outcome {
        1.0
    } else {
        0.0
    }
}

/// One recorded submission against one grammar point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub rule_id: i32,
    pub correct: bool,
    pub hint_shown: bool,
    pub occurred_at: DateTime<Utc>,
}

/// The one rating an interaction contributes to scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalRating {
    pub session_id: Uuid,
    pub rule_id: i32,
    pub rating: ReviewRating,
    /// The closing attempt, which also dates the rating for replay.
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
}

/// Reduces an attempt stream to one rating per interaction.
///
/// `Scheduler::replay` schedules every event handed to it, so raw submissions
/// would advance a point's schedule several times for a single sitting. Within
/// one session the first item served for a rule opens an interaction, every
/// submission against it belongs to that interaction, and the interaction
/// closes on the first correct answer or, failing one, on the last submission
/// made. Correct on the first submission with no hint rates `Good`; correct
/// later or after a hint rates `Hard`; closing without a correct answer rates
/// `Again`. `Easy` is unused, because nothing in a cloze answer distinguishes
/// effortless recall from merely correct recall.
///
/// The result is a function of the whole stream rather than of arrival order,
/// so an attempt arriving late from a device that was offline — even one older
/// than attempts already processed — re-forms its interaction and yields what
/// it would have had it never been delayed.
#[must_use]
pub fn reduce_to_ratings(attempts: &[Attempt]) -> Vec<CanonicalRating> {
    let mut interactions: HashMap<(Uuid, i32), Vec<Attempt>> = HashMap::new();
    for attempt in attempts {
        interactions
            .entry((attempt.session_id, attempt.rule_id))
            .or_default()
            .push(*attempt);
    }

    let mut ratings: Vec<CanonicalRating> = interactions
        .into_values()
        .filter_map(|mut interaction| {
            interaction.sort_unstable_by_key(|attempt| (attempt.occurred_at, attempt.event_id));
            close(&interaction)
        })
        .collect();
    ratings.sort_unstable_by_key(|rating| (rating.occurred_at, rating.event_id));
    ratings
}

/// Rates one interaction's submissions, which must already be in order.
fn close(interaction: &[Attempt]) -> Option<CanonicalRating> {
    let opening = interaction.first()?;
    let closing = interaction
        .iter()
        .find(|attempt| attempt.correct)
        .or_else(|| interaction.last())?;

    let rating = if !closing.correct {
        ReviewRating::Again
    } else if closing.event_id == opening.event_id && !closing.hint_shown {
        ReviewRating::Good
    } else {
        ReviewRating::Hard
    };

    Some(CanonicalRating {
        session_id: closing.session_id,
        rule_id: closing.rule_id,
        rating,
        event_id: closing.event_id,
        occurred_at: closing.occurred_at,
    })
}

/// Both mastery states rebuilt from one baseline and one attempt stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    /// FSRS state for the grammar point. Its `translation_id` is whatever
    /// subject identifier the caller placed in the baseline; for grammar that
    /// is the rule, not a translation.
    pub card: CardState,
    pub accuracy: f64,
}

/// Rebuilds scheduling and accuracy from the same attempts.
///
/// Scheduling consumes one canonical rating per interaction; accuracy consumes
/// every submission, so a learner who fails twice and then succeeds is
/// recorded as having struggled even though the point is scheduled as `Hard`
/// once.
///
/// # Errors
///
/// Returns a typed learning error when a rating cannot be replayed, which
/// happens when an attempt precedes the baseline or two attempts share an
/// identifier.
pub fn project(baseline: &CardState, attempts: &[Attempt]) -> Result<Projection, LearningError> {
    let events: Vec<ReviewEvent> = reduce_to_ratings(attempts)
        .into_iter()
        .map(|rating| ReviewEvent {
            event_id: rating.event_id,
            occurred_at: rating.occurred_at,
            rating: rating.rating,
        })
        .collect();
    let card = FsrsScheduler.replay(baseline, &events)?;

    let mut ordered: Vec<&Attempt> = attempts.iter().collect();
    ordered.sort_unstable_by_key(|attempt| (attempt.occurred_at, attempt.event_id));
    let outcomes: Vec<bool> = ordered.iter().map(|attempt| attempt.correct).collect();

    Ok(Projection {
        card,
        accuracy: accuracy(&outcomes),
    })
}

#[cfg(test)]
mod tests {
    /// The two ways of computing the same figure must not drift apart: one is
    /// the server's, the other a device's, and they colour the same map.
    #[test]
    fn folding_one_outcome_at_a_time_matches_reducing_them_together() {
        for outcomes in [
            vec![true],
            vec![false],
            vec![true, false, true, true],
            vec![false, false, true, false, true, true],
        ] {
            let folded = outcomes
                .iter()
                .fold(None, |running, &outcome| {
                    Some(fold_accuracy(running, outcome))
                })
                .expect("at least one outcome");
            assert!(
                (folded - accuracy(&outcomes)).abs() < f64::EPSILON,
                "{outcomes:?}: folded {folded}, reduced {}",
                accuracy(&outcomes)
            );
        }
    }

    use super::*;
    use crate::srs::CardStatus;
    use chrono::TimeZone;
    use proptest::prelude::*;
    use rstest::rstest;

    fn ts(offset: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000_i64.saturating_add(offset), 0)
            .single()
            .expect("valid timestamp")
    }

    fn attempt(session: u8, rule: i32, correct: bool, hint: bool, at: i64) -> Attempt {
        Attempt {
            event_id: Uuid::from_u128(
                u128::try_from(at)
                    .expect("non-negative offset")
                    .saturating_mul(1000)
                    .saturating_add(1),
            ),
            session_id: Uuid::from_u128(session.into()),
            rule_id: rule,
            correct,
            hint_shown: hint,
            occurred_at: ts(at),
        }
    }

    #[test]
    fn accuracy_initialises_at_first_outcome() {
        assert_eq!(accuracy(&[true]), 1.0);
        assert_eq!(accuracy(&[false]), 0.0);
    }

    #[test]
    fn accuracy_weights_recent_attempts_more_heavily() {
        let improving = accuracy(&[false, false, true, true, true]);
        let declining = accuracy(&[true, true, true, false, false]);
        assert!(improving > declining);
    }

    #[rstest]
    #[case(&[true, true, true, true], Band::Green)]
    #[case(&[true, true, false, true], Band::Amber)]
    #[case(&[false, false, false], Band::Red)]
    #[case(&[], Band::Unseen)]
    #[case(&[false], Band::ProvisionalRed)]
    #[case(&[true], Band::ProvisionalGreen)]
    #[case(&[true, false], Band::ProvisionalAmber)]
    // One slip four attempts ago has already decayed past the green threshold;
    // the case is pinned because it sits within a thousandth of it.
    #[case(&[true, false, true, true], Band::Green)]
    fn bands_follow_the_policy_table(#[case] outcomes: &[bool], #[case] expected: Band) {
        assert_eq!(band(outcomes), expected);
    }

    #[test]
    fn one_interaction_yields_one_rating() {
        let attempts = vec![
            attempt(1, 7, false, false, 10),
            attempt(1, 7, false, true, 20),
            attempt(1, 7, true, true, 30),
        ];
        let ratings = reduce_to_ratings(&attempts);
        assert_eq!(ratings.len(), 1);
        assert_eq!(
            ratings[0].rating,
            ReviewRating::Hard,
            "correct only after a hint"
        );
    }

    #[test]
    fn clean_first_answer_rates_good_and_failure_rates_again() {
        assert_eq!(
            reduce_to_ratings(&[attempt(1, 7, true, false, 10)])[0].rating,
            ReviewRating::Good
        );
        assert_eq!(
            reduce_to_ratings(&[attempt(1, 7, false, false, 10)])[0].rating,
            ReviewRating::Again
        );
    }

    #[test]
    fn a_hint_on_the_first_answer_rates_hard() {
        assert_eq!(
            reduce_to_ratings(&[attempt(1, 7, true, true, 10)])[0].rating,
            ReviewRating::Hard
        );
    }

    #[test]
    fn separate_sessions_yield_separate_ratings() {
        let attempts = vec![
            attempt(1, 7, true, false, 10),
            attempt(2, 7, false, false, 20),
        ];
        assert_eq!(reduce_to_ratings(&attempts).len(), 2);
    }

    #[test]
    fn separate_rules_in_one_session_yield_separate_ratings() {
        let attempts = vec![
            attempt(1, 7, true, false, 10),
            attempt(1, 8, true, false, 20),
        ];
        assert_eq!(reduce_to_ratings(&attempts).len(), 2);
    }

    #[test]
    fn reduction_is_independent_of_arrival_order() {
        let forwards = vec![
            attempt(1, 7, false, false, 10),
            attempt(1, 7, true, false, 20),
            attempt(2, 7, true, false, 90),
        ];
        let mut backwards = forwards.clone(); // clone: the reversed copy must not disturb the original
        backwards.reverse();
        assert_eq!(reduce_to_ratings(&forwards), reduce_to_ratings(&backwards));
    }

    #[test]
    fn a_late_attempt_reopens_its_interaction() {
        let complete = vec![
            attempt(1, 7, false, false, 10),
            attempt(1, 7, true, false, 20),
        ];
        // The first submission arrives after the second has been processed;
        // the reduction must downgrade the interaction from `Good` to `Hard`.
        let delayed = vec![attempt(1, 7, true, false, 20)];
        assert_eq!(reduce_to_ratings(&delayed)[0].rating, ReviewRating::Good);
        assert_eq!(reduce_to_ratings(&complete)[0].rating, ReviewRating::Hard);
    }

    fn baseline() -> CardState {
        CardState::new(7, ts(0), CardStatus::New)
    }

    #[test]
    fn projection_schedules_once_per_session_and_scores_every_attempt() {
        let mut attempts = Vec::new();
        let mut outcomes = Vec::new();
        for session in 1..=4u8 {
            let base = i64::from(session).saturating_mul(1000);
            for (index, correct) in [false, false, true].into_iter().enumerate() {
                let at = base.saturating_add(
                    i64::try_from(index)
                        .expect("small index")
                        .saturating_mul(10),
                );
                attempts.push(attempt(session, 7, correct, false, at));
                outcomes.push(correct);
            }
        }

        let projection = project(&baseline(), &attempts).expect("projection");
        assert_eq!(projection.card.reps, 4, "one rating per session");
        assert_eq!(projection.accuracy, accuracy(&outcomes));

        let mut shuffled = attempts.clone(); // clone: the reordered copy must not disturb the original
        shuffled.reverse();
        assert_eq!(
            project(&baseline(), &shuffled).expect("projection"),
            projection
        );
    }

    #[test]
    fn an_empty_stream_leaves_the_baseline_untouched() {
        let projection = project(&baseline(), &[]).expect("projection");
        assert_eq!(projection.card, baseline());
        assert_eq!(projection.accuracy, 0.0);
    }

    proptest! {
        #[test]
        fn accuracy_stays_within_unit_interval(
            outcomes in prop::collection::vec(any::<bool>(), 0..50),
        ) {
            let value = accuracy(&outcomes);
            prop_assert!((0.0..=1.0).contains(&value));
        }

        #[test]
        fn every_interaction_yields_exactly_one_rating(
            sessions in prop::collection::vec(
                prop::collection::vec(any::<bool>(), 1..5),
                1..6,
            ),
        ) {
            let mut attempts = Vec::new();
            for (session, submissions) in sessions.iter().enumerate() {
                let session = u8::try_from(session).expect("few sessions");
                for (index, correct) in submissions.iter().enumerate() {
                    let at = i64::from(session)
                        .saturating_mul(1000)
                        .saturating_add(i64::try_from(index).expect("few submissions"));
                    attempts.push(attempt(session, 7, *correct, false, at));
                }
            }

            prop_assert_eq!(reduce_to_ratings(&attempts).len(), sessions.len());
        }
    }
}
