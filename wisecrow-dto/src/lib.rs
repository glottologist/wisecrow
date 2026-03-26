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
pub struct ClozeQuizDto {
    pub sentence_with_blank: String,
    pub answer: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleChoiceQuizDto {
    pub question: String,
    pub options: Vec<String>,
    pub correct_index: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum QuizItemDto {
    Cloze(ClozeQuizDto),
    MultipleChoice(MultipleChoiceQuizDto),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageInfo {
    pub code: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub cards_seen: usize,
    pub total: usize,
    pub streak: usize,
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
