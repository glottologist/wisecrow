use dioxus::prelude::*;

#[component]
pub fn StatsPanel(
    cards_seen: usize,
    total: usize,
    streak: usize,
    speed_ms: u32,
    paused: bool,
    on_speed_up: EventHandler<()>,
    on_slow_down: EventHandler<()>,
    on_pause_toggle: EventHandler<()>,
) -> Element {
    let pct = if total > 0 {
        cards_seen
            .saturating_mul(100)
            .checked_div(total)
            .unwrap_or(0)
    } else {
        0
    };

    let speed_secs = f64::from(speed_ms) / 1000.0;

    rsx! {
        div { class: "bg-gray-800 rounded-xl p-6 space-y-6",
            h3 { class: "text-lg font-semibold text-gray-300", "Session Stats" }

            div { class: "space-y-3",
                StatRow { label: "Progress", value: format!("{cards_seen}/{total} ({pct}%)") }
                StatRow { label: "Streak", value: format!("{streak}") }
                StatRow { label: "Speed", value: format!("{speed_secs:.1}s") }
            }

            div { class: "flex flex-col gap-2",
                button {
                    class: "w-full bg-gray-700 hover:bg-gray-600 rounded px-3 py-2 text-sm transition",
                    onclick: move |_| on_speed_up.call(()),
                    "[-] Faster"
                }
                button {
                    class: "w-full bg-gray-700 hover:bg-gray-600 rounded px-3 py-2 text-sm transition",
                    onclick: move |_| on_slow_down.call(()),
                    "[+] Slower"
                }
                button {
                    class: "w-full bg-gray-700 hover:bg-gray-600 rounded px-3 py-2 text-sm transition",
                    onclick: move |_| on_pause_toggle.call(()),
                    if paused { "[P] Resume" } else { "[P] Pause" }
                }
            }

            div { class: "text-xs text-gray-600 space-y-1",
                p { "Space — flip card" }
                p { "1-4 — rate card" }
                p { "+/- — adjust speed" }
                p { "P — pause timer" }
            }
        }
    }
}

#[component]
fn StatRow(label: String, value: String) -> Element {
    rsx! {
        div { class: "flex justify-between",
            span { class: "text-gray-500", "{label}" }
            span { class: "text-white font-mono", "{value}" }
        }
    }
}
