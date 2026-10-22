use dioxus::prelude::*;
use uuid::Uuid;
use wisecrow_dto::GrammarItemDto;

use super::{gloss_for, selected_phrase, submission_for};
use crate::api::grammar::{complete_grammar_session, start_grammar_session, submit_grammar_answer};
use crate::api::media::{get_answer_audio_data, get_example_audio_data};
use crate::components::area::{requested_area, AreaSelect};

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
    let mut playing: Signal<Option<(i32, String)>> = use_signal(|| None);
    let mut audio_failed: Signal<Option<i32>> = use_signal(|| None);
    let area = use_signal(|| String::from("all"));

    use_effect(move || {
        let _chosen = area();
        empty_bank.set(false);
    });

    if session_id().is_none() {
        let native_code = native.clone(); // clone: moved into the async start handler
        let foreign_code = foreign.clone(); // clone: moved into the async start handler
        return rsx! {
            div { class: "max-w-lg mx-auto space-y-6",
                h1 { class: "text-3xl font-bold text-center", "Grammar practice" }
                p { class: "text-gray-400 text-center",
                    "Points chosen by what you are due to revisit and what you keep getting wrong."
                }
                AreaSelect { value: area }

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
                            let chosen = requested_area(&area());
                            async move {
                                loading.set(true);
                                error_msg.set(None);
                                match start_grammar_session(native_code, foreign_code, None, chosen)
                                    .await
                                {
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
                    Answered::Right => {
                        let phrase = selected_phrase(&item, &typed());
                        let gloss = phrase
                            .as_deref()
                            .and_then(|phrase| gloss_for(&item, phrase))
                            .map(ToOwned::to_owned); // clone: the player keeps the gloss
                        let answer = typed().trim().to_owned();
                        rsx! {
                            div { class: "text-center space-y-3",
                                p { class: "text-xl text-emerald-400 font-bold", "Correct" }
                                p { class: "text-gray-400", "{item.rule_explanation}" }
                                if let Some(phrase) = phrase {
                                    AnswerAudio {
                                        phrase,
                                        gloss,
                                        request: AnswerRequest {
                                            item_id: item.item_id,
                                            revision: item.revision,
                                            answer,
                                            chose_option: !item.options.is_empty(),
                                            card: position,
                                        },
                                        index,
                                        playing,
                                        audio_failed,
                                    }
                                }
                            }
                        }
                    },
                    Answered::Wrong => rsx! {
                        div { class: "text-center space-y-3",
                            p { class: "text-xl text-red-400 font-bold", "Not quite" }
                            p { class: "text-gray-400", "{item.rule_explanation}" }
                            Examples { examples: item.examples.clone(), playing, audio_failed } // clone: the component takes owned props
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
                                    playing.set(None);
                                    audio_failed.set(None);
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

/// What the server needs in order to speak the phrase just chosen.
#[derive(Clone, PartialEq, Eq)]
struct AnswerRequest {
    item_id: i32,
    revision: i32,
    answer: String,
    chose_option: bool,
    card: usize,
}

/// The phrase the learner just got right, played as soon as its clip is ready.
///
/// The clip is the option they chose, or the cloze with their answer filled
/// in. A sample sentence of the grammar point is a different sentence and is
/// not what this plays. The server refuses the request unless that phrase is
/// the stored answer, so a wrong form is never spoken.
#[component]
fn AnswerAudio(
    phrase: String,
    gloss: Option<String>,
    request: AnswerRequest,
    index: Signal<usize>,
    playing: Signal<Option<(i32, String)>>,
    audio_failed: Signal<Option<i32>>,
) -> Element {
    let request_for_mount = request.clone(); // clone: the mount effect owns its request
    use_effect(move || {
        let request = request_for_mount.clone(); // clone: a rerun must keep a copy to send
        spawn(async move {
            publish_answer_clip(request, index, playing, audio_failed).await;
        });
    });

    let item_id = request.item_id;
    let src = playing()
        .filter(|(id, _)| *id == item_id)
        .map(|(_, src)| src);
    rsx! {
        div { class: "text-left bg-gray-800 rounded-lg px-4 py-2 flex items-center gap-3 mt-4",
            button {
                class: "rounded bg-emerald-600 hover:bg-emerald-500 px-3 py-1 font-bold transition",
                aria_label: "Play answer",
                onclick: {
                    let request = request.clone(); // clone: each tap owns its request
                    move |_| {
                        let request = request.clone(); // clone: the handler may fire again
                        async move {
                            publish_answer_clip(request, index, playing, audio_failed).await;
                        }
                    }
                },
                "▶"
            }
            div { class: "space-y-1",
                p { class: "text-gray-300", "{phrase}" }
                if let Some(gloss) = gloss {
                    p { class: "text-sm text-gray-500", "{gloss}" }
                }
                if let Some(src) = src {
                    audio { src: "{src}", autoplay: true, controls: true, class: "mt-1" }
                }
                if audio_failed() == Some(item_id) {
                    p { class: "text-sm text-red-400", "Audio unavailable" }
                }
            }
        }
    }
}

async fn publish_answer_clip(
    request: AnswerRequest,
    index: Signal<usize>,
    mut playing: Signal<Option<(i32, String)>>,
    mut audio_failed: Signal<Option<i32>>,
) {
    let result = get_answer_audio_data(
        request.item_id,
        request.revision,
        request.answer,
        request.chose_option,
    )
    .await;
    if index() != request.card {
        return;
    }
    match result {
        Ok(url) => {
            audio_failed.set(None);
            playing.set(Some((request.item_id, url)));
        }
        Err(_) => {
            playing.set(None);
            audio_failed.set(Some(request.item_id));
        }
    }
}

/// A point's correct examples, each with a play control.
///
/// One clip plays at a time: pressing another example replaces the audio
/// element rather than layering a second voice over the first.
#[component]
fn Examples(
    examples: Vec<wisecrow_dto::RuleExampleDto>,
    playing: Signal<Option<(i32, String)>>,
    audio_failed: Signal<Option<i32>>,
) -> Element {
    if examples.is_empty() {
        return rsx! {};
    }
    rsx! {
        ul { class: "text-left space-y-2 mt-4",
            for example in examples {
                li { class: "bg-gray-800 rounded-lg px-4 py-2 flex items-center gap-3",
                    button {
                        class: "rounded bg-emerald-600 hover:bg-emerald-500 px-3 py-1 font-bold transition",
                        aria_label: "Play example",
                        onclick: {
                            let id = example.id;
                            move |_| {
                                let mut playing = playing;
                                let mut audio_failed = audio_failed;
                                async move {
                                    audio_failed.set(None);
                                    match get_example_audio_data(id).await {
                                        Ok(url) => playing.set(Some((id, url))),
                                        Err(_) => {
                                            playing.set(None);
                                            audio_failed.set(Some(id));
                                        }
                                    }
                                }
                            }
                        },
                        "▶"
                    }
                    div { class: "space-y-1",
                        p { class: "text-gray-300", "{example.sentence}" }
                        if let Some(ref translation) = example.translation {
                            p { class: "text-sm text-gray-500", "{translation}" }
                        }
                        if let Some(src) = playing().filter(|(id, _)| *id == example.id).map(|(_, src)| src) {
                            audio { src: "{src}", autoplay: true, controls: true, class: "mt-1" }
                        }
                        if audio_failed() == Some(example.id) {
                            p { class: "text-sm text-red-400", "Audio unavailable" }
                        }
                    }
                }
            }
        }
    }
}
