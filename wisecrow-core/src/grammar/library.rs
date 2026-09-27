//! Finds the grammar documents a path stands for, and reads their language.
//!
//! The source documents are kept as `grammar/<language code>/<document>.pdf`, one
//! directory per language. Two things follow from that layout, and this module
//! provides both: a path may name a single document, a language's directory or
//! the whole shelf, and the language of a document need not be repeated on the
//! command line when its directory already states it.

use std::path::{Path, PathBuf};

use crate::cli::SUPPORTED_LANGUAGE_INFO;
use crate::errors::WisecrowError;

/// Collects the PDFs that `path` stands for, in a stable order.
///
/// A file stands for itself, whatever its extension, since a path given
/// explicitly is a deliberate choice. A directory stands for every PDF beneath
/// it, sorted so that two runs report in the same order.
///
/// # Errors
///
/// Returns an error when the path cannot be read, or when a directory holds no
/// PDF at all -- almost always a mistyped language code.
pub fn documents(path: &Path) -> Result<Vec<PathBuf>, WisecrowError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        WisecrowError::InvalidInput(format!("{} cannot be read: {error}", path.display()))
    })?;

    if !metadata.is_dir() {
        return Ok(vec![path.to_path_buf()]);
    }

    let mut found = Vec::new();
    collect_pdfs(path, &mut found)?;
    found.sort();

    if found.is_empty() {
        return Err(WisecrowError::InvalidInput(format!(
            "{} holds no PDF",
            path.display()
        )));
    }
    Ok(found)
}

/// Reads the language code from a document's path.
///
/// The code is the deepest directory component naming a supported language, so
/// `grammar/gd/calder-1923.pdf` is Scottish Gaelic however the shelf is nested
/// beneath. A document sitting outside any such directory yields `None`, and its
/// language then has to be given on the command line.
#[must_use]
pub fn language_from_path(path: &Path) -> Option<&'static str> {
    path.parent()?
        .ancestors()
        .filter_map(Path::file_name)
        .filter_map(std::ffi::OsStr::to_str)
        .find_map(supported_code)
}

/// Walks a directory tree, gathering PDFs.
///
/// Symbolic links to directories are passed over rather than followed, which
/// keeps a link back up the tree from turning the walk into a loop. A link to a
/// file is still collected, since that is a document someone deliberately placed.
fn collect_pdfs(directory: &Path, found: &mut Vec<PathBuf>) -> Result<(), WisecrowError> {
    let entries = std::fs::read_dir(directory).map_err(|error| {
        WisecrowError::InvalidInput(format!("{} cannot be listed: {error}", directory.display()))
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            WisecrowError::InvalidInput(format!(
                "{} cannot be listed: {error}",
                directory.display()
            ))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            WisecrowError::InvalidInput(format!(
                "{} cannot be inspected: {error}",
                entry.path().display()
            ))
        })?;
        let path = entry.path();

        if file_type.is_dir() {
            collect_pdfs(&path, found)?;
        } else if is_pdf(&path) && path.is_file() {
            found.push(path);
        }
    }
    Ok(())
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn supported_code(name: &str) -> Option<&'static str> {
    SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(code, _)| *code == name)
        .map(|(code, _)| *code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Builds a shelf of empty files under a temporary root.
    fn shelf(paths: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temporary directory");
        for path in paths {
            let full = root.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("create language directory");
            }
            std::fs::write(&full, b"%PDF-1.7").expect("write document");
        }
        root
    }

    #[test]
    fn a_directory_stands_for_every_pdf_beneath_it() {
        let root = shelf(&[
            "gd/calder-1923.pdf",
            "gd/goc-2009.pdf",
            "ga/caighdean-oifigiuil-2017.pdf",
            "SOURCES.md",
        ]);

        let found = documents(root.path()).expect("documents");

        let names: Vec<String> = found
            .iter()
            .map(|path| {
                path.strip_prefix(root.path())
                    .expect("inside the shelf")
                    .display()
                    .to_string()
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "ga/caighdean-oifigiuil-2017.pdf",
                "gd/calder-1923.pdf",
                "gd/goc-2009.pdf"
            ]
        );
    }

    #[test]
    fn a_file_stands_for_itself() {
        let root = shelf(&["gd/calder-1923.pdf"]);
        let document = root.path().join("gd/calder-1923.pdf");

        assert_eq!(documents(&document).expect("documents"), vec![document]);
    }

    #[test]
    fn an_upper_case_extension_is_still_a_pdf() {
        let root = shelf(&["es/coester-1912.PDF"]);
        assert_eq!(documents(root.path()).expect("documents").len(), 1);
    }

    #[test]
    fn a_directory_without_documents_is_an_error() {
        let root = shelf(&["SOURCES.md"]);
        assert!(documents(root.path()).is_err());
    }

    #[test]
    fn a_missing_path_is_an_error() {
        assert!(documents(Path::new("grammar/no-such-language")).is_err());
    }

    #[test]
    fn a_link_that_points_back_up_the_shelf_does_not_loop() {
        let root = shelf(&["gd/calder-1923.pdf"]);
        std::os::unix::fs::symlink(root.path(), root.path().join("gd/shelf"))
            .expect("link back to the root");

        assert_eq!(documents(root.path()).expect("documents").len(), 1);
    }

    #[rstest]
    #[case("grammar/gd/calder-1923.pdf", Some("gd"))]
    #[case("grammar/ga/caighdean-oifigiuil-2017.pdf", Some("ga"))]
    #[case("/home/someone/grammar/es/sub/olmsted-1920.pdf", Some("es"))]
    #[case("grammar/daccordo.pdf", None)]
    #[case("gd", None)]
    fn language_read_from_the_path(#[case] path: &str, #[case] expected: Option<&str>) {
        assert_eq!(language_from_path(Path::new(path)), expected);
    }

    #[test]
    fn the_deepest_language_directory_wins() {
        assert_eq!(
            language_from_path(Path::new("grammar/es/fr/bevier-1896.pdf")),
            Some("fr")
        );
    }
}
