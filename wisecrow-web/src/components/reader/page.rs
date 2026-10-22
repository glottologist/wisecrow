//! One chapter of a translated book.

use dioxus::prelude::*;
use wisecrow_dto::{ChapterDto, DeckDto, GlossDto, TokenDto};

use super::popover::WordPopover;

const READER_AUDIO_ID: &str = "wisecrow-reader-audio";

enum Piece {
    Text(String),
    Token { surface: String, normalised: String },
}

fn pieces(text: &str, tokens: &[TokenDto]) -> Vec<Piece> {
    let mut rest = text;
    let mut found = Vec::new();
    for token in tokens {
        let Some(index) = rest.find(&token.surface) else {
            continue;
        };
        let (before, after) = rest.split_at(index);
        if !before.is_empty() {
            found.push(Piece::Text(before.to_owned()));
        }
        let end = token.surface.len();
        if after.len() < end {
            continue;
        }
        let (surface, tail) = after.split_at(end);
        found.push(Piece::Token {
            surface: surface.to_owned(),
            normalised: token.normalised.clone(), // clone: the tap handler keeps the key
        });
        rest = tail;
    }
    if !rest.is_empty() {
        found.push(Piece::Text(rest.to_owned()));
    }
    found
}

fn ordinal(index: usize) -> i32 {
    i32::try_from(index).map_or(i32::MAX, |value| value.saturating_add(1))
}

#[component]
pub fn ReaderPage(native: String, foreign: String, book: i32) -> Element {
    let mut chapter_row: Signal<Option<ChapterDto>> = use_signal(|| None);
    let mut decks = use_signal(Vec::new);
    let show_english = use_signal(|| false);
    let mut loading = use_signal(|| true);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let native_code = native.clone(); // clone: loading the chapter runs while the page owns the pair
    let foreign_code = foreign.clone(); // clone: loading the chapter runs while the page owns the pair

    use_future(move || {
        let native_code = native_code.clone(); // clone: the future closure may run again
        let foreign_code = foreign_code.clone(); // clone: the future closure may run again
        async move {
            let start = match crate::api::reader::library(native_code.clone(), foreign_code.clone())
                .await
            {
                Ok(shelf) => shelf
                    .books
                    .iter()
                    .find(|row| row.id == book)
                    .and_then(|row| row.chapter)
                    .unwrap_or(1),
                Err(error) => {
                    tracing::warn!("reader shelf was not loaded: {error}");
                    1
                }
            };
            match crate::api::reader::chapter(
                native_code.clone(),
                foreign_code.clone(),
                book,
                start,
            )
            .await
            {
                Ok(text) => chapter_row.set(Some(text)),
                Err(error) => {
                    error_msg.set(Some(format!("This chapter is not available: {error}")));
                }
            }
            match crate::api::decks::list_decks(native_code, foreign_code).await {
                Ok(rows) => decks.set(rows),
                Err(error) => tracing::warn!("reader decks were not listed: {error}"),
            }
            loading.set(false);
        }
    });

    if loading() {
        return rsx! {
            div { class: "text-center text-gray-400 text-xl py-20", "Loading chapter..." }
        };
    }
    if let Some(error) = error_msg() {
        return rsx! {
            div { class: "text-center text-red-400 text-xl py-20", "{error}" }
        };
    }
    let Some(chapter) = chapter_row() else {
        return rsx! {
            div { class: "text-center text-gray-400 text-xl py-20", "Loading chapter..." }
        };
    };

    rsx! {
        ChapterView {
            chapter,
            gloss: GlossDto {
                status: "pending".into(),
                meaning: None,
                lemma: None,
                class: None,
                note: None,
            },
            decks: decks(), // clone: the chapter takes the list for this render
            show_english,
            native,
            foreign,
            start_open: false,
        }
    }
}

#[component]
pub fn ChapterView(
    chapter: ChapterDto,
    gloss: GlossDto,
    decks: Vec<DeckDto>,
    show_english: Signal<bool>,
    #[props(default)] native: String,
    #[props(default)] foreign: String,
    #[props(default = true)] start_open: bool,
) -> Element {
    let book_id = chapter.book_id;
    let chapter_number = chapter.chapter;
    let mut note = use_signal(|| gloss.clone()); // clone: the signal keeps the note after the first render
    let mut shelf = use_signal(|| decks.clone()); // clone: adding a deck replaces this list locally
    let mut showing = use_signal(move || start_open);
    let mut open_word = use_signal(|| None::<String>);
    let mut selected = use_signal(|| None::<i32>);
    let native_code = use_signal(|| native);
    let foreign_code = use_signal(|| foreign);

    rsx! {
        audio { id: READER_AUDIO_ID }
        button {
            r#type: "button",
            onclick: move |_| show_english.set(!show_english()),
            "English"
        }
        for paragraph in chapter.paragraphs {
            {
                let paragraph_number = paragraph.paragraph;
                rsx! {
                    div { class: "space-y-3",
                        for (index, sentence) in paragraph.sentences.into_iter().enumerate() {
                            {
                                let sentence_number = ordinal(index);
                                let sentence_id = sentence.id;
                                let highlighted = selected() == Some(sentence_id);
                                rsx! {
                                    div {
                                        class: if highlighted { "bg-gray-700" } else { "" },
                                        onclick: move |_| {
                                            selected.set(Some(sentence_id));
                                            spawn(async move {
                                                if let Err(error) = crate::api::reader::save_position(
                                                    book_id,
                                                    chapter_number,
                                                    paragraph_number,
                                                    sentence_number,
                                                )
                                                .await
                                                {
                                                    tracing::warn!(
                                                        "reader position was not saved: {error}"
                                                    );
                                                }
                                            });
                                        },
                                        p { "data-sentence": "{sentence.text}",
                                            for piece in pieces(&sentence.text, &sentence.tokens) {
                                                match piece {
                                                    Piece::Text(text) => rsx! { span { "{text}" } },
                                                    Piece::Token { surface, normalised } => rsx! {
                                                        button {
                                                            class: "reader-token",
                                                            r#type: "button",
                                                            onclick: move |_| {
                                                                if open_word() == Some(normalised.clone()) {
                                                                    open_word.set(None);
                                                                    showing.set(false);
                                                                    return;
                                                                }
                                                                let word = normalised.clone(); // clone: the request owns the key
                                                                open_word.set(Some(word.clone())); // clone: the signal keeps the key while the request runs
                                                                let native = native_code();
                                                                let foreign = foreign_code();
                                                                spawn(async move {
                                                                    match crate::api::reader::gloss(native, foreign, word.clone()).await {
                                                                        Ok(loaded) => {
                                                                            if open_word() == Some(word) {
                                                                                note.set(loaded);
                                                                                showing.set(true);
                                                                            }
                                                                        }
                                                                        Err(error) => tracing::warn!("reader note was not loaded: {error}"),
                                                                    }
                                                                });
                                                            },
                                                            "{surface}"
                                                        }
                                                    },
                                                }
                                            }
                                        }
                                        button {
                                            r#type: "button",
                                            onclick: move |_| {
                                                spawn(async move {
                                                    match crate::api::reader::sentence_audio(sentence_id).await {
                                                        Ok(url) => play_clip(&url).await,
                                                        Err(error) => {
                                                            tracing::warn!("reader sentence audio was not ready: {error}");
                                                        }
                                                    }
                                                });
                                            },
                                            "Play"
                                        }
                                        if show_english() {
                                            p { class: "reader-source", "{sentence.source}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if showing() {
            WordPopover {
                gloss: note(),
                decks: shelf(),
                on_play: move |_| {
                    let Some(word) = open_word() else {
                        return;
                    };
                    let native = native_code();
                    let foreign = foreign_code();
                    spawn(async move {
                        match crate::api::reader::word_audio(native, foreign, word).await {
                            Ok(url) => play_clip(&url).await,
                            Err(error) => tracing::warn!("reader word audio was not ready: {error}"),
                        }
                    });
                },
                on_add: move |(deck_id, name): (Option<i32>, String)| {
                    let Some(word) = open_word() else {
                        return;
                    };
                    let native = native_code();
                    let foreign = foreign_code();
                    spawn(async move {
                        let created = if name.trim().is_empty() {
                            deck_id
                        } else {
                            match crate::api::decks::create_deck(native.clone(), foreign.clone(), name).await {
                                Ok(deck) => Some(deck.id),
                                Err(error) => {
                                    tracing::warn!("reader deck was not created: {error}");
                                    return;
                                }
                            }
                        };
                        let Some(created) = created else {
                            return;
                        };
                        if let Err(error) = crate::api::decks::add_to_deck(created, word).await {
                            tracing::warn!("reader card was not added: {error}");
                            return;
                        }
                        match crate::api::decks::list_decks(native, foreign).await {
                            Ok(rows) => shelf.set(rows),
                            Err(error) => tracing::warn!("reader decks were not listed: {error}"),
                        }
                    });
                },
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
async fn play_clip(src: &str) {
    use wasm_bindgen::JsCast;

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Some(element) = document.get_element_by_id(READER_AUDIO_ID) else {
        tracing::warn!("reader audio element is missing");
        return;
    };
    let Ok(audio) = element.dyn_into::<web_sys::HtmlAudioElement>() else {
        tracing::warn!("reader audio element is missing");
        return;
    };
    audio.set_src(src);
    let Ok(promise) = audio.play() else {
        tracing::warn!("reader audio was rejected");
        return;
    };
    if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
        tracing::warn!("reader audio was rejected");
    }
}

#[cfg(all(
    not(target_arch = "wasm32"),
    feature = "native",
    not(feature = "server")
))]
async fn play_clip(src: &str) {
    let eval = dioxus::document::eval(
        r#"
        const [id, source] = await dioxus.recv();
        const audio = document.getElementById(id);
        if (!audio) return "unavailable";
        audio.src = source;
        try { await audio.play(); return "playing"; }
        catch { return "rejected"; }
    "#,
    );
    if eval.send((READER_AUDIO_ID, src)).is_err() {
        tracing::warn!("reader audio could not be sent");
        return;
    }
    if !matches!(eval.join::<String>().await.as_deref(), Ok("playing")) {
        tracing::warn!("reader audio did not play");
    }
}

#[cfg(not(any(
    target_arch = "wasm32",
    all(feature = "native", not(feature = "server"))
)))]
async fn play_clip(_src: &str) {}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;
    use wisecrow_dto::{ParagraphDto, SentenceDto};

    fn chapter(pending: bool) -> ChapterDto {
        ChapterDto {
            book_id: 4,
            chapter: 1,
            paragraphs: vec![ParagraphDto {
                paragraph: 1,
                sentences: vec![SentenceDto {
                    id: 7,
                    text: "Tom vivía.".into(),
                    source: "Tom lived.".into(),
                    tokens: vec![TokenDto {
                        surface: "vivía".into(),
                        normalised: "vivia".into(),
                        pending,
                    }],
                }],
            }],
        }
    }

    fn gloss(pending: bool) -> GlossDto {
        GlossDto {
            status: if pending { "pending" } else { "ok" }.into(),
            meaning: if pending { None } else { Some("lived".into()) },
            lemma: None,
            class: None,
            note: None,
        }
    }

    #[component]
    fn fixture(show_english: bool, pending: bool) -> Element {
        let show_english = use_signal(move || show_english);
        rsx! {
            ChapterView {
                chapter: chapter(pending),
                gloss: gloss(pending),
                decks: Vec::<DeckDto>::new(),
                show_english,
            }
        }
    }

    fn render(show_english: bool, pending: bool) -> String {
        let mut dom = VirtualDom::new_with_props(
            fixture,
            fixtureProps {
                show_english,
                pending,
            },
        );
        dom.rebuild_in_place();
        dioxus::ssr::render(&dom)
    }

    #[test]
    fn a_chapter_hides_english_until_dual_mode_and_a_pending_note_cannot_be_added() {
        let hidden = render(false, false);
        assert!(hidden.contains("Tom vivía."), "{hidden}");
        assert!(!hidden.contains("Tom lived."), "{hidden}");
        assert!(hidden.contains("lived"), "{hidden}");
        assert!(hidden.contains("Add to deck"), "{hidden}");
        assert!(hidden.contains("Play"), "{hidden}");

        let shown = render(true, false);
        assert!(shown.contains("Tom vivía."), "{shown}");
        assert!(shown.contains("Tom lived."), "{shown}");
        assert!(shown.contains("reader-source"), "{shown}");

        let pending = render(false, true);
        assert!(pending.contains("Note pending"), "{pending}");
        assert!(!pending.contains("Add to deck"), "{pending}");
        assert!(!pending.contains("Tom lived."), "{pending}");
    }
}
