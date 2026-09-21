pub mod api;
#[cfg(any(feature = "server", feature = "web"))]
pub mod components;
#[cfg(any(feature = "server", feature = "web"))]
pub mod router;

#[cfg(feature = "server")]
pub mod server;

#[cfg(any(feature = "server", feature = "web"))]
use dioxus::prelude::*;

#[cfg(any(feature = "server", feature = "web"))]
pub fn app() -> Element {
    use components::brand::BrandHead;
    use components::theme::ThemeProvider;

    rsx! {
        document::Stylesheet { href: asset!("/assets/style.css") }
        BrandHead {}
        ThemeProvider {
            Router::<router::Route> {}
        }
    }
}

#[cfg(feature = "server")]
pub fn run_server() -> ! {
    tracing::info!("Starting Wisecrow web UI");
    // The pool's I/O and maintenance tasks must outlive every request.
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    runtime
        .block_on(server::init_pool())
        .expect("Failed to initialise database pool");

    if let Some((cert, key)) = server::tls::tls_paths_from_env() {
        // Production: terminate TLS in-process with axum-server + rustls.
        let addr = server::tls::bind_addr(8443);
        runtime
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
