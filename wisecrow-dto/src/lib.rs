use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardDto {
    pub card_id: i32,
    pub translation_id: i32,
    pub from_phrase: String,
    pub to_phrase: String,
    pub frequency: i32,
    pub stability: f64,
    pub difficulty: f64,
    pub state: CardStatusDto,
    pub due: DateTime<Utc>,
    pub reps: i32,
    pub lapses: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardStatusDto {
    New,
    Learning,
    Review,
    Relearning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewRatingDto {
    Again,
    Hard,
    Good,
    Easy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDto {
    pub id: i32,
    pub native_lang: String,
    pub foreign_lang: String,
    pub deck_size: i32,
    pub speed_ms: i32,
    pub current_index: i32,
    pub cards: Vec<CardDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserDto {
    pub id: i32,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClozeQuizDto {
    pub sentence_with_blank: String,
    pub answer: String,
    pub hint: Option<String>,
    pub rule_context: Option<RuleContextDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleChoiceQuizDto {
    pub question: String,
    pub options: Vec<String>,
    pub correct_index: usize,
    pub rule_context: Option<RuleContextDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleContextDto {
    pub rule_title: String,
    pub rule_explanation: String,
    pub cefr_level: String,
    pub extra_examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QuizItemDto {
    Cloze(ClozeQuizDto),
    MultipleChoice(MultipleChoiceQuizDto),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptDirection {
    Ltr,
    Rtl,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageInfo {
    pub code: String,
    pub name: String,
    pub script_direction: ScriptDirection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub cards_seen: usize,
    pub total: usize,
    pub streak: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CefrLevelDto {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrammarRuleDto {
    pub id: i32,
    pub title: String,
    pub explanation: String,
    pub cefr_level: String,
    pub source: String,
    pub examples: Vec<RuleExampleDto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleExampleDto {
    pub sentence: String,
    pub translation: Option<String>,
    pub is_correct: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrammarRuleImport {
    pub title: String,
    pub explanation: String,
    pub cefr_level: String,
    pub examples: Vec<RuleExampleImport>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuleExampleImport {
    pub sentence: String,
    pub translation: Option<String>,
    #[serde(default = "default_true")]
    pub is_correct: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncLanguageDto {
    pub id: i32,
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncTranslationDto {
    pub id: i32,
    pub from_language_code: String,
    pub from_phrase: String,
    pub to_language_code: String,
    pub to_phrase: String,
    pub frequency: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncGrammarRuleDto {
    pub id: i32,
    pub language_code: String,
    pub cefr_level_code: String,
    pub title: String,
    pub explanation: String,
    pub source: String,
    pub examples: Vec<SyncRuleExampleDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRuleExampleDto {
    pub sentence: String,
    pub translation: Option<String>,
    pub is_correct: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncProgressDto {
    pub table: String,
    pub synced: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnbModeDto {
    AudioWritten,
    WordTranslation,
    AudioImage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnbConfigDto {
    pub mode: DnbModeDto,
    pub n_level: u8,
    pub interval_ms: u32,
    pub native_lang: String,
    pub foreign_lang: String,
    pub user_id: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnbTrialDto {
    pub trial_number: u32,
    pub n_level: u8,
    pub audio_phrase: String,
    pub visual_phrase: String,
    pub audio_match: bool,
    pub visual_match: bool,
    pub interval_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnbTrialResultDto {
    pub trial_number: u32,
    pub audio_response: Option<bool>,
    pub visual_response: Option<bool>,
    pub response_time_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnbAdaptationDto {
    pub new_n_level: u8,
    pub new_interval_ms: u32,
    pub audio_accuracy: f32,
    pub visual_accuracy: f32,
    pub should_terminate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DnbSessionResultsDto {
    pub session_id: i32,
    pub mode: DnbModeDto,
    pub n_level_start: u8,
    pub n_level_peak: u8,
    pub n_level_end: u8,
    pub trials_completed: u32,
    pub accuracy_audio: Option<f32>,
    pub accuracy_visual: Option<f32>,
    pub interval_ms_start: u32,
    pub interval_ms_end: u32,
}

const MIN_SPEED_MS: u32 = 500;
const MAX_SPEED_MS: u32 = 10_000;
const SPEED_STEP_MS: u32 = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedController {
    interval_ms: u32,
    remaining_ms: u32,
    paused: bool,
}

impl SpeedController {
    #[must_use]
    pub fn new(interval_ms: u32) -> Self {
        let clamped = interval_ms.clamp(MIN_SPEED_MS, MAX_SPEED_MS);
        Self {
            interval_ms: clamped,
            remaining_ms: clamped,
            paused: false,
        }
    }

    /// Advances the timer by `elapsed_ms`. Returns `true` if the timer expired.
    pub fn tick(&mut self, elapsed_ms: u32) -> bool {
        if self.paused {
            return false;
        }
        self.remaining_ms = self.remaining_ms.saturating_sub(elapsed_ms);
        self.remaining_ms == 0
    }

    pub fn reset(&mut self) {
        self.remaining_ms = self.interval_ms;
    }

    pub fn speed_up(&mut self) {
        self.interval_ms = self
            .interval_ms
            .saturating_sub(SPEED_STEP_MS)
            .max(MIN_SPEED_MS);
    }

    pub fn slow_down(&mut self) {
        self.interval_ms = self
            .interval_ms
            .saturating_add(SPEED_STEP_MS)
            .min(MAX_SPEED_MS);
    }

    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn unpause(&mut self) {
        self.paused = false;
    }

    #[must_use]
    pub const fn is_paused(&self) -> bool {
        self.paused
    }

    #[must_use]
    pub fn remaining_fraction(&self) -> f64 {
        if self.interval_ms == 0 {
            return 0.0;
        }
        f64::from(self.remaining_ms) / f64::from(self.interval_ms)
    }

    #[must_use]
    pub const fn interval_ms(&self) -> u32 {
        self.interval_ms
    }

    #[must_use]
    pub const fn remaining_ms(&self) -> u32 {
        self.remaining_ms
    }
}
