//! Gets the text of a PDF from whichever extractor can actually read it.
//!
//! `pdf-extract` is a pure-Rust extractor, which is why we reach for it first: it
//! needs nothing installed and it reads the public-domain scans and the open
//! textbooks well. It is also defeated by commercial ebooks whose fonts carry
//! custom encodings. Five French grammars from McGraw-Hill and Routledge yielded
//! between twenty-one and eighty-eight characters a page through it, where
//! poppler's `pdftotext` read the same files at six hundred to two thousand,
//! and a sixth made it panic outright.
//!
//! The strategy follows from that. We take the native reading when it is dense
//! enough to be prose, and otherwise ask poppler and keep whichever text holds
//! more. A machine without poppler installed is no worse off than before, since
//! the thin native reading is still returned and the caller still decides what to
//! do with it.

use std::path::Path;

use crate::errors::WisecrowError;

/// The external extractor, used when the native one comes back thin.
const POPPLER_BINARY: &str = "pdftotext";
/// Characters per page at which a reading is taken to be prose.
///
/// A page of a grammar book carries a couple of thousand characters. Two hundred
/// is the level at which a scan's copyright notice sits, so a reading below it is
/// either a document with no text layer or an extractor that has failed to read
/// one, and both are worth a second attempt.
pub const MIN_CHARS_PER_PAGE: usize = 200;
/// Page separator `pdftotext` writes between pages.
const PAGE_BREAK: char = '\u{c}';

/// Which extractor produced a document's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extractor {
    /// `pdf-extract`, in process.
    Native,
    /// poppler's `pdftotext`, as a child process.
    Poppler,
}

impl Extractor {
    /// How the extractor is named in a report.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Native => "pdf-extract",
            Self::Poppler => "poppler",
        }
    }
}

/// A document's text, one string per page, and where it came from.
#[derive(Debug, Clone)]
pub struct PageText {
    pub pages: Vec<String>,
    pub extractor: Extractor,
}

impl PageText {
    /// Characters per page of the reading.
    #[must_use]
    pub fn density(&self) -> usize {
        density(&self.pages)
    }
}

/// Reads the text of the PDF at `path`, page by page.
///
/// # Errors
///
/// Returns an error only when neither extractor can read the document; the
/// message names what each of them said.
pub fn pages(path: &Path) -> Result<PageText, WisecrowError> {
    prefer(native_pages(path), || poppler_pages(path))
}

/// Settles which reading of a document to use.
///
/// `fallback` is taken lazily because it starts a child process, and a document
/// the native extractor reads well should not pay for one.
fn prefer<F>(native: Result<Vec<String>, String>, fallback: F) -> Result<PageText, WisecrowError>
where
    F: FnOnce() -> Result<Vec<String>, String>,
{
    match native {
        Ok(pages) if density(&pages) >= MIN_CHARS_PER_PAGE => Ok(PageText {
            pages,
            extractor: Extractor::Native,
        }),
        Ok(thin) => match fallback() {
            Ok(pages) if density(&pages) > density(&thin) => Ok(PageText {
                pages,
                extractor: Extractor::Poppler,
            }),
            _ => Ok(PageText {
                pages: thin,
                extractor: Extractor::Native,
            }),
        },
        Err(reason) => match fallback() {
            Ok(pages) => Ok(PageText {
                pages,
                extractor: Extractor::Poppler,
            }),
            Err(fallback_reason) => Err(WisecrowError::PdfExtractionError(format!(
                "{reason}; {POPPLER_BINARY} could not read it either: {fallback_reason}"
            ))),
        },
    }
}

/// Reads the document in process, surviving a malformed one.
///
/// `pdf-extract` panics rather than returning an error on some content streams,
/// and a document of unknown provenance is exactly what this module is for, so a
/// panic is caught and reported as a failed reading.
fn native_pages(path: &Path) -> Result<Vec<String>, String> {
    match silenced(|| pdf_extract::extract_text_by_pages(path)) {
        Ok(Ok(pages)) => Ok(pages),
        Ok(Err(error)) => Err(format!("pdf-extract failed: {error}")),
        Err(()) => Err("pdf-extract panicked on a malformed document".to_owned()),
    }
}

/// Runs `work`, turning a panic on this thread into `Err(())` without the
/// panic being printed.
///
/// A caught panic is an expected reading failure here, and the default hook
/// would still write `thread 'main' panicked at …` to stderr before the catch,
/// which reads as a crash in a run that goes on to succeed through poppler.
/// The hook is global, so the replacement silences this thread only and hands
/// every other thread's panic to the hook that was there, then puts that hook
/// back.
fn silenced<T>(work: impl FnOnce() -> T + std::panic::UnwindSafe) -> Result<T, ()> {
    let previous = std::panic::take_hook();
    let quiet_thread = std::thread::current().id();
    let delegate = std::sync::Arc::new(previous);
    let hook_delegate = std::sync::Arc::clone(&delegate); // clone: the hook and the restore both own the previous hook
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() != quiet_thread {
            hook_delegate(info);
        }
    }));
    let outcome = std::panic::catch_unwind(work);
    let _ = std::panic::take_hook();
    match std::sync::Arc::try_unwrap(delegate) {
        Ok(previous) => std::panic::set_hook(previous),
        Err(shared) => std::panic::set_hook(Box::new(move |info| shared(info))),
    }
    outcome.map_err(|_| ())
}

/// Reads the document with poppler, keeping its column layout.
///
/// The layout is worth keeping: a paradigm table arrives as runs of spaces, which
/// is how [`crate::grammar::pdf`] recognises a table row and declines to read it
/// as a sentence.
fn poppler_pages(path: &Path) -> Result<Vec<String>, String> {
    let output = std::process::Command::new(POPPLER_BINARY)
        .arg("-layout")
        .arg(path)
        .arg("-")
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!("{POPPLER_BINARY} is not installed")
            } else {
                format!("{POPPLER_BINARY} could not be run: {error}")
            }
        })?;

    if !output.status.success() {
        return Err(format!(
            "{POPPLER_BINARY} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(split_pages(&String::from_utf8_lossy(&output.stdout)))
}

/// Splits poppler's output into pages.
///
/// A page break follows every page including the last, so the trailing empty
/// segment is dropped to keep page numbers honest.
fn split_pages(text: &str) -> Vec<String> {
    let mut pages: Vec<String> = text.split(PAGE_BREAK).map(str::to_owned).collect();
    if pages.last().is_some_and(String::is_empty) {
        pages.pop();
    }
    pages
}

fn density(pages: &[String]) -> usize {
    let characters = pages
        .iter()
        .map(|page| page.chars().count())
        .fold(0, usize::saturating_add);
    characters.checked_div(pages.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pages of the given density, as a reading of a document would hold.
    fn reading(pages: usize, chars_per_page: usize) -> Vec<String> {
        (0..pages).map(|_| "x".repeat(chars_per_page)).collect()
    }

    #[test]
    fn a_dense_native_reading_is_used_without_starting_poppler() {
        let chosen = prefer(Ok(reading(10, 2000)), || {
            panic!("poppler must not be asked for a document already read")
        })
        .expect("a reading");

        assert_eq!(chosen.extractor, Extractor::Native);
        assert_eq!(chosen.density(), 2000);
    }

    #[test]
    fn a_thin_native_reading_gives_way_to_a_denser_one() {
        let chosen = prefer(Ok(reading(10, 21)), || Ok(reading(10, 649))).expect("a reading");

        assert_eq!(chosen.extractor, Extractor::Poppler);
        assert_eq!(chosen.density(), 649);
    }

    #[test]
    fn a_thin_native_reading_is_kept_when_poppler_reads_no_more() {
        let chosen = prefer(Ok(reading(10, 90)), || Ok(reading(10, 40))).expect("a reading");

        assert_eq!(chosen.extractor, Extractor::Native);
        assert_eq!(chosen.density(), 90);
    }

    #[test]
    fn a_thin_native_reading_survives_a_machine_without_poppler() {
        let chosen = prefer(Ok(reading(10, 90)), || {
            Err("pdftotext is not installed".to_owned())
        })
        .expect("a reading");

        assert_eq!(chosen.extractor, Extractor::Native);
        assert_eq!(chosen.density(), 90);
    }

    #[test]
    fn a_silenced_panic_is_an_error_and_the_previous_hook_survives() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let seen = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&seen); // clone: the hook counts what the test reads
        let this_thread = std::thread::current().id();
        std::panic::set_hook(Box::new(move |_| {
            if std::thread::current().id() == this_thread {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        }));

        assert_eq!(silenced(|| 7), Ok(7));
        assert_eq!(silenced(|| -> u8 { panic!("malformed") }), Err(()));
        assert_eq!(
            seen.load(Ordering::SeqCst),
            0,
            "the silenced panic reached no hook"
        );

        let _ = std::panic::catch_unwind(|| panic!("after"));
        assert_eq!(seen.load(Ordering::SeqCst), 1, "the previous hook is back");
        let _ = std::panic::take_hook();
    }

    #[test]
    fn a_panic_in_the_native_extractor_is_answered_by_poppler() {
        let chosen = prefer(
            Err("pdf-extract panicked on a malformed document".to_owned()),
            || Ok(reading(226, 2228)),
        )
        .expect("a reading");

        assert_eq!(chosen.extractor, Extractor::Poppler);
        assert_eq!(chosen.pages.len(), 226);
    }

    #[test]
    fn a_document_neither_extractor_can_read_names_both_failures() {
        let refused = prefer(Err("pdf-extract panicked".to_owned()), || {
            Err("pdftotext is not installed".to_owned())
        })
        .expect_err("no reading");

        let message = refused.to_string();
        assert!(message.contains("pdf-extract panicked"), "{message}");
        assert!(message.contains("pdftotext is not installed"), "{message}");
    }

    #[test]
    fn poppler_output_splits_on_page_breaks_without_a_trailing_empty_page() {
        let pages = split_pages("first page\u{c}second page\u{c}");

        assert_eq!(pages, vec!["first page", "second page"]);
    }

    #[test]
    fn an_empty_reading_has_no_density() {
        assert_eq!(density(&[]), 0);
    }
}
