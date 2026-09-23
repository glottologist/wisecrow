use dioxus::prelude::*;
use uuid::Uuid;
use wisecrow_dto::GrammarItemDto;

use super::submission_for;
use crate::api::grammar::{complete_grammar_session, start_grammar_session, submit_grammar_answer};

/// What the learner is doing with the item in front of them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answered {
    No,
    Right,
    Wrong,
}

#[component]
pub fn GrammarPage(native: String, foreign: String) -> Element {
    let mut session_id: Signal<Option<Uuid>> = use_signal(|| None);
    let mut items: Signal<Vec<GrammarItemDto>> = use_signal(Vec::new);
    let mut index = use_signal(|| 0usize);
    let mut typed = use_signal(String::new);
    let mut answered = use_signal(|| Answered::No);
    let mut hint_shown = use_signal(|| false);
    let mut ordinal = use_signal(|| 0u32);
    let mut correct_count = use_signal(|| 0usize);
    let mut empty_bank = use_signal(|| false);
    let mut loading = use_signal(|| false);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);

    if session_id().is_none() {
        let native_code = native.clone(); // clone: moved into the async start handler
        let foreign_code = foreign.clone(); // clone: moved into the async start handler
        return rsx! {
            div { class: "max-w-lg mx-auto space-y-6",
                h1 { class: "text-3xl font-bold text-center", "Grammar practice" }
                p { class: "text-gray-400 text-center",
                    "Points chosen by what you are due to revisit and what you keep getting wrong."
                }

                if empty_bank() {
                    div { class: "bg-gray-800 rounded-xl p-6 text-center space-y-2 border border-gray-700",
                        p { class: "text-gray-300",
                            "No grammar exercises have been approved for {foreign_code} yet."
                        }
                        p { class: "text-sm text-gray-500",
                            "The syllabus exists; the exercise bank is still being reviewed."
                        }
                    }
                } else if loading() {
                    div { class: "text-center text-gray-400 py-8", "Choosing your points..." }
                } else {
                    button {
                        class: "w-full bg-emerald-600 hover:bg-emerald-500 rounded-xl px-6 py-3 font-semibold transition",
                        onclick: move |_| {
                            let native_code = native_code.clone(); // clone: moved into the async block
                            let foreign_code = foreign_code.clone(); // clone: moved into the async block
                            async move {
                                loading.set(true);
                                error_msg.set(None);
                                match start_grammar_session(native_code, foreign_code, None).await {
                                    Ok(Some(session)) => {
                                        items.set(session.items);
                                        session_id.set(Some(session.session_id));
                                    }
                                    Ok(None) => empty_bank.set(true),
                                    Err(error) => {
                                        error_msg.set(Some(format!("Could not start: {error}")));
                                    }
                                }
                                loading.set(false);
                            }
                        },
                        "Start"
                    }
                }

                if let Some(message) = error_msg() {
                    div { class: "text-red-400 text-center", "{message}" }
                }
            }
        };
    }

    let all = items();
    let position = index();
    let total = all.len();
    let Some(session) = session_id() else {
        return rsx! {};
    };

    if position >= total {
        return rsx! {
            div { class: "text-center space-y-4 py-20",
                h2 { class: "text-3xl font-bold text-emerald-400", "Session complete" }
                p { class: "text-xl text-gray-300", "{correct_count()}/{total} first time" }
            }
        };
    }

    let item = all[position].clone(); // clone: the handlers outlive this render
    let item_for_check = item.clone(); // clone: a second owned copy for the check handler
    let display = position.saturating_add(1);

    rsx! {
        div { class: "max-w-2xl mx-auto space-y-4",
            div { class: "flex justify-between text-sm text-gray-500",
                span { "{display} / {total}" }
                span { "{item.level} · {item.rule_title}" }
            }

            div { class: "bg-gray-800 rounded-xl p-8 space-y-6 border border-gray-700",
                p { class: "text-2xl text-cyan-400 font-bold text-center py-4", "{item.prompt}" }

                if hint_shown() {
                    if let Some(ref hint) = item.hint {
                        p { class: "text-yellow-400 text-center", "{hint}" }
                    }
                }

                match answered() {
                    Answered::No => rsx! {
                        div { class: "space-y-4",
                            if item.options.is_empty() {
                                div { class: "flex justify-center",
                                    input {
                                        r#type: "text",
                                        class: "bg-gray-700 text-white rounded px-4 py-2 w-full max-w-md text-center text-lg",
                                        placeholder: "Type your answer...",
                                        value: "{typed}",
                                        oninput: move |evt: Event<FormData>| typed.set(evt.value()),
                                    }
                                }
                            } else {
                                div { class: "space-y-3",
                                    for option in item.options.iter() {
                                        {
                                            let chosen = option.id.clone(); // clone: moved into this option's handler
                                            rsx! {
                                                button {
                                                    key: "{option.id}",
                                                    class: "w-full text-left px-4 py-3 rounded bg-gray-700 hover:bg-gray-600 text-white transition",
                                                    onclick: move |_| typed.set(chosen.clone()), // clone: the handler may fire again
                                                    "{option.text}"
                                                }
                                            }
                                        }
                                    }
                                    if !typed().is_empty() {
                                        p { class: "text-center text-gray-400", "Chosen: {typed}" }
                                    }
                                }
                            }

                            div { class: "flex justify-center gap-4",
                                if !hint_shown() && item.hint.is_some() {
                                    button {
                                        class: "bg-gray-700 hover:bg-gray-600 rounded px-4 py-2 transition",
                                        onclick: move |_| hint_shown.set(true),
                                        "Show hint"
                                    }
                                }
                                button {
                                    class: "bg-cyan-600 hover:bg-cyan-500 rounded px-6 py-2 font-semibold transition",
                                    disabled: typed().trim().is_empty(),
                                    onclick: {
                                        let item = item_for_check.clone(); // clone: moved into the async block
                                        move |_| {
                                            let item = item.clone(); // clone: each click needs its own copy
                                            async move {
                                                let attempt = ordinal().saturating_add(1);
                                                ordinal.set(attempt);
                                                let submission = submission_for(
                                                    &item,
                                                    session,
                                                    &typed(),
                                                    hint_shown(),
                                                    attempt,
                                                );
                                                match submit_grammar_answer(submission).await {
                                                    Ok(verdict) => {
                                                        if verdict.correct {
                                                            if attempt == 1 && !hint_shown() {
                                                                correct_count.set(
                                                                    correct_count().saturating_add(1),
                                                                );
                                                            }
                                                            answered.set(Answered::Right);
                                                        } else {
                                                            answered.set(Answered::Wrong);
                                                        }
                                                    }
                                                    Err(error) => {
                                                        error_msg
                                                            .set(Some(format!("Could not submit: {error}")));
                                                    }
                                                }
                                            }
                                        }
                                    },
                                    "Check"
                                }
                            }
                        }
                    },
                    Answered::Right => rsx! {
                        div { class: "text-center space-y-3",
                            p { class: "text-xl text-emerald-400 font-bold", "Correct" }
                            p { class: "text-gray-400", "{item.rule_explanation}" }
                        }
                    },
                    Answered::Wrong => rsx! {
                        div { class: "text-center space-y-3",
                            p { class: "text-xl text-red-400 font-bold", "Not quite" }
                            p { class: "text-gray-400", "{item.rule_explanation}" }
                        }
                    },
                }

                if answered() != Answered::No {
                    div { class: "text-center",
                        button {
                            class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-2 font-semibold transition",
                            onclick: move |_| {
                                async move {
                                    let next = position.saturating_add(1);
                                    typed.set(String::new());
                                    answered.set(Answered::No);
                                    hint_shown.set(false);
                                    ordinal.set(0);
                                    index.set(next);
                                    if next >= total {
                                        if let Err(error) = complete_grammar_session(session).await {
                                            error_msg
                                                .set(Some(format!("Could not close the session: {error}")));
                                        }
                                    }
                                }
                            },
                            if position.saturating_add(1) >= total { "Finish" } else { "Next" }
                        }
                    }
                }
            }

            if let Some(message) = error_msg() {
                div { class: "text-red-400 text-center", "{message}" }
            }
        }
    }
}
