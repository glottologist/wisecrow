//! Binary entry point. Delegates to the `wisecrow_web` library: under the
//! `server` feature it serves the fullstack app via the custom router; otherwise
//! it launches the WASM client.

fn main() {
    #[cfg(feature = "server")]
    wisecrow_web::run_server();

    #[cfg(not(feature = "server"))]
    dioxus::prelude::launch(wisecrow_web::app);
}
