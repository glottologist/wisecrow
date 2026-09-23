use dioxus::prelude::*;
use std::collections::HashMap;
use wisecrow_dto::{PlacementResultDto, PlacementStateDto, PlacementStepDto};

use super::submission_for;
use crate::api::grammar::{start_placement, submit_placement};

#[component]
pub fn PlacementPage(native: String, foreign: String) -> Element {
    let mut step: Signal<Option<PlacementStepDto>> = use_signal(|| None);
    let mut result: Signal<Option<PlacementResultDto>> = use_signal(|| None);
    let mut answers: Signal<HashMap<i32, String>> = use_signal(HashMap::new);
    let mut started = use_signal(|| false);
    let mut loading = use_signal(|| false);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);

    let mut apply = move |state: PlacementStateDto| match state {
        PlacementStateDto::Testing(next) => {
            answers.set(HashMap::new());
            step.set(Some(next));
        }
        PlacementStateDto::Finished(outcome) => {
            step.set(None);
            result.set(Some(outcome));
        }
    };

    if let Some(outcome) = result() {
        let level = outcome
            .level_reached
            .clone() // clone: the rendered copy outlives the signal read
            .unwrap_or_else(|| String::from("below A1"));
        return rsx! {
            div { class: "max-w-lg mx-auto space-y-6 text-center py-12",
                h1 { class: "text-3xl font-bold", "Placement complete" }
                p { class: "text-2xl text-emerald-400 font-bold", "{level}" }
                p { class: "text-gray-400",
                    "Only the points you were actually asked about have been recorded. "
                    "The rest of the map stays unmarked."
                }
                div { class: "bg-gray-800 rounded-xl p-6 space-y-2 border border-gray-700 text-left",
                    for score in outcome.tested.iter() {
                        p { key: "{score.level}", class: "text-gray-300",
                            "{score.level}: {score.correct}/{score.asked} "
                            if score.passed { span { class: "text-emerald-400", "passed" } }
                            else { span { class: "text-red-400", "not passed" } }
                        }
                    }
                    for skipped in outcome.skipped.iter() {
                        p { key: "{skipped}", class: "text-gray-500",
                            "{skipped}: untested, too few approved exercises"
                        }
                    }
                }
            }
        };
    }

    let Some(current) = step() else {
        let native_code = native.clone(); // clone: moved into the async start handler
        let foreign_code = foreign.clone(); // clone: moved into the async start handler
        return rsx! {
            div { class: "max-w-lg mx-auto space-y-6",
                h1 { class: "text-3xl font-bold text-center", "Find your level" }
                p { class: "text-gray-400 text-center",
                    "A handful of questions per level, climbing until two levels in a row go badly."
                }

                if loading() || started() {
                    div { class: "text-center text-gray-400 py-8", "Preparing..." }
                } else {
                    button {
                        class: "w-full bg-emerald-600 hover:bg-emerald-500 rounded-xl px-6 py-3 font-semibold transition",
                        onclick: move |_| {
                            let native_code = native_code.clone(); // clone: moved into the async block
                            let foreign_code = foreign_code.clone(); // clone: moved into the async block
                            async move {
                                loading.set(true);
                                started.set(true);
                                error_msg.set(None);
                                match start_placement(native_code, foreign_code).await {
                                    Ok(state) => apply(state),
                                    Err(error) => {
                                        started.set(false);
                                        error_msg.set(Some(format!("Could not start: {error}")));
                                    }
                                }
                                loading.set(false);
                            }
                        },
                        "Begin"
                    }
                }

                if let Some(message) = error_msg() {
                    div { class: "text-red-400 text-center", "{message}" }
                }
            }
        };
    };

    let answered = answers().len();
    let asked = current.items.len();
    let step_for_submit = current.clone(); // clone: the submit handler outlives this render

    rsx! {
        div { class: "max-w-2xl mx-auto space-y-4",
            div { class: "flex justify-between text-sm text-gray-500",
                span { "Level {current.level}" }
                span { "{answered} / {asked} answered" }
            }

            for item in current.items.iter() {
                div {
                    key: "{item.item_id}",
                    class: "bg-gray-800 rounded-xl p-6 space-y-4 border border-gray-700",
                    p { class: "text-lg text-cyan-400 font-bold", "{item.prompt}" }

                    if item.options.is_empty() {
                        input {
                            r#type: "text",
                            class: "bg-gray-700 text-white rounded px-4 py-2 w-full",
                            placeholder: "Type your answer...",
                            value: "{answers().get(&item.item_id).cloned().unwrap_or_default()}",
                            oninput: {
                                let item_id = item.item_id;
                                move |evt: Event<FormData>| {
                                    let mut next = answers();
                                    next.insert(item_id, evt.value());
                                    answers.set(next);
                                }
                            },
                        }
                    } else {
                        div { class: "space-y-2",
                            for option in item.options.iter() {
                                {
                                    let item_id = item.item_id;
                                    let chosen = option.id.clone(); // clone: moved into this option's handler
                                    let selected = answers().get(&item_id) == Some(&option.id);
                                    let class = if selected {
                                        "w-full text-left px-4 py-3 rounded bg-cyan-700 text-white transition"
                                    } else {
                                        "w-full text-left px-4 py-3 rounded bg-gray-700 hover:bg-gray-600 text-white transition"
                                    };
                                    rsx! {
                                        button {
                                            key: "{option.id}",
                                            class: "{class}",
                                            onclick: move |_| {
                                                let mut next = answers();
                                                next.insert(item_id, chosen.clone()); // clone: the handler may fire again
                                                answers.set(next);
                                            },
                                            "{option.text}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            div { class: "text-center",
                button {
                    class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-3 font-semibold transition",
                    disabled: loading(),
                    onclick: {
                        let step = step_for_submit.clone(); // clone: moved into the async block
                        move |_| {
                            let step = step.clone(); // clone: each click needs its own copy
                            async move {
                                loading.set(true);
                                error_msg.set(None);
                                // An unanswered item is submitted empty rather than
                                // withheld: the ladder scores a level over what it
                                // asked, so skipping would quietly shrink the sample.
                                let given = answers();
                                let submissions = step
                                    .items
                                    .iter()
                                    .map(|item| {
                                        let answer = given
                                            .get(&item.item_id)
                                            .map_or("", String::as_str);
                                        submission_for(item, step.session_id, answer, false, 1)
                                    })
                                    .collect();
                                match submit_placement(step.attempt_id, submissions).await {
                                    Ok(state) => apply(state),
                                    Err(error) => {
                                        error_msg.set(Some(format!("Could not submit: {error}")));
                                    }
                                }
                                loading.set(false);
                            }
                        }
                    },
                    "Submit level"
                }
            }

            if let Some(message) = error_msg() {
                div { class: "text-red-400 text-center", "{message}" }
            }
        }
    }
}
