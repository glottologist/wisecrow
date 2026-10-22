//! The note shown for one tapped word.

use dioxus::prelude::*;
use wisecrow_dto::{DeckDto, GlossDto};

fn chosen_deck(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        trimmed.parse().ok()
    }
}

#[component]
pub fn WordPopover(
    gloss: GlossDto,
    decks: Vec<DeckDto>,
    on_play: EventHandler<()>,
    on_add: EventHandler<(Option<i32>, String)>,
) -> Element {
    let mut chosen = use_signal(String::new);
    let mut draft = use_signal(String::new);
    let pending = gloss.status != "ok";

    rsx! {
        div { class: "reader-popover",
            if pending {
                p { "Note pending" }
            } else {
                div { class: "space-y-2",
                    if let Some(meaning) = gloss.meaning {
                        p { "{meaning}" }
                    }
                    if let Some(lemma) = gloss.lemma {
                        p { "{lemma}" }
                    }
                    if let Some(class) = gloss.class {
                        p { "{class}" }
                    }
                    if let Some(note) = gloss.note {
                        p { "{note}" }
                    }
                    button {
                        r#type: "button",
                        onclick: move |_| on_play.call(()),
                        "Play"
                    }
                    select {
                        class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
                        value: "{chosen}",
                        onchange: move |event| chosen.set(event.value()),
                        option { value: "", "Select a deck" }
                        for deck in decks {
                            option { value: "{deck.id}", "{deck.name}" }
                        }
                    }
                    input {
                        class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
                        placeholder: "New deck",
                        value: "{draft}",
                        oninput: move |event| draft.set(event.value()),
                    }
                    button {
                        r#type: "button",
                        onclick: move |_| on_add.call((chosen_deck(&chosen()), draft())),
                        "Add to deck"
                    }
                }
            }
        }
    }
}
