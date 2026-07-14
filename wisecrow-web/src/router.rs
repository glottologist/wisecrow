use dioxus::prelude::*;

use crate::components::home::Home;
use crate::components::layout::Layout;
use crate::components::learn::LearnPage;
use crate::components::login::LoginPage;
use crate::components::nback::NbackPage;
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
        #[route("/quiz")]
        QuizPage {},
}
