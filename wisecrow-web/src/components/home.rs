use dioxus::prelude::*;

use wisecrow_dto::LanguageInfo;

use crate::api::learn::list_languages;
use crate::router::Route;

#[component]
pub fn Home() -> Element {
    let languages = use_server_future(list_languages)?;

    let mut native = use_signal(|| "en".to_string());
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
                        // Choosing the language already set as foreign would
                        // otherwise leave both equal and offer `/learn/x/x`,
                        // the same invalid session the filter below prevents
                        // from the other direction.
                        onchange: move |evt| {
                            let chosen = evt.value();
                            if foreign() == chosen {
                                foreign.set(String::new());
                            }
                            native.set(chosen);
                        },
                        for lang in langs.iter() {
                            option {
                                value: "{lang.code}",
                                selected: native() == lang.code,
                                "{lang.name} ({lang.code})"
                            }
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
                        // A session translating a language into itself teaches
                        // nothing; `wisecrow-mobile` filters the same way.
                        for lang in langs.iter().filter(|l| l.code != native()) {
                            option {
                                value: "{lang.code}",
                                selected: foreign() == lang.code,
                                "{lang.name} ({lang.code})"
                            }
                        }
                    }
                }

                // `foreign` starts empty and the "Select..." option sets it back,
                // so an always-live link offers `/learn/en/` — a URL the router
                // reads as `/learn/en` and matches against nothing, dropping the
                // learner on the route-parse error page.
                if foreign().is_empty() {
                    button {
                        class: "block w-full text-center bg-gray-700 text-gray-500 rounded px-4 py-3 font-semibold cursor-not-allowed",
                        disabled: true,
                        "Select a foreign language"
                    }
                } else {
                    Link {
                        to: Route::LearnPage {
                            native: native(),
                            foreign: foreign(),
                        },
                        class: "block w-full text-center bg-emerald-600 hover:bg-emerald-500 rounded px-4 py-3 font-semibold transition",
                        "Start Session"
                    }
                    Link {
                        to: Route::FastPage {
                            native: native(),
                            foreign: foreign(),
                        },
                        class: "block w-full text-center bg-cyan-700 hover:bg-cyan-600 rounded px-4 py-3 font-semibold transition mt-2",
                        "Fast Download"
                    }
                }
            }
        }
    }
}
