//! Grammar practice, placement and the brainmap.
//!
//! None of these components decide whether an answer is right. They report a
//! structured submission and render the verdict the server sends back, which
//! is the only ruling that counts.

mod brainmap;
mod placement;
mod session;

pub use brainmap::BrainmapPage;
pub use placement::PlacementPage;
pub use session::GrammarPage;

use uuid::Uuid;
use wisecrow_dto::{GrammarItemDto, MasteryBandDto, SubmissionDto};

/// Builds the report one answer sends.
///
/// `chose_option` travels with the answer because an option identifier and a
/// typed word are both strings: without it the server could not tell a chosen
/// `o2` from a learner who typed `o2` into a cloze. The event identifier is
/// generated here so that a resend is recorded once.
#[must_use]
pub(crate) fn submission_for(
    item: &GrammarItemDto,
    session_id: Uuid,
    answer: &str,
    hint_shown: bool,
    ordinal: u32,
) -> SubmissionDto {
    SubmissionDto {
        session_id,
        event_id: Uuid::new_v4(),
        item_id: item.item_id,
        revision: item.revision,
        answer: answer.trim().to_owned(),
        chose_option: !item.options.is_empty(),
        hint_shown,
        ordinal,
        occurred_at: chrono_now(),
    }
}

/// The browser's clock, which the server uses only as a hint.
///
/// A web answer is recorded at the moment the server hears it; the field
/// exists for the mobile client, whose answers may be days old on arrival.
fn chrono_now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

/// How a brainmap cell is painted.
///
/// The classes are named for the policy rather than composed from utilities,
/// because `assets/style.css` is a curated stylesheet rather than a generated
/// one: a class it does not define simply does nothing, and a band that paints
/// nothing is worse than no band at all.
///
/// A provisional band is outlined rather than filled, because fewer than three
/// attempts should not present as settled knowledge — least of all settled
/// knowledge of failure. An unseen point is grey and never reads as known.
#[must_use]
pub(crate) const fn band_class(band: MasteryBandDto) -> &'static str {
    match band {
        MasteryBandDto::Unseen => "band band-unseen",
        MasteryBandDto::Green => "band band-green",
        MasteryBandDto::Amber => "band band-amber",
        MasteryBandDto::Red => "band band-red",
        MasteryBandDto::ProvisionalGreen => "band band-provisional-green",
        MasteryBandDto::ProvisionalAmber => "band band-provisional-amber",
        MasteryBandDto::ProvisionalRed => "band band-provisional-red",
    }
}

/// What a learner should make of where a grammar point came from.
#[must_use]
pub(crate) fn provenance_label(provenance: &str) -> &'static str {
    match provenance {
        "reference" => "Curated",
        _ => "Generated",
    }
}

/// How many attempts a cell reports, in words that read correctly at one.
#[must_use]
pub(crate) fn attempts_label(attempts: i32) -> String {
    if attempts == 1 {
        String::from("1 attempt")
    } else {
        format!("{attempts} attempts")
    }
}

/// The percentage a cell shows, or a dash where nothing has been attempted.
#[must_use]
pub(crate) fn accuracy_label(accuracy: Option<f32>) -> String {
    accuracy.map_or_else(
        || String::from("—"),
        |value| format!("{:.0}%", value * 100.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisecrow_dto::GrammarOptionDto;

    fn cloze() -> GrammarItemDto {
        GrammarItemDto {
            item_id: 3,
            revision: 2,
            rule_id: 9,
            rule_slug: "ser-vs-estar".into(),
            rule_title: "Ser vs estar".into(),
            rule_explanation: "Permanent against temporary.".into(),
            level: "A1".into(),
            prompt: "Yo ___ cansado.".into(),
            hint: Some("A passing state.".into()),
            options: Vec::new(),
        }
    }

    fn choice() -> GrammarItemDto {
        GrammarItemDto {
            options: vec![
                GrammarOptionDto {
                    id: "o1".into(),
                    text: "soy".into(),
                },
                GrammarOptionDto {
                    id: "o2".into(),
                    text: "estoy".into(),
                },
            ],
            ..cloze()
        }
    }

    #[test]
    fn a_submission_carries_item_identity_and_its_place_in_the_interaction() {
        let session_id = Uuid::from_u128(11);
        let submission = submission_for(&cloze(), session_id, "  estoy ", false, 2);

        assert_eq!(submission.session_id, session_id);
        assert_eq!(submission.item_id, 3);
        assert_eq!(submission.revision, 2);
        assert_eq!(submission.answer, "estoy");
        assert_eq!(submission.ordinal, 2);
        assert!(!submission.chose_option);
    }

    #[test]
    fn revealing_a_hint_is_reported_rather_than_discarded() {
        let hinted = submission_for(&cloze(), Uuid::nil(), "estoy", true, 1);
        assert!(
            hinted.hint_shown,
            "a hint downgrades the rating, so the server has to be told"
        );
        assert!(!submission_for(&cloze(), Uuid::nil(), "estoy", false, 1).hint_shown);
    }

    #[test]
    fn a_chosen_option_is_marked_as_one() {
        assert!(submission_for(&choice(), Uuid::nil(), "o2", false, 1).chose_option);
    }

    #[test]
    fn each_answer_carries_its_own_event_identifier() {
        let first = submission_for(&cloze(), Uuid::nil(), "estoy", false, 1);
        let second = submission_for(&cloze(), Uuid::nil(), "estoy", false, 2);
        assert_ne!(first.event_id, second.event_id);
    }

    #[test]
    fn settled_bands_are_filled_and_provisional_ones_outlined() {
        for band in [
            MasteryBandDto::Green,
            MasteryBandDto::Amber,
            MasteryBandDto::Red,
        ] {
            assert!(!band_class(band).contains("provisional"));
        }
        for band in [
            MasteryBandDto::ProvisionalGreen,
            MasteryBandDto::ProvisionalAmber,
            MasteryBandDto::ProvisionalRed,
        ] {
            assert!(
                band_class(band).contains("provisional"),
                "fewer than three attempts must not look settled"
            );
        }
    }

    /// A class the stylesheet does not define paints nothing, which would hide
    /// the very distinction the brainmap exists to draw.
    #[test]
    fn every_band_class_is_defined_by_the_stylesheet() {
        const STYLES: &str = include_str!("../../../assets/style.css");
        for band in [
            MasteryBandDto::Unseen,
            MasteryBandDto::Green,
            MasteryBandDto::Amber,
            MasteryBandDto::Red,
            MasteryBandDto::ProvisionalGreen,
            MasteryBandDto::ProvisionalAmber,
            MasteryBandDto::ProvisionalRed,
        ] {
            for class in band_class(band).split_whitespace() {
                assert!(
                    STYLES.contains(&format!(".{class}")),
                    "{class} is not in the stylesheet"
                );
            }
        }
    }

    #[test]
    fn an_unseen_point_is_grey_and_shows_no_figure() {
        assert_eq!(band_class(MasteryBandDto::Unseen), "band band-unseen");
        assert_eq!(accuracy_label(None), "—");
        assert_eq!(accuracy_label(Some(0.837)), "84%");
    }

    #[test]
    fn a_single_attempt_is_not_reported_in_the_plural() {
        assert_eq!(attempts_label(0), "0 attempts");
        assert_eq!(attempts_label(1), "1 attempt");
        assert_eq!(attempts_label(2), "2 attempts");
    }

    #[test]
    fn provenance_distinguishes_a_curated_point_from_a_generated_one() {
        assert_eq!(provenance_label("reference"), "Curated");
        assert_eq!(provenance_label("llm"), "Generated");
    }
}
