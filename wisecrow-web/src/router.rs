use dioxus::prelude::*;

use crate::components::fast::FastPage;
use crate::components::grammar::{BrainmapPage, GrammarPage, PlacementPage};
use crate::components::home::Home;
use crate::components::layout::Layout;
use crate::components::learn::LearnPage;
use crate::components::login::LoginPage;
use crate::components::nback::NbackPage;
use crate::components::not_found::NotFound;
use crate::components::quiz::QuizPage;

#[derive(Clone, Routable, Debug, PartialEq, Eq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/login")]
    LoginPage {},
    #[layout(Layout)]
        #[route("/")]
        Home {},
        #[route("/learn/:native/:foreign")]
        LearnPage { native: String, foreign: String },
        #[route("/nback/:native/:foreign")]
        NbackPage { native: String, foreign: String },
        #[route("/fast/:native/:foreign")]
        FastPage { native: String, foreign: String },
        #[route("/quiz")]
        QuizPage {},
        // The four-segment placement and brainmap routes are declared before
        // the three-segment practice route only for readability; they cannot
        // collide, since the router matches on segment count first.
        #[route("/grammar/placement/:native/:foreign")]
        PlacementPage { native: String, foreign: String },
        #[route("/grammar/brainmap/:native/:foreign")]
        BrainmapPage { native: String, foreign: String },
        #[route("/grammar/:native/:foreign")]
        GrammarPage { native: String, foreign: String },
        // Must stay last: the router tries variants in order and this matches
        // anything. Inside the layout so a wrong URL keeps the site's chrome
        // rather than dropping the reader onto a bare page.
        #[route("/:..segments")]
        NotFound { segments: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::Route;
    use std::str::FromStr;

    /// The error type carries no `PartialEq`, so routes are compared as parsed
    /// values rather than as results.
    fn parse(path: &str) -> Route {
        Route::from_str(path).unwrap_or_else(|error| panic!("{path}: {error}"))
    }

    #[test]
    fn grammar_routes_parse_and_do_not_shadow_one_another() {
        assert_eq!(
            parse("/grammar/en/es"),
            Route::GrammarPage {
                native: String::from("en"),
                foreign: String::from("es"),
            }
        );
        assert_eq!(
            parse("/grammar/placement/en/es"),
            Route::PlacementPage {
                native: String::from("en"),
                foreign: String::from("es"),
            }
        );
        assert_eq!(
            parse("/grammar/brainmap/en/es"),
            Route::BrainmapPage {
                native: String::from("en"),
                foreign: String::from("es"),
            }
        );
    }

    /// A pair whose first code happens to read like a section name must still
    /// reach practice rather than be swallowed by the longer route.
    #[test]
    fn a_pair_is_not_mistaken_for_a_section() {
        assert_eq!(
            parse("/grammar/placement/es"),
            Route::GrammarPage {
                native: String::from("placement"),
                foreign: String::from("es"),
            }
        );
    }
}
