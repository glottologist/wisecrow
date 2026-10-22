//! The learner's progress in every language they have touched.
//!
//! Everything here is drawn from the one `DashboardDto` the server sends; the
//! browser computes nothing beyond scaling the review bars. Fast download runs
//! are not recorded anywhere, so the page says so rather than leaving the
//! learner to wonder why they are missing.

use dioxus::prelude::*;
use wisecrow_dto::{
    DailyCountDto, DashboardDto, LanguageProgressDto, LevelBandsDto, MasteryBandDto, PairNbackDto,
    PairVocabularyDto, PlacementSummaryDto, VerbFormProgressDto,
};

use crate::api::dashboard::dashboard;
use crate::components::grammar::band_class;
use crate::router::Route;

/// Where a learner goes to see how far they have come.
#[component]
pub fn DashboardPage() -> Element {
    let mut loaded = use_server_future(dashboard)?;

    let reading = loaded.read();
    match reading.as_ref() {
        Some(Ok(progress)) => rsx! {
            DashboardView { progress: progress.clone() } // clone: the view owns its props
        },
        Some(Err(ServerFnError::ServerError { code: 401, .. })) => rsx! {
            div { class: "text-center space-y-4", role: "alert",
                p { "Please sign in to see your progress." }
                Link {
                    to: Route::LoginPage {},
                    class: "inline-block bg-emerald-600 hover:bg-emerald-500 rounded-lg px-4 py-3 font-semibold transition",
                    "Sign in"
                }
            }
        },
        Some(Err(_)) => rsx! {
            div { class: "text-center space-y-4", role: "alert",
                p { "Could not load your progress. Please try again." }
                button {
                    r#type: "button",
                    class: "bg-emerald-600 hover:bg-emerald-500 rounded-lg px-4 py-3 font-semibold transition",
                    onclick: move |_| loaded.restart(),
                    "Retry"
                }
            }
        },
        None => rsx! { p { role: "status", "Loading your progress…" } },
    }
}

/// The dashboard itself, kept apart from loading so it renders from a value.
#[component]
fn DashboardView(progress: DashboardDto) -> Element {
    rsx! {
        div { class: "max-w-4xl mx-auto space-y-8",
            h1 { class: "text-3xl font-bold text-center", "Your progress" }
            p { class: "text-gray-500 text-center text-sm",
                "Flashcards, grammar, placement and dual n-back are counted here. "
                "Fast download runs are not recorded, so they do not appear."
            }
            if progress.languages.is_empty() {
                div { class: "dash-card text-center space-y-4",
                    p { "No progress yet." }
                    Link {
                        to: Route::Home {},
                        class: "inline-block bg-emerald-600 hover:bg-emerald-500 rounded-lg px-4 py-3 font-semibold transition",
                        "Choose a language and start with Flashcards"
                    }
                }
            }
            for language in progress.languages.iter() {
                LanguageCard {
                    key: "{language.foreign}",
                    language: language.clone(), // clone: each card owns its language
                }
            }
        }
    }
}

#[component]
fn LanguageCard(language: LanguageProgressDto) -> Element {
    rsx! {
        section { class: "dash-card space-y-6",
            h2 { class: "text-2xl font-bold", "{language.foreign_name}" }
            for pair in language.vocabulary.iter() {
                VocabularyBlock {
                    key: "{pair.native}",
                    pair: pair.clone(), // clone: each block owns its pair
                }
            }
            if let Some(books) = language.books {
                p { class: "text-sm text-gray-400",
                    "Books {books.started}"
                    if books.finished > 0 {
                        " finished {books.finished}"
                    }
                }
            }
            if !language.grammar.is_empty() {
                div { class: "space-y-3",
                    h3 { class: "text-sm font-semibold uppercase tracking-wide text-gray-500", "Grammar" }
                    BandGrid { levels: language.grammar.clone() } // clone: the grid owns its levels
                }
            }
            p { class: "text-sm text-gray-400", "{placement_line(language.placement.as_ref())}" }
            for nback in language.nback.iter() {
                p { key: "{nback.native}", class: "text-sm text-gray-400", "{nback_line(nback)}" }
            }
        }
    }
}

#[component]
fn VocabularyBlock(pair: PairVocabularyDto) -> Element {
    let counts = pair.counts;
    rsx! {
        div { class: "space-y-3",
            h3 { class: "text-sm font-semibold uppercase tracking-wide text-gray-500",
                "Words from {pair.native}"
            }
            div { class: "dash-grid",
                Tile { label: "New", value: counts.new }
                Tile { label: "Learning", value: counts.learning }
                Tile { label: "Review", value: counts.review }
                Tile { label: "Relearning", value: counts.relearning }
                Tile { label: "Due today", value: counts.due_today }
            }
            p { class: "text-sm text-gray-400", "{verb_forms_line(pair.verb_forms)}" }
            ReviewStrip { days: pair.reviews.clone() } // clone: the strip owns its days
        }
    }
}

fn verb_forms_line(forms: VerbFormProgressDto) -> String {
    if forms.forms == 0 {
        String::from("Verbs —")
    } else {
        format!("Verbs {}/{}", forms.voiced, forms.forms)
    }
}

#[component]
fn Tile(label: &'static str, value: i64) -> Element {
    rsx! {
        div { class: "dash-tile",
            p { class: "dash-tile-value", "{value}" }
            p { class: "dash-tile-label", "{label}" }
        }
    }
}

/// Thirty bars scaled to the busiest day, oldest on the left.
///
/// The bars are for sighted readers; the same figures are read out from a
/// visually hidden list, since a bar's height means nothing to a screen reader.
#[component]
fn ReviewStrip(days: Vec<DailyCountDto>) -> Element {
    let busiest = days.iter().map(|entry| entry.count).max().unwrap_or(0);
    let total: i64 = days.iter().map(|entry| entry.count).sum();
    rsx! {
        div { class: "space-y-2",
            p { class: "text-sm text-gray-400", "{reviews_label(total, days.len())}" }
            div { class: "review-strip", aria_hidden: "true",
                for entry in days.iter() {
                    div {
                        key: "{entry.day}",
                        class: bar_class(entry.count),
                        style: "height: {bar_height(entry.count, busiest)}%",
                        title: "{entry.day}: {entry.count}",
                    }
                }
            }
            ul { class: "sr-only",
                for entry in days.iter() {
                    li { key: "{entry.day}", "{entry.day}: {entry.count} reviews" }
                }
            }
        }
    }
}

/// One row per CEFR level, one chip per band the level holds.
///
/// The chips reuse the brainmap's band classes, so a level reads in the same
/// colours here as on the map.
#[component]
fn BandGrid(levels: Vec<LevelBandsDto>) -> Element {
    rsx! {
        div { class: "space-y-2",
            for level in levels.iter() {
                div { key: "{level.level}", class: "dash-bands",
                    span { class: "text-sm font-semibold", "{level.level}" }
                    for (band, count) in level.counts().into_iter().filter(|(_, count)| *count > 0) {
                        span {
                            key: "{band_label(band)}",
                            class: "{band_class(band)} dash-band",
                            title: "{band_label(band)}",
                            "{count} {band_label(band)}"
                        }
                    }
                }
            }
        }
    }
}

/// A bar's height as a percentage of the busiest day.
#[must_use]
fn bar_height(count: i64, busiest: i64) -> i64 {
    if busiest <= 0 {
        0
    } else {
        count.saturating_mul(100) / busiest
    }
}

/// An empty day keeps a flat, neutral bar so the strip still reads as thirty days.
#[must_use]
const fn bar_class(count: i64) -> &'static str {
    if count == 0 {
        "review-bar review-bar-empty"
    } else {
        "review-bar"
    }
}

#[must_use]
fn reviews_label(total: i64, days: usize) -> String {
    if total == 1 {
        format!("1 review in the last {days} days")
    } else {
        format!("{total} reviews in the last {days} days")
    }
}

/// What each band means, in the learner's words rather than the policy's.
#[must_use]
const fn band_label(band: MasteryBandDto) -> &'static str {
    match band {
        MasteryBandDto::Unseen => "not asked yet",
        MasteryBandDto::Green => "secure",
        MasteryBandDto::Amber => "patchy",
        MasteryBandDto::Red => "weak",
        MasteryBandDto::ProvisionalGreen => "secure so far",
        MasteryBandDto::ProvisionalAmber => "patchy so far",
        MasteryBandDto::ProvisionalRed => "weak so far",
    }
}

/// The placement line: a passed-nothing run reads differently from no run.
#[must_use]
fn placement_line(placement: Option<&PlacementSummaryDto>) -> String {
    let Some(placement) = placement else {
        return String::from("No placement test taken yet.");
    };
    let date = placement.completed_at.format("%-d %B %Y");
    let mut line = placement.level_reached.as_deref().map_or_else(
        || format!("Placement on {date}: no level passed yet."),
        |level| format!("Placement on {date}: reached {level}."),
    );
    if !placement.tested.is_empty() {
        line.push_str(&format!(" Tested {}.", placement.tested.join(", ")));
    }
    if !placement.skipped.is_empty() {
        line.push_str(&format!(
            " Too few questions to test {}.",
            placement.skipped.join(", ")
        ));
    }
    line
}

#[must_use]
fn nback_line(nback: &PairNbackDto) -> String {
    let sessions = if nback.sessions == 1 {
        String::from("1 session")
    } else {
        format!("{} sessions", nback.sessions)
    };
    format!(
        "Dual n-back from {}: highest n reached {} over {sessions}.",
        nback.native, nback.highest_n
    )
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use chrono::{Days, NaiveDate, TimeZone, Utc};
    use wisecrow_dto::{BookProgressDto, VerbFormProgressDto, VocabularyCountsDto};

    #[derive(Clone, Debug, PartialEq, Eq, Routable)]
    enum ViewRoute {
        #[route("/")]
        Fixture {},
    }

    /// A routed fixture cannot take props, so the DTO under test reaches it
    /// through context.
    #[component]
    fn Root(progress: DashboardDto) -> Element {
        use_context_provider(|| progress.clone()); // clone: the context owns its copy
        rsx! { Router::<ViewRoute> {} }
    }

    #[component]
    fn Fixture() -> Element {
        let progress = use_context::<DashboardDto>();
        rsx! { DashboardView { progress } }
    }

    fn render(progress: DashboardDto) -> String {
        let mut dom = VirtualDom::new_with_props(Root, RootProps { progress });
        dom.rebuild_in_place();
        dioxus::ssr::render(&dom)
    }

    fn seeded() -> DashboardDto {
        let today = NaiveDate::from_ymd_opt(2026, 10, 20).expect("date");
        let reviews = (0..30u64)
            .rev()
            .map(|back| DailyCountDto {
                day: today - Days::new(back),
                count: if back == 0 { 7 } else { 0 },
            })
            .collect();
        let spanish = LanguageProgressDto {
            foreign: "es".into(),
            foreign_name: "Spanish".into(),
            vocabulary: vec![PairVocabularyDto {
                native: "en".into(),
                counts: VocabularyCountsDto {
                    new: 11,
                    learning: 12,
                    review: 13,
                    relearning: 14,
                    due_today: 15,
                },
                reviews,
                verb_forms: VerbFormProgressDto {
                    lemmas: 2,
                    forms: 39,
                    attested: 10,
                    voiced: 4,
                },
            }],
            grammar: vec![LevelBandsDto {
                level: "A1".into(),
                green: 2,
                unseen: 3,
                ..LevelBandsDto::default()
            }],
            placement: Some(PlacementSummaryDto {
                completed_at: Utc
                    .with_ymd_and_hms(2026, 10, 19, 0, 0, 0)
                    .single()
                    .expect("time"),
                level_reached: None,
                tested: vec!["A1".into(), "A2".into()],
                skipped: Vec::new(),
            }),
            nback: vec![PairNbackDto {
                native: "en".into(),
                highest_n: 4,
                sessions: 1,
            }],
            books: Some(BookProgressDto {
                started: 2,
                finished: 1,
            }),
        };
        let gaelic = LanguageProgressDto {
            foreign: "gd".into(),
            foreign_name: "Scottish Gaelic".into(),
            vocabulary: Vec::new(),
            grammar: Vec::new(),
            placement: None,
            nback: Vec::new(),
            books: None,
        };
        DashboardDto {
            languages: vec![spanish, gaelic],
        }
    }

    #[test]
    fn an_empty_dashboard_sends_the_learner_home_to_start_with_flashcards() {
        let html = render(DashboardDto {
            languages: Vec::new(),
        });

        assert!(html.contains("No progress yet."));
        assert!(html.contains("href=\"/\""));
        assert!(html.contains("start with Flashcards"));
        assert!(!html.contains("class=\"dash-card space-y-6\""));
    }

    #[test]
    fn a_seeded_dashboard_renders_one_card_per_language_with_its_figures() {
        let html = render(seeded());

        assert_eq!(html.matches("class=\"dash-card space-y-6\"").count(), 2);
        assert!(html.contains(">Spanish<"));
        assert!(html.contains(">Scottish Gaelic<"));
        for figure in [">11<", ">12<", ">13<", ">14<", ">15<"] {
            assert!(html.contains(figure), "{figure} is missing");
        }
        assert_eq!(html.matches("class=\"review-bar").count(), 30);
        assert!(html.contains("height: 100%"));
        assert!(html.contains("2026-10-20: 7 reviews"));
        assert!(html.contains("band band-green dash-band"));
        assert!(html.contains("3 not asked yet"));
        assert!(html.contains("Placement on 19 October 2026: no level passed yet. Tested A1, A2."));
        assert!(html.contains("Dual n-back from en: highest n reached 4 over 1 session."));
        assert!(html.contains("No placement test taken yet."));
        assert!(html.contains("Fast download runs are not recorded"));
        assert!(html.contains("Verbs 4/39"));
        assert!(html.contains("Books 2"));
        assert!(html.contains("finished 1"));
    }

    #[test]
    fn a_language_without_book_progress_does_not_show_the_books_tile() {
        let mut progress = seeded();
        progress.languages[0].books = None;
        let html = render(progress);

        assert!(!html.contains("Books"));
        assert!(!html.contains("finished"));
    }

    #[test]
    fn the_verb_tile_dashes_when_no_forms_are_stored() {
        let mut progress = seeded();
        progress.languages[0].vocabulary[0].verb_forms = VerbFormProgressDto::default();
        let html = render(progress);

        assert!(html.contains("Verbs —"));
        assert!(!html.contains("Verbs 0/"));
    }

    #[test]
    fn bars_scale_to_the_busiest_day_and_an_empty_month_draws_flat() {
        assert_eq!(bar_height(7, 7), 100);
        assert_eq!(bar_height(3, 12), 25);
        assert_eq!(bar_height(0, 0), 0);
        assert_eq!(bar_class(0), "review-bar review-bar-empty");
        assert_eq!(bar_class(2), "review-bar");
    }

    #[test]
    fn labels_read_correctly_at_one() {
        assert_eq!(reviews_label(1, 30), "1 review in the last 30 days");
        assert_eq!(reviews_label(9, 30), "9 reviews in the last 30 days");
    }

    /// A class the stylesheet does not define paints nothing, which would leave
    /// the strip invisible and the tiles unstyled.
    #[test]
    fn every_dashboard_class_is_defined_by_the_stylesheet() {
        const STYLES: &str = include_str!("../../../assets/style.css");
        for class in [
            "dash-card",
            "dash-grid",
            "dash-tile",
            "dash-tile-value",
            "dash-tile-label",
            "review-strip",
            "review-bar",
            "review-bar-empty",
            "dash-bands",
            "dash-band",
            "sr-only",
        ] {
            assert!(
                STYLES.contains(&format!(".{class} ")) || STYLES.contains(&format!(".{class},")),
                "{class} is not in the stylesheet"
            );
        }
    }
}
