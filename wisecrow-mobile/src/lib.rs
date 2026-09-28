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

/// The app with a store, an API and a media root, so grammar practice can
/// fetch and play example clips. The launch paths that have an API wire it
/// here; the others keep `app_with_store` and show examples as text.
pub fn app_with_media(
    store: Arc<dyn LocalStore>,
    api: Arc<dyn application::MobileApi>,
    media_root: std::path::PathBuf,
) -> Element {
    use_context_provider(|| Arc::clone(&store)); // clone: every page shares the one store
    use_context_provider(|| Arc::clone(&api)); // clone: every page shares the one API
    use_context_provider(|| application::MediaRoot(media_root.clone())); // clone: the root is shared by value
    rsx! {
        Router::<router::Route> {}
    }
}
