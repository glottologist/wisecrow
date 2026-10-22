use dioxus::prelude::*;

/// The practice area a learner asked for. `all` and blank mean no filter.
#[must_use]
pub fn requested_area(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "all" {
        None
    } else {
        Some(trimmed.to_owned()) // clone: the server function takes an owned area
    }
}

#[component]
pub fn AreaSelect(value: Signal<String>) -> Element {
    rsx! {
        label { class: "block space-y-2",
            span { class: "block text-sm text-gray-400", "Area" }
            select {
                class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
                value: "{value}",
                onchange: move |event| value.set(event.value()),
                option { value: "all", "All" }
                option { value: "verb", "Verbs" }
                option { value: "preposition", "Prepositions" }
                option { value: "pronoun", "Pronouns" }
                option { value: "adjective", "Adjectives" }
                option { value: "adverb", "Adverbs" }
            }
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;

    #[component]
    fn area_fixture() -> Element {
        let value = use_signal(|| String::from("all"));
        rsx! { AreaSelect { value } }
    }

    #[test]
    fn area_select_offers_six_areas_and_not_nouns() {
        let mut dom = VirtualDom::new(area_fixture);
        dom.rebuild_in_place();
        let html = dioxus::ssr::render(&dom);
        for (label, value) in [
            ("All", "all"),
            ("Verbs", "verb"),
            ("Prepositions", "preposition"),
            ("Pronouns", "pronoun"),
            ("Adjectives", "adjective"),
            ("Adverbs", "adverb"),
        ] {
            assert!(html.contains(label), "{label}");
            assert!(html.contains(&format!("value=\"{value}\"")), "{value}");
        }
        assert!(!html.contains("Nouns"));
    }
}
