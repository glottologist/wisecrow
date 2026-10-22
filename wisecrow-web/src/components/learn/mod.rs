mod card;
pub mod preload;
mod stats;
mod timer;

use dioxus::prelude::*;

use wisecrow_dto::{DeckDto, ReviewRatingDto, SessionDto, SpeedController, MAX_DECK_SIZE};

use crate::api::decks::list_decks;
use crate::api::learn::{
    answer_card, complete_session, create_session, pause_session, resume_session,
};
use crate::api::media::{get_audio_data, get_image_data};
use crate::components::area::{requested_area, AreaSelect};
use crate::components::fast::{requested_size, DECK_SIZE_PRESETS};

const DEFAULT_DECK_SIZE: u32 = 50;
const DEFAULT_SPEED_MS: u32 = 3000;
const TICK_INTERVAL_MS: u64 = 100;
const PRELOAD_WINDOW: usize = 5;

#[cfg(target_arch = "wasm32")]
async fn async_sleep(ms: u64) {
    gloo_timers::future::TimeoutFuture::new(u32::try_from(ms).unwrap_or(100)).await;
}

#[cfg(all(
    not(target_arch = "wasm32"),
    any(feature = "server", feature = "native")
))]
async fn async_sleep(ms: u64) {
    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
}

#[cfg(all(
    not(target_arch = "wasm32"),
    not(any(feature = "server", feature = "native"))
))]
async fn async_sleep(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

fn chosen_deck(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        trimmed.parse().ok()
    }
}

#[component]
fn DeckSelect(
    decks: Vec<DeckDto>,
    value: Signal<String>,
    on_pick: EventHandler<Option<i32>>,
) -> Element {
    rsx! {
        label { class: "block space-y-2",
            span { class: "block text-sm text-gray-400", "Deck" }
            select {
                class: "w-full bg-gray-700 rounded-lg px-3 py-3 text-white",
                value: "{value}",
                onchange: move |event| {
                    let chosen = event.value();
                    let deck_id = chosen_deck(&chosen);
                    value.set(chosen);
                    on_pick.call(deck_id);
                },
                option { value: "", "All words" }
                for deck in decks {
                    option { value: "{deck.id}", "{deck.name}" }
                }
            }
        }
    }
}

#[component]
fn DeckSizeFields(mut preset_size: Signal<u32>, mut custom_size: Signal<String>) -> Element {
    let size_error = requested_size(&custom_size.read(), preset_size(), MAX_DECK_SIZE).err();
    let typed_custom = !custom_size.read().trim().is_empty();
    let presets: Vec<u32> = DECK_SIZE_PRESETS
        .into_iter()
        .filter(|preset| *preset <= MAX_DECK_SIZE)
        .collect();
    rsx! {
        div { class: "flex flex-wrap justify-center gap-3",
            for preset in presets {
                button {
                    key: "{preset}",
                    aria_pressed: preset == preset_size() && !typed_custom,
                    class: if preset == preset_size() && !typed_custom { "bg-emerald-700 rounded px-5 py-2 font-semibold" } else { "bg-gray-700 hover:bg-gray-600 rounded px-5 py-2 transition" },
                    onclick: move |_| {
                        preset_size.set(preset);
                        custom_size.set(String::new());
                    },
                    "{preset}"
                }
            }
        }
        div { class: "flex items-center justify-center gap-2",
            label { r#for: "learn-size", class: "text-sm text-gray-400", "or choose" }
            input {
                id: "learn-size",
                r#type: "number",
                min: "1",
                max: "{MAX_DECK_SIZE}",
                placeholder: "1–{MAX_DECK_SIZE}",
                class: "bg-gray-700 rounded px-3 py-2 w-24 text-center",
                value: "{custom_size}",
                oninput: move |event| custom_size.set(event.value()),
            }
        }
        if let Some(problem) = size_error {
            p { class: "text-center text-sm text-red-400", "{problem}" }
        }
    }
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
    let area = use_signal(|| String::from("all"));
    let deck_choice = use_signal(String::new);
    let preset_size = use_signal(|| DEFAULT_DECK_SIZE);
    let custom_size = use_signal(String::new);
    let mut decks: Signal<Vec<DeckDto>> = use_signal(Vec::new);
    let mut media: Signal<std::collections::HashMap<usize, preload::CardChannels>> =
        use_signal(std::collections::HashMap::new);

    let native_clone = native.clone(); // clone: resume runs while the chooser still owns the props
    let foreign_clone = foreign.clone(); // clone: resume runs while the chooser still owns the props
    let pick_native = native.clone(); // clone: a deck change resumes while the page still owns the props
    let pick_foreign = foreign.clone(); // clone: a deck change resumes while the page still owns the props

    use_future(move || {
        let native = native_clone.clone(); // clone: the future closure may run again
        let foreign = foreign_clone.clone(); // clone: the future closure may run again
        async move {
            match resume_session(
                native.clone(),  // clone: the deck list still needs the code
                foreign.clone(), // clone: the deck list still needs the code
                None,
            )
            .await
            {
                Ok(Some(s)) => {
                    let idx = usize::try_from(s.current_index).unwrap_or(0);
                    current_index.set(idx);
                    speed.set(SpeedController::new(
                        u32::try_from(s.speed_ms).unwrap_or(DEFAULT_SPEED_MS),
                    ));
                    session.set(Some(s));
                }
                Ok(None) => {}
                Err(e) => error_msg.set(Some(format!("Failed to load session: {e}"))),
            }
            match list_decks(native, foreign).await {
                Ok(rows) => decks.set(rows),
                Err(error) => tracing::error!("Failed to list decks: {error}"),
            }
            loading.set(false);
        }
    });

    let on_pick = move |deck_id: Option<i32>| {
        let Ok(size) = requested_size(&custom_size.read(), preset_size(), MAX_DECK_SIZE) else {
            return;
        };
        let native_code = pick_native.clone(); // clone: each change needs its own copy
        let foreign_code = pick_foreign.clone(); // clone: each change needs its own copy
        loading.set(true);
        error_msg.set(None);
        spawn(async move {
            match resume_session(
                native_code.clone(),  // clone: create still needs the code when nothing is paused
                foreign_code.clone(), // clone: create still needs the code when nothing is paused
                deck_id,
            )
            .await
            {
                Ok(Some(loaded)) => {
                    let idx = usize::try_from(loaded.current_index).unwrap_or(0);
                    current_index.set(idx);
                    speed.set(SpeedController::new(
                        u32::try_from(loaded.speed_ms).unwrap_or(DEFAULT_SPEED_MS),
                    ));
                    flipped.set(false);
                    session.set(Some(loaded));
                }
                Ok(None) => {
                    match create_session(
                        native_code,
                        foreign_code,
                        size,
                        DEFAULT_SPEED_MS,
                        None,
                        deck_id,
                    )
                    .await
                    {
                        Ok(created) => {
                            current_index.set(0);
                            speed.set(SpeedController::new(DEFAULT_SPEED_MS));
                            flipped.set(false);
                            session.set(Some(created));
                        }
                        Err(error) => {
                            error_msg.set(Some(format!("Failed to create session: {error}")));
                        }
                    }
                }
                Err(error) => error_msg.set(Some(format!("Failed to load session: {error}"))),
            }
            loading.set(false);
        });
    };

    use_effect(move || {
        let sess = session();
        let idx = current_index();
        let Some(ref s) = sess else { return };
        let tracked: std::collections::HashSet<usize> = media.read().keys().copied().collect();
        for stale in preload::indices_to_evict(&tracked, idx, PRELOAD_WINDOW) {
            media.write().remove(&stale);
        }
        for fetch_index in preload::indices_to_fetch(idx, s.cards.len(), PRELOAD_WINDOW, &tracked) {
            let Some(card) = s.cards.get(fetch_index) else {
                continue;
            };
            let tid = card.translation_id;
            let fetch_image = preload::should_fetch_image(card.is_phrase, card.image_allowed);
            media
                .write()
                .insert(fetch_index, preload::CardChannels::begin(fetch_image));
            spawn(async move {
                let audio = get_audio_data(tid).await.map_err(|_| ());
                preload::publish_audio(&mut media.write(), fetch_index, audio);
            });
            if fetch_image {
                spawn(async move {
                    let image = match get_image_data(tid).await {
                        Ok(Some(image)) => Ok(Some(preload::ImageReady {
                            url: image.data_url,
                            credit: image.attribution,
                        })),
                        Ok(None) => Ok(None),
                        Err(_) => Err(()),
                    };
                    preload::publish_image(&mut media.write(), fetch_index, image);
                });
            }
        }
    });

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
        let native_code = native.clone(); // clone: moved into the async start handler
        let foreign_code = foreign.clone(); // clone: moved into the async start handler
        let size_ok = requested_size(&custom_size.read(), preset_size(), MAX_DECK_SIZE).is_ok();
        return rsx! {
            div { class: "max-w-lg mx-auto space-y-6",
                h1 { class: "text-3xl font-bold text-center", "Learn" }
                AreaSelect { value: area }
                DeckSelect {
                    decks: decks(), // clone: the picker takes the list for this render
                    value: deck_choice,
                    on_pick,
                }
                DeckSizeFields { preset_size, custom_size }
                button {
                    class: "w-full bg-emerald-600 hover:bg-emerald-500 rounded-xl px-6 py-3 font-semibold transition",
                    disabled: !size_ok,
                    onclick: move |_| {
                        let native_code = native_code.clone(); // clone: each click needs its own copy
                        let foreign_code = foreign_code.clone(); // clone: each click needs its own copy
                        let deck_id = chosen_deck(&deck_choice());
                        let chosen = if deck_id.is_some() {
                            None
                        } else {
                            requested_area(&area())
                        };
                        let Ok(size) = requested_size(&custom_size.read(), preset_size(), MAX_DECK_SIZE) else {
                            return;
                        };
                        loading.set(true);
                        error_msg.set(None);
                        spawn(async move {
                            match create_session(
                                native_code,
                                foreign_code,
                                size,
                                DEFAULT_SPEED_MS,
                                chosen,
                                deck_id,
                            )
                            .await
                            {
                                Ok(s) => session.set(Some(s)),
                                Err(e) => {
                                    error_msg.set(Some(format!("Failed to create session: {e}")));
                                }
                            }
                            loading.set(false);
                        });
                    },
                    "Start"
                }
            }
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
    let translation_id = current_card.translation_id;
    let session_id = sess.id;
    let timer_fraction = speed().remaining_fraction();
    let is_flipped = flipped();
    let current_channels = media
        .read()
        .get(&current_index())
        .cloned()
        .unwrap_or_else(|| preload::CardChannels::begin(false));
    let audio_url = current_channels.audio_url().map(str::to_owned);
    let image_url = current_channels.image_url().map(str::to_owned);
    let image_credit = current_channels.image_credit().map(str::to_owned);
    let audio_failed = current_channels.audio_failed();
    let retry_index = idx;

    rsx! {
        div {
            div { class: "max-w-lg mx-auto mb-6",
                DeckSelect {
                    decks: decks(), // clone: the picker takes the list for this render
                    value: deck_choice,
                    on_pick,
                }
            }
            div { class: "grid grid-cols-1 lg:grid-cols-4 gap-6",
            div { class: "lg:col-span-3 space-y-4",
                timer::TimerBar { fraction: timer_fraction }
                card::CardDisplay {
                    card: current_card.clone(), // clone: component prop requires owned card data
                    flipped: is_flipped,
                    index: idx,
                    total: total,
                    audio_url: audio_url,
                    image_url: image_url,
                    image_credit: image_credit,
                    audio_failed: audio_failed,
                    on_retry_audio: move |_| {
                        let mut media_write = media.write();
                        if let Some(channels) = media_write.get_mut(&retry_index) {
                            if channels.retry_audio() {
                                let tid = translation_id;
                                drop(media_write);
                                spawn(async move {
                                    let audio = get_audio_data(tid).await.map_err(|_| ());
                                    preload::publish_audio(&mut media.write(), retry_index, audio);
                                });
                            }
                        }
                    },
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
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use wisecrow_dto::{DeckDto, MAX_DECK_SIZE};

    #[component]
    fn picker_fixture() -> Element {
        let value = use_signal(String::new);
        let decks = vec![
            DeckDto {
                id: 1,
                name: "Keeper".to_owned(),
                cards: 0,
            },
            DeckDto {
                id: 2,
                name: "Sea".to_owned(),
                cards: 1,
            },
        ];
        rsx! {
            DeckSelect {
                decks,
                value,
                on_pick: move |_| {},
            }
        }
    }

    fn at(html: &str, needle: &str) -> usize {
        html.find(needle)
            .unwrap_or_else(|| panic!("missing {needle} in {html}"))
    }

    #[component]
    fn size_fixture() -> Element {
        let preset_size = use_signal(|| DEFAULT_DECK_SIZE);
        let custom_size = use_signal(String::new);
        rsx! {
            DeckSizeFields { preset_size, custom_size }
        }
    }

    #[test]
    fn learn_size_offers_the_same_counts_as_fast_download() {
        let mut dom = VirtualDom::new(size_fixture);
        dom.rebuild_in_place();
        let html = dioxus::ssr::render(&dom);
        for preset in DECK_SIZE_PRESETS {
            assert!(
                html.contains(&format!(">{preset}<")),
                "missing preset {preset}"
            );
        }
        assert!(html.contains("or choose"));
        assert!(html.contains("id=\"learn-size\""));
        assert!(html.contains(&format!("max=\"{MAX_DECK_SIZE}\"")));
    }

    #[test]
    fn deck_picker_lists_all_words_then_the_users_decks() {
        let mut dom = VirtualDom::new(picker_fixture);
        dom.rebuild_in_place();
        let html = dioxus::ssr::render(&dom);
        let all = at(&html, "All words");
        let keeper = at(&html, "Keeper");
        let sea = at(&html, "Sea");
        assert!(all < keeper && keeper < sea);
    }
}
