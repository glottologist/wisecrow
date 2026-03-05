mod card;
mod stats;
mod timer;

use dioxus::prelude::*;

use wisecrow_dto::{ReviewRatingDto, SessionDto, SpeedController};

#[cfg(feature = "server")]
use crate::server::learn::{
    answer_card, complete_session, create_session, pause_session, resume_session,
};

#[cfg(all(feature = "server", feature = "audio"))]
use crate::server::media::get_audio_data;

#[cfg(all(feature = "server", feature = "images"))]
use crate::server::media::get_image_data;

#[cfg(not(feature = "server"))]
mod server_stubs {
    use dioxus::prelude::*;
    use wisecrow_dto::{CardDto, ReviewRatingDto, SessionDto};

    #[server]
    pub async fn create_session(
        native: String,
        foreign: String,
        deck_size: u32,
        speed_ms: u32,
    ) -> Result<SessionDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn resume_session(
        native: String,
        foreign: String,
    ) -> Result<Option<SessionDto>, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn answer_card(
        session_id: i32,
        card_id: i32,
        rating: ReviewRatingDto,
    ) -> Result<CardDto, ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn pause_session(session_id: i32) -> Result<(), ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }

    #[server]
    pub async fn complete_session(session_id: i32) -> Result<(), ServerFnError> {
        Err(ServerFnError::new("server-only"))
    }
}

#[cfg(not(feature = "server"))]
use server_stubs::*;

#[cfg(not(feature = "audio"))]
async fn get_audio_data(
    _translation_id: i32,
    _foreign_phrase: String,
    _foreign_lang: String,
) -> Result<String, ServerFnError> {
    Err(ServerFnError::new("audio feature not enabled"))
}

#[cfg(not(feature = "images"))]
async fn get_image_data(
    _translation_id: i32,
    _word: String,
    _unsplash_api_key: String,
) -> Result<String, ServerFnError> {
    Err(ServerFnError::new("images feature not enabled"))
}

const DEFAULT_DECK_SIZE: u32 = 50;
const DEFAULT_SPEED_MS: u32 = 3000;
const TICK_INTERVAL_MS: u64 = 100;

#[cfg(target_arch = "wasm32")]
async fn async_sleep(ms: u64) {
    gloo_timers::future::TimeoutFuture::new(u32::try_from(ms).unwrap_or(100)).await;
}

#[cfg(all(not(target_arch = "wasm32"), feature = "server"))]
async fn async_sleep(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

#[cfg(all(not(target_arch = "wasm32"), not(feature = "server")))]
async fn async_sleep(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

#[component]
pub fn LearnPage(native: String, foreign: String) -> Element {
    let mut session: Signal<Option<SessionDto>> = use_signal(|| None);
    let mut current_index = use_signal(|| 0usize);
    let mut flipped = use_signal(|| false);
    let mut streak = use_signal(|| 0usize);
    let mut speed = use_signal(|| SpeedController::new(DEFAULT_SPEED_MS));
    let mut loading = use_signal(|| true);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let mut audio_url: Signal<Option<String>> = use_signal(|| None);
    let mut image_url: Signal<Option<String>> = use_signal(|| None);

    let native_clone = native.clone(); // clone: need owned copies for async closure
    let foreign_clone = foreign.clone(); // clone: need owned copies for async closure

    use_future(move || {
        let native = native_clone.clone(); // clone: moving into async block
        let foreign = foreign_clone.clone(); // clone: moving into async block
        async move {
            match resume_session(native.clone(), foreign.clone()).await {
                // clone: resume may fail, need values for create
                Ok(Some(s)) => {
                    let idx = usize::try_from(s.current_index).unwrap_or(0);
                    current_index.set(idx);
                    speed.set(SpeedController::new(
                        u32::try_from(s.speed_ms).unwrap_or(DEFAULT_SPEED_MS),
                    ));
                    session.set(Some(s));
                }
                Ok(None) => {
                    match create_session(native, foreign, DEFAULT_DECK_SIZE, DEFAULT_SPEED_MS).await
                    {
                        Ok(s) => session.set(Some(s)),
                        Err(e) => error_msg.set(Some(format!("Failed to create session: {e}"))),
                    }
                }
                Err(e) => error_msg.set(Some(format!("Failed to load session: {e}"))),
            }
            loading.set(false);
        }
    });

    let _ = use_future(move || async move {
        loop {
            async_sleep(TICK_INTERVAL_MS).await;

            if session().is_some() && !flipped() {
                let elapsed = u32::try_from(TICK_INTERVAL_MS).unwrap_or(100);
                let expired = speed.write().tick(elapsed);
                if expired {
                    flipped.set(true);
                    speed.write().reset();
                }
            }
        }
    });

    if loading() {
        return rsx! {
            div { class: "text-center text-gray-400 text-xl py-20", "Loading session..." }
        };
    }

    if let Some(err) = error_msg() {
        return rsx! {
            div { class: "text-center text-red-400 text-xl py-20", "{err}" }
        };
    }

    let Some(sess) = session() else {
        return rsx! {
            div { class: "text-center text-gray-400 text-xl py-20", "No cards available." }
        };
    };

    let idx = current_index();
    let total = sess.cards.len();

    if idx >= total {
        let session_id = sess.id;
        return rsx! {
            div { class: "text-center space-y-4 py-20",
                h2 { class: "text-3xl font-bold text-emerald-400",
                    "Session Complete!"
                }
                p { class: "text-xl text-gray-300",
                    "{total} cards reviewed"
                }
                button {
                    class: "bg-emerald-600 hover:bg-emerald-500 rounded px-6 py-3 font-semibold transition",
                    onclick: move |_| {
                        async move {
                            let _ = complete_session(session_id).await;
                        }
                    },
                    "Finish"
                }
            }
        };
    }

    let current_card = &sess.cards[idx];
    let card_id = current_card.card_id;
    let translation_id = current_card.translation_id;
    let session_id = sess.id;
    let timer_fraction = speed().remaining_fraction();
    let is_flipped = flipped();
    let foreign_lang = sess.foreign_lang.clone(); // clone: need owned for async closure
    let foreign_phrase = current_card.to_phrase.clone(); // clone: need owned for async closure
    let from_phrase = current_card.from_phrase.clone(); // clone: need owned for async closure

    rsx! {
        div { class: "grid grid-cols-1 lg:grid-cols-4 gap-6",
            div { class: "lg:col-span-3 space-y-4",
                timer::TimerBar { fraction: timer_fraction }
                card::CardDisplay {
                    card: current_card.clone(),
                    flipped: is_flipped,
                    index: idx,
                    total: total,
                    audio_url: audio_url(),
                    image_url: image_url(),
                    on_flip: move |_| {
                        flipped.set(true);
                        speed.write().reset();
                        audio_url.set(None);
                        image_url.set(None);
                        let phrase = foreign_phrase.clone(); // clone: moving into spawned async
                        let lang = foreign_lang.clone(); // clone: moving into spawned async
                        let word = from_phrase.clone(); // clone: moving into spawned async
                        spawn(async move {
                            if let Ok(url) = get_audio_data(translation_id, phrase, lang).await {
                                audio_url.set(Some(url));
                            }
                            if let Ok(url) = get_image_data(
                                translation_id,
                                word,
                                String::new(),
                            ).await {
                                image_url.set(Some(url));
                            }
                        });
                    },
                    on_rate: move |rating: ReviewRatingDto| {
                        async move {
                            match answer_card(session_id, card_id, rating).await {
                                Ok(_) => {
                                    if rating == ReviewRatingDto::Again {
                                        streak.set(0);
                                    } else {
                                        streak.set(streak().saturating_add(1));
                                    }
                                    current_index.set(idx.saturating_add(1));
                                    flipped.set(false);
                                    speed.write().reset();
                                    audio_url.set(None);
                                    image_url.set(None);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to answer card: {e}");
                                }
                            }
                        }
                    },
                }
            }
            div { class: "lg:col-span-1",
                stats::StatsPanel {
                    cards_seen: idx,
                    total: total,
                    streak: streak(),
                    speed_ms: speed().interval_ms(),
                    paused: speed().is_paused(),
                    on_speed_up: move |_| { speed.write().speed_up(); },
                    on_slow_down: move |_| { speed.write().slow_down(); },
                    on_pause_toggle: move |_| {
                        let is_paused = speed().is_paused();
                        if is_paused {
                            speed.write().unpause();
                        } else {
                            speed.write().pause();
                            if let Some(sess) = session() {
                                let sid = sess.id;
                                spawn(async move {
                                    let _ = pause_session(sid).await;
                                });
                            }
                        }
                    },
                }
            }
        }
    }
}
