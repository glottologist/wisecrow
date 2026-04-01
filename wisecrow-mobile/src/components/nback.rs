use dioxus::prelude::*;
use wisecrow_dto::{
    DnbConfigDto, DnbModeDto, DnbSessionResultsDto, DnbTrialDto, DnbTrialResultDto,
};

use crate::server_fns::{complete_nback_session, start_nback_session, submit_nback_trial};

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
    let mut final_results: Signal<Option<DnbSessionResultsDto>> = use_signal(|| None);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let mut loading = use_signal(|| false);

    match phase() {
        Phase::ModeSelect => {
            let modes = [
                ("Audio + Written", DnbModeDto::AudioWritten),
                ("Word + Translation", DnbModeDto::WordTranslation),
                ("Audio + Image", DnbModeDto::AudioImage),
            ];

            rsx! {
                div { style: "padding: 16px;",
                    h1 {
                        style: "font-size: 28px; font-weight: bold; text-align: center; margin-bottom: 8px;",
                        "Dual N-Back"
                    }
                    p {
                        style: "color: #9ca3af; text-align: center; margin-bottom: 32px;",
                        "Train working memory with {native}-{foreign} vocabulary"
                    }

                    if loading() {
                        div { style: "text-align: center; color: #9ca3af; padding: 24px;",
                            "Starting session..."
                        }
                    } else {
                        div { style: "display: flex; flex-direction: column; gap: 16px;",
                            {modes.iter().map(|(label, mode_dto)| {
                                let nat = native.clone(); // clone: captured by async closure
                                let for_lang = foreign.clone(); // clone: captured by async closure
                                let mode = *mode_dto;
                                rsx! {
                                    button {
                                        style: "background: #1f2937; border: none; border-radius: 16px; padding: 24px; color: #f3f4f6; font-size: 18px; font-weight: 600; text-align: center; min-height: 72px;",
                                        onclick: move |_| {
                                            let nat = nat.clone(); // clone: moved into async
                                            let for_lang = for_lang.clone(); // clone: moved into async
                                            async move {
                                                loading.set(true);
                                                error_msg.set(None);
                                                let config = DnbConfigDto {
                                                    mode,
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
                                                        phase.set(Phase::Playing);
                                                    }
                                                    Err(e) => {
                                                        error_msg.set(Some(format!("Failed: {e}")));
                                                    }
                                                }
                                                loading.set(false);
                                            }
                                        },
                                        "{label}"
                                    }
                                }
                            })}
                        }
                    }

                    if let Some(err) = error_msg() {
                        div { style: "color: #ef4444; text-align: center; margin-top: 16px;",
                            "{err}"
                        }
                    }
                }
            }
        }

        Phase::Playing => {
            let all_trials = trials();
            let idx = current_idx();

            if idx >= all_trials.len() {
                let sid = session_id();
                let peak = n_level_peak();
                let count = total_responded();
                let total = total_responded();
                let a_acc = if total > 0 {
                    audio_correct() as f32 / total as f32
                } else {
                    0.0
                };
                let v_acc = if total > 0 {
                    visual_correct() as f32 / total as f32
                } else {
                    0.0
                };

                use_effect(move || {
                    spawn(async move {
                        if let Ok(res) =
                            complete_nback_session(sid, 2, 4000, peak, count, a_acc, v_acc).await
                        {
                            final_results.set(Some(res));
                            phase.set(Phase::Results);
                        }
                    });
                });

                return rsx! {
                    div { style: "text-align: center; padding: 48px 0; color: #9ca3af;",
                        "Finishing session..."
                    }
                };
            }

            let trial = all_trials[idx].clone(); // clone: Dioxus component needs owned value
            let total = all_trials.len();
            let responded = total_responded();
            let audio_pct = if responded > 0 {
                (audio_correct() as f32 / responded as f32 * 100.0) as u32
            } else {
                0
            };
            let visual_pct = if responded > 0 {
                (visual_correct() as f32 / responded as f32 * 100.0) as u32
            } else {
                0
            };
            let display_idx = idx.saturating_add(1);

            rsx! {
                div { style: "padding: 16px; display: flex; flex-direction: column; min-height: calc(100vh - 96px);",

                    div { style: "display: flex; justify-content: space-between; color: #6b7280; font-size: 14px; margin-bottom: 16px;",
                        span { "Trial {display_idx}/{total}" }
                        span { "N={trial.n_level}" }
                    }

                    div { style: "flex: 1; display: flex; flex-direction: column; justify-content: center; gap: 24px;",
                        div { style: "background: #1e3a5f; border-radius: 16px; padding: 32px; text-align: center;",
                            p { style: "color: #60a5fa; font-size: 14px; margin-bottom: 8px;",
                                "Audio"
                            }
                            p { style: "font-size: 36px; font-weight: bold;",
                                "{trial.audio_phrase}"
                            }
                        }

                        div { style: "background: #3b1f5e; border-radius: 16px; padding: 32px; text-align: center;",
                            p { style: "color: #a78bfa; font-size: 14px; margin-bottom: 8px;",
                                "Visual"
                            }
                            p { style: "font-size: 36px; font-weight: bold;",
                                "{trial.visual_phrase}"
                            }
                        }
                    }

                    {
                        let trial_for_submit = trial.clone(); // clone: moved into event handler
                        let sid = session_id();
                        rsx! {
                            div { style: "display: flex; gap: 12px; margin-top: 16px;",
                                MobileMatchButton {
                                    label: "Audio Match",
                                    color: "#2563eb",
                                    trial: trial_for_submit.clone(), // clone: used in both buttons
                                    session_id: sid,
                                    is_audio: true,
                                    on_done: move |correct: (bool, bool)| {
                                        if correct.0 { audio_correct.set(audio_correct().saturating_add(1)); }
                                        if correct.1 { visual_correct.set(visual_correct().saturating_add(1)); }
                                        total_responded.set(total_responded().saturating_add(1));
                                        current_idx.set(current_idx().saturating_add(1));
                                    },
                                }
                            }

                            div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-top: 12px; margin-bottom: 16px;",
                                MatchResponseButtons {
                                    trial: trial.clone(), // clone: Dioxus component props
                                    session_id: sid,
                                    on_respond: move |(a_correct, v_correct): (bool, bool)| {
                                        if a_correct { audio_correct.set(audio_correct().saturating_add(1)); }
                                        if v_correct { visual_correct.set(visual_correct().saturating_add(1)); }
                                        total_responded.set(total_responded().saturating_add(1));
                                        current_idx.set(current_idx().saturating_add(1));
                                    },
                                }
                            }
                        }
                    }

                    div { style: "display: flex; justify-content: space-between; color: #6b7280; font-size: 12px;",
                        span { "Audio: {audio_pct}%" }
                        span { "Visual: {visual_pct}%" }
                    }
                }
            }
        }

        Phase::Results => {
            if let Some(res) = final_results() {
                let audio_pct = res.accuracy_audio.map(|a| (a * 100.0) as u32).unwrap_or(0);
                let visual_pct = res.accuracy_visual.map(|a| (a * 100.0) as u32).unwrap_or(0);

                rsx! {
                    div { style: "padding: 16px; text-align: center;",
                        h2 {
                            style: "font-size: 28px; font-weight: bold; color: #34d399; margin-bottom: 24px;",
                            "Session Complete"
                        }

                        div { style: "background: #1f2937; border-radius: 16px; padding: 24px; margin-bottom: 24px;",
                            div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 16px;",
                                div {
                                    p { style: "color: #6b7280; font-size: 14px;", "Trials" }
                                    p { style: "font-size: 28px; font-weight: bold;",
                                        "{res.trials_completed}"
                                    }
                                }
                                div {
                                    p { style: "color: #6b7280; font-size: 14px;", "Peak N" }
                                    p { style: "font-size: 28px; font-weight: bold; color: #f59e0b;",
                                        "{res.n_level_peak}"
                                    }
                                }
                                div {
                                    p { style: "color: #6b7280; font-size: 14px;", "Audio" }
                                    p { style: "font-size: 28px; font-weight: bold;",
                                        "{audio_pct}%"
                                    }
                                }
                                div {
                                    p { style: "color: #6b7280; font-size: 14px;", "Visual" }
                                    p { style: "font-size: 28px; font-weight: bold;",
                                        "{visual_pct}%"
                                    }
                                }
                            }
                        }

                        button {
                            style: "background: #059669; color: white; border: none; border-radius: 12px; padding: 16px 32px; font-size: 18px; font-weight: 600;",
                            onclick: move |_| {
                                phase.set(Phase::ModeSelect);
                                final_results.set(None);
                            },
                            "Play Again"
                        }
                    }
                }
            } else {
                rsx! {
                    div { style: "text-align: center; padding: 48px 0; color: #9ca3af;",
                        "Loading results..."
                    }
                }
            }
        }
    }
}

#[component]
fn MobileMatchButton(
    label: &'static str,
    color: &'static str,
    trial: DnbTrialDto,
    session_id: i32,
    is_audio: bool,
    on_done: EventHandler<(bool, bool)>,
) -> Element {
    rsx! {}
}

#[component]
fn MatchResponseButtons(
    trial: DnbTrialDto,
    session_id: i32,
    on_respond: EventHandler<(bool, bool)>,
) -> Element {
    let mut audio_pressed = use_signal(|| false);
    let mut visual_pressed = use_signal(|| false);

    rsx! {
        button {
            style: if audio_pressed() {
                "background: #2563eb; color: white; border: none; border-radius: 12px; padding: 20px; font-size: 16px; font-weight: 600; min-height: 64px; ring: 2px solid #60a5fa;"
            } else {
                "background: #374151; color: white; border: none; border-radius: 12px; padding: 20px; font-size: 16px; font-weight: 600; min-height: 64px;"
            },
            onclick: move |_| audio_pressed.set(!audio_pressed()),
            "Audio Match"
        }
        button {
            style: if visual_pressed() {
                "background: #7c3aed; color: white; border: none; border-radius: 12px; padding: 20px; font-size: 16px; font-weight: 600; min-height: 64px; ring: 2px solid #a78bfa;"
            } else {
                "background: #374151; color: white; border: none; border-radius: 12px; padding: 20px; font-size: 16px; font-weight: 600; min-height: 64px;"
            },
            onclick: move |_| visual_pressed.set(!visual_pressed()),
            "Visual Match"
        }
        button {
            style: "background: #059669; color: white; border: none; border-radius: 12px; padding: 20px; font-size: 16px; font-weight: 600; min-height: 64px; grid-column: span 2;",
            onclick: move |_| {
                let t = trial.clone(); // clone: captured by async closure
                let sid = session_id;
                async move {
                    let a_resp = audio_pressed();
                    let v_resp = visual_pressed();
                    let result = DnbTrialResultDto {
                        trial_number: t.trial_number,
                        audio_response: Some(a_resp),
                        visual_response: Some(v_resp),
                        response_time_ms: None,
                    };
                    let _ = submit_nback_trial(sid, result, t.clone()).await; // clone: submit needs owned copy
                    let a_correct = a_resp == t.audio_match;
                    let v_correct = v_resp == t.visual_match;
                    on_respond.call((a_correct, v_correct));
                    audio_pressed.set(false);
                    visual_pressed.set(false);
                }
            },
            "Submit"
        }
    }
}
