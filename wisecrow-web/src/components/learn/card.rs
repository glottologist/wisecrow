use dioxus::prelude::*;

use wisecrow_dto::{CardDto, ReviewRatingDto, ScriptDirection};

#[component]
pub fn CardDisplay(
    card: CardDto,
    flipped: bool,
    index: usize,
    total: usize,
    audio_url: Option<String>,
    image_url: Option<String>,
    /// Photographer and source, shown under the image because Pixabay and
    /// Unsplash both require the origin to be visible wherever it is displayed.
    #[props(default = None)]
    image_credit: Option<String>,
    #[props(default = false)] audio_failed: bool,
    #[props(default)] on_retry_audio: EventHandler<()>,
    on_flip: EventHandler<()>,
    on_rate: EventHandler<ReviewRatingDto>,
    #[props(default = ScriptDirection::Ltr)] script_direction: ScriptDirection,
) -> Element {
    let display_num = index.saturating_add(1);
    let dir_class = match script_direction {
        ScriptDirection::Rtl => "dir-rtl font-intl",
        ScriptDirection::Ltr => "font-intl",
    };

    rsx! {
        div { class: "bg-gray-800 rounded-xl p-8 min-h-[300px] flex flex-col {dir_class}",
            div { class: "text-sm text-gray-500 mb-4",
                "Card {display_num} / {total}"
            }

            div { class: "flex-1 flex items-center justify-center",
                if flipped {
                    div { class: "text-center space-y-4",
                        if let Some(ref img_src) = image_url {
                            img {
                                class: "mx-auto mb-4 rounded max-w-[200px] max-h-[200px]",
                                src: "{img_src}",
                                alt: "{card.from_phrase}",
                            }
                            if let Some(ref credit) = image_credit {
                                p { class: "text-xs text-gray-600 -mt-3 mb-3", "{credit}" }
                            }
                        }
                        p { class: "text-2xl font-bold text-cyan-400",
                            "{card.to_phrase}"
                        }
                        p { class: "text-xl text-emerald-400",
                            "{card.from_phrase}"
                        }
                        if card.frequency > 0 {
                            p { class: "text-sm text-gray-500",
                                "Frequency rank: {card.frequency}"
                            }
                        }
                        AudioStatus {
                            audio_url: audio_url.clone(),
                            audio_failed: audio_failed,
                            on_retry_audio: on_retry_audio,
                        }
                    }
                } else {
                    div { class: "text-center cursor-pointer",
                        onclick: move |_| on_flip.call(()),
                        if let Some(ref img_src) = image_url {
                            img {
                                class: "mx-auto mb-4 rounded max-w-[200px] max-h-[200px]",
                                src: "{img_src}",
                                alt: "{card.to_phrase}",
                            }
                            if let Some(ref credit) = image_credit {
                                p { class: "text-xs text-gray-600 -mt-3 mb-3", "{credit}" }
                            }
                        }
                        p { class: "text-3xl font-bold text-cyan-400 mb-4",
                            "{card.to_phrase}"
                        }
                        p { class: "text-gray-500 text-sm",
                            "Click or press Space to reveal"
                        }
                        AudioStatus {
                            audio_url: audio_url.clone(),
                            audio_failed: audio_failed,
                            on_retry_audio: on_retry_audio,
                        }
                    }
                }
            }

            if flipped {
                div { class: "flex justify-center gap-3 mt-6",
                    RatingButton { label: "Again", shortcut: "1", color: "red", rating: ReviewRatingDto::Again, on_rate: on_rate }
                    RatingButton { label: "Hard", shortcut: "2", color: "orange", rating: ReviewRatingDto::Hard, on_rate: on_rate }
                    RatingButton { label: "Good", shortcut: "3", color: "emerald", rating: ReviewRatingDto::Good, on_rate: on_rate }
                    RatingButton { label: "Easy", shortcut: "4", color: "blue", rating: ReviewRatingDto::Easy, on_rate: on_rate }
                }
            }
        }
    }
}

#[component]
fn AudioStatus(
    audio_url: Option<String>,
    audio_failed: bool,
    on_retry_audio: EventHandler<()>,
) -> Element {
    if let Some(ref audio_src) = audio_url {
        return rsx! {
            audio {
                src: "{audio_src}",
                autoplay: true,
                controls: true,
                class: "mx-auto mt-2",
            }
        };
    }
    if audio_failed {
        return rsx! {
            div { class: "mt-2 space-y-2",
                p { class: "text-sm text-red-400", "Audio unavailable" }
                button {
                    class: "bg-gray-700 hover:bg-gray-600 rounded px-3 py-1 text-sm",
                    onclick: move |_| on_retry_audio.call(()),
                    "Retry"
                }
            }
        };
    }
    rsx! {}
}

#[component]
fn RatingButton(
    label: &'static str,
    shortcut: &'static str,
    color: &'static str,
    rating: ReviewRatingDto,
    on_rate: EventHandler<ReviewRatingDto>,
) -> Element {
    let btn_class = format!(
        "px-4 py-2 rounded font-semibold transition bg-{color}-600 hover:bg-{color}-500 text-white"
    );

    rsx! {
        button {
            class: "{btn_class}",
            onclick: move |_| on_rate.call(rating),
            "[{shortcut}] {label}"
        }
    }
}
