//! The shelf of books a learner can open.
//!
//! Tiles run from A1 to C2. A readable book is a light green link. A book that
//! is not readable yet is light red, its cover is grey, and it is not a link.

use dioxus::prelude::*;
use wisecrow_dto::BookDto;

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
    let mut books = books;
    books.sort_by(shelf_order);
    rsx! {
        ul { class: "shelf-grid",
            for book in books {
                ShelfTile {
                    href: open_path(&native, &foreign, book.id, book.readable),
                    book,
                }
            }
        }
    }
}

#[component]
fn ShelfTile(href: Option<String>, book: BookDto) -> Element {
    let tile_class = if href.is_some() {
        "shelf-tile shelf-ready"
    } else {
        "shelf-tile shelf-waiting"
    };
    if let Some(href) = href {
        rsx! {
            li { class: "{tile_class}",
                a { class: "shelf-link", href,
                    ShelfFace { book }
                }
            }
        }
    } else {
        rsx! {
            li { class: "{tile_class}",
                div { class: "shelf-link shelf-unavailable",
                    ShelfFace { book }
                }
            }
        }
    }
}

#[component]
fn ShelfFace(book: BookDto) -> Element {
    let cover = cover_src(book.id);
    rsx! {
        if let Some(src) = cover {
            img {
                class: "shelf-cover",
                alt: "",
                src,
                loading: "lazy",
                decoding: "async",
            }
        }
        span { class: "shelf-caption",
            span { class: "shelf-title", "{book.title}" }
            span { class: "shelf-level", "{book.cefr}" }
            if book.readable {
                if let Some(chapter) = book.chapter {
                    span { class: "shelf-place", "Chapter {chapter}" }
                }
            } else {
                span { class: "shelf-status", "not ready yet" }
            }
        }
    }
}

fn shelf_order(left: &BookDto, right: &BookDto) -> std::cmp::Ordering {
    level_rank(&left.cefr)
        .cmp(&level_rank(&right.cefr))
        .then_with(|| left.title.cmp(&right.title))
        .then_with(|| left.id.cmp(&right.id))
}

fn level_rank(code: &str) -> u8 {
    match code {
        "A1" => 0,
        "A2" => 1,
        "B1" => 2,
        "B2" => 3,
        "C1" => 4,
        "C2" => 5,
        _ => u8::MAX,
    }
}

fn open_path(native: &str, foreign: &str, book_id: i32, readable: bool) -> Option<String> {
    if !readable {
        return None;
    }
    book_path(native, foreign, book_id)
}

/// A route parameter may only be a language code, so the link cannot gain a slash.
fn book_path(native: &str, foreign: &str, book_id: i32) -> Option<String> {
    if !language_code(native) || !language_code(foreign) || book_id < 1 {
        return None;
    }
    let mut path = String::with_capacity(native.len() + foreign.len() + 16);
    path.push_str("/read/");
    path.push_str(native);
    path.push('/');
    path.push_str(foreign);
    path.push('/');
    path.push_str(&book_id.to_string());
    Some(path)
}

fn cover_src(book_id: i32) -> Option<String> {
    if book_id < 1 {
        return None;
    }
    let mut src = String::from("/api/reader/covers/");
    src.push_str(&book_id.to_string());
    Some(src)
}

fn language_code(code: &str) -> bool {
    !code.is_empty() && code.len() <= 10 && code.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use dioxus::prelude::*;

    use super::*;

    fn book(id: i32, title: &str, cefr: &str, readable: bool) -> BookDto {
        BookDto {
            id,
            title: title.into(),
            series: "Penguin Readers".into(),
            cefr: cefr.into(),
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
    fn a_book_still_being_stored_is_a_grey_tile_and_not_a_link() {
        let html = render(vec![book(5, "Waiting", "A2", false)]);

        assert!(html.contains("shelf-waiting"), "{html}");
        assert!(html.contains("shelf-unavailable"), "{html}");
        assert!(html.contains("Waiting"), "{html}");
        assert!(html.contains("not ready yet"), "{html}");
        assert!(html.contains("/api/reader/covers/5"), "{html}");
        assert!(!html.contains("href="), "{html}");
    }

    #[test]
    fn a_readable_book_is_a_green_tile_that_opens() {
        let mut ready = book(4, "Ready", "B1", true);
        ready.chapter = Some(2);
        let html = render(vec![ready]);

        assert!(html.contains("shelf-ready"), "{html}");
        assert!(html.contains("href=\"/read/en/es/4\""), "{html}");
        assert!(html.contains("/api/reader/covers/4"), "{html}");
        assert!(html.contains("Chapter 2"), "{html}");
        assert!(!html.contains("not ready yet"), "{html}");
        assert!(!html.contains("shelf-unavailable"), "{html}");
    }

    #[test]
    fn books_are_tiles_in_level_order_not_title_order() {
        let html = render(vec![
            book(3, "Airport", "B2", false),
            book(9, "Same", "A2", false),
            book(1, "Zebra", "A1", true),
            book(2, "Same", "A2", false),
            book(8, "Mystery", "C9", false),
        ]);

        let zebra = html.find("Zebra").expect("A1");
        let same_first = html.find("Same").expect("A2");
        let airport = html.find("Airport").expect("B2");
        let mystery = html.find("Mystery").expect("unknown level");
        assert!(zebra < same_first, "{html}");
        assert!(same_first < airport, "{html}");
        assert!(airport < mystery, "{html}");
        assert!(
            html.find("href=\"/read/en/es/1\"").is_some(),
            "the A1 book stays a link: {html}"
        );
        let same_lower_id = html.find("/api/reader/covers/2").expect("lower id");
        let same_higher_id = html.find("/api/reader/covers/9").expect("higher id");
        assert!(same_lower_id < same_higher_id, "{html}");
    }

    #[test]
    fn a_language_code_with_a_slash_is_not_a_link() {
        let mut dom = VirtualDom::new_with_props(
            LibraryList,
            LibraryListProps {
                native: "en/es".into(),
                foreign: "es".into(),
                books: vec![book(4, "Ready", "A2", true)],
            },
        );
        dom.rebuild_in_place();
        let html = dioxus::ssr::render(&dom);

        assert!(!html.contains("href="), "{html}");
        assert!(html.contains("shelf-unavailable"), "{html}");
    }
}
