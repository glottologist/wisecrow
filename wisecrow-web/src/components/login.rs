use dioxus::prelude::*;

use crate::api::auth::login;
use crate::components::brand::{BrandLockup, Lockup, LANDSCAPE_AVIF, LANDSCAPE_JPG};
use crate::components::theme::ThemeSelector;
use crate::router::Route;

#[component]
pub fn LoginPage() -> Element {
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let navigator = use_navigator();

    rsx! {
        div { class: "min-h-screen bg-gray-900 text-white flex items-center justify-center relative",
            div { class: "login-theme-selector",
                ThemeSelector {}
            }
            div { class: "login-shell",
                picture { class: "login-art",
                    source { r#type: "image/avif", "srcset": LANDSCAPE_AVIF }
                    img {
                        src: LANDSCAPE_JPG,
                        alt: "",
                        width: "960",
                        height: "640",
                        loading: "lazy",
                    }
                }
                div { class: "w-full max-w-sm bg-gray-800 rounded-xl p-8 space-y-4 login-card",
                h1 { class: "login-brand",
                    BrandLockup { lockup: Lockup::Stacked }
                }
                h2 { class: "text-lg text-center text-gray-300", "Sign in" }

                if let Some(err) = error_msg() {
                    p { class: "text-red-400 text-sm text-center", "{err}" }
                }

                input {
                    class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
                    r#type: "email",
                    placeholder: "Email",
                    value: "{email}",
                    oninput: move |e| email.set(e.value()),
                }
                input {
                    class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
                    r#type: "password",
                    placeholder: "Password",
                    value: "{password}",
                    oninput: move |e| password.set(e.value()),
                }
                button {
                    class: "w-full bg-emerald-600 hover:bg-emerald-500 rounded px-4 py-3 font-semibold transition",
                    onclick: move |_| {
                        let em = email();
                        let pw = password();
                        async move {
                            match login(em, pw).await {
                                Ok(_) => {
                                    navigator.push(Route::Home {});
                                }
                                Err(_) => {
                                    error_msg
                                        .set(Some(String::from("Invalid email or password")));
                                }
                            }
                        }
                    },
                    "Sign in"
                }
                }
            }
        }
    }
}
