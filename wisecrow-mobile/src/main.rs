mod components;
mod router;
mod server_fns;

use dioxus::prelude::*;

fn main() {
    tracing::info!("Starting Wisecrow mobile");
    launch(app);
}

fn app() -> Element {
    rsx! {
        Router::<router::Route> {}
    }
}
