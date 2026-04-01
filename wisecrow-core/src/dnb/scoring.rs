use super::CompletedTrial;

const ACCURACY_INCREASE_THRESHOLD: f64 = 0.80;
const ACCURACY_DECREASE_THRESHOLD: f64 = 0.50;
const ACCURACY_TERMINATE_THRESHOLD: f64 = 0.40;
const ADAPTATION_WINDOW: usize = 5;
const TERMINATION_WINDOW: usize = 5;
const TIMING_STEP_MS: u32 = 200;
const MIN_INTERVAL_MS: u32 = 1500;
const MAX_INTERVAL_MS: u32 = 5000;
const CONSECUTIVE_BELOW_START_LIMIT: u8 = 3;
const MIN_N_LEVEL: u8 = 1;
const MAX_N_LEVEL: u8 = 9;

#[derive(Debug, Clone)]
pub struct AdaptationState {
    pub n_level: u8,
    pub n_level_start: u8,
    pub n_level_peak: u8,
    pub interval_ms: u32,
    pub interval_ms_start: u32,
    pub consecutive_below_start: u8,
}

impl AdaptationState {
    #[must_use]
    pub fn new(n_level: u8, interval_ms: u32) -> Self {
        let clamped_n = n_level.clamp(MIN_N_LEVEL, MAX_N_LEVEL);
        let clamped_interval = interval_ms.clamp(MIN_INTERVAL_MS, MAX_INTERVAL_MS);
        Self {
            n_level: clamped_n,
            n_level_start: clamped_n,
            n_level_peak: clamped_n,
            interval_ms: clamped_interval,
            interval_ms_start: clamped_interval,
            consecutive_below_start: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Audio,
    Visual,
}

/// Computes accuracy for a channel over the most recent `window` completed trials.
/// Returns a value in [0.0, 1.0]. Returns 0.0 if no trials exist in the window.
#[must_use]
pub fn channel_accuracy(trials: &[CompletedTrial], channel: Channel, window: usize) -> f64 {
    if trials.is_empty() || window == 0 {
        return 0.0;
    }

    let start = trials.len().saturating_sub(window);
    let slice = &trials[start..];
    let correct = slice
        .iter()
        .filter(|t| match channel {
            Channel::Audio => t.audio_correct(),
            Channel::Visual => t.visual_correct(),
        })
        .count();

    let total = slice.len();
    if total == 0 {
        return 0.0;
    }
    correct as f64 / total as f64
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdaptationAction {
    IncreaseN,
    DecreaseN,
    Hold,
}

fn evaluate_adaptation(audio_acc: f64, visual_acc: f64) -> AdaptationAction {
    if audio_acc >= ACCURACY_INCREASE_THRESHOLD && visual_acc >= ACCURACY_INCREASE_THRESHOLD {
        AdaptationAction::IncreaseN
    } else if audio_acc < ACCURACY_DECREASE_THRESHOLD || visual_acc < ACCURACY_DECREASE_THRESHOLD {
        AdaptationAction::DecreaseN
    } else {
        AdaptationAction::Hold
    }
}

pub fn apply_adaptation(state: &mut AdaptationState, trials: &[CompletedTrial]) {
    if trials.len() < ADAPTATION_WINDOW {
        return;
    }

    if trials.len() % ADAPTATION_WINDOW != 0 {
        return;
    }

    let audio_acc = channel_accuracy(trials, Channel::Audio, ADAPTATION_WINDOW);
    let visual_acc = channel_accuracy(trials, Channel::Visual, ADAPTATION_WINDOW);

    match evaluate_adaptation(audio_acc, visual_acc) {
        AdaptationAction::IncreaseN => {
            if state.n_level < MAX_N_LEVEL {
                state.n_level = state.n_level.saturating_add(1);
                state.interval_ms = state
                    .interval_ms
                    .saturating_sub(TIMING_STEP_MS)
                    .max(MIN_INTERVAL_MS);
            }
            if state.n_level > state.n_level_peak {
                state.n_level_peak = state.n_level;
            }
            state.consecutive_below_start = 0;
        }
        AdaptationAction::DecreaseN => {
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
        AdaptationAction::Hold => {
            if state.n_level < state.n_level_start {
                state.consecutive_below_start = state.consecutive_below_start.saturating_add(1);
            }
        }
    }
}

#[must_use]
pub fn should_terminate(state: &AdaptationState, trials: &[CompletedTrial]) -> bool {
    if state.consecutive_below_start >= CONSECUTIVE_BELOW_START_LIMIT {
        return true;
    }

    if trials.len() >= TERMINATION_WINDOW {
        let audio_acc = channel_accuracy(trials, Channel::Audio, TERMINATION_WINDOW);
        let visual_acc = channel_accuracy(trials, Channel::Visual, TERMINATION_WINDOW);
        if audio_acc < ACCURACY_TERMINATE_THRESHOLD && visual_acc < ACCURACY_TERMINATE_THRESHOLD {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dnb::{DnbVocab, Trial, TrialResponse};
    use proptest::prelude::*;
    use rstest::rstest;

    fn dummy_vocab() -> DnbVocab {
        DnbVocab {
            translation_id: 1,
            from_phrase: "test".to_owned(),
            to_phrase: "test".to_owned(),
        }
    }

    fn make_trial(audio_match: bool, visual_match: bool) -> Trial {
        Trial {
            trial_number: 1,
            n_level: 2,
            audio_vocab: dummy_vocab(),
            visual_vocab: dummy_vocab(),
            audio_match,
            visual_match,
            interval_ms: 3000,
        }
    }

    fn make_completed(
        audio_match: bool,
        visual_match: bool,
        audio_response: Option<bool>,
        visual_response: Option<bool>,
    ) -> CompletedTrial {
        CompletedTrial {
            trial: make_trial(audio_match, visual_match),
            response: TrialResponse {
                audio_response,
                visual_response,
                response_time_ms: Some(500),
            },
        }
    }

    proptest! {
        #[test]
        fn accuracy_always_bounded(
            correct_count in 0usize..=20,
            total in 1usize..=20,
        ) {
            let count = correct_count.min(total);
            let trials: Vec<CompletedTrial> = (0..total)
                .map(|i| {
                    let is_match = i < count;
                    make_completed(is_match, is_match, Some(is_match), Some(is_match))
                })
                .collect();

            let audio_acc = channel_accuracy(&trials, Channel::Audio, total);
            let visual_acc = channel_accuracy(&trials, Channel::Visual, total);

            prop_assert!(audio_acc >= 0.0);
            prop_assert!(audio_acc <= 1.0);
            prop_assert!(visual_acc >= 0.0);
            prop_assert!(visual_acc <= 1.0);
        }

        #[test]
        fn adaptation_state_clamps_inputs(n in 0u8..=20, interval in 0u32..=10000) {
            let state = AdaptationState::new(n, interval);
            prop_assert!(state.n_level >= MIN_N_LEVEL);
            prop_assert!(state.n_level <= MAX_N_LEVEL);
            prop_assert!(state.interval_ms >= MIN_INTERVAL_MS);
            prop_assert!(state.interval_ms <= MAX_INTERVAL_MS);
        }
    }

    #[test]
    fn accuracy_empty_returns_zero() {
        assert!((channel_accuracy(&[], Channel::Audio, 5) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn accuracy_all_correct() {
        let trials: Vec<CompletedTrial> = (0..5)
            .map(|_| make_completed(true, true, Some(true), Some(true)))
            .collect();
        let acc = channel_accuracy(&trials, Channel::Audio, 5);
        assert!((acc - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn accuracy_all_wrong() {
        let trials: Vec<CompletedTrial> = (0..5)
            .map(|_| make_completed(true, true, Some(false), Some(false)))
            .collect();
        let acc = channel_accuracy(&trials, Channel::Audio, 5);
        assert!((acc - 0.0).abs() < f64::EPSILON);
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
        #[case] audio_acc: f64,
        #[case] visual_acc: f64,
        #[case] expected: AdaptationAction,
    ) {
        assert_eq!(evaluate_adaptation(audio_acc, visual_acc), expected);
    }

    #[test]
    fn n_increases_after_high_accuracy_window() {
        let mut state = AdaptationState::new(2, 4000);
        let trials: Vec<CompletedTrial> = (0..5)
            .map(|_| make_completed(true, true, Some(true), Some(true)))
            .collect();

        apply_adaptation(&mut state, &trials);
        assert_eq!(state.n_level, 3);
        assert_eq!(state.n_level_peak, 3);
        assert_eq!(state.interval_ms, 3800);
    }

    #[test]
    fn n_decreases_after_low_accuracy_window() {
        let mut state = AdaptationState::new(3, 3800);
        let trials: Vec<CompletedTrial> = (0..5)
            .map(|_| make_completed(true, true, Some(false), Some(false)))
            .collect();

        apply_adaptation(&mut state, &trials);
        assert_eq!(state.n_level, 2);
        assert_eq!(state.interval_ms, 4000);
    }

    #[test]
    fn terminates_after_consecutive_below_start() {
        let mut state = AdaptationState::new(3, 4000);
        state.n_level = 2;
        state.consecutive_below_start = 3;
        assert!(should_terminate(&state, &[]));
    }

    #[test]
    fn terminates_on_sustained_low_accuracy() {
        let state = AdaptationState::new(2, 4000);
        let trials: Vec<CompletedTrial> = (0..5)
            .map(|_| make_completed(true, true, Some(false), Some(false)))
            .collect();
        assert!(should_terminate(&state, &trials));
    }

    #[test]
    fn does_not_terminate_with_acceptable_accuracy() {
        let state = AdaptationState::new(2, 4000);
        let trials: Vec<CompletedTrial> = (0..5)
            .map(|_| make_completed(true, true, Some(true), Some(true)))
            .collect();
        assert!(!should_terminate(&state, &trials));
    }
}
