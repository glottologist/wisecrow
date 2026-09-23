use std::sync::Arc;

use dioxus::prelude::*;

use super::{accuracy_label, band_colour, band_of, is_provisional};
use crate::application::LocalStore;

/// The syllabus at a glance, drawn from the mirrored mastery rows.
///
/// A point the device has never been told about is absent rather than grey:
/// the brainmap shows what the learner has demonstrated on this device, and
/// what has not arrived cannot be reported either way.
#[component]
pub fn GrammarBrainmapPage(native: String, foreign: String) -> Element {
    let _ = native;
    let store = use_context::<Arc<dyn LocalStore>>();
    let language = foreign.clone(); // clone: the resource owns its language code

    let map = use_resource(move || {
        let store = Arc::clone(&store); // clone: the future shares the process-wide store
        let language = language.clone(); // clone: the future owns its language code
        async move {
            let rules = store.grammar_rules(&language).await.unwrap_or_default();
            let mastery = store.grammar_mastery(&language).await.unwrap_or_default();
            (rules, mastery)
        }
    });

    let reading = map.read();
    let Some((rules, mastery)) = reading.as_ref() else {
        return rsx! { p { "Loading your map..." } };
    };
    if rules.is_empty() {
        return rsx! {
            section {
                h1 { "Grammar map" }
                p { "No grammar points have reached this device for {foreign} yet." }
            }
        };
    }

    rsx! {
        section {
            h1 { "Grammar map" }
            p { style: "color: #9ca3af; font-size: 12px;",
                "Solid means settled. An outline means fewer than three attempts so far."
            }
            div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 8px; margin-top: 16px;",
                for rule in rules.iter() {
                    {
                        let held = mastery.iter().find(|row| row.rule_id == rule.rule_id);
                        let attempts = held.map_or(0, |row| row.attempts);
                        let accuracy = held.and_then(|row| row.accuracy);
                        let band = band_of(accuracy, attempts);
                        let (fill, edge) = band_colour(band);
                        let style = format!(
                            "border-radius: 8px; padding: 12px; background: {fill}; \
                             border: 2px {} {edge};",
                            if is_provisional(band) { "dashed" } else { "solid" },
                        );
                        rsx! {
                            div { key: "{rule.rule_id}", style: "{style}",
                                p { style: "font-size: 13px; font-weight: 600;", "{rule.title}" }
                                p { style: "font-size: 12px; color: #d1d5db;",
                                    "{accuracy_label(accuracy, attempts)} · {rule.level}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
