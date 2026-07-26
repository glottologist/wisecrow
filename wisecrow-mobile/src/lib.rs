pub mod auth;
mod components;
mod router;
mod server_fns;
pub mod transport;

use dioxus::prelude::*;

pub fn app() -> Element {
    rsx! {
        Router::<router::Route> {}
    }
}
