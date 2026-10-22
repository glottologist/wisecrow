use chrono::{DateTime, NaiveDate, Utc};
use num_traits::ToPrimitive;
use serde::{Deserialize, Serialize};

pub mod grammar;
pub mod mobile;
pub mod reader;

pub use grammar::*;
pub use mobile::*;
pub use reader::*;

/// Fraction `correct / total` in `[0.0, 1.0]`, returning `0.0` when `total` is
/// zero. Shared by the web and mobile n-back views so the guarded division and
/// its cast live in one place.
#[must_use]
pub fn channel_ratio(correct: u32, total: u32) -> f32 {
    if total == 0 {
        return 0.0;
    }
    let (Some(correct), Some(total)) = (correct.to_f32(), total.to_f32()) else {
        return 0.0;
    };
    correct / total
}

/// Largest deck a learning or fast run may request.
pub const MAX_DECK_SIZE: u32 = 500;

/// One item in a passive fast-mode deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FastCardDto {
    pub translation_id: i32,
    pub from_phrase: String,
    pub to_phrase: String,
    pub frequency: i32,
    /// Phrases get audio only; image queries are word-shaped.
    pub image_allowed: bool,
}

/// A passive fast-mode deck: no session row, no SRS state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FastDeckDto {
    pub cards: Vec<FastCardDto>,
}

/// A card image and the credit its licence requires on display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardImageDto {
    /// `data:image/jpeg;base64,…`
    pub data_url: String,
    pub attribution: Option<String>,
}

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
    /// A promoted phrase; phrase cards get audio but no image.
    /// `default` keeps payloads from older servers deserialising.
    #[serde(default)]
    pub is_phrase: bool,
    /// Whether the canonical presentation has a concrete image query.
    #[serde(default)]
    pub image_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CardStatusDto {
    New,
    Learning,
    Review,
    Relearning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDto {
    pub id: i32,
    pub native_lang: String,
    pub foreign_lang: String,
    pub deck_size: i32,
    pub speed_ms: i32,
    pub current_index: i32,
    #[serde(default)]
    pub deck_id: Option<i32>,
    pub cards: Vec<CardDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDto {
    pub id: i32,
    pub display_name: String,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileSessionDto {
    pub token: String,
    pub user: UserDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClozeQuizDto {
    pub sentence_with_blank: String,
    pub answer: String,
    pub hint: Option<String>,
    pub rule_context: Option<RuleContextDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultipleChoiceQuizDto {
    pub question: String,
    pub options: Vec<String>,
    pub correct_index: usize,
    pub rule_context: Option<RuleContextDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleContextDto {
    pub rule_title: String,
    pub rule_explanation: String,
    pub cefr_level: String,
    pub extra_examples: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuizItemDto {
    Cloze(ClozeQuizDto),
    MultipleChoice(MultipleChoiceQuizDto),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptDirection {
    Ltr,
    Rtl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageInfo {
    pub code: String,
    pub name: String,
    pub script_direction: ScriptDirection,
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

const fn default_true() -> bool {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnbModeDto {
    AudioWritten,
    WordTranslation,
    AudioImage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlossaryEntryDto {
    pub word: String,
    pub translation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradedReaderDto {
    pub passage: String,
    pub glossary: Vec<GlossaryEntryDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenStatusDto {
    Known,
    Learning,
    New,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotatedTokenDto {
    pub token: String,
    pub frequency: Option<i32>,
    pub status: TokenStatusDto,
    pub llm_translation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubtitleFormatDto {
    Srt,
    Vtt,
    Ass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnbConfigDto {
    pub mode: DnbModeDto,
    pub n_level: u8,
    pub interval_ms: u32,
    pub native_lang: String,
    pub foreign_lang: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DnbTrialDto {
    pub trial_number: u32,
    pub n_level: u8,
    pub audio_translation_id: i32,
    pub visual_translation_id: i32,
    pub audio_phrase: String,
    pub visual_phrase: String,
    pub audio_match: bool,
    pub visual_match: bool,
    pub interval_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Everything the progress dashboard shows, one entry per foreign language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardDto {
    pub languages: Vec<LanguageProgressDto>,
}

/// One foreign language's progress.
///
/// Grammar and placement belong to the language as a whole, because grammar is
/// recorded without a native language. Vocabulary and n-back are grouped by the
/// native language they were practised from, so two native languages learning
/// one target are both shown and neither is assumed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageProgressDto {
    pub foreign: String,
    pub foreign_name: String,
    pub vocabulary: Vec<PairVocabularyDto>,
    pub grammar: Vec<LevelBandsDto>,
    pub placement: Option<PlacementSummaryDto>,
    pub nback: Vec<PairNbackDto>,
    /// Books the learner has opened in this language. Absent when the reader
    /// tables could not be counted; zeros still count as present.
    #[serde(default)]
    pub books: Option<BookProgressDto>,
}

/// One native language's flashcard progress in a foreign language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairVocabularyDto {
    pub native: String,
    pub counts: VocabularyCountsDto,
    /// One entry per UTC day, oldest first, ending today.
    pub reviews: Vec<DailyCountDto>,
    /// Conjugated forms stored for this pair. Zeros when the pair has none.
    pub verb_forms: VerbFormProgressDto,
}

/// Stored verb forms for one language pair.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerbFormProgressDto {
    pub lemmas: i64,
    pub forms: i64,
    pub attested: i64,
    pub voiced: i64,
}

/// Cards per FSRS state, with those due before the end of the UTC day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularyCountsDto {
    pub new: i64,
    pub learning: i64,
    pub review: i64,
    pub relearning: i64,
    pub due_today: i64,
}

/// Reviews made on one UTC day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyCountDto {
    pub day: NaiveDate,
    pub count: i64,
}

/// Grammar points of one CEFR level per brainmap band.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelBandsDto {
    pub level: String,
    pub unseen: u32,
    pub red: u32,
    pub amber: u32,
    pub green: u32,
    pub provisional_red: u32,
    pub provisional_amber: u32,
    pub provisional_green: u32,
}

impl LevelBandsDto {
    /// Each band with its count, settled bands before provisional ones and
    /// unseen last, which is the order the dashboard draws them in.
    #[must_use]
    pub const fn counts(&self) -> [(MasteryBandDto, u32); 7] {
        [
            (MasteryBandDto::Green, self.green),
            (MasteryBandDto::Amber, self.amber),
            (MasteryBandDto::Red, self.red),
            (MasteryBandDto::ProvisionalGreen, self.provisional_green),
            (MasteryBandDto::ProvisionalAmber, self.provisional_amber),
            (MasteryBandDto::ProvisionalRed, self.provisional_red),
            (MasteryBandDto::Unseen, self.unseen),
        ]
    }
}

/// The latest finished placement run for a language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementSummaryDto {
    pub completed_at: DateTime<Utc>,
    /// `None` where the run passed no level, which is a result rather than an
    /// absence of one.
    pub level_reached: Option<String>,
    pub tested: Vec<String>,
    /// Levels whose bank was too thin to test. Untested is not failed.
    pub skipped: Vec<String>,
}

/// Dual n-back practised from one native language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairNbackDto {
    pub native: String,
    /// The highest n reached in a finished session with answered trials: a
    /// ceiling the learner touched, not a level they have mastered.
    pub highest_n: i16,
    pub sessions: i64,
}

const MIN_SPEED_MS: u32 = 500;
const MAX_SPEED_MS: u32 = 10_000;
const SPEED_STEP_MS: u32 = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub const fn tick(&mut self, elapsed_ms: u32) -> bool {
        if self.paused {
            return false;
        }
        self.remaining_ms = self.remaining_ms.saturating_sub(elapsed_ms);
        self.remaining_ms == 0
    }

    pub const fn reset(&mut self) {
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

    pub const fn pause(&mut self) {
        self.paused = true;
    }

    pub const fn unpause(&mut self) {
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

#[cfg(test)]
mod tests {
    use super::{
        channel_ratio, DailyCountDto, DashboardDto, LanguageProgressDto, LevelBandsDto,
        MasteryBandDto, PairNbackDto, PairVocabularyDto, PlacementSummaryDto, VerbFormProgressDto,
        VocabularyCountsDto,
    };
    use chrono::{DateTime, NaiveDate, Utc};

    #[test]
    fn channel_ratio_guards_zero_and_computes_fraction() {
        assert!((channel_ratio(0, 0) - 0.0).abs() < f32::EPSILON);
        assert!((channel_ratio(3, 0) - 0.0).abs() < f32::EPSILON);
        assert!((channel_ratio(1, 2) - 0.5).abs() < f32::EPSILON);
        assert!((channel_ratio(4, 4) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_dashboard_survives_the_wire() {
        let dashboard = DashboardDto {
            languages: vec![LanguageProgressDto {
                foreign: "es".into(),
                foreign_name: "Spanish".into(),
                vocabulary: vec![PairVocabularyDto {
                    native: "en".into(),
                    counts: VocabularyCountsDto {
                        new: 1,
                        ..VocabularyCountsDto::default()
                    },
                    reviews: vec![DailyCountDto {
                        day: NaiveDate::from_ymd_opt(2026, 10, 20).expect("date"),
                        count: 3,
                    }],
                    verb_forms: VerbFormProgressDto::default(),
                }],
                grammar: vec![LevelBandsDto {
                    level: "A1".into(),
                    green: 2,
                    ..LevelBandsDto::default()
                }],
                placement: Some(PlacementSummaryDto {
                    completed_at: DateTime::<Utc>::from_timestamp(1_760_918_400, 0).expect("time"),
                    level_reached: None,
                    tested: vec!["A1".into()],
                    skipped: vec!["A2".into()],
                }),
                nback: vec![PairNbackDto {
                    native: "en".into(),
                    highest_n: 4,
                    sessions: 1,
                }],
                books: None,
            }],
        };

        let json = serde_json::to_string(&dashboard).expect("serialise");
        let back: DashboardDto = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, dashboard);
    }

    #[test]
    fn level_bands_list_every_band_once_settled_first() {
        let level = LevelBandsDto {
            level: "B1".into(),
            unseen: 1,
            red: 2,
            amber: 3,
            green: 4,
            provisional_red: 5,
            provisional_amber: 6,
            provisional_green: 7,
        };
        let counts = level.counts();
        assert_eq!(counts.iter().map(|(_, count)| count).sum::<u32>(), 28);
        assert_eq!(counts[0], (MasteryBandDto::Green, 4));
        assert_eq!(counts[6], (MasteryBandDto::Unseen, 1));
    }
}
