fn main() {
    tracing::info!("Starting Wisecrow mobile");
    dioxus::prelude::launch(wisecrow_mobile::app);
}
