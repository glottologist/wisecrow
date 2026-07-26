use dioxus::prelude::*;

use crate::api::auth::logout;
use crate::router::Route;

#[component]
pub fn Layout() -> Element {
    let navigator = use_navigator();
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
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
                                match logout().await {
                                    Ok(()) => {
                                        navigator.push(Route::LoginPage {});
                                    }
                                    Err(_) => {
                                        error_msg.set(Some(String::from("Logout failed")));
                                    }
                                }
                            },
                            "Logout"
                        }
                    }
                }
                if let Some(error) = error_msg() {
                    p { class: "max-w-6xl mx-auto pt-2 text-sm text-red-400", "{error}" }
                }
            }
            main { class: "max-w-6xl mx-auto px-6 py-8",
                Outlet::<Route> {}
            }
        }
    }
}
