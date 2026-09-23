//! Grammar practice on the device.
//!
//! These pages read the mirrored bank and write to the outbox; they never talk
//! to the server. The verdict they show is the shared grader's, run against the
//! cached item, and it is provisional: the server regrades every answer when
//! the outbox drains, and the mastery feed then overwrites whatever the device
//! projected for itself.

mod brainmap;
mod session;

pub use brainmap::GrammarBrainmapPage;
pub use session::GrammarSessionPage;

use chrono::Utc;
use uuid::Uuid;
use wisecrow_learning::mastery::{band_from, Band};

use crate::application::{LocalGrammarItem, QueuedAttempt};

/// Items one practice run puts to the learner.
///
/// The same length the web session uses, so that a learner moving between the
/// two meets the same sitting.
pub const SESSION_LENGTH: usize = 12;

/// Builds the record of one answer.
///
/// The event identifier is generated here so that an answer written to the
/// outbox twice — the app killed between the write and its acknowledgement —
/// is recognised as the same answer rather than counted again.
#[must_use]
pub(crate) fn queued_attempt(
    item: &LocalGrammarItem,
    session_id: Uuid,
    answer: &str,
    chose_option: bool,
    hint_shown: bool,
    ordinal: u32,
    level: Option<&str>,
) -> QueuedAttempt {
    QueuedAttempt {
        event_id: Uuid::new_v4(),
        session_id,
        item_id: item.item_id,
        revision: item.revision,
        answer: answer.trim().to_owned(),
        chose_option,
        hint_shown,
        ordinal,
        occurred_at: Utc::now(),
        language: item.language.clone(), // clone: the record owns its language code
        level: level.map(str::to_owned),
    }
}

/// How a brainmap cell is painted on the device.
///
/// The colours are inline rather than classed, because the mobile shell has no
/// stylesheet: every page here styles itself. The policy is the shared one, so
/// a point reads the same on the phone as in the browser.
#[must_use]
pub(crate) const fn band_colour(band: Band) -> (&'static str, &'static str) {
    match band {
        Band::Unseen => ("transparent", "#6b7280"),
        Band::Green => ("#047857", "#047857"),
        Band::Amber => ("#b45309", "#b45309"),
        Band::Red => ("#b91c1c", "#b91c1c"),
        Band::ProvisionalGreen => ("transparent", "#047857"),
        Band::ProvisionalAmber => ("transparent", "#b45309"),
        Band::ProvisionalRed => ("transparent", "#b91c1c"),
    }
}

/// Whether a band is settled enough to be drawn filled.
#[must_use]
pub(crate) const fn is_provisional(band: Band) -> bool {
    matches!(
        band,
        Band::ProvisionalGreen | Band::ProvisionalAmber | Band::ProvisionalRed
    )
}

/// Colours one mirrored mastery row.
#[must_use]
pub(crate) fn band_of(accuracy: Option<f32>, attempts: i32) -> Band {
    let attempts = usize::try_from(attempts).unwrap_or(0);
    band_from(f64::from(accuracy.unwrap_or_default()), attempts)
}

/// The percentage a cell shows, or a dash where nothing has been attempted.
#[must_use]
pub(crate) fn accuracy_label(accuracy: Option<f32>, attempts: i32) -> String {
    if attempts == 0 {
        return String::from("—");
    }
    accuracy.map_or_else(
        || String::from("—"),
        |value| format!("{:.0}%", value * 100.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisecrow_dto::GrammarOptionDto;

    fn cloze() -> LocalGrammarItem {
        LocalGrammarItem {
            item_id: 3,
            rule_id: 9,
            revision: 2,
            language: String::from("es"),
            prompt: String::from("Yo ___ cansado."),
            hint: Some(String::from("A passing state.")),
            options: Vec::new(),
            answer: Some(String::from("estoy")),
            accepted: Vec::new(),
            correct_option: None,
        }
    }

    fn choice() -> LocalGrammarItem {
        LocalGrammarItem {
            options: vec![GrammarOptionDto {
                id: String::from("o1"),
                text: String::from("estoy"),
            }],
            answer: None,
            correct_option: Some(String::from("o1")),
            ..cloze()
        }
    }

    #[test]
    fn an_answer_records_the_item_revision_it_was_given_against() {
        let session = Uuid::from_u128(11);
        let attempt = queued_attempt(&cloze(), session, "  estoy ", false, false, 1, Some("A1"));
        assert_eq!(attempt.revision, 2, "the server regrades against this");
        assert_eq!(attempt.answer, "estoy");
        assert_eq!(attempt.session_id, session);
        assert_eq!(attempt.level.as_deref(), Some("A1"));
    }

    #[test]
    fn each_answer_carries_its_own_event_identifier() {
        let first = queued_attempt(&cloze(), Uuid::nil(), "estoy", false, false, 1, None);
        let second = queued_attempt(&cloze(), Uuid::nil(), "estoy", false, false, 2, None);
        assert_ne!(first.event_id, second.event_id);
    }

    #[test]
    fn the_device_marks_with_the_same_grader_the_server_uses() {
        let item = cloze();
        let attempt = queued_attempt(&item, Uuid::nil(), "ESTOY", false, false, 1, None);
        assert!(
            wisecrow_learning::grading::grade(&item.gradable(), &attempt.submission()).correct,
            "case folding is the grader's business, not the page's"
        );

        let item = choice();
        let chosen = queued_attempt(&item, Uuid::nil(), "o1", true, false, 1, None);
        assert!(wisecrow_learning::grading::grade(&item.gradable(), &chosen.submission()).correct);
    }

    #[test]
    fn a_point_never_attempted_is_grey_and_shows_no_figure() {
        assert_eq!(band_of(None, 0), Band::Unseen);
        assert_eq!(band_colour(Band::Unseen).0, "transparent");
        assert_eq!(accuracy_label(Some(1.0), 0), "—");
        assert_eq!(accuracy_label(Some(0.837), 4), "84%");
    }

    #[test]
    fn fewer_than_three_attempts_never_reads_as_settled() {
        assert!(is_provisional(band_of(Some(1.0), 2)));
        assert!(!is_provisional(band_of(Some(1.0), 3)));
    }
}
