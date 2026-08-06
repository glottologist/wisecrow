use dioxus::prelude::*;

use crate::router::Route;

/// Catch-all for a URL no route matches.
///
/// Without one the router renders its own parse failure, which lists every
/// declared route and the segment each failed on. That is the page a learner met
/// after `Home`'s "Start Session" link built `/learn/en/` from an unset foreign
/// language, and it is what any stale bookmark or mistyped path still reaches.
#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = segments.join("/");
    rsx! {
        div { class: "space-y-6 text-center py-12",
            h1 { class: "text-4xl font-bold", "Page not found" }
            p { class: "text-gray-400 text-lg", "Nothing lives at /{path}." }
            Link {
                to: Route::Home {},
                class: "inline-block bg-emerald-600 hover:bg-emerald-500 rounded px-4 py-3 font-semibold transition",
                "Back to the start"
            }
        }
    }
}
