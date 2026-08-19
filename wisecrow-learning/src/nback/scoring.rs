use num_traits::ToPrimitive;

use super::CompletedTrial;

const ACCURACY_INCREASE_THRESHOLD: f64 = 0.80;
const ACCURACY_DECREASE_THRESHOLD: f64 = 0.50;
const ACCURACY_TERMINATE_THRESHOLD: f64 = 0.40;
const ADAPTATION_WINDOW: usize = 5;
const TERMINATION_WINDOW: usize = 5;
const TIMING_STEP_MS: u32 = 200;
const MIN_INTERVAL_MS: u32 = 1_500;
const MAX_INTERVAL_MS: u32 = 5_000;
const CONSECUTIVE_BELOW_START_LIMIT: u8 = 3;
const MIN_N_LEVEL: u8 = 1;
const MAX_N_LEVEL: u8 = 9;

/// Mutable adaptive difficulty state for a Dual N-Back session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptationState {
    pub n_level: u8,
    pub n_level_start: u8,
    pub n_level_peak: u8,
    pub interval_ms: u32,
    pub interval_ms_start: u32,
    pub consecutive_below_start: u8,
}

impl AdaptationState {
    /// Creates an adaptation baseline constrained to supported bounds.
    #[must_use]
    pub fn new(n_level: u8, interval_ms: u32) -> Self {
        let n_level = n_level.clamp(MIN_N_LEVEL, MAX_N_LEVEL);
        let interval_ms = interval_ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS);
        Self {
            n_level,
            n_level_start: n_level,
            n_level_peak: n_level,
            interval_ms,
            interval_ms_start: interval_ms,
            consecutive_below_start: 0,
        }
    }
}

/// One independently scored N-Back channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Audio,
    Visual,
}

/// Computes channel accuracy over the most recent completed trials.
#[must_use]
pub fn channel_accuracy(trials: &[CompletedTrial], channel: Channel, window: usize) -> f64 {
    if trials.is_empty() || window == 0 {
        return 0.0;
    }

    let recent = &trials[trials.len().saturating_sub(window)..];
    let correct = recent
        .iter()
        .filter(|trial| match channel {
            Channel::Audio => trial.audio_correct(),
            Channel::Visual => trial.visual_correct(),
        })
        .count();

    match (correct.to_f64(), recent.len().to_f64()) {
        (Some(correct), Some(total)) if total > 0.0 => correct / total,
        _ => 0.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdaptationAction {
    IncreaseN,
    DecreaseN,
    Hold,
}

fn evaluate_adaptation(audio_accuracy: f64, visual_accuracy: f64) -> AdaptationAction {
    if audio_accuracy >= ACCURACY_INCREASE_THRESHOLD
        && visual_accuracy >= ACCURACY_INCREASE_THRESHOLD
    {
        AdaptationAction::IncreaseN
    } else if audio_accuracy < ACCURACY_DECREASE_THRESHOLD
        || visual_accuracy < ACCURACY_DECREASE_THRESHOLD
    {
        AdaptationAction::DecreaseN
    } else {
        AdaptationAction::Hold
    }
}

/// Evolves adaptive difficulty at each complete scoring window.
pub fn apply_adaptation(state: &mut AdaptationState, trials: &[CompletedTrial]) {
    if trials.len() < ADAPTATION_WINDOW || !trials.len().is_multiple_of(ADAPTATION_WINDOW) {
        return;
    }

    let audio_accuracy = channel_accuracy(trials, Channel::Audio, ADAPTATION_WINDOW);
    let visual_accuracy = channel_accuracy(trials, Channel::Visual, ADAPTATION_WINDOW);
    match evaluate_adaptation(audio_accuracy, visual_accuracy) {
        AdaptationAction::IncreaseN => increase_difficulty(state),
        AdaptationAction::DecreaseN => decrease_difficulty(state),
        AdaptationAction::Hold => hold_difficulty(state),
    }
}

fn increase_difficulty(state: &mut AdaptationState) {
    if state.n_level < MAX_N_LEVEL {
        state.n_level = state.n_level.saturating_add(1);
        state.interval_ms = state
            .interval_ms
            .saturating_sub(TIMING_STEP_MS)
            .max(MIN_INTERVAL_MS);
    }
    state.n_level_peak = state.n_level_peak.max(state.n_level);
    state.consecutive_below_start = 0;
}

fn decrease_difficulty(state: &mut AdaptationState) {
    if state.n_level > MIN_N_LEVEL {
        state.n_level = state.n_level.saturating_sub(1);
        state.interval_ms = state
            .interval_ms
            .saturating_add(TIMING_STEP_MS)
            .min(MAX_INTERVAL_MS);
    }
    if state.n_level < state.n_level_start {
        state.consecutive_below_start = state.consecutive_below_start.saturating_add(1);
    } else {
        state.consecutive_below_start = 0;
    }
}

fn hold_difficulty(state: &mut AdaptationState) {
    if state.n_level < state.n_level_start {
        state.consecutive_below_start = state.consecutive_below_start.saturating_add(1);
    }
}

/// Reports whether performance has reached a termination boundary.
#[must_use]
pub fn should_terminate(state: &AdaptationState, trials: &[CompletedTrial]) -> bool {
    if state.consecutive_below_start >= CONSECUTIVE_BELOW_START_LIMIT {
        return true;
    }
    if trials.len() < TERMINATION_WINDOW {
        return false;
    }

    channel_accuracy(trials, Channel::Audio, TERMINATION_WINDOW) < ACCURACY_TERMINATE_THRESHOLD
        && channel_accuracy(trials, Channel::Visual, TERMINATION_WINDOW)
            < ACCURACY_TERMINATE_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nback::{DnbVocab, Trial, TrialResponse};
    use proptest::prelude::*;
    use rstest::rstest;

    fn completed(correct: bool) -> CompletedTrial {
        let vocab = DnbVocab::new(1, "test", "test");
        CompletedTrial {
            trial: Trial {
                trial_number: 1,
                n_level: 2,
                audio_vocab: vocab.clone(),
                visual_vocab: vocab,
                audio_match: true,
                visual_match: true,
                interval_ms: 3_000,
            },
            response: TrialResponse {
                audio_response: Some(correct),
                visual_response: Some(correct),
                response_time_ms: Some(500),
            },
        }
    }

    proptest! {
        #[test]
        fn accuracy_always_bounded(correct_count in 0usize..=20, total in 1usize..=20) {
            let correct_count = correct_count.min(total);
            let trials: Vec<_> = (0..total)
                .map(|index| completed(index < correct_count))
                .collect();

            let audio_accuracy = channel_accuracy(&trials, Channel::Audio, total);
            let visual_accuracy = channel_accuracy(&trials, Channel::Visual, total);
            prop_assert!((0.0..=1.0).contains(&audio_accuracy));
            prop_assert!((0.0..=1.0).contains(&visual_accuracy));
        }

        #[test]
        fn adaptation_state_clamps_inputs(n in 0u8..=20, interval in 0u32..=10_000) {
            let state = AdaptationState::new(n, interval);
            prop_assert!((MIN_N_LEVEL..=MAX_N_LEVEL).contains(&state.n_level));
            prop_assert!((MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&state.interval_ms));
        }
    }

    #[rstest]
    #[case(0.9, 0.9, AdaptationAction::IncreaseN)]
    #[case(0.85, 0.81, AdaptationAction::IncreaseN)]
    #[case(0.3, 0.9, AdaptationAction::DecreaseN)]
    #[case(0.9, 0.3, AdaptationAction::DecreaseN)]
    #[case(0.4, 0.4, AdaptationAction::DecreaseN)]
    #[case(0.6, 0.6, AdaptationAction::Hold)]
    #[case(0.7, 0.79, AdaptationAction::Hold)]
    fn adaptation_action_cases(
        #[case] audio_accuracy: f64,
        #[case] visual_accuracy: f64,
        #[case] expected: AdaptationAction,
    ) {
        assert_eq!(
            evaluate_adaptation(audio_accuracy, visual_accuracy),
            expected
        );
    }

    #[test]
    fn adaptation_and_termination_boundaries_are_preserved() {
        let correct = vec![completed(true); ADAPTATION_WINDOW];
        let incorrect = vec![completed(false); ADAPTATION_WINDOW];
        let mut state = AdaptationState::new(2, 4_000);

        apply_adaptation(&mut state, &correct);
        assert_eq!(
            (state.n_level, state.n_level_peak, state.interval_ms),
            (3, 3, 3_800)
        );

        apply_adaptation(&mut state, &incorrect);
        assert_eq!((state.n_level, state.interval_ms), (2, 4_000));
        assert!(should_terminate(&state, &incorrect));
        assert!(!should_terminate(&state, &correct));
    }
}
