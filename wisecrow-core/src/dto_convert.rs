use wisecrow_dto::{
    CardDto, CardStatusDto, ClozeQuizDto, LanguageInfo, MultipleChoiceQuizDto, QuizItemDto,
    ReviewRatingDto, SessionDto,
};

use crate::grammar::quiz::{ClozeQuiz, MultipleChoiceQuiz};
use crate::srs::scheduler::{CardState, CardStatus, ReviewRating};
use crate::srs::session::Session;

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

impl From<&ClozeQuiz> for ClozeQuizDto {
    fn from(quiz: &ClozeQuiz) -> Self {
        Self {
            sentence_with_blank: quiz.sentence_with_blank.clone(), // clone: building owned DTO
            answer: quiz.answer.clone(),                           // clone: building owned DTO
            hint: quiz.hint.clone(),                               // clone: building owned DTO
        }
    }
}

impl From<&MultipleChoiceQuiz> for MultipleChoiceQuizDto {
    fn from(quiz: &MultipleChoiceQuiz) -> Self {
        Self {
            question: quiz.question.clone(), // clone: building owned DTO
            options: quiz.options.clone(),   // clone: building owned DTO
            correct_index: quiz.correct_index,
        }
    }
}

#[must_use]
pub fn language_info(code: &str, name: &str) -> LanguageInfo {
    LanguageInfo {
        code: code.to_owned(),
        name: name.to_owned(),
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
