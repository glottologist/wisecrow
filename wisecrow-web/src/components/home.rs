use dioxus::prelude::*;

use wisecrow_dto::{LanguageInfo, UserDto};

use crate::router::Route;

#[cfg(feature = "server")]
use crate::server::learn::{create_user, list_languages, list_users};

#[cfg(not(feature = "server"))]
mod server_stubs {
    use dioxus::prelude::*;
    use wisecrow_dto::{LanguageInfo, UserDto};

    #[server]
    pub async fn list_languages() -> Result<Vec<LanguageInfo>, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn list_users() -> Result<Vec<UserDto>, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn create_user(display_name: String) -> Result<UserDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }
}

#[cfg(not(feature = "server"))]
use server_stubs::*;

#[component]
pub fn Home() -> Element {
    let languages = use_server_future(list_languages)?;

    let mut users: Signal<Vec<UserDto>> = use_signal(Vec::new);
    let mut selected_user_id = use_signal(|| 1i32);
    let mut native = use_signal(String::new);
    let mut foreign = use_signal(String::new);
    let mut new_user_name = use_signal(String::new);
    let mut users_loaded = use_signal(|| false);

    use_effect(move || {
        spawn(async move {
            if let Ok(u) = list_users().await {
                if let Some(first) = u.first() {
                    selected_user_id.set(first.id);
                }
                users.set(u);
            }
            users_loaded.set(true);
        });
    });

    let lang_list = languages.read();
    let langs: &[LanguageInfo] = match lang_list.as_ref() {
        Some(Ok(ref v)) => v,
        _ => &[],
    };

    rsx! {
        div { class: "space-y-8",
            h1 { class: "text-4xl font-bold text-center mb-4",
                "Welcome to Wisecrow"
            }
            p { class: "text-gray-400 text-center text-lg",
                "Frequency-based language learning flashcards"
            }

            div { class: "bg-gray-800 rounded-xl p-6 max-w-md mx-auto space-y-4",
                h2 { class: "text-xl font-semibold mb-2", "Start Learning" }

                div { class: "space-y-2",
                    label { class: "block text-sm text-gray-400", "User" }
                    div { class: "flex gap-2",
                        select {
                            class: "flex-1 bg-gray-700 rounded px-3 py-2 text-white",
                            value: "{selected_user_id}",
                            onchange: move |evt| {
                                if let Ok(id) = evt.value().parse::<i32>() {
                                    selected_user_id.set(id);
                                }
                            },
                            for user in users().iter() {
                                option { value: "{user.id}", "{user.display_name}" }
                            }
                        }
                    }
                    div { class: "flex gap-2",
                        input {
                            class: "flex-1 bg-gray-700 rounded px-3 py-2 text-white text-sm",
                            placeholder: "New user name",
                            value: "{new_user_name}",
                            oninput: move |evt| new_user_name.set(evt.value()),
                        }
                        button {
                            class: "bg-emerald-700 hover:bg-emerald-600 rounded px-3 py-2 text-sm font-semibold transition",
                            onclick: move |_| {
                                let name = new_user_name();
                                async move {
                                    if name.trim().is_empty() {
                                        return;
                                    }
                                    if let Ok(user) = create_user(name).await {
                                        selected_user_id.set(user.id);
                                        if let Ok(u) = list_users().await {
                                            users.set(u);
                                        }
                                        new_user_name.set(String::new());
                                    }
                                }
                            },
                            "Add"
                        }
                    }
                }

                div { class: "space-y-2",
                    label { class: "block text-sm text-gray-400", "Native Language" }
                    select {
                        class: "w-full bg-gray-700 rounded px-3 py-2 text-white",
                        value: "{native}",
                        onchange: move |evt| native.set(evt.value()),
                        option { value: "", "Select..." }
                        for lang in langs.iter() {
                            option { value: "{lang.code}", "{lang.name} ({lang.code})" }
                        }
                    }
                }

                div { class: "space-y-2",
                    label { class: "block text-sm text-gray-400", "Foreign Language" }
                    select {
                        class: "w-full bg-gray-700 rounded px-3 py-2 text-white",
                        value: "{foreign}",
                        onchange: move |evt| foreign.set(evt.value()),
                        option { value: "", "Select..." }
                        for lang in langs.iter() {
                            option { value: "{lang.code}", "{lang.name} ({lang.code})" }
                        }
                    }
                }

                Link {
                    to: Route::LearnPage {
                        user_id: selected_user_id(),
                        native: native(),
                        foreign: foreign(),
                    },
                    class: "block w-full text-center bg-emerald-600 hover:bg-emerald-500 rounded px-4 py-3 font-semibold transition",
                    "Start Session"
                }
            }
        }
    }
}
