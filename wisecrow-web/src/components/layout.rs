use dioxus::prelude::*;

use crate::components::server_api::logout;
use crate::router::Route;

#[component]
pub fn Layout() -> Element {
    let navigator = use_navigator();
    rsx! {
        div { class: "min-h-screen bg-gray-900 text-white",
            nav { class: "bg-gray-800 border-b border-gray-700 px-6 py-4",
                div { class: "flex items-center justify-between max-w-6xl mx-auto",
                    Link { to: Route::Home {}, class: "text-2xl font-bold text-emerald-400 hover:text-emerald-300",
                        "Wisecrow"
                    }
                    div { class: "flex gap-4 items-center",
                        Link { to: Route::Home {}, class: "px-3 py-2 rounded hover:bg-gray-700 transition",
                            "Home"
                        }
                        Link {
                            to: Route::QuizPage {},
                            class: "px-3 py-2 rounded hover:bg-gray-700 transition",
                            "Quiz"
                        }
                        button {
                            class: "px-3 py-2 rounded hover:bg-gray-700 transition text-gray-300",
                            onclick: move |_| async move {
                                let _ = logout().await;
                                navigator.push(Route::LoginPage {});
                            },
                            "Logout"
                        }
                    }
                }
            }
            main { class: "max-w-6xl mx-auto px-6 py-8",
                Outlet::<Route> {}
            }
        }
    }
}
