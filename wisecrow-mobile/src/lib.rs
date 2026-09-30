pub mod application;
pub mod auth;
mod components;
mod online;
pub mod platform;
mod router;
pub mod storage;
pub mod sync;
pub mod transport;

use std::sync::Arc;

use dioxus::prelude::*;

use application::LocalStore;

pub fn app() -> Element {
    online::app()
}

/// Configures HTTPS requests for the shared native interface before launch.
pub fn configure_online() -> Result<(), ServerFnError> {
    online::configure()
}

/// The offline interface backed by a local store.
///
/// The normal launch path uses the shared online interface instead.
pub fn app_with_store(store: Arc<dyn LocalStore>) -> Element {
    use_context_provider(|| Arc::clone(&store)); // clone: every page shares the one store
    rsx! {
        Router::<router::Route> {}
    }
}

/// The offline interface with an API and media root for grammar example clips.
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
