use wisecrow_dto::{
    CardDto, CardStatusDto, ClozeQuizDto, DnbAdaptationDto, DnbModeDto, DnbSessionResultsDto,
    DnbTrialDto, GrammarRuleDto, LanguageInfo, MultipleChoiceQuizDto, QuizItemDto, ReviewRatingDto,
    RuleExampleDto, ScriptDirection, SessionDto, UserDto,
};

use crate::dnb::scoring::AdaptationState;
use crate::dnb::{DnbMode, Trial};
use crate::grammar::quiz::{ClozeQuiz, MultipleChoiceQuiz};
use crate::grammar::rules::{GrammarRule, RuleExample};
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

/// Converts a grammar rule to its DTO representation.
/// Requires the CEFR level code since the domain type stores only the level ID.
#[must_use]
pub fn grammar_rule_to_dto(rule: &GrammarRule, cefr_level_code: &str) -> GrammarRuleDto {
    GrammarRuleDto {
        id: rule.id,
        title: rule.title.clone(), // clone: building owned DTO from borrowed domain type
        explanation: rule.explanation.clone(), // clone: building owned DTO from borrowed domain type
        cefr_level: cefr_level_code.to_owned(),
        source: rule.source.as_str().to_owned(),
        examples: rule.examples.iter().map(RuleExampleDto::from).collect(),
    }
}

impl From<&RuleExample> for RuleExampleDto {
    fn from(ex: &RuleExample) -> Self {
        Self {
            sentence: ex.sentence.clone(), // clone: building owned DTO from borrowed domain type
            translation: ex.translation.clone(), // clone: building owned DTO from borrowed domain type
            is_correct: ex.is_correct,
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

impl From<DnbMode> for DnbModeDto {
    fn from(mode: DnbMode) -> Self {
        match mode {
            DnbMode::AudioWritten => Self::AudioWritten,
            DnbMode::WordTranslation => Self::WordTranslation,
            DnbMode::AudioImage => Self::AudioImage,
        }
    }
}

impl From<DnbModeDto> for DnbMode {
    fn from(dto: DnbModeDto) -> Self {
        match dto {
            DnbModeDto::AudioWritten => Self::AudioWritten,
            DnbModeDto::WordTranslation => Self::WordTranslation,
            DnbModeDto::AudioImage => Self::AudioImage,
        }
    }
}

impl From<&Trial> for DnbTrialDto {
    fn from(trial: &Trial) -> Self {
        Self {
            trial_number: trial.trial_number,
            n_level: trial.n_level,
            audio_phrase: trial.audio_vocab.to_phrase.clone(), // clone: building owned DTO from borrowed domain type
            visual_phrase: trial.visual_vocab.from_phrase.clone(), // clone: building owned DTO from borrowed domain type
            audio_match: trial.audio_match,
            visual_match: trial.visual_match,
            interval_ms: trial.interval_ms,
        }
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
        #[expect(clippy::cast_possible_truncation)]
        audio_accuracy: audio_acc as f32,
        #[expect(clippy::cast_possible_truncation)]
        visual_accuracy: visual_acc as f32,
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
        mode: DnbModeDto::from(mode),
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
