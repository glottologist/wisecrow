//! The shelf of books a learner can open.

use dioxus::prelude::*;
use wisecrow_dto::BookDto;

use crate::router::Route;

#[component]
pub fn LibraryPage(native: String, foreign: String) -> Element {
    let mut books = use_signal(Vec::new);
    let mut loading = use_signal(|| true);
    let mut error_msg: Signal<Option<String>> = use_signal(|| None);
    let native_code = native.clone(); // clone: the shelf request runs while the list still owns the pair
    let foreign_code = foreign.clone(); // clone: the shelf request runs while the list still owns the pair

    use_future(move || {
        let native_code = native_code.clone(); // clone: the future closure may run again
        let foreign_code = foreign_code.clone(); // clone: the future closure may run again
        async move {
            match crate::api::reader::library(native_code, foreign_code).await {
                Ok(shelf) => books.set(shelf.books),
                Err(error) => error_msg.set(Some(format!("The shelf is not available: {error}"))),
            }
            loading.set(false);
        }
    });

    if loading() {
        return rsx! {
            div { class: "text-center text-gray-400 text-xl py-20", "Loading books..." }
        };
    }
    if let Some(error) = error_msg() {
        return rsx! {
            div { class: "text-center text-red-400 text-xl py-20", "{error}" }
        };
    }

    rsx! {
        LibraryList { native, foreign, books: books() }
    }
}

#[component]
pub fn LibraryList(native: String, foreign: String, books: Vec<BookDto>) -> Element {
    if books.is_empty() {
        return rsx! {
            div { class: "text-center space-y-2 py-20",
                h1 { class: "text-2xl font-bold", "No books for this language yet" }
                p { class: "text-gray-400",
                    "They show up here as the reader job stores them."
                }
            }
        };
    }
    rsx! {
        ul { class: "space-y-3",
            for book in books {
                li {
                    if book.readable {
                        Link {
                            to: Route::ReaderPage {
                                native: native.clone(), // clone: each book link owns the pair
                                foreign: foreign.clone(), // clone: each book link owns the pair
                                book: book.id,
                            },
                            "{book.title} {book.cefr} {book.sentences}"
                            if let Some(chapter) = book.chapter {
                                " Chapter {chapter}"
                            }
                        }
                    } else {
                        span { "{book.title} · {book.cefr} · not ready yet" }
                    }
                }
            }
        }
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use dioxus::prelude::*;

    use super::*;

    fn book(id: i32, title: &str, readable: bool) -> BookDto {
        BookDto {
            id,
            title: title.into(),
            series: "Penguin Readers".into(),
            cefr: "A2".into(),
            sentences: 12,
            readable,
            chapter: None,
            paragraph: None,
            sentence: None,
        }
    }

    fn render(books: Vec<BookDto>) -> String {
        let mut dom = VirtualDom::new_with_props(
            LibraryList,
            LibraryListProps {
                native: "en".into(),
                foreign: "es".into(),
                books,
            },
        );
        dom.rebuild_in_place();
        dioxus::ssr::render(&dom)
    }

    #[test]
    fn an_empty_shelf_says_there_are_no_books_yet() {
        let html = render(Vec::new());

        assert!(html.contains("No books for this language yet"), "{html}");
    }

    #[test]
    fn a_book_still_being_stored_is_named_and_not_linked() {
        let html = render(vec![book(5, "Waiting", false)]);

        assert!(html.contains("Waiting · A2 · not ready yet"), "{html}");
        assert!(!html.contains("href="), "{html}");
    }
}
