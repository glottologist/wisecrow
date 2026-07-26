fn main() {
    #[cfg(feature = "server")]
    wisecrow_web::run_server();

    #[cfg(all(feature = "web", not(feature = "server")))]
    dioxus::prelude::launch(wisecrow_web::app);
}
