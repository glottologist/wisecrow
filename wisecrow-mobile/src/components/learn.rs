use dioxus::prelude::*;

#[component]
pub fn LearnPage(native: String, foreign: String) -> Element {
    rsx! {
        section {
            h1 { "Learn" }
            p { "Offline learning for {native} → {foreign} is being prepared." }
        }
    }
}
