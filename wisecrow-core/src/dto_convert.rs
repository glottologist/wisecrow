use num_traits::ToPrimitive;
use wisecrow_dto::{
    AnnotatedTokenDto, BrainmapCellDto, CardDto, CardStatusDto, ClozeQuizDto, DnbAdaptationDto,
    DnbModeDto, DnbSessionResultsDto, DnbTrialDto, GlossaryEntryDto, GradedReaderDto,
    GrammarItemDto, GrammarMasteryStateDto, GrammarOptionDto, LanguageInfo, MasteryBandDto,
    MultipleChoiceQuizDto, OfflineGrammarItemDto, PlacementLevelDto, PlacementResultDto,
    PlacementStateDto, PlacementStepDto, QuizItemDto, ReviewRatingDto, ScriptDirection, SessionDto,
    SubmissionDto, TokenStatusDto, UserDto,
};
use wisecrow_learning::grading::{Answer, Submission};
use wisecrow_learning::mastery::{band_from, Band};

use crate::dnb::scoring::AdaptationState;
use crate::dnb::{DnbMode, Trial};
use crate::grammar::graded_reader::{GlossaryEntry, GradedReader};
use crate::grammar::mastery::MasteryRow;
use crate::grammar::placement::PlacementState;
use crate::grammar::quiz::{ClozeQuiz, MultipleChoiceQuiz};
use crate::grammar::selection::PracticeItem;
use crate::grammar::sync::{BankItem, MasteryState};
use crate::preview::annotate::{AnnotatedToken, Status};
use crate::srs::scheduler::{CardState, CardStatus, ReviewRating};
use crate::srs::session::Session;
use crate::users::User;

impl From<&CardState> for CardDto {
    fn from(card: &CardState) -> Self {
        Self {
            card_id: card.card_id,
            translation_id: card.translation_id,
            from_phrase: card.from_phrase.clone(), // clone: building owned DTO from borrowed domain type
            to_phrase: card.to_phrase.clone(), // clone: building owned DTO from borrowed domain type
            frequency: card.frequency,
            stability: card.stability,
            difficulty: card.difficulty,
            state: CardStatusDto::from(card.state),
            due: card.due,
            reps: card.reps,
            lapses: card.lapses,
            is_phrase: card.is_phrase,
            image_allowed: card.image_allowed,
        }
    }
}

impl From<CardStatus> for CardStatusDto {
    fn from(s: CardStatus) -> Self {
        match s {
            CardStatus::New => Self::New,
            CardStatus::Learning => Self::Learning,
            CardStatus::Review => Self::Review,
            CardStatus::Relearning => Self::Relearning,
        }
    }
}

impl From<ReviewRatingDto> for ReviewRating {
    fn from(r: ReviewRatingDto) -> Self {
        match r {
            ReviewRatingDto::Again => Self::Again,
            ReviewRatingDto::Hard => Self::Hard,
            ReviewRatingDto::Good => Self::Good,
            ReviewRatingDto::Easy => Self::Easy,
        }
    }
}

impl From<&Session> for SessionDto {
    fn from(session: &Session) -> Self {
        Self {
            id: session.id,
            native_lang: session.native_lang.clone(), // clone: building owned DTO from borrowed domain type
            foreign_lang: session.foreign_lang.clone(), // clone: building owned DTO from borrowed domain type
            deck_size: session.deck_size,
            speed_ms: session.speed_ms,
            current_index: session.current_index,
            cards: session.cards.iter().map(CardDto::from).collect(),
        }
    }
}

impl From<&User> for UserDto {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            display_name: user.display_name.clone(), // clone: building owned DTO from borrowed domain type
        }
    }
}

impl From<&ClozeQuiz> for ClozeQuizDto {
    fn from(quiz: &ClozeQuiz) -> Self {
        Self {
            sentence_with_blank: quiz.sentence_with_blank.clone(), // clone: building owned DTO
            answer: quiz.answer.clone(),                           // clone: building owned DTO
            hint: quiz.hint.clone(),                               // clone: building owned DTO
            rule_context: None,
        }
    }
}

impl From<&MultipleChoiceQuiz> for MultipleChoiceQuizDto {
    fn from(quiz: &MultipleChoiceQuiz) -> Self {
        Self {
            question: quiz.question.clone(), // clone: building owned DTO
            options: quiz.options.clone(),   // clone: building owned DTO
            correct_index: quiz.correct_index,
            rule_context: None,
        }
    }
}

const RTL_LANGUAGES: &[&str] = &["ar", "he", "fa", "ur", "ps", "sd", "yi"];

#[must_use]
pub fn script_direction_for(code: &str) -> ScriptDirection {
    if RTL_LANGUAGES.contains(&code) {
        ScriptDirection::Rtl
    } else {
        ScriptDirection::Ltr
    }
}

#[must_use]
pub fn language_info(code: &str, name: &str) -> LanguageInfo {
    LanguageInfo {
        code: code.to_owned(),
        name: name.to_owned(),
        script_direction: script_direction_for(code),
    }
}

#[must_use]
pub const fn dnb_mode_to_dto(mode: DnbMode) -> DnbModeDto {
    match mode {
        DnbMode::AudioWritten => DnbModeDto::AudioWritten,
        DnbMode::WordTranslation => DnbModeDto::WordTranslation,
        DnbMode::AudioImage => DnbModeDto::AudioImage,
    }
}

#[must_use]
pub const fn dnb_mode_from_dto(mode: DnbModeDto) -> DnbMode {
    match mode {
        DnbModeDto::AudioWritten => DnbMode::AudioWritten,
        DnbModeDto::WordTranslation => DnbMode::WordTranslation,
        DnbModeDto::AudioImage => DnbMode::AudioImage,
    }
}

#[must_use]
pub fn dnb_trial_to_dto(trial: &Trial) -> DnbTrialDto {
    DnbTrialDto {
        trial_number: trial.trial_number,
        n_level: trial.n_level,
        audio_translation_id: trial.audio_vocab.translation_id,
        visual_translation_id: trial.visual_vocab.translation_id,
        audio_phrase: trial.audio_vocab.to_phrase.clone(), // clone: DTO must own borrowed phrase
        visual_phrase: trial.visual_vocab.from_phrase.clone(), // clone: DTO must own borrowed phrase
        audio_match: trial.audio_match,
        visual_match: trial.visual_match,
        interval_ms: trial.interval_ms,
    }
}

#[must_use]
pub fn adaptation_to_dto(
    state: &AdaptationState,
    trials: &[crate::dnb::CompletedTrial],
    terminated: bool,
) -> DnbAdaptationDto {
    use crate::dnb::scoring::{channel_accuracy, Channel};

    let audio_acc = channel_accuracy(trials, Channel::Audio, 5);
    let visual_acc = channel_accuracy(trials, Channel::Visual, 5);

    DnbAdaptationDto {
        new_n_level: state.n_level,
        new_interval_ms: state.interval_ms,
        audio_accuracy: audio_acc.to_f32().unwrap_or(0.0),
        visual_accuracy: visual_acc.to_f32().unwrap_or(0.0),
        should_terminate: terminated,
    }
}

#[must_use]
pub fn dnb_results_to_dto(
    session_id: i32,
    mode: DnbMode,
    state: &AdaptationState,
    trials_completed: u32,
    accuracy_audio: Option<f32>,
    accuracy_visual: Option<f32>,
) -> DnbSessionResultsDto {
    DnbSessionResultsDto {
        session_id,
        mode: dnb_mode_to_dto(mode),
        n_level_start: state.n_level_start,
        n_level_peak: state.n_level_peak,
        n_level_end: state.n_level,
        trials_completed,
        accuracy_audio,
        accuracy_visual,
        interval_ms_start: state.interval_ms_start,
        interval_ms_end: state.interval_ms,
    }
}

impl From<&GlossaryEntry> for GlossaryEntryDto {
    fn from(entry: &GlossaryEntry) -> Self {
        Self {
            word: entry.word.clone(), // clone: building owned DTO from borrowed domain type
            translation: entry.translation.clone(), // clone: building owned DTO from borrowed domain type
        }
    }
}

impl From<&GradedReader> for GradedReaderDto {
    fn from(reader: &GradedReader) -> Self {
        Self {
            passage: reader.passage.clone(), // clone: building owned DTO from borrowed domain type
            glossary: reader.glossary.iter().map(GlossaryEntryDto::from).collect(),
        }
    }
}

impl From<&Status> for TokenStatusDto {
    fn from(s: &Status) -> Self {
        match s {
            Status::Known => Self::Known,
            Status::Learning => Self::Learning,
            Status::New => Self::New,
            Status::Unknown => Self::Unknown,
        }
    }
}

impl From<&AnnotatedToken> for AnnotatedTokenDto {
    fn from(t: &AnnotatedToken) -> Self {
        Self {
            token: t.token.clone(), // clone: building owned DTO from borrowed domain type
            frequency: t.frequency,
            status: TokenStatusDto::from(&t.status),
            llm_translation: t.llm_translation.clone(), // clone: building owned DTO from borrowed domain type
        }
    }
}

/// Converts cloze and multiple-choice quizzes into a unified DTO list.
#[must_use]
pub fn quizzes_to_dto(cloze: &[ClozeQuiz], mc: &[MultipleChoiceQuiz]) -> Vec<QuizItemDto> {
    let mut items = Vec::with_capacity(mc.len().saturating_add(cloze.len()));

    items.extend(
        mc.iter()
            .map(|q| QuizItemDto::MultipleChoice(MultipleChoiceQuizDto::from(q))),
    );

    items.extend(
        cloze
            .iter()
            .map(|q| QuizItemDto::Cloze(ClozeQuizDto::from(q))),
    );

    items
}

/// Presents a served item without the means to answer it.
///
/// The answer and the correct option are deliberately dropped: the server
/// regrades every submission against the stored revision, so sending them
/// would buy the client nothing and give the exercise away.
#[must_use]
pub fn grammar_item(item: &PracticeItem) -> GrammarItemDto {
    let options = item
        .options
        .as_ref()
        .and_then(|value| serde_json::from_value::<Vec<(String, String)>>(value.clone()).ok()) // clone: deserialising consumes the value
        .unwrap_or_default()
        .into_iter()
        .map(|(id, text)| GrammarOptionDto { id, text })
        .collect();

    GrammarItemDto {
        item_id: item.item_id,
        revision: item.revision,
        rule_id: item.rule_id,
        rule_slug: item.slug.clone(), // clone: building owned DTO from borrowed domain type
        rule_title: item.rule_title.clone(), // clone: building owned DTO
        rule_explanation: item.explanation.clone(), // clone: building owned DTO
        level: item.level.clone(),    // clone: building owned DTO
        prompt: item.prompt.clone(),  // clone: building owned DTO
        hint: item.hint.clone(),      // clone: building owned DTO
        options,
    }
}

/// Presents an item as a device holds it offline, answer and all.
///
/// This is the one place the answer crosses the wire. A device out of contact
/// cannot ask the server whether the learner was right, and a practice session
/// that cannot say so is not practice; the verdict it shows is provisional,
/// and the server regrades the attempt when the outbox drains.
#[must_use]
pub fn offline_grammar_item(item: &BankItem) -> OfflineGrammarItemDto {
    let options = item
        .options
        .as_ref()
        .and_then(|value| serde_json::from_value::<Vec<(String, String)>>(value.clone()).ok()) // clone: deserialising consumes the value
        .unwrap_or_default()
        .into_iter()
        .map(|(id, text)| GrammarOptionDto { id, text })
        .collect();
    let accepted = serde_json::from_value::<Vec<String>>(item.accepted.clone()).unwrap_or_default(); // clone: deserialising consumes the value

    OfflineGrammarItemDto {
        item_id: item.item_id,
        revision: item.revision,
        rule_id: item.rule_id,
        rule_slug: item.rule_slug.clone(), // clone: building owned DTO from borrowed domain type
        rule_title: item.rule_title.clone(), // clone: building owned DTO
        rule_explanation: item.rule_explanation.clone(), // clone: building owned DTO
        level: item.level.clone(),         // clone: building owned DTO
        language: item.language.clone(),   // clone: building owned DTO
        prompt: item.prompt.clone(),       // clone: building owned DTO
        hint: item.hint.clone(),           // clone: building owned DTO
        options,
        answer: item.answer.clone(), // clone: building owned DTO
        accepted,
        correct_option: item.correct_option.clone(), // clone: building owned DTO
    }
}

/// Mirrors one mastery row onto a device.
///
/// The figures narrow to single precision for the reason the brainmap cell
/// gives: `serde_json` without `float_roundtrip` can return a double one unit
/// in the last place adrift, and a device comparing its copy with the server's
/// would see a difference that is not there.
#[must_use]
pub fn grammar_mastery_state(state: &MasteryState) -> GrammarMasteryStateDto {
    GrammarMasteryStateDto {
        rule_id: state.rule_id,
        rule_slug: state.rule_slug.clone(), // clone: building owned DTO from borrowed domain type
        stability: state.stability as f32,
        difficulty: state.difficulty as f32,
        elapsed_days: state.elapsed_days,
        scheduled_days: state.scheduled_days,
        reps: state.reps,
        lapses: state.lapses,
        state: state.state,
        accuracy: state.accuracy.map(|value| value as f32),
        attempts: state.attempts,
        last_review: state.last_review,
        due: state.due,
    }
}

/// Colours one grammar point for the brainmap.
#[must_use]
pub fn brainmap_cell(row: &MasteryRow) -> BrainmapCellDto {
    let attempts = usize::try_from(row.attempts).unwrap_or(0);
    BrainmapCellDto {
        rule_id: row.rule_id,
        slug: row.slug.clone(),   // clone: building owned DTO
        title: row.title.clone(), // clone: building owned DTO
        level: row.level.clone(), // clone: building owned DTO
        band: mastery_band(band_from(row.accuracy.unwrap_or(0.0), attempts)),
        accuracy: row.accuracy.and_then(|value| value.to_f32()),
        attempts: row.attempts,
        provenance: row.source.clone(), // clone: building owned DTO
    }
}

const fn mastery_band(band: Band) -> MasteryBandDto {
    match band {
        Band::Unseen => MasteryBandDto::Unseen,
        Band::Red => MasteryBandDto::Red,
        Band::Amber => MasteryBandDto::Amber,
        Band::Green => MasteryBandDto::Green,
        Band::ProvisionalRed => MasteryBandDto::ProvisionalRed,
        Band::ProvisionalAmber => MasteryBandDto::ProvisionalAmber,
        Band::ProvisionalGreen => MasteryBandDto::ProvisionalGreen,
    }
}

/// Reports where a placement run stands.
#[must_use]
pub fn placement_state(state: &PlacementState) -> PlacementStateDto {
    match state {
        PlacementState::Testing(step) => PlacementStateDto::Testing(PlacementStepDto {
            attempt_id: step.attempt_id,
            session_id: step.session_id,
            level: step.level.clone(), // clone: building owned DTO
            items: step.items.iter().map(grammar_item).collect(),
        }),
        PlacementState::Finished(outcome) => PlacementStateDto::Finished(PlacementResultDto {
            attempt_id: outcome.attempt_id,
            level_reached: outcome.level_reached.clone(), // clone: building owned DTO
            tested: outcome
                .tested
                .iter()
                .map(|score| PlacementLevelDto {
                    level: score.level.clone(), // clone: building owned DTO
                    correct: score.correct,
                    asked: score.asked,
                    passed: score.passed,
                })
                .collect(),
            skipped: outcome.skipped.clone(), // clone: building owned DTO
        }),
    }
}

/// Turns a client's report into the submission the grader understands.
///
/// `chose_option` decides how the answer is read, because an option identifier
/// and a typed word are both strings on the wire and grading them alike would
/// let a learner type `o2` into a cloze.
#[must_use]
pub fn submission(dto: &SubmissionDto) -> Submission {
    let answer = if dto.chose_option {
        Answer::Option(dto.answer.clone()) // clone: the submission owns its answer
    } else {
        Answer::Text(dto.answer.clone()) // clone: the submission owns its answer
    };
    Submission {
        item_id: dto.item_id,
        revision: dto.revision,
        session_id: dto.session_id,
        event_id: dto.event_id,
        answer,
        hint_shown: dto.hint_shown,
        ordinal: dto.ordinal,
    }
}
