use dioxus::prelude::*;
use wisecrow_dto::DnbSessionResultsDto;

#[component]
pub fn NbackResults(results: DnbSessionResultsDto, on_restart: EventHandler<()>) -> Element {
    let audio_pct = results
        .accuracy_audio
        .map(|a| (a * 100.0) as u32)
        .unwrap_or(0);
    let visual_pct = results
        .accuracy_visual
        .map(|a| (a * 100.0) as u32)
        .unwrap_or(0);

    rsx! {
        div { class: "max-w-lg mx-auto space-y-6 py-10",
            h2 { class: "text-3xl font-bold text-center text-emerald-400",
                "Session Complete"
            }

            div { class: "bg-gray-800 rounded-xl p-6 space-y-4",
                div { class: "grid grid-cols-2 gap-4 text-center",
                    div {
                        p { class: "text-gray-400 text-sm", "Trials" }
                        p { class: "text-2xl font-bold", "{results.trials_completed}" }
                    }
                    div {
                        p { class: "text-gray-400 text-sm", "Peak N-Level" }
                        p { class: "text-2xl font-bold text-amber-400",
                            "{results.n_level_peak}"
                        }
                    }
                    div {
                        p { class: "text-gray-400 text-sm", "Audio Accuracy" }
                        p { class: "text-2xl font-bold", "{audio_pct}%" }
                    }
                    div {
                        p { class: "text-gray-400 text-sm", "Visual Accuracy" }
                        p { class: "text-2xl font-bold", "{visual_pct}%" }
                    }
                }

                div { class: "text-center text-gray-500 text-sm pt-2",
                    "N-Level: {results.n_level_start} → {results.n_level_peak} → {results.n_level_end}"
                }
            }

            div { class: "text-center",
                button {
                    class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-3 font-semibold transition",
                    onclick: move |_| on_restart.call(()),
                    "Play Again"
                }
            }
        }
    }
}
