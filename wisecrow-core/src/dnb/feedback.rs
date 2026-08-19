use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use wisecrow_dto::{ReviewEventDto, ReviewRatingDto};

use crate::errors::WisecrowError;
use crate::srs::reviews::{ReviewLedger, ReviewSource};

use super::CompletedTrial;

/// Applies SRS feedback from n-back trial results, scoped to the given user.
///
/// For each translation seen during the session, aggregates correct/incorrect
/// recognitions across both channels. Applies a fractional FSRS rating:
/// - Net positive recognition → Good rating with reduced weight
/// - Net negative recognition → Again rating with reduced weight
///
/// # Errors
///
/// Returns an error if card lookup or review operations fail.
pub async fn apply_srs_feedback(
    pool: &PgPool,
    user_id: i32,
    trials: &[CompletedTrial],
) -> Result<u32, WisecrowError> {
    let events = review_events(Uuid::new_v4(), Utc::now(), trials);
    let updated = u32::try_from(events.len())
        .map_err(|_| WisecrowError::InvalidInput("too many n-back feedback events".into()))?;
    ReviewLedger::new(pool)
        .apply_batch(user_id, None, ReviewSource::NBack, &events)
        .await?;
    Ok(updated)
}

pub(crate) fn review_events(
    namespace: Uuid,
    occurred_at: DateTime<Utc>,
    trials: &[CompletedTrial],
) -> Vec<ReviewEventDto> {
    scores(trials)
        .into_iter()
        .filter_map(|(translation_id, (correct, incorrect))| {
            rating(correct, incorrect).map(|rating| ReviewEventDto {
                event_id: Uuid::new_v5(&namespace, &translation_id.to_be_bytes()),
                translation_id,
                rating,
                occurred_at,
            })
        })
        .collect()
}

fn scores(trials: &[CompletedTrial]) -> BTreeMap<i32, (u32, u32)> {
    let mut scores = BTreeMap::new();
    for trial in trials {
        record_channel_score(
            &mut scores,
            trial.trial.audio_vocab.translation_id,
            trial.audio_correct(),
        );
        record_channel_score(
            &mut scores,
            trial.trial.visual_vocab.translation_id,
            trial.visual_correct(),
        );
    }
    scores
}

fn rating(correct: u32, incorrect: u32) -> Option<ReviewRatingDto> {
    match correct.cmp(&incorrect) {
        std::cmp::Ordering::Greater => Some(ReviewRatingDto::Good),
        std::cmp::Ordering::Less => Some(ReviewRatingDto::Again),
        std::cmp::Ordering::Equal => None,
    }
}

fn record_channel_score(
    scores: &mut BTreeMap<i32, (u32, u32)>,
    translation_id: i32,
    correct: bool,
) {
    let entry = scores.entry(translation_id).or_insert((0, 0));
    if correct {
        entry.0 = entry.0.saturating_add(1);
    } else {
        entry.1 = entry.1.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::dnb::{DnbVocab, Trial, TrialResponse};

    proptest! {
        #[test]
        fn feedback_ids_and_ratings_are_deterministic(
            translation_id in 1i32..i32::MAX,
            correct in any::<bool>(),
        ) {
            let namespace = Uuid::from_u128(42);
            let occurred_at = Utc::now();
            let vocab = DnbVocab::new(translation_id, "fremd", "native");
            let trials = vec![CompletedTrial {
                trial: Trial {
                    trial_number: 1,
                    n_level: 2,
                    audio_vocab: vocab.clone(),
                    visual_vocab: vocab,
                    audio_match: true,
                    visual_match: true,
                    interval_ms: 4_000,
                },
                response: TrialResponse {
                    audio_response: Some(correct),
                    visual_response: Some(correct),
                    response_time_ms: Some(100),
                },
            }];

            let events = review_events(namespace, occurred_at, &trials);

            prop_assert_eq!(events.len(), 1);
            prop_assert_eq!(events[0].event_id, Uuid::new_v5(&namespace, &translation_id.to_be_bytes()));
            let expected = if correct { ReviewRatingDto::Good } else { ReviewRatingDto::Again };
            prop_assert_eq!(events[0].rating, expected);
        }
    }
}
