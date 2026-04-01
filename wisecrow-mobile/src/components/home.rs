use dioxus::prelude::*;
use wisecrow_dto::LanguageInfo;

use crate::router::Route;
use crate::server_fns::list_languages;

#[component]
pub fn Home() -> Element {
    let mut languages: Signal<Vec<LanguageInfo>> = use_signal(Vec::new);
    let mut native_lang: Signal<Option<String>> = use_signal(|| None);
    let mut loading = use_signal(|| true);

    use_effect(move || {
        spawn(async move {
            if let Ok(langs) = list_languages().await {
                languages.set(langs);
            }
            loading.set(false);
        });
    });

    if loading() {
        return rsx! {
            div { style: "text-align: center; padding: 48px 0; color: #9ca3af;",
                "Loading languages..."
            }
        };
    }

    if native_lang().is_none() {
        return rsx! {
            div { style: "padding: 16px;",
                h1 {
                    style: "font-size: 28px; font-weight: bold; text-align: center; margin-bottom: 8px;",
                    "Wisecrow"
                }
                p {
                    style: "color: #9ca3af; text-align: center; margin-bottom: 24px;",
                    "Select your native language"
                }

                div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 12px;",
                    {languages().iter().take(20).map(|lang| {
                        let code = lang.code.clone(); // clone: captured by event handler
                        let name = lang.name.clone(); // clone: rendered in button
                        rsx! {
                            button {
                                style: "background: #1f2937; border: none; border-radius: 12px; padding: 16px; color: #f3f4f6; font-size: 16px; text-align: left; min-height: 56px;",
                                onclick: move |_| {
                                    native_lang.set(Some(code.clone())); // clone: captured by closure
                                },
                                span { style: "font-weight: 600;", "{name}" }
                                br {}
                                span { style: "font-size: 12px; color: #6b7280;", "{lang.code}" }
                            }
                        }
                    })}
                }
            }
        };
    }

    let native = native_lang().unwrap_or_default();

    rsx! {
        div { style: "padding: 16px;",
            h1 {
                style: "font-size: 28px; font-weight: bold; text-align: center; margin-bottom: 8px;",
                "Learn a language"
            }
            p {
                style: "color: #9ca3af; text-align: center; margin-bottom: 24px;",
                "Native: {native} — select a foreign language"
            }

            div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-bottom: 24px;",
                {languages().iter().filter(|l| l.code != native).take(20).map(|lang| {
                    let nat = native.clone(); // clone: captured by link
                    let code = lang.code.clone(); // clone: captured by link
                    let name = lang.name.clone(); // clone: rendered in button
                    rsx! {
                        Link {
                            to: Route::LearnPage { native: nat.clone(), foreign: code.clone() }, // clone: Dioxus route params are owned
                            style: "background: #1f2937; border-radius: 12px; padding: 16px; color: #f3f4f6; font-size: 16px; text-decoration: none; display: block; min-height: 56px;",
                            span { style: "font-weight: 600;", "{name}" }
                            br {}
                            span { style: "font-size: 12px; color: #6b7280;", "{code}" }
                        }
                    }
                })}
            }

            h2 {
                style: "font-size: 20px; font-weight: 600; margin-bottom: 12px;",
                "N-Back Training"
            }
            div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 12px;",
                {languages().iter().filter(|l| l.code != native).take(8).map(|lang| {
                    let nat = native.clone(); // clone: captured by link
                    let code = lang.code.clone(); // clone: captured by link
                    let name = lang.name.clone(); // clone: rendered
                    rsx! {
                        Link {
                            to: Route::NbackPage { native: nat.clone(), foreign: code.clone() }, // clone: Dioxus route params are owned
                            style: "background: #374151; border-radius: 12px; padding: 12px; color: #d1d5db; font-size: 14px; text-decoration: none; display: block;",
                            "{name}"
                        }
                    }
                })}
            }

            button {
                style: "display: block; margin: 24px auto 0; color: #6b7280; background: none; border: none; font-size: 14px;",
                onclick: move |_| native_lang.set(None),
                "Change native language"
            }
        }
    }
}
