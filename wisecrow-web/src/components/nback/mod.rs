mod game;
mod results;

use dioxus::prelude::*;
use wisecrow_dto::{
    DnbConfigDto, DnbModeDto, DnbSessionResultsDto, DnbTrialDto, DnbTrialResultDto,
};

use crate::api::nback::{complete_nback_session, start_nback_session, submit_nback_trial};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    ModeSelect,
    Playing,
    Results,
}

#[component]
pub fn NbackPage(native: String, foreign: String) -> Element {
    let mut phase = use_signal(|| Phase::ModeSelect);
    let mut session_id = use_signal(|| 0i32);
    let mut trials: Signal<Vec<DnbTrialDto>> = use_signal(Vec::new);
    let mut current_idx = use_signal(|| 0usize);
    let mut audio_correct = use_signal(|| 0u32);
    let mut visual_correct = use_signal(|| 0u32);
    let mut total_responded = use_signal(|| 0u32);
    let mut final_results: Signal<Option<DnbSessionResultsDto>> = use_signal(|| None);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let mut loading = use_signal(|| false);

    let native_code = native.clone(); // clone: captured by multiple closures
    let foreign_code = foreign.clone(); // clone: captured by multiple closures

    match phase() {
        Phase::ModeSelect => {
            rsx! {
                div { class: "max-w-lg mx-auto space-y-6",
                    h1 { class: "text-3xl font-bold text-center", "Dual N-Back" }
                    p { class: "text-gray-400 text-center",
                        "Train working memory with {native_code}-{foreign_code} vocabulary"
                    }

                    if loading() {
                        div { class: "text-center text-gray-400 py-8",
                            "Starting session..."
                        }
                    } else {
                        div { class: "space-y-3",
                            {["AudioWritten", "WordTranslation", "AudioImage"].iter().map(|mode_name| {
                                let mode_dto = match *mode_name {
                                    "AudioWritten" => DnbModeDto::AudioWritten,
                                    "WordTranslation" => DnbModeDto::WordTranslation,
                                    _ => DnbModeDto::AudioImage,
                                };
                                let nat = native.clone(); // clone: captured by async closure
                                let for_lang = foreign.clone(); // clone: captured by async closure
                                let label = match *mode_name {
                                    "AudioWritten" => "Audio + Written",
                                    "WordTranslation" => "Word + Translation",
                                    _ => "Audio + Image",
                                };
                                rsx! {
                                    button {
                                        class: "w-full bg-gray-800 hover:bg-gray-700 rounded-xl p-4 text-left transition",
                                        onclick: move |_| {
                                            let nat = nat.clone(); // clone: moved into async block
                                            let for_lang = for_lang.clone(); // clone: moved into async block
                                            async move {
                                                loading.set(true);
                                                error_msg.set(None);
                                                let config = DnbConfigDto {
                                                    mode: mode_dto,
                                                    n_level: 2,
                                                    interval_ms: 4000,
                                                    native_lang: nat,
                                                    foreign_lang: for_lang,
                                                };
                                                match start_nback_session(config).await {
                                                    Ok((sid, trial_list)) => {
                                                        session_id.set(sid);
                                                        trials.set(trial_list);
                                                        current_idx.set(0);
                                                        audio_correct.set(0);
                                                        visual_correct.set(0);
                                                        total_responded.set(0);
                                                        phase.set(Phase::Playing);
                                                    }
                                                    Err(e) => {
                                                        error_msg.set(Some(format!("Failed to start: {e}")));
                                                    }
                                                }
                                                loading.set(false);
                                            }
                                        },
                                        span { class: "text-lg font-semibold", "{label}" }
                                    }
                                }
                            })}
                        }
                    }

                    if let Some(err) = error_msg() {
                        div { class: "text-red-400 text-center", "{err}" }
                    }
                }
            }
        }

        Phase::Playing => {
            let all_trials = trials();
            let idx = current_idx();

            if idx >= all_trials.len() {
                let sid = session_id();

                use_effect(move || {
                    spawn(async move {
                        match complete_nback_session(sid).await {
                            Ok(res) => {
                                final_results.set(Some(res));
                                phase.set(Phase::Results);
                            }
                            Err(error) => {
                                error_msg.set(Some(format!("Failed to complete session: {error}")));
                            }
                        }
                    });
                });

                return rsx! {
                    div { class: "text-center text-gray-400 py-20",
                        "Finishing session..."
                    }
                };
            }

            let trial = all_trials[idx].clone(); // clone: Dioxus component props require owned values
            let total = all_trials.len();
            let responded = total_responded();
            let a_acc = wisecrow_dto::channel_ratio(audio_correct(), responded);
            let v_acc = wisecrow_dto::channel_ratio(visual_correct(), responded);

            rsx! {
                game::NbackGame {
                    trial: trial.clone(), // clone: Dioxus component props require owned values
                    trial_index: idx,
                    total_trials: total,
                    audio_accuracy: a_acc,
                    visual_accuracy: v_acc,
                    on_respond: move |result: DnbTrialResultDto| {
                        let t = trial.clone(); // clone: captured by async closure
                        let sid = session_id();
                        async move {
                            let audio_was_correct =
                                result.audio_response == Some(t.audio_match);
                            let visual_was_correct =
                                result.visual_response == Some(t.visual_match);
                            match submit_nback_trial(sid, result).await {
                                Ok(_) => {
                                    if audio_was_correct {
                                        audio_correct.set(audio_correct().saturating_add(1));
                                    }
                                    if visual_was_correct {
                                        visual_correct.set(visual_correct().saturating_add(1));
                                    }
                                    total_responded.set(total_responded().saturating_add(1));
                                    current_idx.set(current_idx().saturating_add(1));
                                }
                                Err(error) => {
                                    error_msg
                                        .set(Some(format!("Failed to submit response: {error}")));
                                }
                            }
                        }
                    },
                }
            }
        }

        Phase::Results => {
            if let Some(res) = final_results() {
                rsx! {
                    results::NbackResults {
                        results: res,
                        on_restart: move |_| {
                            phase.set(Phase::ModeSelect);
                            final_results.set(None);
                        },
                    }
                }
            } else {
                rsx! {
                    div { class: "text-center text-gray-400 py-20",
                        "Loading results..."
                    }
                }
            }
        }
    }
}
