# wisecrow-dto

`wisecrow-dto` is the small types-only crate shared between the server, the
web UI, and the mobile shell. It depends only on `chrono` and `serde` so it
can be compiled to WASM without dragging in a Tokio runtime.

## When to depend on it

- You are writing a Dioxus client that calls server-functions.
- You need to deserialize Wisecrow JSON payloads from another tool.
- You want to keep your binary small and cannot afford `wisecrow-core`'s
  dependency footprint.

If you also need the database, scheduling, or LLM logic, depend on
`wisecrow-core` instead — it re-exports `wisecrow-dto` indirectly via
`dto_convert`.

## Cards and reviews

```rust,ignore
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

pub enum CardStatusDto { New, Learning, Review, Relearning }
pub enum ReviewRatingDto { Again, Hard, Good, Easy }
```

`CardDto` is a snapshot — once mutated server-side, expect a fresh
`CardDto` back from the next call. Do not patch in place.

## Sessions

```rust,ignore
pub struct SessionDto {
    pub id: i32,
    pub native_lang: String,
    pub foreign_lang: String,
    pub deck_size: i32,
    pub speed_ms: i32,
    pub current_index: i32,
    pub cards: Vec<CardDto>,
}

pub struct SessionSummary {
    pub cards_seen: usize,
    pub total: usize,
    pub streak: usize,
}
```

## Quizzes

```rust,ignore
pub struct ClozeQuizDto {
    pub sentence_with_blank: String,
    pub answer: String,
    pub hint: Option<String>,
    pub rule_context: Option<RuleContextDto>,
}

pub struct MultipleChoiceQuizDto {
    pub question: String,
    pub options: Vec<String>,
    pub correct_index: usize,
    pub rule_context: Option<RuleContextDto>,
}

pub struct RuleContextDto {
    pub rule_title: String,
    pub rule_explanation: String,
    pub cefr_level: String,
    pub extra_examples: Vec<String>,
}

pub enum QuizItemDto {
    Cloze(ClozeQuizDto),
    MultipleChoice(MultipleChoiceQuizDto),
}
```

`rule_context` is filled in for quizzes generated from stored grammar rules
and left `None` for quizzes generated ad-hoc (for example via
`wisecrow quiz`).

## Languages and users

```rust,ignore
pub struct LanguageInfo {
    pub code: String,
    pub name: String,
    pub script_direction: ScriptDirection,
}

pub enum ScriptDirection { Ltr, Rtl }

pub struct UserDto {
    pub id: i32,
    pub display_name: String,
}
```

`ScriptDirection::Rtl` is set for `ar`, `he`, `fa`, `ur`, `ps`, `sd`, `yi`.
The list lives in `wisecrow_core::dto_convert::RTL_LANGUAGES`.

## Grammar rules

```rust,ignore
pub struct CefrLevelDto { pub code: String, pub name: String }

pub struct GrammarRuleDto {
    pub id: i32,
    pub title: String,
    pub explanation: String,
    pub cefr_level: String,
    pub source: String,
    pub examples: Vec<RuleExampleDto>,
}

pub struct RuleExampleDto {
    pub sentence: String,
    pub translation: Option<String>,
    pub is_correct: bool,
}

pub struct GrammarRuleImport {
    pub title: String,
    pub explanation: String,
    pub cefr_level: String,
    pub examples: Vec<RuleExampleImport>,
}

pub struct RuleExampleImport {
    pub sentence: String,
    pub translation: Option<String>,
    pub is_correct: bool,   // serde default = true
}
```

`GrammarRuleImport` is the file format consumed by
`wisecrow import-grammar --file rules.json`.

## Sync DTOs

```rust,ignore
pub struct SyncLanguageDto    { pub id: i32, pub code: String, pub name: String }
pub struct SyncTranslationDto { pub id: i32, pub from_language_code: String, pub from_phrase: String,
                                pub to_language_code: String, pub to_phrase: String, pub frequency: i32 }
pub struct SyncGrammarRuleDto { pub id, pub language_code, pub cefr_level_code, pub title, pub explanation,
                                pub source, pub examples: Vec<SyncRuleExampleDto> }
pub struct SyncRuleExampleDto { pub sentence, pub translation, pub is_correct }
pub struct SyncProgressDto    { pub table: String, pub synced: usize, pub total: usize }
```

These mirror the production schema closely but use codes instead of foreign
keys so a sync can succeed against a server with a different `languages.id`
mapping.

## Dual n-back

```rust,ignore
pub enum DnbModeDto { AudioWritten, WordTranslation, AudioImage }

pub struct DnbConfigDto {
    pub mode: DnbModeDto,
    pub n_level: u8,
    pub interval_ms: u32,
    pub native_lang: String,
    pub foreign_lang: String,
    pub user_id: i32,
}

pub struct DnbTrialDto { /* trial_number, n_level, audio_phrase, visual_phrase, audio_match, visual_match, interval_ms */ }
pub struct DnbTrialResultDto { /* trial_number, audio_response, visual_response, response_time_ms */ }
pub struct DnbAdaptationDto { /* new_n_level, new_interval_ms, audio_accuracy, visual_accuracy, should_terminate */ }
pub struct DnbSessionResultsDto { /* session_id, mode, n_level_start/peak/end, trials_completed, accuracy_audio/visual, interval_ms_start/end */ }
```

## SpeedController

The crate also exposes a small value type used by the auto-advance UI:

```rust,ignore
pub struct SpeedController { /* private fields */ }

impl SpeedController {
    pub fn new(interval_ms: u32) -> Self;          // clamped to [500, 10000]
    pub fn tick(&mut self, elapsed_ms: u32) -> bool;
    pub fn reset(&mut self);
    pub fn speed_up(&mut self);                    // -500ms, min 500
    pub fn slow_down(&mut self);                   // +500ms, max 10000
    pub fn pause(&mut self);
    pub fn unpause(&mut self);
    pub fn is_paused(&self) -> bool;
    pub fn remaining_fraction(&self) -> f64;       // 0.0..=1.0
    pub fn interval_ms(&self) -> u32;
    pub fn remaining_ms(&self) -> u32;
}
```

`SpeedController` is `Serialize + Deserialize` so it can survive a round-trip
through a server function.
