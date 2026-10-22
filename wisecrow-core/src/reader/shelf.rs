//! Reads the PDFs on the reader shelf, with `readers/SOURCES.md` as optional
//! notes for a file.
//!
//! The shelf is English text. The pair file names the languages that text is
//! translated into. A PDF whose folder names a mapped level is read. The
//! ledger can name the body's pages or a CEFR level, and a `Translate` cell
//! of `no` holds that
//! one file back. A blank cell does not. Answer keys, factsheets, tests,
//! worksheets and teacher's notes are not books.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::errors::WisecrowError;
use crate::grammar::syllabus::ALL_LEVELS;
use crate::markdown_table::{tables, Table};

/// Name of the ledger at the shelf's root.
pub const LEDGER_NAME: &str = "SOURCES.md";
const SERIES_HEADER: &str = "Series";
const LEVEL_HEADER: &str = "Level";
const TITLE_HEADER: &str = "Title";
const FILE_HEADER: &str = "File";
const CEFR_HEADER: &str = "CEFR";
const PAGES_HEADER: &str = "Pages";
const TRANSLATE_HEADER: &str = "Translate";

/// Publisher levels and the CEFR level each is read at, as
/// `(series, level cell, CEFR level)`.
pub const LEVEL_MAP: &[(&str, &str, &str)] = &[
    ("Penguin Readers", "0", "A1"),
    ("Penguin Readers", "1", "A1"),
    ("Penguin Readers", "0-1", "A1"),
    ("Penguin Readers", "2", "A2"),
    ("Penguin Readers", "3", "B1"),
    ("Penguin Readers", "4", "B1"),
    ("Penguin Readers", "5", "B2"),
    ("Penguin Readers", "6", "B2"),
    ("Penguin Readers", "5-6", "B2"),
];

/// The body of a book as the ledger names it, in printed page numbers
/// counted from one, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRange {
    pub first: u32,
    pub last: u32,
}

impl PageRange {
    /// The range as it enters a book's import identity.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{}-{}", self.first, self.last)
    }
}

/// One row of the ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReaderEntry {
    pub series: String,
    pub level: String,
    pub title: String,
    /// Path as the ledger writes it, relative to the ledger's directory.
    pub file: String,
    /// The CEFR level the book is read at, or why none can be named.
    pub cefr: Result<&'static str, String>,
    /// The body's pages when the row names them, or why the cell is unusable.
    pub pages: Result<Option<PageRange>, String>,
    /// The row's `Translate` cell reads `yes`. A blank cell is not `yes`.
    pub translate: bool,
    /// The row's `Translate` cell reads `no`, so the file is left unread.
    pub withheld: bool,
}

impl ReaderEntry {
    /// Whether this file is a book the job may store and send to a model.
    ///
    /// A mapped level is enough. `Translate` `no`, a support file, or a
    /// level that does not map to CEFR keeps it out.
    #[must_use]
    pub fn cleared(&self) -> bool {
        !self.withheld
            && !is_support_material(&self.file)
            && self.cefr.is_ok()
            && self.pages.is_ok()
    }

    /// The book's location on a shelf rooted at `shelf`.
    #[must_use]
    pub fn path(&self, shelf: &Path) -> PathBuf {
        shelf.join(&self.file)
    }
}

/// Reads the ledger at the root of `shelf`.
///
/// # Errors
///
/// Returns an error when the ledger cannot be read or is refused by
/// [`parse_ledger`].
pub fn read_ledger(shelf: &Path) -> Result<Vec<ReaderEntry>, WisecrowError> {
    let ledger = shelf.join(LEDGER_NAME);
    let markdown = std::fs::read_to_string(&ledger).map_err(|error| {
        WisecrowError::InvalidInput(format!("{} cannot be read: {error}", ledger.display()))
    })?;
    parse_ledger(&markdown)
}

/// The books on `shelf` the job may read, in path order.
///
/// A missing ledger leaves the PDFs themselves as the list. A ledger row
/// overlays the file it names. A row whose file is not on the shelf is not
/// a book.
///
/// # Errors
///
/// Returns an error when the shelf or a present ledger cannot be read.
pub fn shelf_entries(shelf: &Path) -> Result<Vec<ReaderEntry>, WisecrowError> {
    let mut overrides = HashMap::new();
    for entry in ledger_rows(shelf)? {
        let file = entry.file.clone(); // clone: the map key outlives the overlay entry
        overrides.insert(file, entry);
    }
    let mut found = Vec::new();
    for relative in pdf_paths(shelf)? {
        let entry = overrides
            .remove(&relative)
            .unwrap_or_else(|| entry_from_path(&relative));
        if entry.cleared() {
            found.push(entry);
        }
    }
    Ok(found)
}

fn ledger_rows(shelf: &Path) -> Result<Vec<ReaderEntry>, WisecrowError> {
    let ledger = shelf.join(LEDGER_NAME);
    if !ledger.exists() {
        return Ok(Vec::new());
    }
    read_ledger(shelf)
}

/// Reads every row of every table that has a `File` column.
///
/// # Errors
///
/// Returns [`WisecrowError::InvalidInput`] when no table has a `File` column.
pub fn parse_ledger(markdown: &str) -> Result<Vec<ReaderEntry>, WisecrowError> {
    let mut found = Vec::new();
    let mut any_table = false;
    for table in tables(markdown) {
        let Some(file) = table.column(FILE_HEADER) else {
            continue;
        };
        any_table = true;
        let columns = Columns {
            series: table.column(SERIES_HEADER),
            level: table.column(LEVEL_HEADER),
            title: table.column(TITLE_HEADER),
            file,
            cefr: table.column(CEFR_HEADER),
            pages: table.column(PAGES_HEADER),
            translate: table.column(TRANSLATE_HEADER),
        };
        found.extend(rows(&table, &columns));
    }
    if !any_table {
        return Err(WisecrowError::InvalidInput(format!(
            "{LEDGER_NAME} has no table with a File column"
        )));
    }
    Ok(found)
}

/// The CEFR level a publisher level maps to, if the map knows it.
#[must_use]
pub fn mapped_level(series: &str, level: &str) -> Option<&'static str> {
    LEVEL_MAP
        .iter()
        .find(|(known_series, known_level, _)| *known_series == series && *known_level == level)
        .map(|(_, _, cefr)| *cefr)
}

struct Columns {
    series: Option<usize>,
    level: Option<usize>,
    title: Option<usize>,
    file: usize,
    cefr: Option<usize>,
    pages: Option<usize>,
    translate: Option<usize>,
}

fn rows(table: &Table<'_>, columns: &Columns) -> Vec<ReaderEntry> {
    let cell = |row: &[&str], index: Option<usize>| -> String {
        index
            .and_then(|index| row.get(index))
            .map_or_else(String::new, |cell| (*cell).to_owned())
    };
    table
        .rows
        .iter()
        .filter_map(|row| {
            let file = row.get(columns.file)?.trim_matches('`').trim();
            if file.is_empty() {
                return None;
            }
            let series = cell(row, columns.series);
            let level = cell(row, columns.level);
            let title = match cell(row, columns.title) {
                title if title.is_empty() => file_stem(file),
                title => title,
            };
            let cefr = cefr_for(&series, &level, &cell(row, columns.cefr));
            let pages = parse_pages(&cell(row, columns.pages));
            let decision = columns
                .translate
                .and_then(|index| row.get(index))
                .copied()
                .unwrap_or("");
            Some(ReaderEntry {
                series,
                level,
                title,
                file: file.to_owned(),
                cefr,
                pages,
                translate: decision.eq_ignore_ascii_case("yes"),
                withheld: decision.eq_ignore_ascii_case("no"),
            })
        })
        .collect()
}

fn file_stem(file: &str) -> String {
    Path::new(file).file_stem().map_or_else(
        || file.to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}

/// Answer keys and the other pages filed beside a book are not the book.
fn is_support_material(file: &str) -> bool {
    let stem = file_stem(file).to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "answer key",
        "factsheet",
        "worksheet",
        "teacher's notes",
        "teachers' notes",
        "teacher notes",
    ];
    MARKERS.iter().any(|marker| stem.contains(marker))
        || stem.ends_with(" test")
        || stem.ends_with(" tests")
}

fn entry_from_path(relative: &str) -> ReaderEntry {
    let path = Path::new(relative);
    let mut parts = path.components();
    let series = component_text(parts.next());
    let level = level_cell(&component_text(parts.next()));
    ReaderEntry {
        cefr: cefr_for(&series, &level, ""),
        pages: Ok(None),
        translate: false,
        withheld: false,
        series,
        level,
        title: file_stem(relative),
        file: relative.to_owned(),
    }
}

fn component_text(component: Option<std::path::Component<'_>>) -> String {
    component.map_or_else(String::new, |component| {
        component.as_os_str().to_string_lossy().into_owned()
    })
}

fn level_cell(directory: &str) -> String {
    directory
        .strip_prefix("Level ")
        .or_else(|| directory.strip_prefix("level "))
        .unwrap_or(directory)
        .to_owned()
}

fn pdf_paths(shelf: &Path) -> Result<Vec<String>, WisecrowError> {
    let mut found = Vec::new();
    collect_pdfs(shelf, shelf, &mut found)?;
    found.sort();
    Ok(found)
}

fn collect_pdfs(shelf: &Path, dir: &Path, found: &mut Vec<String>) -> Result<(), WisecrowError> {
    let entries = std::fs::read_dir(dir).map_err(|error| {
        WisecrowError::InvalidInput(format!("{} cannot be read: {error}", dir.display()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            WisecrowError::InvalidInput(format!("{} cannot be read: {error}", dir.display()))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            WisecrowError::InvalidInput(format!("{} cannot be read: {error}", path.display()))
        })?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_pdfs(shelf, &path, found)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            return Err(WisecrowError::InvalidInput(format!(
                "{} is not valid UTF-8",
                path.display()
            )));
        };
        if !name.to_ascii_lowercase().ends_with(".pdf") {
            continue;
        }
        let relative = path.strip_prefix(shelf).map_err(|_| {
            WisecrowError::InvalidInput(format!("{} is outside the shelf", path.display()))
        })?;
        let relative = relative.to_str().ok_or_else(|| {
            WisecrowError::InvalidInput(format!("{} is not valid UTF-8", path.display()))
        })?;
        if relative
            .split('/')
            .any(|part| part.is_empty() || part == "..")
        {
            return Err(WisecrowError::InvalidInput(format!(
                "{relative} leaves the shelf"
            )));
        }
        found.push(relative.to_owned());
    }
    Ok(())
}

/// The CEFR cell when it is filled, otherwise the level map.
fn cefr_for(series: &str, level: &str, cell: &str) -> Result<&'static str, String> {
    let cell = cell.trim();
    if !cell.is_empty() && cell != "—" && cell != "-" {
        return ALL_LEVELS
            .iter()
            .copied()
            .find(|known| known.eq_ignore_ascii_case(cell))
            .ok_or_else(|| format!("its CEFR cell \"{cell}\" is not a CEFR level"));
    }
    mapped_level(series, level)
        .ok_or_else(|| format!("its level \"{level}\" maps to no CEFR level; fill the CEFR cell"))
}

/// Reads a `Pages` cell: empty, `6-33`, or `6–33` (en dash).
fn parse_pages(cell: &str) -> Result<Option<PageRange>, String> {
    let cell = cell.trim();
    if cell.is_empty() || cell == "—" || cell == "-" {
        return Ok(None);
    }
    let invalid = || format!("its Pages cell \"{cell}\" is not a range such as 6-33");
    let (first, last) = cell.split_once(['–', '-']).ok_or_else(invalid)?;
    let first: u32 = first.trim().parse().map_err(|_| invalid())?;
    let last: u32 = last.trim().parse().map_err(|_| invalid())?;
    if first == 0 || last < first {
        return Err(invalid());
    }
    Ok(Some(PageRange { first, last }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const LEDGER: &str = "\
# Readers

| Series | Level | Title | File | CEFR | Pages | Translate | Notes |
|--------|-------|-------|------|------|-------|-----------|-------|
| Penguin Readers | 2 | Treasure Island | `Penguin Readers/Level 2/Treasure Island.pdf` | | | yes | |
| Penguin Readers | 2 | Robin Hood | `Penguin Readers/Level 2/Robin Hood.pdf` | B1 | 4–41 | yes | |
| Penguin Readers | Unlevelled | Dante's Peak | `Penguin Readers/Unlevelled/Dante's Peak.pdf` | | | yes | |
| Penguin Readers | 3 | Hamlet | `Penguin Readers/Level 3/Hamlet.pdf` | | | | not yet |
| Penguin Readers | 4 | Odd | `Penguin Readers/Level 4/Odd.pdf` | A3 | 9-2 | YES | |
";

    #[test]
    fn every_row_is_read_with_its_cells() {
        let entries = parse_ledger(LEDGER).expect("a valid ledger");

        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].series, "Penguin Readers");
        assert_eq!(entries[0].level, "2");
        assert_eq!(entries[0].title, "Treasure Island");
        assert_eq!(
            entries[0].file,
            "Penguin Readers/Level 2/Treasure Island.pdf"
        );
        assert!(entries[0].translate);
        assert!(!entries[3].translate, "a blank cell is not the word yes");
        assert!(
            !entries[3].withheld,
            "a blank cell does not hold the book back"
        );
        assert!(entries[4].translate, "yes is read without regard to case");
    }

    #[test]
    fn the_level_map_names_a_level_and_the_cefr_cell_overrides_it() {
        let entries = parse_ledger(LEDGER).expect("a valid ledger");

        assert_eq!(entries[0].cefr, Ok("A2"));
        assert_eq!(entries[1].cefr, Ok("B1"), "the row's CEFR cell wins");
        assert!(
            entries[2].cefr.is_err(),
            "an unlevelled row must state a level"
        );
        assert_eq!(
            entries[4].cefr,
            Err("its CEFR cell \"A3\" is not a CEFR level".to_owned())
        );
    }

    #[rstest]
    #[case("Penguin Readers", "0", Some("A1"))]
    #[case("Penguin Readers", "1", Some("A1"))]
    #[case("Penguin Readers", "0-1", Some("A1"))]
    #[case("Penguin Readers", "2", Some("A2"))]
    #[case("Penguin Readers", "3", Some("B1"))]
    #[case("Penguin Readers", "4", Some("B1"))]
    #[case("Penguin Readers", "5", Some("B2"))]
    #[case("Penguin Readers", "6", Some("B2"))]
    #[case("Penguin Readers", "5-6", Some("B2"))]
    #[case("Penguin Readers", "Unlevelled", None)]
    #[case("Black Cat Levels", "2", None)]
    fn the_publisher_level_map(
        #[case] series: &str,
        #[case] level: &str,
        #[case] expected: Option<&'static str>,
    ) {
        assert_eq!(mapped_level(series, level), expected);
    }

    #[test]
    fn a_pages_cell_is_a_range_or_nothing() {
        let entries = parse_ledger(LEDGER).expect("a valid ledger");

        assert_eq!(entries[0].pages, Ok(None));
        assert_eq!(entries[1].pages, Ok(Some(PageRange { first: 4, last: 41 })));
        assert!(entries[4].pages.is_err(), "a range must run upward");
        assert_eq!(
            parse_pages("6-33"),
            Ok(Some(PageRange { first: 6, last: 33 }))
        );
        assert_eq!(parse_pages(" — "), Ok(None));
        assert!(parse_pages("0-3").is_err(), "pages count from one");
        assert!(parse_pages("six").is_err());
        assert_eq!(PageRange { first: 6, last: 33 }.label(), "6-33");
    }

    #[test]
    fn a_mapped_level_is_read_unless_a_cell_is_unusable() {
        let entries = parse_ledger(LEDGER).expect("a valid ledger");
        let cleared: Vec<&str> = entries
            .iter()
            .filter(|entry| entry.cleared())
            .map(|entry| entry.title.as_str())
            .collect();

        assert_eq!(cleared, ["Treasure Island", "Robin Hood", "Hamlet"]);
    }

    #[test]
    fn a_table_without_a_translate_column_still_lists_the_file() {
        let entries = parse_ledger("| File | Notes |\n|---|---|\n| `a.pdf` | x |\n")
            .expect("a missing Translate column does not refuse the shelf");

        assert_eq!(entries.len(), 1);
        assert!(!entries[0].translate);
        assert!(!entries[0].withheld);
    }

    #[test]
    fn a_ledger_without_a_file_column_is_refused() {
        let refused = parse_ledger("# Readers\n\nNo table yet.\n").expect_err("nothing to read");

        assert!(
            refused.to_string().contains("no table with a File column"),
            "{refused}"
        );
    }

    #[test]
    fn a_row_without_a_file_is_passed_over() {
        let entries =
            parse_ledger("| File | Translate |\n|---|---|\n|  | yes |\n| `a.pdf` | yes |\n")
                .expect("a valid ledger");

        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].title, "a",
            "the title falls back to the file's stem"
        );
    }

    #[test]
    fn the_ledger_is_read_from_the_shelf_root() {
        let shelf = tempfile::tempdir().expect("temporary directory");
        std::fs::write(shelf.path().join(LEDGER_NAME), LEDGER).expect("write ledger");

        let entries = read_ledger(shelf.path()).expect("a readable ledger");

        assert_eq!(entries.len(), 5);
        assert_eq!(
            entries[0].path(shelf.path()),
            shelf
                .path()
                .join("Penguin Readers/Level 2/Treasure Island.pdf")
        );
    }

    #[test]
    fn a_missing_ledger_is_an_error() {
        let shelf = tempfile::tempdir().expect("temporary directory");

        assert!(read_ledger(shelf.path()).is_err());
    }

    #[test]
    fn the_seeded_ledger_lists_eighty_five_books_and_reads_the_levelled_ones() {
        let seeded = include_str!("../../../readers/SOURCES.md");

        let entries = parse_ledger(seeded).expect("the seeded ledger parses");

        assert_eq!(entries.len(), 85);
        assert!(entries.iter().all(|entry| !entry.translate));
        assert!(entries.iter().all(|entry| !entry.withheld));
        assert!(entries
            .iter()
            .all(|entry| entry.series == "Penguin Readers"));
        let unlevelled = entries.iter().filter(|entry| entry.cefr.is_err()).count();
        assert_eq!(
            unlevelled, 7,
            "an unlevelled folder does not name a CEFR level"
        );
        assert_eq!(entries.iter().filter(|entry| entry.cleared()).count(), 78);
    }

    #[test]
    fn a_pdf_on_the_shelf_is_read_without_a_ledger_yes() {
        let shelf = tempfile::tempdir().expect("temporary directory");
        let level = shelf.path().join("Penguin Readers/Level 2");
        std::fs::create_dir_all(&level).expect("level directory");
        std::fs::write(level.join("Treasure Island.pdf"), b"%PDF-1.4").expect("book");
        std::fs::write(level.join("Lost Love Answer Keys.pdf"), b"%PDF-1.4").expect("keys");
        std::fs::write(level.join("Held.pdf"), b"%PDF-1.4").expect("held book");
        std::fs::write(
            shelf.path().join(LEDGER_NAME),
            "\
| Series | Level | Title | File | CEFR | Pages | Translate | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Penguin Readers | 2 | Held back | `Penguin Readers/Level 2/Held.pdf` | | | no | |
",
        )
        .expect("ledger");

        let entries = shelf_entries(shelf.path()).expect("shelf");
        let titles: Vec<&str> = entries.iter().map(|entry| entry.title.as_str()).collect();

        assert_eq!(titles, ["Treasure Island"]);
    }

    #[test]
    fn a_shelf_without_a_ledger_still_lists_its_pdfs() {
        let shelf = tempfile::tempdir().expect("temporary directory");
        let level = shelf.path().join("Penguin Readers/Level 0-1");
        std::fs::create_dir_all(&level).expect("level directory");
        std::fs::write(level.join("Brown Eyes.pdf"), b"%PDF-1.4").expect("book");

        let entries = shelf_entries(shelf.path()).expect("shelf");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "Brown Eyes");
        assert_eq!(entries[0].cefr, Ok("A1"));
        assert!(read_ledger(shelf.path()).is_err());
    }
}
