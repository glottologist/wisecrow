use dioxus::prelude::*;

use wisecrow_dto::ClozeQuizDto;

#[derive(Clone, Copy, PartialEq)]
enum ClozeState {
    Unanswered,
    Correct,
    Incorrect,
    Revealed,
}

#[component]
pub fn ClozeQuestion(
    quiz: ClozeQuizDto,
    on_answer: EventHandler<bool>,
    on_next: EventHandler<()>,
) -> Element {
    let mut user_input = use_signal(String::new);
    let mut state = use_signal(|| ClozeState::Unanswered);
    let mut show_hint = use_signal(|| false);

    let answer = quiz.answer.clone(); // clone: need owned String for closure capture below

    rsx! {
        div { class: "bg-gray-800 rounded-xl p-8 space-y-6",
            h3 { class: "text-sm text-gray-500 uppercase tracking-wide", "Fill in the blank" }

            p { class: "text-2xl text-cyan-400 font-bold text-center py-4",
                "{quiz.sentence_with_blank}"
            }

            match state() {
                ClozeState::Unanswered => rsx! {
                    div { class: "space-y-4",
                        div { class: "flex justify-center",
                            input {
                                r#type: "text",
                                class: "bg-gray-700 text-white rounded px-4 py-2 w-full max-w-md text-center text-lg focus:outline-none focus:ring-2 focus:ring-cyan-500",
                                placeholder: "Type your answer...",
                                value: "{user_input}",
                                oninput: move |evt: Event<FormData>| {
                                    user_input.set(evt.value());
                                },
                            }
                        }

                        if show_hint() {
                            if let Some(ref hint) = quiz.hint {
                                p { class: "text-yellow-400 text-center", "{hint}" }
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
                                disabled: user_input().trim().is_empty(),
                                onclick: {
                                    let answer_for_check = answer.clone(); // clone: need second owned copy for separate closure
                                    move |_| {
                                        let correct = user_input()
                                            .trim()
                                            .eq_ignore_ascii_case(answer_for_check.trim());
                                        if correct {
                                            state.set(ClozeState::Correct);
                                        } else {
                                            state.set(ClozeState::Incorrect);
                                        }
                                        on_answer.call(correct);
                                    }
                                },
                                "Check Answer"
                            }
                            button {
                                class: "bg-gray-600 hover:bg-gray-500 rounded px-4 py-2 transition text-gray-300",
                                onclick: move |_| {
                                    state.set(ClozeState::Revealed);
                                    on_answer.call(false);
                                },
                                "Reveal Answer"
                            }
                        }
                    }
                },
                ClozeState::Correct => rsx! {
                    div { class: "text-center space-y-2",
                        p { class: "text-xl text-emerald-400 font-bold", "Correct!" }
                        p { class: "text-lg",
                            span { class: "text-gray-400", "Answer: " }
                            span { class: "text-emerald-400 font-bold", "{quiz.answer}" }
                        }
                        button {
                            class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-2 font-semibold transition mt-4",
                            onclick: move |_| {
                                user_input.set(String::new());
                                state.set(ClozeState::Unanswered);
                                show_hint.set(false);
                                on_next.call(());
                            },
                            "Next Question"
                        }
                    }
                },
                ClozeState::Incorrect => rsx! {
                    div { class: "text-center space-y-2",
                        p { class: "text-xl text-red-400 font-bold", "Incorrect" }
                        p { class: "text-lg",
                            span { class: "text-gray-400", "Your answer: " }
                            span { class: "text-red-400", "{user_input}" }
                        }
                        p { class: "text-lg",
                            span { class: "text-gray-400", "Correct answer: " }
                            span { class: "text-emerald-400 font-bold", "{quiz.answer}" }
                        }
                        button {
                            class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-2 font-semibold transition mt-4",
                            onclick: move |_| {
                                user_input.set(String::new());
                                state.set(ClozeState::Unanswered);
                                show_hint.set(false);
                                on_next.call(());
                            },
                            "Next Question"
                        }
                    }
                },
                ClozeState::Revealed => rsx! {
                    div { class: "text-center space-y-2",
                        p { class: "text-xl text-yellow-400 font-bold", "Answer Revealed" }
                        p { class: "text-lg",
                            span { class: "text-gray-400", "Answer: " }
                            span { class: "text-emerald-400 font-bold", "{quiz.answer}" }
                        }
                        button {
                            class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-2 font-semibold transition mt-4",
                            onclick: move |_| {
                                user_input.set(String::new());
                                state.set(ClozeState::Unanswered);
                                show_hint.set(false);
                                on_next.call(());
                            },
                            "Next Question"
                        }
                    }
                },
            }
        }
    }
}
