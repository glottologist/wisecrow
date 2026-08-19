use dioxus::prelude::*;

#[component]
pub fn NbackPage(native: String, foreign: String) -> Element {
    rsx! {
        section {
            h1 { "N-Back" }
            p { "Offline N-Back for {native} → {foreign} is being prepared." }
        }
    }
}
