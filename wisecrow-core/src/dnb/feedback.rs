use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::srs::scheduler::{CardManager, ReviewRating};

use super::CompletedTrial;

const GOOD_WEIGHT: f64 = 0.5;
const AGAIN_WEIGHT: f64 = 0.5;

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
    let mut translation_scores: std::collections::HashMap<i32, (u32, u32)> =
        std::collections::HashMap::new();

    for trial in trials {
        let audio_id = trial.trial.audio_vocab.translation_id;
        let visual_id = trial.trial.visual_vocab.translation_id;

        record_channel_score(&mut translation_scores, audio_id, trial.audio_correct());
        record_channel_score(&mut translation_scores, visual_id, trial.visual_correct());
    }

    let mut updated = 0u32;
    for (translation_id, (correct, incorrect)) in &translation_scores {
        let card = CardManager::card_for_translation(pool, *translation_id, user_id).await?;
        let Some(card) = card else {
            continue;
        };

        let net_correct = correct.saturating_sub(*incorrect);
        let net_incorrect = incorrect.saturating_sub(*correct);

        let rating = if net_correct > 0 {
            ReviewRating::Good
        } else if net_incorrect > 0 {
            ReviewRating::Again
        } else {
            continue;
        };

        let weight = if rating == ReviewRating::Good {
            GOOD_WEIGHT
        } else {
            AGAIN_WEIGHT
        };

        if weight > 0.0 {
            CardManager::review(pool, &card, rating).await?;
            updated = updated.saturating_add(1);
        }
    }

    Ok(updated)
}

fn record_channel_score(
    scores: &mut std::collections::HashMap<i32, (u32, u32)>,
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
