use dioxus::prelude::*;

use wisecrow_dto::MultipleChoiceQuizDto;

#[component]
pub fn McQuestion(
    quiz: MultipleChoiceQuizDto,
    on_answer: EventHandler<bool>,
    on_next: EventHandler<()>,
) -> Element {
    let mut selected: Signal<Option<usize>> = use_signal(|| None);
    let mut answered = use_signal(|| false);

    rsx! {
        div { class: "bg-gray-800 rounded-xl p-8 space-y-6",
            h3 { class: "text-sm text-gray-500 uppercase tracking-wide", "Multiple Choice" }

            p { class: "text-xl text-white font-bold text-center py-4",
                "{quiz.question}"
            }

            div { class: "space-y-3",
                for (i, option) in quiz.options.iter().enumerate() {
                    {
                        let num = i.saturating_add(1);
                        let is_correct = i == quiz.correct_index;
                        let is_selected = selected() == Some(i);

                        let btn_class = if answered() {
                            if is_correct {
                                "w-full text-left px-4 py-3 rounded transition bg-emerald-700 text-white"
                            } else if is_selected {
                                "w-full text-left px-4 py-3 rounded transition bg-red-700 text-white"
                            } else {
                                "w-full text-left px-4 py-3 rounded transition bg-gray-700 text-gray-500"
                            }
                        } else {
                            "w-full text-left px-4 py-3 rounded transition bg-gray-700 hover:bg-gray-600 text-white cursor-pointer"
                        };

                        let prefix = if answered() && is_correct {
                            "✓"
                        } else if answered() && is_selected {
                            "✗"
                        } else {
                            " "
                        };

                        let correct_idx = quiz.correct_index;
                        let option_text = option.clone(); // clone: need owned copy for closure capture
                        rsx! {
                            button {
                                key: "{i}",
                                class: "{btn_class}",
                                disabled: answered(),
                                onclick: move |_| {
                                    if !answered() {
                                        selected.set(Some(i));
                                        answered.set(true);
                                        on_answer.call(i == correct_idx);
                                    }
                                },
                                "{prefix} [{num}] {option_text}"
                            }
                        }
                    }
                }
            }

            if answered() {
                div { class: "text-center mt-4",
                    button {
                        class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-2 font-semibold transition",
                        onclick: move |_| {
                            selected.set(None);
                            answered.set(false);
                            on_next.call(());
                        },
                        "Next Question"
                    }
                }
            }
        }
    }
}
