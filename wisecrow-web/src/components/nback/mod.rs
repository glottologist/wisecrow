mod game;
mod results;

use dioxus::prelude::*;
use wisecrow_dto::{
    DnbConfigDto, DnbModeDto, DnbSessionResultsDto, DnbTrialDto, DnbTrialResultDto,
};

#[cfg(feature = "server")]
use crate::server::nback::{complete_nback_session, start_nback_session, submit_nback_trial};

#[cfg(not(feature = "server"))]
mod server_stubs {
    use dioxus::prelude::*;
    use wisecrow_dto::{
        DnbAdaptationDto, DnbConfigDto, DnbSessionResultsDto, DnbTrialDto, DnbTrialResultDto,
    };

    #[server]
    pub async fn start_nback_session(
        config: DnbConfigDto,
    ) -> Result<(i32, Vec<DnbTrialDto>), ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn submit_nback_trial(
        session_id: i32,
        trial_result: DnbTrialResultDto,
        trial_dto: DnbTrialDto,
    ) -> Result<DnbAdaptationDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn complete_nback_session(
        session_id: i32,
        n_level: u8,
        interval_ms: u32,
        n_level_peak: u8,
        trials_completed: u32,
        accuracy_audio: f32,
        accuracy_visual: f32,
    ) -> Result<DnbSessionResultsDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }
}

#[cfg(not(feature = "server"))]
use server_stubs::*;

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
    let mut n_level_peak = use_signal(|| 2u8);
    let mut current_n = use_signal(|| 2u8);
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
                                                    user_id: 1,
                                                };
                                                match start_nback_session(config).await {
                                                    Ok((sid, trial_list)) => {
                                                        session_id.set(sid);
                                                        trials.set(trial_list);
                                                        current_idx.set(0);
                                                        audio_correct.set(0);
                                                        visual_correct.set(0);
                                                        total_responded.set(0);
                                                        n_level_peak.set(2);
                                                        current_n.set(2);
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
                let n = current_n();
                let peak = n_level_peak();
                let count = total_responded();
                let a_correct = audio_correct();
                let v_correct = visual_correct();
                let total = total_responded();
                let a_acc = if total > 0 {
                    a_correct as f32 / total as f32
                } else {
                    0.0
                };
                let v_acc = if total > 0 {
                    v_correct as f32 / total as f32
                } else {
                    0.0
                };

                use_effect(move || {
                    spawn(async move {
                        if let Ok(res) =
                            complete_nback_session(sid, n, 4000, peak, count, a_acc, v_acc).await
                        {
                            final_results.set(Some(res));
                            phase.set(Phase::Results);
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
            let a_acc = if responded > 0 {
                audio_correct() as f32 / responded as f32
            } else {
                0.0
            };
            let v_acc = if responded > 0 {
                visual_correct() as f32 / responded as f32
            } else {
                0.0
            };

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
                            if let Some(audio_resp) = result.audio_response {
                                if audio_resp == t.audio_match {
                                    audio_correct.set(audio_correct().saturating_add(1));
                                }
                            }
                            if let Some(visual_resp) = result.visual_response {
                                if visual_resp == t.visual_match {
                                    visual_correct.set(visual_correct().saturating_add(1));
                                }
                            }
                            total_responded.set(total_responded().saturating_add(1));

                            let _ = submit_nback_trial(sid, result, t).await;
                            current_idx.set(current_idx().saturating_add(1));
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
