use thiserror::Error;

/// A failure while calculating learning state.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LearningError {
    #[error("review rating {0} is outside 1..=4")]
    InvalidReviewRating(u8),
    #[error("review events contain a duplicate identifier")]
    DuplicateReview,
    #[error("review timestamp precedes the card baseline")]
    ReviewBeforeBaseline,
    #[error("at least eight vocabulary items are required")]
    InsufficientVocabulary,
    #[error("the trial does not match the next expected trial")]
    UnexpectedTrial,
    #[error("unknown n-back mode")]
    InvalidNbackMode,
    #[error("card field {0} must be finite")]
    NonFiniteCardField(&'static str),
}
