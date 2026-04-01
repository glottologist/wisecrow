use dioxus::prelude::*;
use wisecrow_dto::{DnbTrialDto, DnbTrialResultDto};

#[component]
pub fn NbackGame(
    trial: DnbTrialDto,
    trial_index: usize,
    total_trials: usize,
    audio_accuracy: f32,
    visual_accuracy: f32,
    on_respond: EventHandler<DnbTrialResultDto>,
) -> Element {
    let mut audio_pressed = use_signal(|| false);
    let mut visual_pressed = use_signal(|| false);
    let mut submitted = use_signal(|| false);

    let display_idx = trial_index.saturating_add(1);
    let audio_pct = (audio_accuracy * 100.0) as u32;
    let visual_pct = (visual_accuracy * 100.0) as u32;

    rsx! {
        div { class: "max-w-lg mx-auto space-y-6",
            div { class: "flex justify-between text-sm text-gray-500",
                span { "Trial {display_idx} / {total_trials}" }
                span { "N = {trial.n_level}" }
            }

            div { class: "bg-gray-800 rounded-xl p-8 text-center space-y-6",
                div {
                    p { class: "text-gray-400 text-sm mb-1", "Audio" }
                    p { class: "text-3xl font-bold text-blue-400",
                        "{trial.audio_phrase}"
                    }
                }

                div {
                    p { class: "text-gray-400 text-sm mb-1", "Visual" }
                    p { class: "text-3xl font-bold text-purple-400",
                        "{trial.visual_phrase}"
                    }
                }
            }

            if !submitted() {
                div { class: "flex gap-4 justify-center",
                    button {
                        class: if audio_pressed() {
                            "bg-blue-600 text-white rounded px-6 py-3 font-semibold ring-2 ring-blue-400"
                        } else {
                            "bg-gray-700 hover:bg-blue-600 text-white rounded px-6 py-3 font-semibold transition"
                        },
                        onclick: move |_| {
                            audio_pressed.set(!audio_pressed());
                        },
                        "Audio Match (A)"
                    }

                    button {
                        class: if visual_pressed() {
                            "bg-purple-600 text-white rounded px-6 py-3 font-semibold ring-2 ring-purple-400"
                        } else {
                            "bg-gray-700 hover:bg-purple-600 text-white rounded px-6 py-3 font-semibold transition"
                        },
                        onclick: move |_| {
                            visual_pressed.set(!visual_pressed());
                        },
                        "Visual Match (L)"
                    }
                }

                div { class: "text-center",
                    button {
                        class: "bg-emerald-600 hover:bg-emerald-500 rounded px-8 py-2 font-semibold transition",
                        onclick: move |_| {
                            submitted.set(true);
                            let result = DnbTrialResultDto {
                                trial_number: trial.trial_number,
                                audio_response: Some(audio_pressed()),
                                visual_response: Some(visual_pressed()),
                                response_time_ms: None,
                            };
                            on_respond.call(result);
                            audio_pressed.set(false);
                            visual_pressed.set(false);
                            submitted.set(false);
                        },
                        "Submit"
                    }
                }
            }

            div { class: "flex justify-between text-sm text-gray-500",
                span { "Audio: {audio_pct}%" }
                span { "Visual: {visual_pct}%" }
            }
        }
    }
}
