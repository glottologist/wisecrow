use dioxus::prelude::*;

use wisecrow_dto::LanguageInfo;

use crate::router::Route;

#[cfg(feature = "server")]
use crate::server::learn::list_languages;

#[cfg(not(feature = "server"))]
#[server]
async fn list_languages() -> Result<Vec<LanguageInfo>, ServerFnError> {
    Err(ServerFnError::new("server-only"))
}

#[component]
pub fn Home() -> Element {
    let languages = use_server_future(list_languages)?;

    let mut native = use_signal(String::new);
    let mut foreign = use_signal(String::new);

    let lang_list = languages.read();
    let langs: &[LanguageInfo] = match lang_list.as_ref() {
        Some(Ok(ref v)) => v,
        _ => &[],
    };

    rsx! {
        div { class: "space-y-8",
            h1 { class: "text-4xl font-bold text-center mb-4",
                "Welcome to Wisecrow"
            }
            p { class: "text-gray-400 text-center text-lg",
                "Frequency-based language learning flashcards"
            }

            div { class: "bg-gray-800 rounded-xl p-6 max-w-md mx-auto space-y-4",
                h2 { class: "text-xl font-semibold mb-2", "Start Learning" }

                div { class: "space-y-2",
                    label { class: "block text-sm text-gray-400", "Native Language" }
                    select {
                        class: "w-full bg-gray-700 rounded px-3 py-2 text-white",
                        value: "{native}",
                        onchange: move |evt| native.set(evt.value()),
                        option { value: "", "Select..." }
                        for lang in langs.iter() {
                            option { value: "{lang.code}", "{lang.name} ({lang.code})" }
                        }
                    }
                }

                div { class: "space-y-2",
                    label { class: "block text-sm text-gray-400", "Foreign Language" }
                    select {
                        class: "w-full bg-gray-700 rounded px-3 py-2 text-white",
                        value: "{foreign}",
                        onchange: move |evt| foreign.set(evt.value()),
                        option { value: "", "Select..." }
                        for lang in langs.iter() {
                            option { value: "{lang.code}", "{lang.name} ({lang.code})" }
                        }
                    }
                }

                Link {
                    to: Route::LearnPage {
                        native: native().to_string(),
                        foreign: foreign().to_string(),
                    },
                    class: "block w-full text-center bg-emerald-600 hover:bg-emerald-500 rounded px-4 py-3 font-semibold transition",
                    "Start Session"
                }
            }
        }
    }
}
