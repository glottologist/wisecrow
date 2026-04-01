use dioxus::prelude::*;
use wisecrow_dto::{ReviewRatingDto, SessionDto};

use crate::server_fns::{answer_card, create_session, pause_session, resume_session};

const DEFAULT_USER_ID: i32 = 1;
const DEFAULT_DECK_SIZE: u32 = 30;
const DEFAULT_SPEED_MS: u32 = 5000;

#[component]
pub fn LearnPage(native: String, foreign: String) -> Element {
    let mut session: Signal<Option<SessionDto>> = use_signal(|| None);
    let mut current_index = use_signal(|| 0usize);
    let mut flipped = use_signal(|| false);
    let mut loading = use_signal(|| true);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);

    let native_code = native.clone(); // clone: captured by multiple closures
    let foreign_code = foreign.clone(); // clone: captured by multiple closures

    use_effect({
        let nat = native.clone(); // clone: moved into async effect
        let for_lang = foreign.clone(); // clone: moved into async effect
        move || {
            let nat = nat.clone(); // clone: FnMut closure may be called multiple times
            let for_lang = for_lang.clone(); // clone: FnMut closure may be called multiple times
            spawn(async move {
                match resume_session(DEFAULT_USER_ID, nat.clone(), for_lang.clone()).await {
                    Ok(Some(s)) => session.set(Some(s)),
                    Ok(None) => {
                        match create_session(
                            DEFAULT_USER_ID,
                            nat,
                            for_lang,
                            DEFAULT_DECK_SIZE,
                            DEFAULT_SPEED_MS,
                        )
                        .await
                        {
                            Ok(s) => session.set(Some(s)),
                            Err(e) => error_msg.set(Some(format!("Failed to create session: {e}"))),
                        }
                    }
                    Err(e) => error_msg.set(Some(format!("Failed to resume session: {e}"))),
                }
                loading.set(false);
            });
        }
    });

    if loading() {
        return rsx! {
            div { style: "text-align: center; padding: 48px 0; color: #9ca3af;",
                "Loading session..."
            }
        };
    }

    if let Some(err) = error_msg() {
        return rsx! {
            div { style: "text-align: center; padding: 48px 16px;",
                p { style: "color: #ef4444; margin-bottom: 16px;", "{err}" }
                p { style: "color: #9ca3af;",
                    "Make sure vocabulary is ingested for {native_code}-{foreign_code}"
                }
            }
        };
    }

    let Some(s) = session() else {
        return rsx! {
            div { style: "text-align: center; padding: 48px 0; color: #9ca3af;",
                "No session available"
            }
        };
    };

    let idx = current_index();
    let total = s.cards.len();

    if idx >= total {
        let session_id = s.id;
        return rsx! {
            div { style: "text-align: center; padding: 48px 16px;",
                h2 { style: "font-size: 28px; font-weight: bold; color: #34d399; margin-bottom: 16px;",
                    "Session Complete"
                }
                p { style: "color: #9ca3af; margin-bottom: 24px;",
                    "{total} cards reviewed"
                }
                button {
                    style: "background: #059669; color: white; border: none; border-radius: 12px; padding: 16px 32px; font-size: 18px; font-weight: 600;",
                    onclick: move |_| {
                        async move {
                            let _ = pause_session(session_id).await;
                        }
                    },
                    "Done"
                }
            }
        };
    }

    let card = &s.cards[idx];
    let display_idx = idx.saturating_add(1);
    let session_id = s.id;
    let card_id = card.card_id;

    rsx! {
        div { style: "display: flex; flex-direction: column; min-height: calc(100vh - 96px); padding: 16px;",

            div { style: "text-align: center; color: #6b7280; font-size: 14px; margin-bottom: 16px;",
                "{display_idx} / {total}"
            }

            div {
                style: "flex: 1; display: flex; align-items: center; justify-content: center;",
                onclick: move |_| flipped.set(!flipped()),

                div { style: "background: #1f2937; border-radius: 16px; padding: 32px; width: 100%; text-align: center; min-height: 200px; display: flex; flex-direction: column; justify-content: center;",
                    if flipped() {
                        p { style: "font-size: 24px; color: #9ca3af; margin-bottom: 8px;",
                            "{card.from_phrase}"
                        }
                        p { style: "font-size: 32px; font-weight: bold; color: #f3f4f6;",
                            "{card.to_phrase}"
                        }
                    } else {
                        p { style: "font-size: 32px; font-weight: bold; color: #f3f4f6;",
                            "{card.from_phrase}"
                        }
                        p { style: "font-size: 14px; color: #6b7280; margin-top: 12px;",
                            "Tap to reveal"
                        }
                    }
                }
            }

            if flipped() {
                div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-top: 16px;",
                    button {
                        style: "background: #dc2626; color: white; border: none; border-radius: 12px; padding: 20px; font-size: 16px; font-weight: 600; min-height: 56px;",
                        onclick: move |_| {
                            async move {
                                if answer_card(session_id, card_id, ReviewRatingDto::Again).await.is_ok() {
                                    current_index.set(idx.saturating_add(1));
                                    flipped.set(false);
                                }
                            }
                        },
                        "Again"
                    }
                    button {
                        style: "background: #f59e0b; color: white; border: none; border-radius: 12px; padding: 20px; font-size: 16px; font-weight: 600; min-height: 56px;",
                        onclick: move |_| {
                            async move {
                                if answer_card(session_id, card_id, ReviewRatingDto::Hard).await.is_ok() {
                                    current_index.set(idx.saturating_add(1));
                                    flipped.set(false);
                                }
                            }
                        },
                        "Hard"
                    }
                    button {
                        style: "background: #10b981; color: white; border: none; border-radius: 12px; padding: 20px; font-size: 16px; font-weight: 600; min-height: 56px;",
                        onclick: move |_| {
                            async move {
                                if answer_card(session_id, card_id, ReviewRatingDto::Good).await.is_ok() {
                                    current_index.set(idx.saturating_add(1));
                                    flipped.set(false);
                                }
                            }
                        },
                        "Good"
                    }
                    button {
                        style: "background: #3b82f6; color: white; border: none; border-radius: 12px; padding: 20px; font-size: 16px; font-weight: 600; min-height: 56px;",
                        onclick: move |_| {
                            async move {
                                if answer_card(session_id, card_id, ReviewRatingDto::Easy).await.is_ok() {
                                    current_index.set(idx.saturating_add(1));
                                    flipped.set(false);
                                }
                            }
                        },
                        "Easy"
                    }
                }
            }
        }
    }
}
