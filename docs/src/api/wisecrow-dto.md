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

## Grammar sessions and mastery

`wisecrow-dto/src/grammar.rs` carries the CEFR grammar shapes. The served item
withholds the answer; grading is the server's, and a browser that could read the
answer out of the payload would not be tested by it.

```rust,ignore
pub struct GrammarItemDto {
    pub item_id: i32, pub revision: i32,
    pub rule_id: i32, pub rule_slug: String, pub rule_title: String, pub rule_explanation: String,
    pub level: String, pub prompt: String, pub hint: Option<String>,
    pub options: Vec<GrammarOptionDto>,       // empty for a cloze
}
pub struct GrammarOptionDto { pub id: String, pub text: String }
pub struct GrammarSessionDto { pub session_id: Uuid, pub items: Vec<GrammarItemDto> }

pub struct SubmissionDto {
    pub session_id: Uuid, pub event_id: Uuid,
    pub item_id: i32, pub revision: i32,
    pub answer: String, pub chose_option: bool, pub hint_shown: bool,
    pub ordinal: u32, pub occurred_at: DateTime<Utc>,
}
pub struct VerdictDto { pub event_id: Uuid, pub correct: bool, /* .. */ }
```

`event_id` is the idempotency key: the same answer sent twice is recorded once.
`revision` is checked on submission, so an item edited between serving and
answering is refused rather than graded against a question the learner never saw.

The brainmap paints one cell per point, banded by what is known and how firmly:

```rust,ignore
pub enum MasteryBandDto {
    Unseen, Red, Amber, Green,
    ProvisionalRed, ProvisionalAmber, ProvisionalGreen,
}
pub struct BrainmapCellDto {
    pub rule_id: i32, pub slug: String, pub title: String, pub level: String,
    pub band: MasteryBandDto, pub accuracy: Option<f32>, pub attempts: i32,
    pub provenance: String, /* .. */
}
pub struct BrainmapDto { pub language: String, pub cells: Vec<BrainmapCellDto> }
```

A `Provisional` band is one drawn from too few attempts to be trusted; the
[policy table](../reference/database-schema.md#grammar_mastery-and-grammar_review_baselines)
gives the thresholds. `BrainmapDto::language` is the language's name, not its
code.

Placement is a state machine the client drives one level at a time:

```rust,ignore
pub enum PlacementStateDto { Testing(PlacementStepDto), Finished(PlacementResultDto) }
pub struct PlacementStepDto { pub attempt_id: Uuid, pub session_id: Uuid, pub level: String,
                              pub items: Vec<GrammarItemDto> }
pub struct PlacementLevelDto { pub level: String, pub correct: i64, pub asked: i64, pub passed: bool }
pub struct PlacementResultDto { pub attempt_id: Uuid, pub level_reached: Option<String>,
                                pub tested: Vec<PlacementLevelDto>, pub skipped: Vec<String> }
```

`level_reached` is `None` when the learner did not pass A1 — the map is then
drawn from nothing but the attempts themselves.

## Grammar sync DTOs (protocol 2)

The device-facing shapes. A device holds answers because it must grade offline,
so `OfflineGrammarItemDto` is the one served shape that carries them:

```rust,ignore
pub enum GrammarChangeOperationDto { Upsert, Delete }

pub struct OfflineGrammarItemDto {
    /* GrammarItemDto's fields, plus: */
    pub language: String,
    pub answer: Option<String>,          // cloze
    pub accepted: Vec<String>,           // further accepted spellings
    pub correct_option: Option<String>,  // multiple choice
}

pub struct GrammarBankChangeRequestDto { pub protocol_version: u16, pub language: String,
                                         pub cursor: i64, pub limit: u16 }
pub struct GrammarBankChangeDto { pub sequence: i64, pub item_id: i32,
                                  pub operation: GrammarChangeOperationDto,
                                  pub item: Option<OfflineGrammarItemDto> }
pub struct GrammarBankChangePageDto { pub protocol_version: u16, pub language: String,
                                      pub changes: Vec<GrammarBankChangeDto>,
                                      pub next_cursor: i64, pub has_more: bool }

pub struct GrammarMasteryStateDto { pub rule_id: i32, pub rule_slug: String,
                                    pub stability: f32, pub difficulty: f32,
                                    pub elapsed_days: i32, pub scheduled_days: i32,
                                    pub reps: i32, pub lapses: i32, pub state: i16,
                                    pub accuracy: Option<f32>, pub attempts: i32,
                                    pub last_review: Option<DateTime<Utc>>, pub due: DateTime<Utc> }
pub struct GrammarMasteryChangeRequestDto { pub protocol_version: u16, pub cursor: i64, pub limit: u16 }
pub struct GrammarMasteryChangeDto { pub sequence: i64, pub rule_id: i32,
                                     pub operation: GrammarChangeOperationDto,
                                     pub mastery: Option<GrammarMasteryStateDto> }
pub struct GrammarMasteryChangePageDto { pub protocol_version: u16,
                                         pub changes: Vec<GrammarMasteryChangeDto>,
                                         pub next_cursor: i64, pub has_more: bool }
```

`item` and `mastery` are absent on a `Delete`, and the figures are `f32` because
a device stores them in SQLite `REAL` and no more precision survives the trip.

Answers travel the other way in batches:

```rust,ignore
pub struct OfflineAttemptDto { /* SubmissionDto's fields, plus language and level */ }
pub struct GrammarAttemptBatchRequestDto { pub protocol_version: u16, pub device_id: Uuid,
                                           pub attempts: Vec<OfflineAttemptDto> }
pub enum OfflineAttemptStatusDto {
    Accepted(VerdictDto),
    Duplicate(VerdictDto),
    Rejected { event_id: Uuid, reason: String },
}
pub struct GrammarAttemptBatchResponseDto { pub protocol_version: u16,
                                            pub results: Vec<OfflineAttemptStatusDto> }
```

Each answer is judged on its own, so one refusal does not cost the batch.
`Duplicate` carries the stored verdict rather than an error, which is what lets
a device that never received the first acknowledgement converge on a replay.
`OfflineAttemptDto` names its language and level because the session it belongs
to may not exist on the server yet: the upload adopts the device's `session_id`
if no row holds it, and rejects it if another learner does.

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

## Mobile protocol negotiation

```rust,ignore
pub const MOBILE_PROTOCOL_VERSION: u16 = 1;     // frozen
pub const MOBILE_PROTOCOL_VERSION_V2: u16 = 2;  // adds the grammar feeds

pub enum MobileFeatureDto {
    CorpusSync, CardSync, ReviewUpload, NbackUpload, QuizCache,
    GrammarBankSync, GrammarMasterySync, GrammarAttemptUpload,
    #[serde(other)] Unknown,
}
```

Version 1 is frozen and a fixture pins its payload, because a device in the
field cannot be asked to upgrade before it may sync. Version 2 is advertised
beside it, not in place of it, and the grammar capabilities are negotiated
individually: a client offered none of them syncs vocabulary and nothing else.

`Unknown` is the reason a version-3 capability will not break this build. Without
`#[serde(other)]` one unrecognised name would fail the whole capabilities payload
to deserialise, and the client would stop syncing altogether — the exact outcome
negotiating capabilities exists to prevent. A server never sends it.

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
