mod card;
pub mod preload;
mod stats;
mod timer;

use dioxus::prelude::*;

use wisecrow_dto::{ReviewRatingDto, SessionDto, SpeedController};

use crate::api::learn::{
    answer_card, complete_session, create_session, pause_session, resume_session,
};
use crate::api::media::{get_audio_data, get_image_data};

const DEFAULT_DECK_SIZE: u32 = 50;
const DEFAULT_SPEED_MS: u32 = 3000;
const TICK_INTERVAL_MS: u64 = 100;
const PRELOAD_WINDOW: usize = 5;

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
    let mut media: Signal<std::collections::HashMap<usize, preload::MediaEntry>> =
        use_signal(std::collections::HashMap::new);

    let native_clone = native.clone(); // clone: need owned copies for async closure
    let foreign_clone = foreign.clone(); // clone: need owned copies for async closure

    use_future(move || {
        let native = native_clone.clone(); // clone: moving into async block
        let foreign = foreign_clone.clone(); // clone: moving into async block
        async move {
            let resume_native = native.clone(); // clone: create fallback retains original
            let resume_foreign = foreign.clone(); // clone: create fallback retains original
            match resume_session(resume_native, resume_foreign).await {
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

    {
        use preload::{CardMedia, MediaEntry};
        use_effect(move || {
            let sess = session();
            let idx = current_index();
            let Some(ref s) = sess else { return };
            let tracked: std::collections::HashSet<usize> = media.read().keys().copied().collect();
            for stale in preload::indices_to_evict(&tracked, idx, PRELOAD_WINDOW) {
                media.write().remove(&stale);
            }
            for fetch_index in
                preload::indices_to_fetch(idx, s.cards.len(), PRELOAD_WINDOW, &tracked)
            {
                let Some(card) = s.cards.get(fetch_index) else {
                    continue;
                };
                let tid = card.translation_id;
                media.write().insert(fetch_index, MediaEntry::Pending);
                spawn(async move {
                    let mut entry = CardMedia::default();
                    let mut any = false;
                    if let Ok(url) = get_audio_data(tid).await {
                        entry.audio_url = Some(url);
                        any = true;
                    }
                    if let Ok(image) = get_image_data(tid).await {
                        entry.image_url = Some(image.data_url);
                        entry.image_credit = image.attribution;
                        any = true;
                    }
                    let state = if any {
                        MediaEntry::Ready(entry)
                    } else {
                        MediaEntry::Failed
                    };
                    media.write().insert(fetch_index, state);
                });
            }
        });
    }

    let _timer_task = use_future(move || async move {
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
                            if let Err(e) = complete_session(session_id).await {
                                tracing::error!("Failed to complete session: {e}");
                            }
                        }
                    },
                    "Finish"
                }
            }
        };
    }

    let current_card = &sess.cards[idx];
    let card_id = current_card.card_id;
    let session_id = sess.id;
    let timer_fraction = speed().remaining_fraction();
    let is_flipped = flipped();
    let current_media = match media.read().get(&current_index()) {
        Some(preload::MediaEntry::Ready(m)) => m.clone(),
        _ => preload::CardMedia::default(),
    };

    rsx! {
        div { class: "grid grid-cols-1 lg:grid-cols-4 gap-6",
            div { class: "lg:col-span-3 space-y-4",
                timer::TimerBar { fraction: timer_fraction }
                card::CardDisplay {
                    card: current_card.clone(), // clone: component prop requires owned card data
                    flipped: is_flipped,
                    index: idx,
                    total: total,
                    audio_url: current_media.audio_url,
                    image_url: current_media.image_url,
                    image_credit: current_media.image_credit,
                    on_flip: move |_| {
                        flipped.set(true);
                        speed.write().reset();
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
                                    if let Err(e) = pause_session(sid).await {
                                        tracing::error!("Failed to pause session: {e}");
                                    }
                                });
                            }
                        }
                    },
                }
            }
        }
    }
}
