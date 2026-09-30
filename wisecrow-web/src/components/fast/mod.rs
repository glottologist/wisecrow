mod playback;

use dioxus::prelude::*;

use wisecrow_dto::{FastDeckDto, SpeedController, MAX_DECK_SIZE};

use crate::api::learn::create_fast_deck;
use crate::api::media::{get_audio_data, get_image_data};
use crate::components::learn::preload::{self, CardChannels, ImageReady};

use self::playback::{Command, Event as PlaybackEvent, PlayStatus, RunMode};

const FAST_DECK_SIZE: u32 = 100;
const FAST_SIZE_PRESETS: [u32; 4] = [25, 50, 100, 250];
const FAST_SPEED_MS: u32 = 2000;
const TICK_INTERVAL_MS: u64 = 100;
const PRELOAD_WINDOW: usize = 10;

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

fn requested_size(custom: &str, preset: u32) -> Result<u32, String> {
    let typed = custom.trim();
    if typed.is_empty() {
        return Ok(preset);
    }
    match typed.parse::<u32>() {
        Ok(size) if (1..=MAX_DECK_SIZE).contains(&size) => Ok(size),
        _ => Err(format!("Choose a number between 1 and {MAX_DECK_SIZE}")),
    }
}

fn apply_playback(mut playback: Signal<playback::Playback>, event: PlaybackEvent) {
    // Effects dispatch events without subscribing to the state they update.
    let (next, command) = playback::reduce(&playback.peek(), event);
    playback.set(next);
    if let Some(command) = command {
        spawn(async move {
            run_command(playback, command).await;
        });
    }
}

async fn run_command(mut playback: Signal<playback::Playback>, command: Command) {
    match command {
        Command::Play(source) => {
            let event = match playback::play_source(&source).await {
                Ok(()) => PlaybackEvent::PlaySucceeded,
                Err(_) => PlaybackEvent::PlayRejected,
            };
            let (next, _) = playback::reduce(&playback(), event);
            playback.set(next);
        }
        Command::Pause => playback::pause_element(),
    }
}

/// Passive "fast download" run: both faces shown at once. Playback starts only
/// after an explicit Start with sound / Start without sound choice.
#[component]
pub fn FastPage(native: String, foreign: String) -> Element {
    let mut deck: Signal<Option<FastDeckDto>> = use_signal(|| None);
    let mut current_index = use_signal(|| 0usize);
    let mut speed = use_signal(|| SpeedController::new(FAST_SPEED_MS));
    let mut loading = use_signal(|| false);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let mut media: Signal<std::collections::HashMap<usize, CardChannels>> =
        use_signal(std::collections::HashMap::new);
    let mut media_generation = use_signal(|| 0u64);
    let mut playback_state = use_signal(playback::initial);

    let mut preset_size = use_signal(|| FAST_DECK_SIZE);
    let mut custom_size = use_signal(String::new);
    // Signals rather than the props themselves so the start handler stays
    // `Copy` and can serve both buttons.
    let native = use_signal(|| native);
    let foreign = use_signal(|| foreign);

    let start_run = move |event: PlaybackEvent| {
        let Ok(size) = requested_size(&custom_size.read(), preset_size()) else {
            return;
        };
        loading.set(true);
        spawn(async move {
            match create_fast_deck(native(), foreign(), size).await {
                Ok(d) => {
                    deck.set(Some(d));
                    apply_playback(playback_state, event);
                }
                Err(e) => error_msg.set(Some(format!("Failed to load deck: {e}"))),
            }
            loading.set(false);
        });
    };

    use_effect(move || {
        let d = deck();
        let idx = current_index();
        let Some(ref d) = d else { return };
        let generation = *media_generation.peek();
        let tracked: std::collections::HashSet<usize> = media.read().keys().copied().collect();
        for stale in preload::indices_to_evict(&tracked, idx, PRELOAD_WINDOW) {
            media.write().remove(&stale);
        }
        for fetch_index in preload::indices_to_fetch(idx, d.cards.len(), PRELOAD_WINDOW, &tracked) {
            let Some(card) = d.cards.get(fetch_index) else {
                continue;
            };
            let tid = card.translation_id;
            let fetch_image = preload::should_fetch_image(false, card.image_allowed);
            media
                .write()
                .insert(fetch_index, CardChannels::begin(fetch_image));
            spawn(async move {
                let audio = get_audio_data(tid).await.map_err(|_| ());
                if generation != *media_generation.peek() {
                    return;
                }
                if fetch_index == current_index() {
                    let event = match &audio {
                        Ok(url) => PlaybackEvent::AudioReady(url.clone()), // clone: reducer owns the URL
                        Err(()) => PlaybackEvent::AudioFailed,
                    };
                    apply_playback(playback_state, event);
                }
                preload::publish_audio(&mut media.write(), fetch_index, audio);
            });
            if fetch_image {
                spawn(async move {
                    let image = match get_image_data(tid).await {
                        Ok(Some(image)) => Ok(Some(ImageReady {
                            url: image.data_url,
                            credit: image.attribution,
                        })),
                        Ok(None) => Ok(None),
                        Err(_) => Err(()),
                    };
                    if generation != *media_generation.peek() {
                        return;
                    }
                    preload::publish_image(&mut media.write(), fetch_index, image);
                });
            }
        }
    });

    use_effect(move || {
        let idx = current_index();
        let source = media
            .read()
            .get(&idx)
            .and_then(|channels| channels.audio_url().map(str::to_owned));
        apply_playback(playback_state, PlaybackEvent::SourceChanged(source));
    });

    let _timer_task = use_future(move || async move {
        loop {
            async_sleep(TICK_INTERVAL_MS).await;

            let running = deck()
                .as_ref()
                .is_some_and(|d| current_index() < d.cards.len());
            if running && playback_state().timer_enabled {
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
        let choice = requested_size(&custom_size.read(), preset_size());
        let size_ok = choice.is_ok();
        let size_error = choice.err();
        let typed_custom = !custom_size.read().trim().is_empty();
        return rsx! {
            div { class: "max-w-2xl mx-auto",
                div { class: "bg-gray-800 rounded-xl p-8 space-y-6",
                    h2 { class: "text-2xl font-bold text-cyan-400", "Fast download" }
                    p { class: "text-sm text-gray-400",
                        "Choose how many common words and phrases to include."
                    }
                    div { class: "flex flex-wrap justify-center gap-3",
                        for preset in FAST_SIZE_PRESETS {
                            button {
                                key: "{preset}",
                                aria_pressed: preset == preset_size() && !typed_custom,
                                class: if preset == preset_size() && !typed_custom { "bg-cyan-700 rounded px-5 py-2 font-semibold" } else { "bg-gray-700 hover:bg-gray-600 rounded px-5 py-2 transition" },
                                onclick: move |_| {
                                    preset_size.set(preset);
                                    custom_size.set(String::new());
                                },
                                "{preset}"
                            }
                        }
                    }
                    div { class: "flex items-center justify-center gap-2",
                        label { r#for: "fast-size", class: "text-sm text-gray-400", "or choose" }
                        input {
                            id: "fast-size",
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
                    div { class: "flex flex-col items-center gap-3",
                        button {
                            class: "bg-cyan-700 hover:bg-cyan-600 rounded px-6 py-3 font-semibold",
                            disabled: !size_ok,
                            onclick: move |_| {
                                let mut run = start_run;
                                run(PlaybackEvent::StartWithSound);
                            },
                            "Start with sound"
                        }
                        button {
                            class: "bg-gray-700 hover:bg-gray-600 rounded px-6 py-3 font-semibold",
                            disabled: !size_ok,
                            onclick: move |_| {
                                let mut run = start_run;
                                run(PlaybackEvent::StartWithoutSound);
                            },
                            "Start without sound"
                        }
                    }
                }
            }
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
                        media_generation.set(media_generation().wrapping_add(1));
                        media.write().clear();
                        deck.set(None);
                        current_index.set(0);
                        speed.write().reset();
                        playback_state.set(playback::initial());
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
    let current_channels = media
        .read()
        .get(&idx)
        .cloned()
        .unwrap_or_else(|| CardChannels::begin(false));
    let playback = playback_state();
    let awaiting_choice = playback.mode == RunMode::AwaitingChoice;
    let audio_failed = playback.status == PlayStatus::Failed || current_channels.audio_failed();
    let timer_on = playback.timer_enabled;

    rsx! {
        div { class: "max-w-2xl mx-auto space-y-6",
            audio {
                id: playback::AUDIO_ELEMENT_ID,
                class: "hidden",
            }
            div { class: "w-full bg-gray-800 rounded h-2",
                div {
                    class: "bg-cyan-500 h-2 rounded transition-all",
                    style: "width: {progress_pct}%",
                }
            }

            div { class: "bg-gray-800 rounded-xl p-8 min-h-[300px] flex flex-col relative",
                div { class: "text-sm text-gray-500 mb-4", "Item {display_num} / {total}" }
                div { class: "flex-1 flex flex-col items-center justify-center space-y-4",
                    if let Some(img_src) = current_channels.image_url() {
                        img {
                            class: "mx-auto rounded max-w-[200px] max-h-[200px]",
                            src: "{img_src}",
                            alt: "{card.from_phrase}",
                        }
                        if let Some(credit) = current_channels.image_credit() {
                            p { class: "text-xs text-gray-600", "{credit}" }
                        }
                    }
                    p { class: "text-3xl font-bold text-cyan-400", "{card.to_phrase}" }
                    p { class: "text-xl text-emerald-400", "{card.from_phrase}" }
                    if audio_failed {
                        p { class: "text-sm text-red-400", "Audio unavailable" }
                    }
                }
                if awaiting_choice {
                    div { class: "absolute inset-0 bg-gray-900/80 rounded-xl flex flex-col items-center justify-center gap-3",
                        button {
                            class: "bg-cyan-700 hover:bg-cyan-600 rounded px-6 py-3 font-semibold",
                            onclick: move |_| apply_playback(playback_state, PlaybackEvent::StartWithSound),
                            "Start with sound"
                        }
                        button {
                            class: "bg-gray-700 hover:bg-gray-600 rounded px-6 py-3 font-semibold",
                            onclick: move |_| apply_playback(playback_state, PlaybackEvent::StartWithoutSound),
                            "Start without sound"
                        }
                        if audio_failed {
                            button {
                                class: "bg-gray-700 hover:bg-gray-600 rounded px-6 py-3 font-semibold",
                                onclick: move |_| apply_playback(playback_state, PlaybackEvent::Retry),
                                "Retry"
                            }
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
                    onclick: move |_| {
                        let event = if timer_on {
                            PlaybackEvent::Pause
                        } else {
                            PlaybackEvent::Resume
                        };
                        apply_playback(playback_state, event);
                    },
                    if timer_on { "Pause" } else { "Resume" }
                }
            }
            p { class: "text-center text-xs text-gray-600", "Speed: {speed_secs:.1}s per item" }
        }
    }
}

#[cfg(test)]
mod tests {
    use dioxus::dioxus_core::ReactiveContext;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("", 25, 25)]
    #[case(" \t\n", 250, 250)]
    #[case("1", 100, 1)]
    #[case("50", 100, 50)]
    #[case("500", 100, MAX_DECK_SIZE)]
    #[case(" 42 ", 100, 42)]
    fn requested_size_uses_preset_or_valid_custom(
        #[case] custom: &str,
        #[case] preset: u32,
        #[case] expected: u32,
    ) {
        assert_eq!(requested_size(custom, preset), Ok(expected));
    }

    #[rstest]
    #[case("0")]
    #[case("501")]
    #[case("abc")]
    #[case("-1")]
    #[case("1.5")]
    #[case("4294967296")]
    #[case("五十")]
    fn requested_size_rejects_invalid_custom(#[case] custom: &str) {
        assert_eq!(
            requested_size(custom, FAST_DECK_SIZE),
            Err(format!("Choose a number between 1 and {MAX_DECK_SIZE}"))
        );
    }

    #[test]
    fn source_change_does_not_reschedule_its_dispatching_effect() {
        let dom = VirtualDom::new(|| rsx! {});
        dom.in_scope(ScopeId::ROOT, || {
            let playback = Signal::new(playback::initial());
            let (effect, mut updates) = ReactiveContext::new();

            effect.run_in(|| apply_playback(playback, PlaybackEvent::SourceChanged(None)));

            assert!(updates.try_recv().is_err());
        });
    }
}
