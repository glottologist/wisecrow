use dioxus::prelude::*;

use crate::components::home::Home;
use crate::components::layout::Layout;
use crate::components::learn::LearnPage;
use crate::components::nback::NbackPage;

#[derive(Clone, Routable, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(Layout)]
        #[route("/")]
        Home {},
        #[route("/learn/:native/:foreign")]
        LearnPage { native: String, foreign: String },
        #[route("/nback/:native/:foreign")]
        NbackPage { native: String, foreign: String },
}
