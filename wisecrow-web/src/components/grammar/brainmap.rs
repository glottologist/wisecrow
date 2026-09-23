use dioxus::prelude::*;

use super::{accuracy_label, attempts_label, band_class, provenance_label};
use crate::api::grammar::grammar_brainmap;

/// The whole syllabus at a glance, grouped by level in syllabus order.
///
/// Points never attempted stay grey rather than being inferred from a
/// neighbouring success, which is the same discipline the placement ladder
/// keeps: the map shows what the learner has demonstrated, not what they
/// probably know.
#[component]
pub fn BrainmapPage(native: String, foreign: String) -> Element {
    let map = use_server_future(move || {
        let native = native.clone(); // clone: the future owns its arguments
        let foreign = foreign.clone(); // clone: the future owns its arguments
        async move { grammar_brainmap(native, foreign).await }
    })?;

    let reading = map.read();
    let Some(Ok(ref brainmap)) = reading.as_ref() else {
        return rsx! {
            div { class: "text-center text-gray-400 py-20", "Could not load the map." }
        };
    };

    // The server returns syllabus order, so consecutive runs share a level and
    // grouping needs no sort of its own.
    let mut levels: Vec<&str> = Vec::new();
    for cell in &brainmap.cells {
        if levels.last() != Some(&cell.level.as_str()) {
            levels.push(&cell.level);
        }
    }

    rsx! {
        div { class: "max-w-4xl mx-auto space-y-8",
            h1 { class: "text-3xl font-bold text-center", "{brainmap.language} grammar" }
            p { class: "text-gray-500 text-center text-sm",
                "Solid means settled. An outline means fewer than three attempts so far. "
                "Grey means you have not been asked yet."
            }

            for level in levels {
                div { key: "{level}", class: "space-y-3",
                    h2 { class: "text-sm text-gray-500 uppercase tracking-wide", "{level}" }
                    div { class: "grid grid-cols-2 md:grid-cols-3 gap-3",
                        for cell in brainmap.cells.iter().filter(|cell| cell.level == level) {
                            div {
                                key: "{cell.rule_id}",
                                class: "rounded-lg p-3 space-y-1 {band_class(cell.band)}",
                                p { class: "font-semibold text-sm", "{cell.title}" }
                                div { class: "flex justify-between items-center text-xs",
                                    span { "{accuracy_label(cell.accuracy)}" }
                                    span { "{provenance_label(&cell.provenance)}" }
                                }
                                p { class: "text-xs", "{attempts_label(cell.attempts)}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
