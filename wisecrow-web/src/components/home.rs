use dioxus::prelude::*;

use wisecrow_dto::LanguageInfo;

use crate::api::learn::list_languages;
use crate::router::Route;

pub(crate) const HOME_TITLE: &str = "Language Learning";

#[component]
pub fn Home() -> Element {
    let mut languages = use_server_future(list_languages)?;

    let mut native = use_signal(|| "en".to_string());
    let mut foreign = use_signal(String::new);

    let lang_list = languages.read();
    let langs: &[LanguageInfo] = match lang_list.as_ref() {
        Some(Ok(langs)) if !langs.is_empty() => langs,
        Some(Err(ServerFnError::ServerError { code: 401, .. })) => {
            return rsx! {
                div { class: "text-center space-y-4", role: "alert",
                    p { "Please sign in to choose your languages." }
                    Link {
                        to: Route::LoginPage {},
                        class: "inline-block bg-emerald-600 hover:bg-emerald-500 rounded-lg px-4 py-3 font-semibold transition",
                        "Sign in"
                    }
                }
            };
        }
        Some(_) => {
            return rsx! {
                div { class: "text-center space-y-4", role: "alert",
                    p { "Could not load languages. Please try again." }
                    button {
                        r#type: "button",
                        class: "bg-emerald-600 hover:bg-emerald-500 rounded-lg px-4 py-3 font-semibold transition",
                        onclick: move |_| languages.restart(),
                        "Retry"
                    }
                }
            };
        }
        None => return rsx! { p { role: "status", "Loading languages…" } },
    };

    rsx! {
        div { class: "flex flex-col items-center",
            h1 { class: "text-3xl font-bold mb-6",
                "{HOME_TITLE}"
            }

            div { class: "bg-gray-800 rounded-xl p-8 w-full max-w-md space-y-4 border border-gray-700",
                div { class: "space-y-2",
                    label { class: "block text-sm text-gray-400", "I speak" }
                    select {
                        class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
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
                    label { class: "block text-sm text-gray-400", "I am learning" }
                    select {
                        class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
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
                        class: "block w-full text-center bg-gray-700 text-gray-500 rounded-lg px-4 py-3 font-semibold cursor-not-allowed",
                        disabled: true,
                        "Choose a language to learn"
                    }
                } else {
                    LearningStages { native: native(), foreign: foreign() }
                }
            }
        }
    }
}

/// Fast download, then flashcards, then grammar, then a story. Each route stores the pair.
#[component]
fn LearningStages(native: String, foreign: String) -> Element {
    // clone: each route owns the language pair it opens.
    let fast = Route::FastPage {
        native: native.clone(),
        foreign: foreign.clone(),
    };
    let cards = Route::LearnPage {
        native: native.clone(),
        foreign: foreign.clone(),
    };
    let practice = Route::GrammarPage {
        native: native.clone(),
        foreign: foreign.clone(),
    };
    let placement = Route::PlacementPage {
        native: native.clone(),
        foreign: foreign.clone(),
    };
    let map = Route::BrainmapPage {
        native: native.clone(),   // clone: the Read link owns the pair as well
        foreign: foreign.clone(), // clone: the Read link owns the pair as well
    };
    let read = Route::LibraryPage { native, foreign };

    rsx! {
        div { class: "space-y-6",
            section { class: "space-y-2",
                h2 { class: "text-sm font-semibold uppercase tracking-wide text-gray-500",
                    "1. Learn words and sounds"
                }
                p { class: "text-sm text-gray-400",
                    "Fast download teaches the words and how they sound."
                }
                Link {
                    to: fast,
                    class: "block w-full text-center bg-cyan-700 hover:bg-cyan-600 rounded-lg px-4 py-3 font-semibold transition",
                    "Fast download"
                }
            }
            section { class: "space-y-2 border-t border-gray-700 pt-2",
                h2 { class: "text-sm font-semibold uppercase tracking-wide text-gray-500",
                    "2. Flashcards"
                }
                p { class: "text-sm text-gray-400",
                    "Solidify the words and sounds."
                }
                Link {
                    to: cards,
                    class: "block w-full text-center bg-emerald-600 hover:bg-emerald-500 rounded-lg px-4 py-3 font-semibold transition",
                    "Flashcards"
                }
            }
            section { class: "space-y-2 border-t border-gray-700 pt-2",
                h2 { class: "text-sm font-semibold uppercase tracking-wide text-gray-500",
                    "3. Grammar"
                }
                p { class: "text-sm text-gray-400",
                    "Understand the rules for using words and phrases."
                }
                Link {
                    to: practice,
                    class: "block w-full text-center bg-blue-600 hover:bg-blue-500 rounded-lg px-4 py-3 font-semibold transition",
                    "Grammar practice"
                }
                div { class: "flex gap-2",
                    Link {
                        to: placement,
                        class: "flex-1 text-center bg-gray-700 hover:bg-gray-600 rounded-lg px-3 py-2 text-sm transition",
                        "Find my level"
                    }
                    Link {
                        to: map,
                        class: "flex-1 text-center bg-gray-700 hover:bg-gray-600 rounded-lg px-3 py-2 text-sm transition",
                        "Grammar map"
                    }
                }
            }
            section { class: "space-y-2 border-t border-gray-700 pt-2",
                h2 { class: "text-sm font-semibold uppercase tracking-wide text-gray-500",
                    "4. Read"
                }
                p { class: "text-sm text-gray-400",
                    "Read a story in the language you are learning."
                }
                Link {
                    to: read,
                    class: "block w-full text-center bg-blue-600 hover:bg-blue-500 rounded-lg px-4 py-3 font-semibold transition",
                    "Read"
                }
            }
            Link {
                to: Route::DashboardPage {},
                class: "block text-center text-sm text-emerald-400",
                "See your progress"
            }
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::LearningStages;
    use super::*;

    #[tokio::test]
    async fn unauthenticated_home_offers_sign_in_instead_of_empty_selectors() {
        let mut dom = VirtualDom::new(crate::app);
        dom.rebuild_in_place();
        dom.wait_for_suspense().await;

        let html = dioxus::ssr::render(&dom);
        assert!(html.contains("Please sign in to choose your languages."));
        assert!(html.contains("href=\"/login\""));
        assert!(!html.contains("<select"));
    }

    #[derive(Clone, Debug, PartialEq, Eq, Routable)]
    enum StageRoute {
        #[route("/")]
        Stages {},
    }

    #[component]
    fn Stages() -> Element {
        rsx! {
            h1 { "{HOME_TITLE}" }
            LearningStages {
                native: "en".to_string(),
                foreign: "gd".to_string(),
            }
        }
    }

    fn stages_fixture() -> Element {
        rsx! { Router::<StageRoute> {} }
    }

    fn at(html: &str, needle: &str) -> usize {
        html.find(needle)
            .unwrap_or_else(|| panic!("missing {needle} in {html}"))
    }

    #[test]
    fn learning_stages_run_fast_then_flashcards_then_grammar() {
        let mut dom = VirtualDom::new(stages_fixture);
        dom.rebuild_in_place();
        let html = dioxus::ssr::render(&dom);

        assert_eq!(HOME_TITLE, "Language Learning");
        let words = at(&html, "1. Learn words and sounds");
        let cards = at(&html, "2. Flashcards");
        let grammar = at(&html, "3. Grammar");
        let read = at(&html, "4. Read");
        assert!(words < cards && cards < grammar && grammar < read);

        let fast = at(&html, "href=\"/fast/en/gd\"");
        let flashcards = at(&html, "href=\"/learn/en/gd\"");
        let practice = at(&html, "href=\"/grammar/en/gd\"");
        let level = at(&html, "href=\"/grammar/placement/en/gd\"");
        let map = at(&html, "href=\"/grammar/brainmap/en/gd\"");
        let read_href = at(&html, "href=\"/read/en/gd\"");
        assert!(words < fast);
        assert!(fast < cards);
        assert!(cards < flashcards);
        assert!(flashcards < grammar);
        assert!(grammar < practice);
        assert!(practice < level);
        assert!(level < map);
        assert!(map < read_href);

        assert!(html.contains("Fast download teaches the words and how they sound."));
        assert!(html.contains("Solidify the words and sounds."));
        assert!(html.contains("Understand the rules for using words and phrases."));
        assert!(html.contains("Read a story in the language you are learning."));
        assert!(html.contains(">Fast download<"));
        assert!(html.contains(">Flashcards<"));
        assert!(html.contains(">Grammar practice<"));
        assert!(html.contains(">Find my level<"));
        assert!(html.contains(">Grammar map<"));
        assert!(!html.contains("Frequency-based flashcards"));
        assert!(!html.contains("Start a session"));
        assert!(!html.contains("Start session"));
        assert!(!html.contains("Session rates cards"));
        let progress = at(&html, "href=\"/dashboard\"");
        assert!(map < progress, "the progress link sits under the stages");
        assert!(html.contains(">See your progress<"));
    }
}
