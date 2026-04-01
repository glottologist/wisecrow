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
                        if let Some(ref audio_src) = audio_url {
                            audio {
                                src: "{audio_src}",
                                autoplay: true,
                                controls: true,
                                class: "mx-auto mt-2",
                            }
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
                        }
                        p { class: "text-3xl font-bold text-cyan-400 mb-4",
                            "{card.to_phrase}"
                        }
                        p { class: "text-gray-500 text-sm",
                            "Click or press Space to reveal"
                        }
                        if let Some(ref audio_src) = audio_url {
                            audio {
                                src: "{audio_src}",
                                autoplay: true,
                                controls: true,
                                class: "mx-auto mt-2",
                            }
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
