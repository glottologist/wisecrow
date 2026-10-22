use dioxus::prelude::*;

use wisecrow_dto::{DashboardDto, LanguageInfo, LanguageProgressDto};

use crate::api::learn::list_languages;
use crate::router::Route;

pub(crate) const HOME_TITLE: &str = "Language Learning";

/// One foreign language's progress, reduced to the figures the default uses.
#[derive(Clone, PartialEq, Eq)]
struct LanguageStanding {
    code: String,
    name: String,
    score: i64,
}

/// `Automatic` follows the strongest language. `Selected` is an explicit
/// choice, including "Select...", and is left alone when progress arrives.
#[derive(Clone, PartialEq, Eq)]
enum LearningChoice {
    Automatic,
    Selected(String),
}

/// How much of the dashboard is this learner's own work in one language.
///
/// New, learning, review and relearning cards count the same: the progress
/// page presents each of them as progress. Reviews, grammar points the learner
/// has been asked, books, a finished placement and n-back sessions count too.
/// A finished book is also a started one, so finishing adds to the score.
/// Unseen grammar is the syllabus ("not asked yet"), not progress. Due counts
/// and stored verb forms repeat figures already covered or describe the
/// catalogue, so they add nothing.
fn progress_score(language: &LanguageProgressDto) -> i64 {
    let cards_and_reviews = language.vocabulary.iter().fold(0i64, |sum, pair| {
        let counts = pair.counts;
        let cards = counts
            .new
            .saturating_add(counts.learning)
            .saturating_add(counts.review)
            .saturating_add(counts.relearning);
        let reviews = pair
            .reviews
            .iter()
            .fold(0i64, |sum, day| sum.saturating_add(day.count));
        sum.saturating_add(cards).saturating_add(reviews)
    });
    let grammar = language.grammar.iter().fold(0i64, |sum, level| {
        sum.saturating_add(
            i64::from(level.red)
                .saturating_add(i64::from(level.amber))
                .saturating_add(i64::from(level.green))
                .saturating_add(i64::from(level.provisional_red))
                .saturating_add(i64::from(level.provisional_amber))
                .saturating_add(i64::from(level.provisional_green)),
        )
    });
    let nback = language
        .nback
        .iter()
        .fold(0i64, |sum, row| sum.saturating_add(row.sessions.max(0)));
    let books = language.books.map_or(0, |books| {
        i64::from(books.started).saturating_add(i64::from(books.finished))
    });
    let placement = if language.placement.is_some() { 1 } else { 0 };
    cards_and_reviews
        .saturating_add(grammar)
        .saturating_add(nback)
        .saturating_add(books)
        .saturating_add(placement)
}

fn language_standings(progress: &DashboardDto) -> Vec<LanguageStanding> {
    progress
        .languages
        .iter()
        .map(|language| LanguageStanding {
            // clone: the form keeps the standings after the dashboard borrow ends
            code: language.foreign.clone(),
            name: language.foreign_name.clone(),
            score: progress_score(language),
        })
        .collect()
}

/// The language the "I am learning" selector shows before the learner picks.
///
/// The native language is excluded: a session translating a language into
/// itself teaches nothing. A language the selector cannot offer is excluded
/// too. Equal scores follow the dashboard's order, name then code.
fn automatic_code<'a>(
    standings: &'a [LanguageStanding],
    native: &str,
    langs: &[LanguageInfo],
) -> Option<&'a str> {
    standings
        .iter()
        .filter(|row| {
            row.score > 0 && row.code != native && langs.iter().any(|lang| lang.code == row.code)
        })
        .max_by(|left, right| {
            left.score.cmp(&right.score).then_with(|| {
                right
                    .name
                    .cmp(&left.name)
                    .then_with(|| right.code.cmp(&left.code))
            })
        })
        .map(|row| row.code.as_str())
}

#[component]
pub fn Home() -> Element {
    // Both requests start together. Suspending on the language list before the
    // progress hook is registered would leave the default waiting on a fetch
    // that had not begun.
    let languages_hook = use_server_future(list_languages);
    let progress_hook = use_server_future(crate::api::dashboard::dashboard);
    let mut languages = languages_hook?;

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

    let progress = progress_hook?;
    let standings = {
        let reading = progress.read();
        match reading.as_ref() {
            Some(Ok(data)) => language_standings(data),
            _ => Vec::new(),
        }
    };

    rsx! {
        div { class: "flex flex-col items-center",
            h1 { class: "text-3xl font-bold mb-6",
                "{HOME_TITLE}"
            }

            div { class: "bg-gray-800 rounded-xl p-8 w-full max-w-md space-y-4 border border-gray-700",
                LanguageForm {
                    langs: langs.to_vec(), // clone: the form owns the language list
                    standings,
                }
            }
        }
    }
}

/// The two selectors and the stages that follow a choice.
#[component]
fn LanguageForm(langs: Vec<LanguageInfo>, standings: Vec<LanguageStanding>) -> Element {
    let mut native = use_signal(|| "en".to_string());
    let mut choice = use_signal(|| LearningChoice::Automatic);

    let native_code = native();
    let choice_now = choice();
    let shown = match &choice_now {
        LearningChoice::Selected(code) => code.clone(),
        LearningChoice::Automatic => automatic_code(&standings, &native_code, &langs)
            .unwrap_or("")
            .to_owned(),
    };
    // clone: the native handler keeps the code it compared against
    let shown_now = shown.clone();

    rsx! {
        div { class: "space-y-4",
            div { class: "space-y-2",
                label { class: "block text-sm text-gray-400", "I speak" }
                select {
                    class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
                    value: "{native_code}",
                    // Choosing the language already set as foreign would
                    // otherwise leave both equal and offer `/learn/x/x`,
                    // the same invalid session the filter below prevents
                    // from the other direction.
                    onchange: move |evt| {
                        let chosen = evt.value();
                        if shown_now == chosen {
                            choice.set(LearningChoice::Selected(String::new()));
                        }
                        native.set(chosen);
                    },
                    for lang in langs.iter() {
                        option {
                            value: "{lang.code}",
                            selected: native_code == lang.code,
                            "{lang.name} ({lang.code})"
                        }
                    }
                }
            }

            div { class: "space-y-2",
                label { class: "block text-sm text-gray-400", "I am learning" }
                select {
                    class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
                    value: "{shown}",
                    onchange: move |evt| choice.set(LearningChoice::Selected(evt.value())),
                    option { value: "", "Select..." }
                    // A session translating a language into itself teaches
                    // nothing; `wisecrow-mobile` filters the same way.
                    for lang in langs.iter().filter(|lang| lang.code != native_code) {
                        option {
                            value: "{lang.code}",
                            selected: shown == lang.code,
                            "{lang.name} ({lang.code})"
                        }
                    }
                }
            }

            // An empty choice would offer `/learn/en/` — a URL the router
            // reads as `/learn/en` and matches against nothing, dropping the
            // learner on the route-parse error page. "Select..." sets that
            // empty choice; with no progress to default to, the button is
            // what they see instead of a broken link.
            if shown.is_empty() {
                button {
                    class: "block w-full text-center bg-gray-700 text-gray-500 rounded-lg px-4 py-3 font-semibold cursor-not-allowed",
                    disabled: true,
                    "Choose a language to learn"
                }
            } else {
                LearningStages {
                    native: native_code.clone(), // clone: the stages own the pair
                    foreign: shown.clone(),      // clone: the stages own the pair
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
    use chrono::{NaiveDate, TimeZone, Utc};
    use wisecrow_dto::{
        BookProgressDto, DailyCountDto, LevelBandsDto, PairNbackDto, PairVocabularyDto,
        PlacementSummaryDto, ScriptDirection, VerbFormProgressDto, VocabularyCountsDto,
    };

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

    fn bare(code: &str, name: &str) -> LanguageProgressDto {
        LanguageProgressDto {
            foreign: code.to_owned(),
            foreign_name: name.to_owned(),
            vocabulary: Vec::new(),
            grammar: Vec::new(),
            placement: None,
            nback: Vec::new(),
            books: None,
        }
    }

    fn with_cards(code: &str, name: &str, counts: VocabularyCountsDto) -> LanguageProgressDto {
        let mut language = bare(code, name);
        language.vocabulary.push(PairVocabularyDto {
            native: "en".to_owned(),
            counts,
            reviews: Vec::new(),
            verb_forms: VerbFormProgressDto::default(),
        });
        language
    }

    fn offered() -> Vec<LanguageInfo> {
        vec![
            LanguageInfo {
                code: "en".to_owned(),
                name: "English".to_owned(),
                script_direction: ScriptDirection::Ltr,
            },
            LanguageInfo {
                code: "fr".to_owned(),
                name: "French".to_owned(),
                script_direction: ScriptDirection::Ltr,
            },
            LanguageInfo {
                code: "es".to_owned(),
                name: "Spanish".to_owned(),
                script_direction: ScriptDirection::Ltr,
            },
        ]
    }

    fn chosen(languages: &[LanguageProgressDto], native: &str) -> Option<String> {
        let progress = DashboardDto {
            languages: languages.to_vec(),
        };
        let standings = language_standings(&progress);
        automatic_code(&standings, native, &offered()).map(str::to_owned)
    }

    #[test]
    fn progress_score_counts_recorded_work_and_ignores_catalogue() {
        let mut french = with_cards(
            "fr",
            "French",
            VocabularyCountsDto {
                new: 46,
                learning: 0,
                review: 4,
                relearning: 0,
                due_today: 50,
            },
        );
        french.vocabulary[0].reviews.push(DailyCountDto {
            day: NaiveDate::from_ymd_opt(2026, 10, 22).expect("date"),
            count: 0,
        });
        french.grammar.push(LevelBandsDto {
            level: "A1".to_owned(),
            unseen: 351,
            ..LevelBandsDto::default()
        });
        french.vocabulary[0].verb_forms = VerbFormProgressDto {
            lemmas: 20,
            forms: 400,
            attested: 10,
            voiced: 4,
        };
        assert_eq!(progress_score(&french), 50);

        let mut practised = bare("es", "Spanish");
        practised.vocabulary.push(PairVocabularyDto {
            native: "en".to_owned(),
            counts: VocabularyCountsDto::default(),
            reviews: vec![DailyCountDto {
                day: NaiveDate::from_ymd_opt(2026, 10, 22).expect("date"),
                count: 7,
            }],
            verb_forms: VerbFormProgressDto::default(),
        });
        practised.grammar.push(LevelBandsDto {
            level: "A1".to_owned(),
            green: 2,
            provisional_amber: 1,
            ..LevelBandsDto::default()
        });
        practised.books = Some(BookProgressDto {
            started: 2,
            finished: 1,
        });
        practised.nback.push(PairNbackDto {
            native: "en".to_owned(),
            highest_n: 9,
            sessions: 2,
        });
        practised.placement = Some(PlacementSummaryDto {
            completed_at: Utc
                .with_ymd_and_hms(2026, 10, 19, 0, 0, 0)
                .single()
                .expect("time"),
            level_reached: None,
            tested: Vec::new(),
            skipped: Vec::new(),
        });
        // 7 reviews + 3 grammar asked + 3 books + 2 n-back sessions + 1 placement.
        // The n-back peak and the unseen syllabus add nothing.
        assert_eq!(progress_score(&practised), 16);

        let due_only = with_cards(
            "de",
            "German",
            VocabularyCountsDto {
                due_today: 50,
                ..VocabularyCountsDto::default()
            },
        );
        assert_eq!(progress_score(&due_only), 0);
    }

    #[test]
    fn the_default_language_is_the_strongest_one_the_selector_can_offer() {
        let french = with_cards(
            "fr",
            "French",
            VocabularyCountsDto {
                new: 46,
                review: 4,
                due_today: 50,
                ..VocabularyCountsDto::default()
            },
        );
        let spanish = with_cards(
            "es",
            "Spanish",
            VocabularyCountsDto {
                new: 10,
                ..VocabularyCountsDto::default()
            },
        );
        let languages = [french, spanish];
        assert_eq!(chosen(&languages, "en").as_deref(), Some("fr"));

        let mut untouched = bare("fr", "French");
        untouched.grammar.push(LevelBandsDto {
            level: "A1".to_owned(),
            unseen: 351,
            ..LevelBandsDto::default()
        });
        let one_card = with_cards(
            "es",
            "Spanish",
            VocabularyCountsDto {
                new: 1,
                ..VocabularyCountsDto::default()
            },
        );
        assert_eq!(chosen(&[untouched, one_card], "en").as_deref(), Some("es"));

        let english = with_cards(
            "en",
            "English",
            VocabularyCountsDto {
                review: 80,
                ..VocabularyCountsDto::default()
            },
        );
        let french = with_cards(
            "fr",
            "French",
            VocabularyCountsDto {
                review: 4,
                ..VocabularyCountsDto::default()
            },
        );
        assert_eq!(chosen(&[english, french], "en").as_deref(), Some("fr"));
        assert_eq!(
            chosen(
                &[with_cards(
                    "en",
                    "English",
                    VocabularyCountsDto {
                        review: 80,
                        ..VocabularyCountsDto::default()
                    },
                )],
                "en",
            ),
            None
        );

        let tied_french = with_cards(
            "fr",
            "French",
            VocabularyCountsDto {
                review: 5,
                ..VocabularyCountsDto::default()
            },
        );
        let tied_spanish = with_cards(
            "es",
            "Spanish",
            VocabularyCountsDto {
                review: 5,
                ..VocabularyCountsDto::default()
            },
        );
        assert_eq!(
            chosen(&[tied_spanish, tied_french], "en").as_deref(),
            Some("fr")
        );

        let unavailable = with_cards(
            "de",
            "German",
            VocabularyCountsDto {
                review: 100,
                ..VocabularyCountsDto::default()
            },
        );
        let french = with_cards(
            "fr",
            "French",
            VocabularyCountsDto {
                new: 1,
                ..VocabularyCountsDto::default()
            },
        );
        assert_eq!(chosen(&[unavailable, french], "en").as_deref(), Some("fr"));
        assert_eq!(chosen(&[], "en"), None);
    }

    #[derive(Clone, PartialEq, Eq, Routable)]
    enum FormRoute {
        #[route("/")]
        Form {},
    }

    #[component]
    fn FormRoot(standings: Vec<LanguageStanding>) -> Element {
        use_context_provider(|| standings.clone()); // clone: the context owns its copy
        rsx! { Router::<FormRoute> {} }
    }

    #[component]
    fn Form() -> Element {
        let standings = use_context::<Vec<LanguageStanding>>();
        rsx! {
            LanguageForm {
                langs: offered(),
                standings,
            }
        }
    }

    fn render_form(standings: Vec<LanguageStanding>) -> String {
        let mut dom = VirtualDom::new_with_props(FormRoot, FormRootProps { standings });
        dom.rebuild_in_place();
        dioxus::ssr::render(&dom)
    }

    #[test]
    fn the_learning_selector_opens_on_the_language_with_the_most_progress() {
        let html = render_form(language_standings(&DashboardDto {
            languages: vec![
                with_cards(
                    "es",
                    "Spanish",
                    VocabularyCountsDto {
                        new: 10,
                        ..VocabularyCountsDto::default()
                    },
                ),
                with_cards(
                    "fr",
                    "French",
                    VocabularyCountsDto {
                        new: 46,
                        review: 4,
                        due_today: 50,
                        ..VocabularyCountsDto::default()
                    },
                ),
            ],
        }));

        assert!(html.contains("value=\"fr\""));
        assert!(html.contains("href=\"/learn/en/fr\""));
        assert!(!html.contains("Choose a language to learn"));
        assert!(!html.contains("href=\"/learn/en/es\""));
    }

    #[test]
    fn the_learning_selector_stays_empty_when_nothing_has_progress() {
        let html = render_form(Vec::new());

        assert!(html.contains("Choose a language to learn"));
        assert!(html.contains("Select..."));
        assert!(!html.contains("href=\"/learn/"));
    }
}
