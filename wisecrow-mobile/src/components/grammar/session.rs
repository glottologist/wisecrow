use std::sync::Arc;

use dioxus::prelude::*;
use uuid::Uuid;
use wisecrow_learning::grading::grade;

use super::{queued_attempt, SESSION_LENGTH};
use crate::application::{LocalGrammarItem, LocalStore};

/// What the learner is doing with the item in front of them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answered {
    No,
    Right,
    Wrong,
}

/// A practice run drawn entirely from the mirrored bank.
///
/// Nothing here reaches the network. The answer is marked against the cached
/// item and written to the outbox, which is what lets the session run on a
/// train and converge later.
#[component]
pub fn GrammarSessionPage(native: String, foreign: String) -> Element {
    let _ = native;
    let store = use_context::<Arc<dyn LocalStore>>();
    let language = foreign.clone(); // clone: the resource owns its language code

    let items = use_resource(move || {
        let store = Arc::clone(&store); // clone: the future shares the process-wide store
        let language = language.clone(); // clone: the future owns its language code
        async move {
            store
                .grammar_items(&language)
                .await
                .map(|mut items| {
                    items.truncate(SESSION_LENGTH);
                    items
                })
                .unwrap_or_default()
        }
    });

    let mut session_id = use_signal(Uuid::new_v4);
    let mut index = use_signal(|| 0usize);
    let mut typed = use_signal(String::new);
    let mut answered = use_signal(|| Answered::No);
    let mut hint_shown = use_signal(|| false);
    let mut ordinal = use_signal(|| 0u32);
    let mut correct_count = use_signal(|| 0usize);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);

    let reading = items.read();
    let Some(deck) = reading.as_ref() else {
        return rsx! { p { "Loading your points..." } };
    };
    if deck.is_empty() {
        return rsx! {
            section {
                h1 { "Grammar" }
                p { "No grammar exercises have reached this device for {foreign} yet." }
            }
        };
    }

    let total = deck.len();
    let position = index();
    if position >= total {
        return rsx! {
            section {
                h1 { "Session complete" }
                p { "{correct_count()}/{total} first time" }
            }
        };
    }

    let item: LocalGrammarItem = deck[position].clone(); // clone: the handlers outlive this render
    let item_for_check = item.clone(); // clone: a second owned copy for the check handler
    let store_for_check = use_context::<Arc<dyn LocalStore>>();
    let display = position.saturating_add(1);

    rsx! {
        section {
            p { style: "color: #9ca3af; font-size: 12px;", "{display} / {total}" }
            p { style: "font-size: 20px; font-weight: 600; margin: 16px 0;", "{item.prompt}" }

            if hint_shown() {
                if let Some(ref hint) = item.hint {
                    p { style: "color: #fbbf24;", "{hint}" }
                }
            }

            match answered() {
                Answered::No => rsx! {
                    div {
                        if item.options.is_empty() {
                            input {
                                r#type: "text",
                                style: "width: 100%; padding: 12px; border-radius: 8px; border: 1px solid #374151; background: #1f2937; color: #f3f4f6;",
                                placeholder: "Type your answer...",
                                value: "{typed}",
                                oninput: move |evt: Event<FormData>| typed.set(evt.value()),
                            }
                        } else {
                            div {
                                for option in item.options.iter() {
                                    {
                                        let chosen = option.id.clone(); // clone: moved into this option's handler
                                        rsx! {
                                            button {
                                                key: "{option.id}",
                                                style: "display: block; width: 100%; text-align: left; padding: 12px; margin-bottom: 8px; border-radius: 8px; border: 1px solid #374151; background: #1f2937; color: #f3f4f6;",
                                                onclick: move |_| typed.set(chosen.clone()), // clone: the handler may fire again
                                                "{option.text}"
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        div { style: "display: flex; gap: 8px; margin-top: 16px;",
                            if !hint_shown() && item.hint.is_some() {
                                button {
                                    style: "padding: 12px 16px; border-radius: 8px; border: none; background: #374151; color: #f3f4f6;",
                                    onclick: move |_| hint_shown.set(true),
                                    "Show hint"
                                }
                            }
                            button {
                                style: "padding: 12px 16px; border-radius: 8px; border: none; background: #0e7490; color: #f3f4f6;",
                                disabled: typed().trim().is_empty(),
                                onclick: {
                                    let item = item_for_check.clone(); // clone: moved into the async block
                                    let store = Arc::clone(&store_for_check); // clone: shares the store resource
                                    move |_| {
                                        let item = item.clone(); // clone: each click needs its own copy
                                        let store = Arc::clone(&store); // clone: each click shares the store
                                        async move {
                                            let attempt_number = ordinal().saturating_add(1);
                                            ordinal.set(attempt_number);
                                            let chose_option = !item.options.is_empty();
                                            let attempt = queued_attempt(
                                                &item,
                                                session_id(),
                                                &typed(),
                                                chose_option,
                                                hint_shown(),
                                                attempt_number,
                                                None,
                                            );
                                            let correct = grade(
                                                &item.gradable(),
                                                &attempt.submission(),
                                            )
                                            .correct;
                                            if let Err(error) = store.queue_attempt(&attempt).await {
                                                error_msg
                                                    .set(Some(format!("Could not record: {error}")));
                                                return;
                                            }
                                            if correct {
                                                if attempt_number == 1 && !hint_shown() {
                                                    correct_count
                                                        .set(correct_count().saturating_add(1));
                                                }
                                                answered.set(Answered::Right);
                                            } else {
                                                answered.set(Answered::Wrong);
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
                    p { style: "color: #34d399; font-weight: 600;", "Correct" }
                },
                Answered::Wrong => rsx! {
                    p { style: "color: #f87171; font-weight: 600;", "Not quite" }
                },
            }

            if answered() != Answered::No {
                button {
                    style: "margin-top: 16px; padding: 12px 16px; border-radius: 8px; border: none; background: #047857; color: #f3f4f6;",
                    onclick: move |_| {
                        typed.set(String::new());
                        answered.set(Answered::No);
                        hint_shown.set(false);
                        ordinal.set(0);
                        let next = position.saturating_add(1);
                        index.set(next);
                        if next >= total {
                            // A finished sitting starts the next one afresh, so
                            // that the interaction rule keeps its meaning.
                            session_id.set(Uuid::new_v4());
                        }
                    },
                    if position.saturating_add(1) >= total { "Finish" } else { "Next" }
                }
            }

            if let Some(message) = error_msg() {
                p { style: "color: #f87171;", "{message}" }
            }
        }
    }
}
