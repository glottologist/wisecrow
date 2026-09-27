//! Reads the shelf's provenance record to decide whether a document may be
//! sent to a model.
//!
//! `grammar/SOURCES.md` records what every document on the shelf is and under
//! what terms it is held. Two of them may not be reproduced to a third party at
//! all, and sending a document's text to a hosted model is exactly that. The
//! record is the authority rather than the operator's memory: a document is
//! cleared for synthesis only when its row in the record says so, and a
//! document with no row is refused, since nothing is known about it.

use std::path::{Path, PathBuf};

/// Name of the record, looked for in the document's directory and above it.
const RECORD_NAME: &str = "SOURCES.md";
const FILE_HEADER: &str = "File";
const SYNTHESIS_HEADER: &str = "Synthesis";
const LICENCE_HEADER: &str = "Licence";

/// Whether a document may be read to a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clearance {
    Permitted,
    /// The document may not be sent, for the reason given.
    Refused(String),
}

/// One row of the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEntry {
    /// Path as the record writes it, relative to the record's directory.
    pub path: String,
    pub synthesis: bool,
    /// The licence column, when the table has one.
    pub licence: Option<String>,
}

/// Looks the document up in the nearest record above it.
///
/// The record is searched for in the document's own directory and then each
/// directory above, so `grammar/es/yo-puedo-1-2021.pdf` is found in
/// `grammar/SOURCES.md` as `es/yo-puedo-1-2021.pdf`.
#[must_use]
pub fn clearance(document: &Path) -> Clearance {
    let Some((record, relative)) = find_record(document) else {
        return Clearance::Refused(format!(
            "{} is not recorded in any {RECORD_NAME}, so nothing is known about its licence",
            document.display()
        ));
    };
    let markdown = match std::fs::read_to_string(&record) {
        Ok(markdown) => markdown,
        Err(error) => {
            return Clearance::Refused(format!("{} cannot be read: {error}", record.display()))
        }
    };

    match entries(&markdown)
        .into_iter()
        .find(|entry| entry.path == relative)
    {
        Some(entry) if entry.synthesis => Clearance::Permitted,
        Some(entry) => Clearance::Refused(match entry.licence {
            Some(licence) => format!(
                "{relative} may not be sent to a hosted model ({}: {licence})",
                record.display()
            ),
            None => format!(
                "{relative} is not cleared for synthesis in {}",
                record.display()
            ),
        }),
        None => Clearance::Refused(format!(
            "{relative} has no row in {}, so nothing is known about its licence",
            record.display()
        )),
    }
}

/// Finds the record governing a document, and the document's path within it.
fn find_record(document: &Path) -> Option<(PathBuf, String)> {
    document
        .parent()?
        .ancestors()
        .map(|directory| directory.join(RECORD_NAME))
        .find(|record| record.is_file())
        .and_then(|record| {
            let base = record.parent()?;
            let relative = document.strip_prefix(base).ok()?;
            Some((record.clone(), relative.to_string_lossy().into_owned()))
        })
}

/// Reads every row of every table in the record that has a `File` column.
///
/// A row is cleared when its `Synthesis` cell says `yes`; a table without that
/// column clears nothing, so the omission fails safe.
#[must_use]
pub fn entries(markdown: &str) -> Vec<SourceEntry> {
    let mut found = Vec::new();
    let mut columns: Option<Columns> = None;

    for line in markdown.lines() {
        let Some(cells) = table_cells(line) else {
            columns = None;
            continue;
        };
        if is_separator(&cells) {
            continue;
        }
        match columns {
            None => columns = Columns::from_header(&cells),
            Some(ref layout) => {
                if let Some(entry) = layout.entry(&cells) {
                    found.push(entry);
                }
            }
        }
    }

    found
}

/// Where the columns that matter sit in one table.
#[derive(Debug, Clone, Copy)]
struct Columns {
    file: usize,
    synthesis: Option<usize>,
    licence: Option<usize>,
}

impl Columns {
    fn from_header(cells: &[&str]) -> Option<Self> {
        let position = |name: &str| cells.iter().position(|cell| *cell == name);
        Some(Self {
            file: position(FILE_HEADER)?,
            synthesis: position(SYNTHESIS_HEADER),
            licence: position(LICENCE_HEADER),
        })
    }

    fn entry(&self, cells: &[&str]) -> Option<SourceEntry> {
        let path = cells.get(self.file)?.trim_matches('`').trim();
        if path.is_empty() {
            return None;
        }
        let synthesis = self
            .synthesis
            .and_then(|index| cells.get(index))
            .is_some_and(|cell| cell.eq_ignore_ascii_case("yes"));
        let licence = self
            .licence
            .and_then(|index| cells.get(index))
            .map(|cell| (*cell).to_owned());
        Some(SourceEntry {
            path: path.to_owned(),
            synthesis,
            licence,
        })
    }
}

/// The cells of a table line, or `None` for a line that is not one.
fn table_cells(line: &str) -> Option<Vec<&str>> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('|')?;
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    Some(inner.split('|').map(str::trim).collect())
}

fn is_separator(cells: &[&str]) -> bool {
    cells
        .iter()
        .all(|cell| !cell.is_empty() && cell.chars().all(|c| matches!(c, '-' | ':')))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORD: &str = "\
# Grammar source documents

| File | Work | Year | Licence | Synthesis |
|------|------|-----:|---------|-----------|
| `gd/calder-1923.pdf` | Calder, *A Gaelic Grammar* | 1923 | public domain | yes |
| `ga/caighdean-oifigiuil-2017.pdf` | *An Caighdeán Oifigiúil* | 2017 | no licence stated | no |

Prose between the tables.

## Personal copies

| File | Work | Pages | Read by | Synthesis |
|------|------|------:|---------|-----------|
| `fr/Schaum's Outline of French Grammar.pdf` | Crocker, *Schaum's Outline* | 398 | pdf-extract | yes |

## A table that clears nothing

| File | Note |
|------|------|
| `cy/evans-1910.pdf` | no synthesis column |
";

    fn shelf() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temporary directory");
        std::fs::write(root.path().join(RECORD_NAME), RECORD).expect("write record");
        for path in [
            "gd/calder-1923.pdf",
            "ga/caighdean-oifigiuil-2017.pdf",
            "fr/Schaum's Outline of French Grammar.pdf",
            "cy/evans-1910.pdf",
            "es/unrecorded.pdf",
        ] {
            let full = root.path().join(path);
            std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
            std::fs::write(full, b"%PDF-1.7").expect("write document");
        }
        root
    }

    #[test]
    fn every_table_with_a_file_column_is_read() {
        let found = entries(RECORD);
        let paths: Vec<&str> = found.iter().map(|entry| entry.path.as_str()).collect();

        assert_eq!(
            paths,
            [
                "gd/calder-1923.pdf",
                "ga/caighdean-oifigiuil-2017.pdf",
                "fr/Schaum's Outline of French Grammar.pdf",
                "cy/evans-1910.pdf",
            ]
        );
    }

    #[test]
    fn a_row_is_cleared_only_by_a_yes_in_the_synthesis_column() {
        let found = entries(RECORD);

        assert!(found[0].synthesis, "public domain, cleared");
        assert!(!found[1].synthesis, "no licence stated, refused");
        assert!(found[2].synthesis, "personal copy, cleared");
        assert!(!found[3].synthesis, "no synthesis column, refused");
    }

    #[test]
    fn the_licence_column_is_carried_when_the_table_has_one() {
        let found = entries(RECORD);

        assert_eq!(found[1].licence.as_deref(), Some("no licence stated"));
        assert_eq!(found[2].licence, None);
    }

    #[test]
    fn a_cleared_document_is_permitted() {
        let root = shelf();

        assert_eq!(
            clearance(&root.path().join("gd/calder-1923.pdf")),
            Clearance::Permitted
        );
        assert_eq!(
            clearance(
                &root
                    .path()
                    .join("fr/Schaum's Outline of French Grammar.pdf")
            ),
            Clearance::Permitted
        );
    }

    #[test]
    fn a_restricted_document_is_refused_with_its_licence() {
        let root = shelf();

        let Clearance::Refused(reason) =
            clearance(&root.path().join("ga/caighdean-oifigiuil-2017.pdf"))
        else {
            panic!("the standard must be refused");
        };
        assert!(reason.contains("no licence stated"), "{reason}");
        assert!(
            reason.contains("ga/caighdean-oifigiuil-2017.pdf"),
            "{reason}"
        );
    }

    #[test]
    fn a_document_without_a_row_is_refused() {
        let root = shelf();

        let Clearance::Refused(reason) = clearance(&root.path().join("es/unrecorded.pdf")) else {
            panic!("an unrecorded document must be refused");
        };
        assert!(reason.contains("has no row"), "{reason}");
    }

    #[test]
    fn a_document_with_no_record_above_it_is_refused() {
        let root = tempfile::tempdir().expect("temporary directory");
        let loose = root.path().join("loose.pdf");
        std::fs::write(&loose, b"%PDF-1.7").expect("write document");

        let Clearance::Refused(reason) = clearance(&loose) else {
            panic!("a loose document must be refused");
        };
        assert!(
            reason.contains("not recorded in any SOURCES.md"),
            "{reason}"
        );
    }

    #[test]
    fn a_line_that_is_not_a_table_row_ends_the_table() {
        let found = entries(
            "| File | Synthesis |\n|---|---|\n| `a.pdf` | yes |\nprose\n| `b.pdf` | yes |\n",
        );

        assert_eq!(
            found.len(),
            1,
            "a row after prose has no header to read it by"
        );
    }
}
