use dioxus::prelude::*;

use wisecrow_dto::ClozeQuizDto;

#[component]
pub fn ClozeQuestion(
    quiz: ClozeQuizDto,
    on_answer: EventHandler<bool>,
    on_next: EventHandler<()>,
) -> Element {
    let mut revealed = use_signal(|| false);
    let mut show_hint = use_signal(|| false);

    rsx! {
        div { class: "bg-gray-800 rounded-xl p-8 space-y-6",
            h3 { class: "text-sm text-gray-500 uppercase tracking-wide", "Fill in the blank" }

            p { class: "text-2xl text-cyan-400 font-bold text-center py-4",
                "{quiz.sentence_with_blank}"
            }

            if revealed() {
                div { class: "text-center space-y-2",
                    p { class: "text-xl",
                        span { class: "text-gray-400", "Answer: " }
                        span { class: "text-emerald-400 font-bold", "{quiz.answer}" }
                    }
                    button {
                        class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-2 font-semibold transition mt-4",
                        onclick: move |_| {
                            revealed.set(false);
                            show_hint.set(false);
                            on_next.call(());
                        },
                        "Next Question"
                    }
                }
            } else {
                div { class: "text-center space-y-4",
                    if show_hint() {
                        if let Some(ref hint) = quiz.hint {
                            p { class: "text-yellow-400", "{hint}" }
                        }
                    }
                    div { class: "flex justify-center gap-4",
                        if !show_hint() {
                            button {
                                class: "bg-gray-700 hover:bg-gray-600 rounded px-4 py-2 transition",
                                onclick: move |_| show_hint.set(true),
                                "Show Hint"
                            }
                        }
                        button {
                            class: "bg-cyan-600 hover:bg-cyan-500 rounded px-6 py-2 font-semibold transition",
                            onclick: move |_| {
                                revealed.set(true);
                                on_answer.call(true);
                            },
                            "Reveal Answer"
                        }
                    }
                }
            }
        }
    }
}
