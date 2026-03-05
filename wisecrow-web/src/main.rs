mod components;
mod router;

#[cfg(feature = "server")]
mod server;

use dioxus::prelude::*;

fn main() {
    tracing::info!("Starting Wisecrow web UI");

    #[cfg(feature = "server")]
    {
        tokio::runtime::Runtime::new()
            .expect("Failed to create tokio runtime")
            .block_on(server::init_pool())
            .expect("Failed to initialise database pool");
    }

    launch(app);
}

fn app() -> Element {
    rsx! {
        Router::<router::Route> {}
    }
}
