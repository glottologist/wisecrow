use dioxus::prelude::*;

#[component]
pub fn Home() -> Element {
    rsx! {
        section {
            h1 { "Wisecrow" }
            p { "Local storage is ready." }
        }
    }
}
