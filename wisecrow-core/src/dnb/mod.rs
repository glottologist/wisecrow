pub mod feedback;
pub mod scoring;
pub mod session;

use std::fmt;

use rand::prelude::*;
use rand::rngs::StdRng;

use crate::errors::WisecrowError;

use self::scoring::{apply_adaptation, should_terminate, AdaptationState};

const MIN_VOCAB_POOL_SIZE: usize = 8;
const MATCH_PROBABILITY: f64 = 0.30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnbMode {
    AudioWritten,
    WordTranslation,
    AudioImage,
}

impl DnbMode {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::AudioWritten => "audio_written",
            Self::WordTranslation => "word_translation",
            Self::AudioImage => "audio_image",
        }
    }
}

impl std::str::FromStr for DnbMode {
    type Err = WisecrowError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "audio_written" => Ok(Self::AudioWritten),
            "word_translation" => Ok(Self::WordTranslation),
            "audio_image" => Ok(Self::AudioImage),
            _ => Err(WisecrowError::InvalidInput(format!(
                "Unknown n-back mode: {s}. Valid: audio_written, word_translation, audio_image"
            ))),
        }
    }
}

impl fmt::Display for DnbMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct DnbConfig {
    pub mode: DnbMode,
    pub n_level: u8,
    pub interval_ms: u32,
}

impl Default for DnbConfig {
    fn default() -> Self {
        Self {
            mode: DnbMode::AudioWritten,
            n_level: 2,
            interval_ms: 4000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DnbVocab {
    pub translation_id: i32,
    pub from_phrase: String,
    pub to_phrase: String,
}

#[derive(Debug, Clone)]
pub struct Trial {
    pub trial_number: u32,
    pub n_level: u8,
    pub audio_vocab: DnbVocab,
    pub visual_vocab: DnbVocab,
    pub audio_match: bool,
    pub visual_match: bool,
    pub interval_ms: u32,
}

#[derive(Debug, Clone)]
pub struct TrialResponse {
    pub audio_response: Option<bool>,
    pub visual_response: Option<bool>,
    pub response_time_ms: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct CompletedTrial {
    pub trial: Trial,
    pub response: TrialResponse,
}

impl CompletedTrial {
    #[must_use]
    pub fn audio_correct(&self) -> bool {
        self.response.audio_response.unwrap_or(false) == self.trial.audio_match
    }

    #[must_use]
    pub fn visual_correct(&self) -> bool {
        self.response.visual_response.unwrap_or(false) == self.trial.visual_match
    }
}

pub struct DnbEngine {
    vocab_pool: Vec<DnbVocab>,
    audio_history: Vec<usize>,
    visual_history: Vec<usize>,
    completed: Vec<CompletedTrial>,
    state: AdaptationState,
    trial_counter: u32,
    rng: StdRng,
}

impl DnbEngine {
    /// Creates a new engine. Requires at least 8 vocabulary items.
    ///
    /// # Errors
    ///
    /// Returns an error if the vocabulary pool is too small.
    pub fn new(
        vocab_pool: Vec<DnbVocab>,
        config: &DnbConfig,
        seed: u64,
    ) -> Result<Self, WisecrowError> {
        if vocab_pool.len() < MIN_VOCAB_POOL_SIZE {
            return Err(WisecrowError::InvalidInput(format!(
                "Need at least {MIN_VOCAB_POOL_SIZE} vocabulary items, got {}",
                vocab_pool.len()
            )));
        }

        Ok(Self {
            vocab_pool,
            audio_history: Vec::new(),
            visual_history: Vec::new(),
            completed: Vec::new(),
            state: AdaptationState::new(config.n_level, config.interval_ms),
            trial_counter: 0,
            rng: StdRng::seed_from_u64(seed),
        })
    }

    #[must_use]
    pub fn next_trial(&mut self) -> Trial {
        let n = usize::from(self.state.n_level);

        let audio_can_match = self.audio_history.len() >= n;
        let visual_can_match = self.visual_history.len() >= n;

        let audio_match = audio_can_match && self.rng.gen_bool(MATCH_PROBABILITY);
        let visual_match = visual_can_match && self.rng.gen_bool(MATCH_PROBABILITY);

        let audio_avoid = self.n_back_index(&self.audio_history, n);
        let visual_avoid = self.n_back_index(&self.visual_history, n);

        let audio_idx = if audio_match {
            audio_avoid.expect("audio_match requires sufficient history")
        } else {
            self.pick_non_match(audio_avoid)
        };

        let visual_idx = if visual_match {
            visual_avoid.expect("visual_match requires sufficient history")
        } else {
            self.pick_non_match(visual_avoid)
        };

        self.audio_history.push(audio_idx);
        self.visual_history.push(visual_idx);
        self.trial_counter = self.trial_counter.saturating_add(1);

        Trial {
            trial_number: self.trial_counter,
            n_level: self.state.n_level,
            audio_vocab: self.vocab_pool[audio_idx].clone(), // clone: building owned Trial from pool reference
            visual_vocab: self.vocab_pool[visual_idx].clone(), // clone: building owned Trial from pool reference
            audio_match,
            visual_match,
            interval_ms: self.state.interval_ms,
        }
    }

    pub fn record_response(&mut self, response: TrialResponse) {
        let trial_idx = self.completed.len();
        let trial = Trial {
            trial_number: u32::try_from(trial_idx.saturating_add(1)).unwrap_or(u32::MAX),
            n_level: self.state.n_level,
            audio_vocab: self.vocab_pool[self.audio_history[trial_idx]].clone(), // clone: reconstructing Trial from history for storage
            visual_vocab: self.vocab_pool[self.visual_history[trial_idx]].clone(), // clone: reconstructing Trial from history for storage
            audio_match: self.was_audio_match(trial_idx),
            visual_match: self.was_visual_match(trial_idx),
            interval_ms: self.state.interval_ms,
        };

        self.completed.push(CompletedTrial { trial, response });
        apply_adaptation(&mut self.state, &self.completed);
    }

    #[must_use]
    pub fn should_terminate(&self) -> bool {
        should_terminate(&self.state, &self.completed)
    }

    #[must_use]
    pub fn state(&self) -> &AdaptationState {
        &self.state
    }

    #[must_use]
    pub fn completed_trials(&self) -> &[CompletedTrial] {
        &self.completed
    }

    #[must_use]
    pub fn trial_count(&self) -> u32 {
        self.trial_counter
    }

    fn n_back_index(&self, history: &[usize], n: usize) -> Option<usize> {
        if history.len() >= n {
            Some(history[history.len().saturating_sub(n)])
        } else {
            None
        }
    }

    fn pick_non_match(&mut self, avoid: Option<usize>) -> usize {
        let pool_len = self.vocab_pool.len();
        loop {
            let idx = self.rng.gen_range(0..pool_len);
            if Some(idx) != avoid {
                return idx;
            }
        }
    }

    fn was_audio_match(&self, trial_idx: usize) -> bool {
        let n = usize::from(self.state.n_level);
        if trial_idx < n {
            return false;
        }
        self.audio_history[trial_idx] == self.audio_history[trial_idx.saturating_sub(n)]
    }

    fn was_visual_match(&self, trial_idx: usize) -> bool {
        let n = usize::from(self.state.n_level);
        if trial_idx < n {
            return false;
        }
        self.visual_history[trial_idx] == self.visual_history[trial_idx.saturating_sub(n)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn make_vocab(count: usize) -> Vec<DnbVocab> {
        (0..i32::try_from(count).unwrap_or(i32::MAX))
            .map(|i| DnbVocab {
                translation_id: i,
                from_phrase: format!("word_{i}"),
                to_phrase: format!("trans_{i}"),
            })
            .collect()
    }

    #[test]
    fn rejects_small_vocab_pool() {
        let vocab = make_vocab(3);
        let config = DnbConfig::default();
        assert!(DnbEngine::new(vocab, &config, 42).is_err());
    }

    #[test]
    fn first_n_trials_have_no_matches() {
        let vocab = make_vocab(20);
        let config = DnbConfig {
            n_level: 3,
            ..DnbConfig::default()
        };
        let mut engine = DnbEngine::new(vocab, &config, 42).unwrap();

        for _ in 0..3 {
            let trial = engine.next_trial();
            assert!(!trial.audio_match);
            assert!(!trial.visual_match);
        }
    }

    proptest! {
        #[test]
        fn generated_trials_have_valid_vocab(seed in 0u64..10000) {
            let vocab = make_vocab(20);
            let pool_len = vocab.len();
            let config = DnbConfig::default();
            let mut engine = DnbEngine::new(vocab, &config, seed)?;

            for _ in 0..50 {
                let trial = engine.next_trial();
                prop_assert!(trial.audio_vocab.translation_id >= 0);
                prop_assert!((trial.audio_vocab.translation_id as usize) < pool_len);
                prop_assert!(trial.visual_vocab.translation_id >= 0);
                prop_assert!((trial.visual_vocab.translation_id as usize) < pool_len);
            }
        }

        #[test]
        fn match_trials_reuse_n_back_item(seed in 0u64..10000) {
            let vocab = make_vocab(20);
            let config = DnbConfig { n_level: 2, ..DnbConfig::default() };
            let mut engine = DnbEngine::new(vocab, &config, seed)?;

            let mut audio_ids = Vec::new();
            let mut visual_ids = Vec::new();

            for _ in 0..30 {
                let trial = engine.next_trial();
                audio_ids.push(trial.audio_vocab.translation_id);
                visual_ids.push(trial.visual_vocab.translation_id);

                let idx = audio_ids.len().saturating_sub(1);
                if trial.audio_match && idx >= 2 {
                    prop_assert_eq!(audio_ids[idx], audio_ids[idx.saturating_sub(2)]);
                }
                if trial.visual_match && idx >= 2 {
                    prop_assert_eq!(visual_ids[idx], visual_ids[idx.saturating_sub(2)]);
                }
            }
        }
    }
}
