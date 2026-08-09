use dioxus::prelude::*;

use wisecrow_dto::{FastDeckDto, SpeedController};

use crate::api::learn::create_fast_deck;
use crate::api::media::{get_audio_data, get_image_data};
use crate::components::learn::preload::{self, CardMedia, MediaEntry};

const FAST_DECK_SIZE: u32 = 100;
const FAST_SPEED_MS: u32 = 2000;
const TICK_INTERVAL_MS: u64 = 100;
// Wider than the learn page's window: the pace here is fixed and known, so
// the preloader can afford to stay further ahead.
const PRELOAD_WINDOW: usize = 10;

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

/// Passive "fast download" run: both faces shown at once, audio autoplaying,
/// advancing on a timer. No ratings, no SRS state, nothing written — the
/// only controls are pacing.
#[component]
pub fn FastPage(native: String, foreign: String) -> Element {
    let mut deck: Signal<Option<FastDeckDto>> = use_signal(|| None);
    let mut current_index = use_signal(|| 0usize);
    let mut paused = use_signal(|| false);
    let mut speed = use_signal(|| SpeedController::new(FAST_SPEED_MS));
    let mut loading = use_signal(|| true);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let mut media: Signal<std::collections::HashMap<usize, MediaEntry>> =
        use_signal(std::collections::HashMap::new);

    let native_clone = native.clone(); // clone: need owned copies for async closure
    let foreign_clone = foreign.clone(); // clone: need owned copies for async closure

    use_future(move || {
        let native = native_clone.clone(); // clone: moving into async block
        let foreign = foreign_clone.clone(); // clone: moving into async block
        async move {
            match create_fast_deck(native, foreign, FAST_DECK_SIZE).await {
                Ok(d) => deck.set(Some(d)),
                Err(e) => error_msg.set(Some(format!("Failed to load deck: {e}"))),
            }
            loading.set(false);
        }
    });

    use_effect(move || {
        let d = deck();
        let idx = current_index();
        let Some(ref d) = d else { return };
        let tracked: std::collections::HashSet<usize> = media.read().keys().copied().collect();
        for stale in preload::indices_to_evict(&tracked, idx, PRELOAD_WINDOW) {
            media.write().remove(&stale);
        }
        for fetch_index in preload::indices_to_fetch(idx, d.cards.len(), PRELOAD_WINDOW, &tracked) {
            let Some(card) = d.cards.get(fetch_index) else {
                continue;
            };
            let tid = card.translation_id;
            let image_allowed = card.image_allowed;
            media.write().insert(fetch_index, MediaEntry::Pending);
            spawn(async move {
                let mut entry = CardMedia::default();
                let mut any = false;
                if let Ok(url) = get_audio_data(tid).await {
                    entry.audio_url = Some(url);
                    any = true;
                }
                if image_allowed {
                    if let Ok(image) = get_image_data(tid).await {
                        entry.image_url = Some(image.data_url);
                        entry.image_credit = image.attribution;
                        any = true;
                    }
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

    let _timer_task = use_future(move || async move {
        loop {
            async_sleep(TICK_INTERVAL_MS).await;

            let running = deck()
                .as_ref()
                .is_some_and(|d| current_index() < d.cards.len());
            if running && !paused() {
                let elapsed = u32::try_from(TICK_INTERVAL_MS).unwrap_or(100);
                if speed.write().tick(elapsed) {
                    current_index.set(current_index().saturating_add(1));
                    speed.write().reset();
                }
            }
        }
    });

    if loading() {
        return rsx! {
            div { class: "text-center text-gray-400 text-xl py-20", "Loading deck..." }
        };
    }

    if let Some(err) = error_msg() {
        return rsx! {
            div { class: "text-center text-red-400 text-xl py-20", "{err}" }
        };
    }

    let Some(d) = deck() else {
        return rsx! {
            div { class: "text-center text-gray-400 text-xl py-20", "No cards available." }
        };
    };

    let idx = current_index();
    let total = d.cards.len();

    if total == 0 {
        return rsx! {
            div { class: "text-center text-gray-400 text-xl py-20", "No cards available." }
        };
    }

    if idx >= total {
        return rsx! {
            div { class: "text-center space-y-4 py-20",
                h2 { class: "text-3xl font-bold text-cyan-400", "Run complete" }
                p { class: "text-xl text-gray-300", "{total} items played" }
                button {
                    class: "bg-cyan-700 hover:bg-cyan-600 rounded px-6 py-3 font-semibold transition",
                    onclick: move |_| {
                        current_index.set(0);
                        speed.write().reset();
                    },
                    "Restart"
                }
            }
        };
    }

    let card = &d.cards[idx];
    let display_num = idx.saturating_add(1);
    let progress_pct = display_num
        .saturating_mul(100)
        .checked_div(total)
        .unwrap_or(0);
    let speed_secs = f64::from(speed().interval_ms()) / 1000.0;
    let current_media = match media.read().get(&idx) {
        Some(MediaEntry::Ready(m)) => m.clone(),
        _ => CardMedia::default(),
    };
    let is_paused = paused();

    rsx! {
        div { class: "max-w-2xl mx-auto space-y-6",
            div { class: "w-full bg-gray-800 rounded h-2",
                div {
                    class: "bg-cyan-500 h-2 rounded transition-all",
                    style: "width: {progress_pct}%",
                }
            }

            div { class: "bg-gray-800 rounded-xl p-8 min-h-[300px] flex flex-col",
                div { class: "text-sm text-gray-500 mb-4", "Item {display_num} / {total}" }
                div { class: "flex-1 flex flex-col items-center justify-center space-y-4",
                    if let Some(ref img_src) = current_media.image_url {
                        img {
                            class: "mx-auto rounded max-w-[200px] max-h-[200px]",
                            src: "{img_src}",
                            alt: "{card.from_phrase}",
                        }
                        if let Some(ref credit) = current_media.image_credit {
                            p { class: "text-xs text-gray-600", "{credit}" }
                        }
                    }
                    p { class: "text-3xl font-bold text-cyan-400", "{card.to_phrase}" }
                    p { class: "text-xl text-emerald-400", "{card.from_phrase}" }
                    if let Some(ref audio_src) = current_media.audio_url {
                        audio {
                            src: "{audio_src}",
                            autoplay: true,
                            class: "mx-auto mt-2",
                        }
                    }
                }
            }

            div { class: "flex justify-center gap-3",
                button {
                    class: "bg-gray-700 hover:bg-gray-600 rounded px-4 py-2 text-sm transition",
                    onclick: move |_| speed.write().speed_up(),
                    "[-] Faster"
                }
                button {
                    class: "bg-gray-700 hover:bg-gray-600 rounded px-4 py-2 text-sm transition",
                    onclick: move |_| speed.write().slow_down(),
                    "[+] Slower"
                }
                button {
                    class: "bg-gray-700 hover:bg-gray-600 rounded px-4 py-2 text-sm transition",
                    onclick: move |_| paused.set(!paused()),
                    if is_paused { "Resume" } else { "Pause" }
                }
            }
            p { class: "text-center text-xs text-gray-600", "Speed: {speed_secs:.1}s per item" }
        }
    }
}
