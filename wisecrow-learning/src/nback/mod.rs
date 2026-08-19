//! Dual N-Back domain contracts.

pub mod scoring;

use std::{fmt, str::FromStr, sync::Arc};

use rand::prelude::*;
use rand::rngs::StdRng;

use crate::LearningError;
use scoring::{apply_adaptation, should_terminate, AdaptationState};

const MIN_VOCAB_POOL_SIZE: usize = 8;
const MATCH_PROBABILITY: f64 = 0.30;

/// Presentation mode for a Dual N-Back session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnbMode {
    AudioWritten,
    WordTranslation,
    AudioImage,
}

impl DnbMode {
    /// Returns the stable persistence representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AudioWritten => "audio_written",
            Self::WordTranslation => "word_translation",
            Self::AudioImage => "audio_image",
        }
    }
}

impl fmt::Display for DnbMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DnbMode {
    type Err = LearningError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "audio_written" => Ok(Self::AudioWritten),
            "word_translation" => Ok(Self::WordTranslation),
            "audio_image" => Ok(Self::AudioImage),
            _ => Err(LearningError::InvalidNbackMode),
        }
    }
}

/// Starting configuration for a Dual N-Back session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DnbConfig {
    pub mode: DnbMode,
    pub n_level: u8,
    pub interval_ms: u32,
}

impl DnbConfig {
    /// Creates a session configuration.
    #[must_use]
    pub const fn new(mode: DnbMode, n_level: u8, interval_ms: u32) -> Self {
        Self {
            mode,
            n_level,
            interval_ms,
        }
    }
}

impl Default for DnbConfig {
    fn default() -> Self {
        Self::new(DnbMode::AudioWritten, 2, 4_000)
    }
}

/// One vocabulary item available to the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnbVocab {
    pub translation_id: i32,
    pub from_phrase: String,
    pub to_phrase: String,
}

impl DnbVocab {
    /// Creates an owned vocabulary item from convertible phrases.
    #[must_use]
    pub fn new(
        translation_id: i32,
        from_phrase: impl Into<String>,
        to_phrase: impl Into<String>,
    ) -> Self {
        Self {
            translation_id,
            from_phrase: from_phrase.into(),
            to_phrase: to_phrase.into(),
        }
    }
}

/// One generated Dual N-Back stimulus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trial {
    pub trial_number: u32,
    pub n_level: u8,
    pub audio_vocab: DnbVocab,
    pub visual_vocab: DnbVocab,
    pub audio_match: bool,
    pub visual_match: bool,
    pub interval_ms: u32,
}

/// Button state recorded for one trial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialResponse {
    pub audio_response: Option<bool>,
    pub visual_response: Option<bool>,
    pub response_time_ms: Option<u32>,
}

impl TrialResponse {
    /// Creates a response with neither match button selected.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            audio_response: None,
            visual_response: None,
            response_time_ms: None,
        }
    }
}

/// A stimulus paired with its recorded response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedTrial {
    pub trial: Trial,
    pub response: TrialResponse,
}

impl CompletedTrial {
    /// Reports whether the audio response matches the stimulus.
    #[must_use]
    pub fn audio_correct(&self) -> bool {
        self.response.audio_response.unwrap_or(false) == self.trial.audio_match
    }

    /// Reports whether the visual response matches the stimulus.
    #[must_use]
    pub fn visual_correct(&self) -> bool {
        self.response.visual_response.unwrap_or(false) == self.trial.visual_match
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GeneratedTrial {
    trial_number: u32,
    n_level: u8,
    audio_index: usize,
    visual_index: usize,
    audio_match: bool,
    visual_match: bool,
    interval_ms: u32,
}

/// Seeded Dual N-Back state machine.
pub struct DnbEngine {
    vocab: Arc<[DnbVocab]>,
    mode: DnbMode,
    seed: u64,
    state: AdaptationState,
    completed: Vec<CompletedTrial>,
    generated: Vec<GeneratedTrial>,
    audio_history: Vec<usize>,
    visual_history: Vec<usize>,
    trial_count: u32,
    rng: StdRng,
}

impl DnbEngine {
    /// Creates a seeded engine with shared immutable vocabulary.
    ///
    /// # Errors
    ///
    /// Returns [`LearningError::InsufficientVocabulary`] for fewer than eight items.
    pub fn new(
        vocab: impl Into<Arc<[DnbVocab]>>,
        config: &DnbConfig,
        seed: u64,
    ) -> Result<Self, LearningError> {
        let vocab = vocab.into();
        if vocab.len() < MIN_VOCAB_POOL_SIZE {
            return Err(LearningError::InsufficientVocabulary);
        }

        Ok(Self {
            vocab,
            mode: config.mode,
            seed,
            state: AdaptationState::new(config.n_level, config.interval_ms),
            completed: Vec::new(),
            generated: Vec::new(),
            audio_history: Vec::new(),
            visual_history: Vec::new(),
            trial_count: 0,
            rng: StdRng::seed_from_u64(seed),
        })
    }

    /// Generates the next deterministic trial.
    ///
    /// # Errors
    ///
    /// Returns a typed learning error if the engine state is inconsistent.
    pub fn next_trial(&mut self) -> Result<Trial, LearningError> {
        let generated = self.generate_trial()?;
        let trial = self.materialize_trial(&generated)?;
        self.generated.push(generated);
        self.trial_count = self.trial_count.saturating_add(1);
        Ok(trial)
    }

    /// Records the response for the exact expected trial.
    ///
    /// # Errors
    ///
    /// Returns [`LearningError::UnexpectedTrial`] when the supplied trial is not
    /// the next generated trial awaiting a response.
    pub fn record_response(
        &mut self,
        trial: &Trial,
        response: TrialResponse,
    ) -> Result<(), LearningError> {
        let Some(expected) = self.generated.get(self.completed.len()) else {
            return Err(LearningError::UnexpectedTrial);
        };
        if !self.matches_trial(expected, trial) {
            return Err(LearningError::UnexpectedTrial);
        }

        self.completed.push(CompletedTrial {
            trial: trial.clone(), // clone: completed history must own the caller-retained trial
            response,
        });
        apply_adaptation(&mut self.state, &self.completed);
        Ok(())
    }

    /// Reports whether the session has reached a termination boundary.
    #[must_use]
    pub fn should_terminate(&self) -> bool {
        should_terminate(&self.state, &self.completed)
    }

    /// Returns the vocabulary shared by this engine.
    #[must_use]
    pub fn vocabulary(&self) -> &[DnbVocab] {
        &self.vocab
    }

    /// Returns the configured presentation mode.
    #[must_use]
    pub const fn mode(&self) -> DnbMode {
        self.mode
    }

    /// Returns the deterministic generation seed.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Returns the current adaptive state.
    #[must_use]
    pub const fn state(&self) -> &AdaptationState {
        &self.state
    }

    /// Returns every durably recorded trial.
    #[must_use]
    pub fn completed_trials(&self) -> &[CompletedTrial] {
        &self.completed
    }

    /// Consumes the engine and returns its completed trial history.
    #[must_use]
    pub fn into_completed_trials(self) -> Vec<CompletedTrial> {
        self.completed
    }

    /// Returns the number of generated trials.
    #[must_use]
    pub const fn trial_count(&self) -> u32 {
        self.trial_count
    }

    fn generate_trial(&mut self) -> Result<GeneratedTrial, LearningError> {
        let n = usize::from(self.state.n_level);
        let audio_avoid = n_back_index(&self.audio_history, n);
        let visual_avoid = n_back_index(&self.visual_history, n);
        let audio_match = audio_avoid.is_some() && self.rng.gen_bool(MATCH_PROBABILITY);
        let visual_match = visual_avoid.is_some() && self.rng.gen_bool(MATCH_PROBABILITY);
        let audio_index = select_index(&mut self.rng, self.vocab.len(), audio_match, audio_avoid)?;
        let visual_index =
            select_index(&mut self.rng, self.vocab.len(), visual_match, visual_avoid)?;
        self.audio_history.push(audio_index);
        self.visual_history.push(visual_index);

        Ok(GeneratedTrial {
            trial_number: self.trial_count.saturating_add(1),
            n_level: self.state.n_level,
            audio_index,
            visual_index,
            audio_match,
            visual_match,
            interval_ms: self.state.interval_ms,
        })
    }

    fn materialize_trial(&self, generated: &GeneratedTrial) -> Result<Trial, LearningError> {
        let audio_vocab = self
            .vocab
            .get(generated.audio_index)
            .ok_or(LearningError::UnexpectedTrial)?;
        let visual_vocab = self
            .vocab
            .get(generated.visual_index)
            .ok_or(LearningError::UnexpectedTrial)?;
        Ok(Trial {
            trial_number: generated.trial_number,
            n_level: generated.n_level,
            audio_vocab: audio_vocab.clone(), // clone: trial owns stimulus from shared vocabulary
            visual_vocab: visual_vocab.clone(), // clone: trial owns stimulus from shared vocabulary
            audio_match: generated.audio_match,
            visual_match: generated.visual_match,
            interval_ms: generated.interval_ms,
        })
    }

    fn matches_trial(&self, generated: &GeneratedTrial, trial: &Trial) -> bool {
        let Some(audio_vocab) = self.vocab.get(generated.audio_index) else {
            return false;
        };
        let Some(visual_vocab) = self.vocab.get(generated.visual_index) else {
            return false;
        };

        trial.trial_number == generated.trial_number
            && trial.n_level == generated.n_level
            && trial.audio_vocab == *audio_vocab
            && trial.visual_vocab == *visual_vocab
            && trial.audio_match == generated.audio_match
            && trial.visual_match == generated.visual_match
            && trial.interval_ms == generated.interval_ms
    }
}

fn n_back_index(history: &[usize], n: usize) -> Option<usize> {
    history
        .len()
        .checked_sub(n)
        .and_then(|index| history.get(index).copied())
}

fn select_index(
    rng: &mut StdRng,
    pool_len: usize,
    is_match: bool,
    avoid: Option<usize>,
) -> Result<usize, LearningError> {
    if is_match {
        return avoid.ok_or(LearningError::UnexpectedTrial);
    }
    loop {
        let index = rng.gen_range(0..pool_len);
        if Some(index) != avoid {
            return Ok(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn vocabulary(count: usize) -> Arc<[DnbVocab]> {
        (0..count)
            .map(|index| {
                let id = i32::try_from(index).expect("bounded count");
                DnbVocab::new(id, format!("from-{id}"), format!("to-{id}"))
            })
            .collect::<Vec<_>>()
            .into()
    }

    #[test]
    fn rejects_small_vocabulary_pool() {
        let config = DnbConfig::default();
        assert!(matches!(
            DnbEngine::new(vocabulary(7), &config, 42),
            Err(LearningError::InsufficientVocabulary)
        ));
    }

    #[test]
    fn first_n_trials_have_no_matches() {
        let config = DnbConfig::new(DnbMode::AudioWritten, 3, 4_000);
        let mut engine = DnbEngine::new(vocabulary(20), &config, 42).expect("valid engine");

        for _ in 0..3 {
            let trial = engine.next_trial().expect("trial");
            assert!(!trial.audio_match);
            assert!(!trial.visual_match);
        }
    }

    #[test]
    fn response_must_match_next_generated_trial() {
        let config = DnbConfig::default();
        let mut engine = DnbEngine::new(vocabulary(20), &config, 42).expect("valid engine");
        let mut trial = engine.next_trial().expect("trial");
        trial.trial_number = trial.trial_number.saturating_add(1);

        assert_eq!(
            engine.record_response(&trial, TrialResponse::none()),
            Err(LearningError::UnexpectedTrial)
        );
    }

    proptest! {
        #[test]
        fn generated_trials_reference_only_supplied_vocabulary(
            seed in any::<u64>(),
            count in 8usize..64,
        ) {
            let vocab: Arc<[DnbVocab]> = (0..count)
                .map(|index| {
                    let id = i32::try_from(index).expect("bounded count");
                    DnbVocab::new(id, format!("from-{id}"), format!("to-{id}"))
                })
                .collect::<Vec<_>>()
                .into();
            let allowed: std::collections::HashSet<i32> =
                vocab.iter().map(|item| item.translation_id).collect();
            let config = DnbConfig::new(DnbMode::AudioWritten, 2, 4_000);
            let mut engine = DnbEngine::new(vocab, &config, seed).expect("valid engine");

            for _ in 0..50 {
                let trial = engine.next_trial().expect("trial");
                prop_assert!(allowed.contains(&trial.audio_vocab.translation_id));
                prop_assert!(allowed.contains(&trial.visual_vocab.translation_id));
                engine
                    .record_response(&trial, TrialResponse::none())
                    .expect("response");
            }
        }

        #[test]
        fn match_trials_reuse_n_back_item(seed in any::<u64>()) {
            let config = DnbConfig::new(DnbMode::AudioWritten, 2, 4_000);
            let mut engine = DnbEngine::new(vocabulary(20), &config, seed)
                .expect("valid engine");
            let mut audio_ids = Vec::new();
            let mut visual_ids = Vec::new();

            for _ in 0..30 {
                let trial = engine.next_trial().expect("trial");
                audio_ids.push(trial.audio_vocab.translation_id);
                visual_ids.push(trial.visual_vocab.translation_id);
                let index = audio_ids.len().saturating_sub(1);
                if trial.audio_match {
                    prop_assert_eq!(audio_ids[index], audio_ids[index.saturating_sub(2)]);
                }
                if trial.visual_match {
                    prop_assert_eq!(visual_ids[index], visual_ids[index.saturating_sub(2)]);
                }
            }
        }
    }
}
