//! What the device holds about grammar, and what it owes the server.
//!
//! The bank and the mastery projection are mirrors of the server's rows. The
//! outbox is the device's own: an answer taken offline lives there until the
//! server has acknowledged it, which is what lets a session survive a restart
//! with nothing lost.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wisecrow_dto::{GrammarOptionDto, OfflineAttemptDto};
use wisecrow_learning::grading::{Answer, GradableItem, Submission};

/// One grammar point as the device knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalGrammarRule {
    pub rule_id: i32,
    pub language: String,
    pub slug: String,
    pub title: String,
    pub explanation: String,
    pub level: String,
}

/// One item the device can pose and mark without a network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalGrammarItem {
    pub item_id: i32,
    pub rule_id: i32,
    pub revision: i32,
    pub language: String,
    pub prompt: String,
    pub hint: Option<String>,
    pub options: Vec<GrammarOptionDto>,
    pub answer: Option<String>,
    pub accepted: Vec<String>,
    pub correct_option: Option<String>,
}

/// The device's copy of one mastery row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalGrammarMastery {
    pub rule_id: i32,
    pub stability: f32,
    pub difficulty: f32,
    pub elapsed_days: i32,
    pub scheduled_days: i32,
    pub reps: i32,
    pub lapses: i32,
    pub state: i16,
    pub accuracy: Option<f32>,
    pub attempts: i32,
    pub last_review: Option<DateTime<Utc>>,
    pub due: DateTime<Utc>,
}

/// An answer taken offline, waiting to be told to the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedAttempt {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub item_id: i32,
    pub revision: i32,
    pub answer: String,
    pub chose_option: bool,
    pub hint_shown: bool,
    pub ordinal: u32,
    pub occurred_at: DateTime<Utc>,
    pub language: String,
    pub level: Option<String>,
}

impl LocalGrammarItem {
    /// The item reduced to what the shared grader needs.
    ///
    /// The device marks an answer with the same code the server does, so that
    /// the verdict a learner sees offline is the one that will be recorded
    /// when the outbox drains. It is still provisional: the server regrades
    /// against its own stored revision, which is the only copy all three
    /// parties can see.
    #[must_use]
    pub fn gradable(&self) -> GradableItem {
        match &self.correct_option {
            Some(correct_option) => GradableItem::MultipleChoice {
                options: self
                    .options
                    .iter()
                    .map(|option| (option.id.clone(), option.text.clone())) // clone: the grader owns its options
                    .collect(),
                correct_option: correct_option.clone(), // clone: the grader owns its answer
            },
            None => GradableItem::Cloze {
                answer: self.answer.clone().unwrap_or_default(), // clone: the grader owns its answer
                accepted: self.accepted.clone(), // clone: the grader owns its alternatives
            },
        }
    }
}

impl QueuedAttempt {
    /// The answer as the shared grader reads it.
    #[must_use]
    pub fn submission(&self) -> Submission {
        Submission {
            item_id: self.item_id,
            revision: self.revision,
            session_id: self.session_id,
            event_id: self.event_id,
            answer: if self.chose_option {
                Answer::Option(self.answer.clone()) // clone: the submission owns its answer
            } else {
                Answer::Text(self.answer.clone()) // clone: the submission owns its answer
            },
            hint_shown: self.hint_shown,
            ordinal: self.ordinal,
        }
    }

    /// The shape the upload endpoint expects.
    #[must_use]
    pub fn as_upload(&self) -> OfflineAttemptDto {
        OfflineAttemptDto {
            session_id: self.session_id,
            event_id: self.event_id,
            item_id: self.item_id,
            revision: self.revision,
            answer: self.answer.clone(), // clone: building an owned DTO from a borrowed record
            chose_option: self.chose_option,
            hint_shown: self.hint_shown,
            ordinal: self.ordinal,
            occurred_at: self.occurred_at,
            language: self.language.clone(), // clone: building an owned DTO
            level: self.level.clone(),       // clone: building an owned DTO
        }
    }
}

/// How far each grammar feed has been followed, for one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrammarCursors {
    pub bank: i64,
    pub mastery: i64,
}
