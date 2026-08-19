pub mod application;
pub mod auth;
mod components;
pub mod platform;
mod router;
pub mod storage;
pub mod sync;
pub mod transport;

use dioxus::prelude::*;

pub fn app() -> Element {
    rsx! {
        Router::<router::Route> {}
    }
}
