pub mod application;
pub mod auth;
mod components;
pub mod platform;
mod router;
pub mod storage;
pub mod sync;
pub mod transport;

use std::sync::Arc;

use dioxus::prelude::*;

use application::LocalStore;

pub fn app() -> Element {
    rsx! {
        Router::<router::Route> {}
    }
}

/// The app with a local store the pages can read.
///
/// The grammar pages draw entirely from the device's own database, so they
/// need it in context; `app` remains for the launch paths that have not yet
/// built one.
pub fn app_with_store(store: Arc<dyn LocalStore>) -> Element {
    use_context_provider(|| Arc::clone(&store)); // clone: every page shares the one store
    rsx! {
        Router::<router::Route> {}
    }
}
