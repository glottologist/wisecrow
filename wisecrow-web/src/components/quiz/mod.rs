mod cloze;
mod multiple_choice;
pub mod rule_explanation;

use dioxus::prelude::*;

use wisecrow_dto::{MultipleChoiceQuizDto, QuizItemDto};
use wisecrow_learning::grading::{grade, GradableItem, Submission};

use crate::api::quiz::{generate_quiz, MAX_PDF_BYTES};

/// Which bank item a submission is made against.
///
/// The quiz generated from an uploaded PDF exists only for the request that
/// produced it and has no stored identity, so it reports zeroes; a grammar
/// session passes the identifiers of the item it served.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ItemIdentity {
    pub item_id: i32,
    pub revision: i32,
}

/// Stable identifier for the option shown at `index`.
///
/// This quiz carries bare option text, so identity is derived from position.
/// Stored bank items are generated with the same `o1…oN` scheme, so a session
/// serving real items grades through exactly this path.
pub(crate) fn option_id(index: usize) -> String {
    format!("o{}", index.saturating_add(1))
}

/// The gradable form of a cloze.
///
/// A request-scoped quiz carries one answer and no alternatives; accepted
/// variants arrive with stored items.
pub(crate) fn cloze_item(answer: &str) -> GradableItem {
    GradableItem::cloze(answer, &[])
}

/// The gradable form of a multiple choice.
pub(crate) fn choice_item(quiz: &MultipleChoiceQuizDto) -> GradableItem {
    GradableItem::MultipleChoice {
        options: quiz
            .options
            .iter()
            .enumerate()
            .map(|(index, text)| (option_id(index), text.clone())) // clone: the gradable item owns its option text
            .collect(),
        correct_option: option_id(quiz.correct_index),
    }
}

/// The submission one check of a cloze reports.
pub(crate) fn cloze_submission(
    identity: ItemIdentity,
    typed: &str,
    hint_shown: bool,
    ordinal: u32,
) -> Submission {
    Submission {
        item_id: identity.item_id,
        revision: identity.revision,
        hint_shown,
        ordinal,
        ..Submission::text(typed.trim())
    }
}

/// The submission choosing the option at `index` reports.
pub(crate) fn choice_submission(identity: ItemIdentity, index: usize, ordinal: u32) -> Submission {
    Submission {
        item_id: identity.item_id,
        revision: identity.revision,
        ordinal,
        ..Submission::option(&option_id(index))
    }
}

/// Grades a submission against the quiz item it was made on.
fn scores(item: &QuizItemDto, submission: &Submission) -> bool {
    let gradable = match item {
        QuizItemDto::Cloze(quiz) => cloze_item(&quiz.answer),
        QuizItemDto::MultipleChoice(quiz) => choice_item(quiz),
    };
    grade(&gradable, submission).correct
}

#[component]
pub fn QuizPage() -> Element {
    let mut items: Signal<Vec<QuizItemDto>> = use_signal(Vec::new);
    let mut current_index = use_signal(|| 0usize);
    let mut correct_count = use_signal(|| 0usize);
    let mut total_answered = use_signal(|| 0usize);
    let mut loading = use_signal(|| false);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let mut started = use_signal(|| false);

    if !started() {
        return rsx! {
            div { class: "max-w-lg mx-auto space-y-6",
                h1 { class: "text-3xl font-bold text-center", "Grammar Quiz" }
                p { class: "text-gray-400 text-center",
                    "Upload a PDF grammar guide to generate quiz questions."
                }

                if loading() {
                    div { class: "text-center text-gray-400 py-8", "Generating quiz..." }
                } else {
                    form {
                        class: "bg-gray-800 rounded-xl p-6 space-y-4",
                        input {
                            r#type: "file",
                            accept: ".pdf",
                            class: "w-full text-gray-300",
                            onchange: move |evt: Event<FormData>| {
                                async move {
                                    let files = evt.data.files();
                                    if let Some(file) = files.first() {
                                        loading.set(true);
                                        error_msg.set(None);
                                        match file.read_bytes().await {
                                            Ok(bytes) if bytes.len() > MAX_PDF_BYTES => {
                                                error_msg.set(Some(format!(
                                                    "PDF is {} MB; the limit is {} MB.",
                                                    bytes.len() / (1024 * 1024),
                                                    MAX_PDF_BYTES / (1024 * 1024),
                                                )));
                                            }
                                            Ok(bytes) => {
                                                match generate_quiz(bytes.into(), 20).await {
                                                    Ok(quiz_items) => {
                                                        if quiz_items.is_empty() {
                                                            error_msg.set(Some(String::from(
                                                                "No quiz questions could be generated from this PDF.",
                                                            )));
                                                        } else {
                                                            items.set(quiz_items);
                                                            started.set(true);
                                                        }
                                                    }
                                                    Err(e) => {
                                                        error_msg.set(Some(format!("Quiz generation failed: {e}")));
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                error_msg.set(Some(format!("Failed to read file: {e}")));
                                            }
                                        }
                                        loading.set(false);
                                    }
                                }
                            },
                        }
                    }
                }

                if let Some(err) = error_msg() {
                    div { class: "text-red-400 text-center", "{err}" }
                }
            }
        };
    }

    let all_items = items();
    let idx = current_index();
    let total = all_items.len();

    if idx >= total {
        let pct = if total_answered() > 0 {
            correct_count()
                .saturating_mul(100)
                .checked_div(total_answered())
                .unwrap_or(0)
        } else {
            0
        };

        return rsx! {
            div { class: "text-center space-y-4 py-20",
                h2 { class: "text-3xl font-bold text-emerald-400", "Quiz Complete!" }
                p { class: "text-xl text-gray-300",
                    "{correct_count()}/{total_answered()} correct ({pct}%)"
                }
                button {
                    class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-3 font-semibold transition",
                    onclick: move |_| {
                        started.set(false);
                        items.set(Vec::new());
                        current_index.set(0);
                        correct_count.set(0);
                        total_answered.set(0);
                    },
                    "Try Another PDF"
                }
            }
        };
    }

    let display_num = idx.saturating_add(1);

    rsx! {
        div { class: "max-w-2xl mx-auto space-y-4",
            div { class: "flex justify-between text-sm text-gray-500",
                span { "Question {display_num} / {total}" }
                span { "Score: {correct_count()}/{total_answered()}" }
            }

            match &all_items[idx] {
                QuizItemDto::Cloze(q) => rsx! {
                    cloze::ClozeQuestion {
                        quiz: q.clone(), // clone: Dioxus component props require owned values
                        on_answer: {
                            let item = all_items[idx].clone(); // clone: the handler outlives this render
                            move |submission: Submission| {
                                total_answered.set(total_answered().saturating_add(1));
                                if scores(&item, &submission) {
                                    correct_count.set(correct_count().saturating_add(1));
                                }
                            }
                        },
                        on_next: move |_| {
                            current_index.set(idx.saturating_add(1));
                        },
                    }
                },
                QuizItemDto::MultipleChoice(q) => rsx! {
                    multiple_choice::McQuestion {
                        quiz: q.clone(), // clone: Dioxus component props require owned values
                        on_answer: {
                            let item = all_items[idx].clone(); // clone: the handler outlives this render
                            move |submission: Submission| {
                                total_answered.set(total_answered().saturating_add(1));
                                if scores(&item, &submission) {
                                    correct_count.set(correct_count().saturating_add(1));
                                }
                            }
                        },
                        on_next: move |_| {
                            current_index.set(idx.saturating_add(1));
                        },
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisecrow_dto::ClozeQuizDto;

    fn cloze(answer: &str) -> ClozeQuizDto {
        ClozeQuizDto {
            sentence_with_blank: String::from("Nous ___ allés."),
            answer: String::from(answer),
            hint: None,
            rule_context: None,
        }
    }

    fn choice() -> MultipleChoiceQuizDto {
        MultipleChoiceQuizDto {
            question: String::from("Which verb?"),
            options: vec![
                String::from("ser"),
                String::from("estar"),
                String::from("haber"),
            ],
            correct_index: 1,
            rule_context: None,
        }
    }

    #[test]
    fn a_cloze_submission_carries_item_identity_hint_and_ordinal() {
        let identity = ItemIdentity {
            item_id: 42,
            revision: 3,
        };
        let submission = cloze_submission(identity, "  été ", true, 2);

        assert_eq!(submission.item_id, 42);
        assert_eq!(submission.revision, 3);
        assert!(submission.hint_shown);
        assert_eq!(submission.ordinal, 2);
        assert_eq!(
            submission.answer,
            wisecrow_learning::grading::Answer::Text(String::from("été"))
        );
    }

    #[test]
    fn a_choice_submission_names_the_option_rather_than_its_position() {
        let submission = choice_submission(ItemIdentity::default(), 1, 1);
        assert_eq!(
            submission.answer,
            wisecrow_learning::grading::Answer::Option(String::from("o2"))
        );
    }

    #[test]
    fn cloze_scoring_folds_unicode_case() {
        let item = QuizItemDto::Cloze(cloze("été"));
        let submission = cloze_submission(ItemIdentity::default(), "ÉTÉ", false, 1);
        assert!(scores(&item, &submission));
    }

    #[test]
    fn revealing_the_answer_scores_nothing() {
        let item = QuizItemDto::Cloze(cloze("été"));
        let submission = cloze_submission(ItemIdentity::default(), "", false, 1);
        assert!(!scores(&item, &submission));
    }

    #[test]
    fn choice_scoring_resolves_by_option_identifier() {
        let item = QuizItemDto::MultipleChoice(choice());
        assert!(scores(
            &item,
            &choice_submission(ItemIdentity::default(), 1, 1)
        ));
        assert!(!scores(
            &item,
            &choice_submission(ItemIdentity::default(), 0, 1)
        ));
    }

    /// Guards the reason the grader was extracted: ASCII case folding holds
    /// `ÉTÉ` and `été` to be different words, so no component may go back to
    /// deciding correctness with it.
    #[test]
    fn the_components_no_longer_grade_with_ascii_case_folding() {
        for source in [include_str!("cloze.rs"), include_str!("multiple_choice.rs")] {
            assert!(!source.contains("eq_ignore_ascii_case"));
        }
    }
}
