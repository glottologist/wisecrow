//! Wisecrow web application (Dioxus fullstack).
//!
//! Exposes the client UI components and the router unconditionally, and—behind
//! the `server` feature—the server functions, authentication, and the custom
//! axum router that the binary serves. Splitting this out of `main.rs` gives the
//! integration tests in `tests/` a seam to build the router and call the server
//! helpers directly.

pub mod components;
pub mod router;

#[cfg(feature = "server")]
pub mod server;

use dioxus::prelude::*;

/// Root component: mounts the application router.
pub fn app() -> Element {
    rsx! {
        Router::<router::Route> {}
    }
}

/// Initialises the database pool, then serves the fullstack application with the
/// custom axum router (auth-enrichment middleware layered on). Never returns.
#[cfg(feature = "server")]
pub fn run_server() -> ! {
    tracing::info!("Starting Wisecrow web UI");
    tokio::runtime::Runtime::new()
        .expect("Failed to create tokio runtime")
        .block_on(server::init_pool())
        .expect("Failed to initialise database pool");

    if let Some((cert, key)) = server::tls::tls_paths_from_env() {
        // Production: terminate TLS in-process with axum-server + rustls.
        let addr = server::tls::bind_addr(8443);
        tokio::runtime::Runtime::new()
            .expect("Failed to create tokio runtime")
            .block_on(server::tls::serve_tls(
                server::build_router(),
                addr,
                &cert,
                &key,
            ))
            .expect("HTTPS server failed");
        std::process::exit(0);
    }

    // Dev / plain HTTP: dioxus serve keeps hot-reload and reads IP/PORT itself.
    dioxus::server::serve(|| async { Ok(server::build_router()) })
}
