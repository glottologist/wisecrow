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

use super::syllabus::ALL_LEVELS;

/// Name of the record, looked for in the document's directory and above it.
const RECORD_NAME: &str = "SOURCES.md";
const FILE_HEADER: &str = "File";
const SYNTHESIS_HEADER: &str = "Synthesis";
const LICENCE_HEADER: &str = "Licence";
const LEVELS_HEADER: &str = "Levels";

/// Whether a document may be read to a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clearance {
    Permitted,
    /// The document may not be sent, for the reason given.
    Refused(String),
}

/// The CEFR levels a row says its document is worth reading at.
///
/// A textbook for beginners has nothing to say about C1, and a historical
/// grammar nothing a learner at A1 should be taught; the record states which
/// levels apply so that a language run reads each book where it belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LevelCoverage {
    /// The levels named, in syllabus order.
    Stated(Vec<&'static str>),
    /// An empty cell, or a table without the column.
    Unstated,
    /// A cell that names something the syllabus does not have, kept so the
    /// skip can quote it.
    Invalid(String),
}

/// What the record says about one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup {
    pub clearance: Clearance,
    pub levels: LevelCoverage,
}

/// One row of the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEntry {
    /// Path as the record writes it, relative to the record's directory.
    pub path: String,
    pub synthesis: bool,
    /// The licence column, when the table has one.
    pub licence: Option<String>,
    pub levels: LevelCoverage,
}

/// Whether the document may be read to a model.
#[must_use]
pub fn clearance(document: &Path) -> Clearance {
    lookup(document).clearance
}

/// Looks the document up in the nearest record above it.
///
/// The record is searched for in the document's own directory and then each
/// directory above, so `grammar/es/yo-puedo-1-2021.pdf` is found in
/// `grammar/SOURCES.md` as `es/yo-puedo-1-2021.pdf`. A document the record
/// does not know is refused with nothing stated about its levels.
#[must_use]
pub fn lookup(document: &Path) -> Lookup {
    let Some((record, relative)) = find_record(document) else {
        return Lookup {
            clearance: Clearance::Refused(format!(
                "{} is not recorded in any {RECORD_NAME}, so nothing is known about its licence",
                document.display()
            )),
            levels: LevelCoverage::Unstated,
        };
    };
    let markdown = match std::fs::read_to_string(&record) {
        Ok(markdown) => markdown,
        Err(error) => {
            return Lookup {
                clearance: Clearance::Refused(format!(
                    "{} cannot be read: {error}",
                    record.display()
                )),
                levels: LevelCoverage::Unstated,
            }
        }
    };

    match entries(&markdown)
        .into_iter()
        .find(|entry| entry.path == relative)
    {
        Some(entry) if entry.synthesis => Lookup {
            clearance: Clearance::Permitted,
            levels: entry.levels,
        },
        Some(entry) => Lookup {
            clearance: Clearance::Refused(match entry.licence {
                Some(licence) => format!(
                    "{relative} may not be sent to a hosted model ({}: {licence})",
                    record.display()
                ),
                None => format!(
                    "{relative} is not cleared for synthesis in {}",
                    record.display()
                ),
            }),
            levels: entry.levels,
        },
        None => Lookup {
            clearance: Clearance::Refused(format!(
                "{relative} has no row in {}, so nothing is known about its licence",
                record.display()
            )),
            levels: LevelCoverage::Unstated,
        },
    }
}

/// The levels a document is read at, given its row and the `--level` flag.
///
/// The flag mirrors `--lang`: absent, the row decides; given, it must be one
/// the row states, or the row must state nothing and leave it to the
/// operator. The error is the reason a run skips the document.
///
/// # Errors
///
/// Returns the skip reason when the row and the flag between them name no
/// level to read.
pub fn levels_to_import(
    coverage: &LevelCoverage,
    requested: Option<&str>,
) -> Result<Vec<&'static str>, String> {
    let requested = match requested {
        None => None,
        Some(code) => {
            Some(canonical_level(code).ok_or_else(|| format!("{code} is not a CEFR level"))?)
        }
    };
    match (coverage, requested) {
        (LevelCoverage::Invalid(cell), _) => Err(format!(
            "its Levels cell \"{cell}\" names no CEFR level the syllabus has"
        )),
        (LevelCoverage::Stated(levels), None) => Ok(levels.clone()), // clone: the caller owns its schedule
        (LevelCoverage::Stated(levels), Some(level)) => {
            if levels.contains(&level) {
                Ok(vec![level])
            } else {
                Err(format!("its row covers {}, not {level}", levels.join(", ")))
            }
        }
        (LevelCoverage::Unstated, None) => {
            Err("its row states no levels; pass --level to read it at one".to_owned())
        }
        (LevelCoverage::Unstated, Some(level)) => Ok(vec![level]),
    }
}

/// The calls a run will make, and the documents it will not read.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportSchedule<'a> {
    /// `(level, document)` pairs: every document covering A1 before any at
    /// A2, in shelf order within a level, so the lowest covering level has
    /// the first claim on a slug.
    pub calls: Vec<(&'static str, &'a Path)>,
    /// Documents left out, each with the reason to log.
    pub skipped: Vec<(&'a Path, String)>,
}

/// Plans a run over cleared documents and their coverage.
///
/// # Errors
///
/// Returns an error when `requested` is not a CEFR level; that is a mistyped
/// flag, not a document to skip.
pub fn import_schedule<'a>(
    documents: &'a [(PathBuf, LevelCoverage)],
    requested: Option<&str>,
) -> Result<ImportSchedule<'a>, crate::errors::WisecrowError> {
    if let Some(code) = requested {
        canonical_level(code).ok_or_else(|| {
            crate::errors::WisecrowError::InvalidInput(format!(
                "{code} is not a CEFR level; expected one of {}",
                ALL_LEVELS.join(", ")
            ))
        })?;
    }
    let mut per_document = Vec::with_capacity(documents.len());
    let mut schedule = ImportSchedule::default();
    for (document, coverage) in documents {
        match levels_to_import(coverage, requested) {
            Ok(levels) => per_document.push((document.as_path(), levels)),
            Err(reason) => schedule.skipped.push((document.as_path(), reason)),
        }
    }
    for level in ALL_LEVELS {
        for (document, levels) in &per_document {
            if levels.contains(&level) {
                schedule.calls.push((level, document));
            }
        }
    }
    Ok(schedule)
}

/// The syllabus's own spelling of a level code, if it has one.
fn canonical_level(code: &str) -> Option<&'static str> {
    ALL_LEVELS
        .iter()
        .copied()
        .find(|level| level.eq_ignore_ascii_case(code.trim()))
}

/// Reads a `Levels` cell: `all`, a range such as `A2–B1` (en dash or
/// hyphen), a comma-separated list, or nothing. A range runs upward; the
/// result is in syllabus order whatever order the cell used.
fn parse_levels(cell: &str) -> LevelCoverage {
    let cell = cell.trim();
    if cell.is_empty() || cell == "—" || cell == "-" {
        return LevelCoverage::Unstated;
    }
    if cell.eq_ignore_ascii_case("all") {
        return LevelCoverage::Stated(ALL_LEVELS.to_vec());
    }
    let invalid = || LevelCoverage::Invalid(cell.to_owned());
    let position = |code: &str| {
        ALL_LEVELS
            .iter()
            .position(|level| level.eq_ignore_ascii_case(code.trim()))
    };

    let mut wanted = [false; ALL_LEVELS.len()];
    for part in cell.split(',') {
        let bounds: Vec<&str> = part.split(['–', '-']).collect();
        match bounds.as_slice() {
            [single] => match position(single) {
                Some(index) => wanted[index] = true,
                None => return invalid(),
            },
            [low, high] => match (position(low), position(high)) {
                (Some(low), Some(high)) if low <= high => {
                    for slot in &mut wanted[low..=high] {
                        *slot = true;
                    }
                }
                _ => return invalid(),
            },
            _ => return invalid(),
        }
    }
    LevelCoverage::Stated(
        ALL_LEVELS
            .iter()
            .zip(wanted)
            .filter(|(_, wanted)| *wanted)
            .map(|(level, _)| *level)
            .collect(),
    )
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
/// A row is cleared when its `Synthesis` cell says `yes`, and covers the
/// levels its `Levels` cell names; a table without either column clears
/// nothing and states no level, so the omission fails safe.
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
    levels: Option<usize>,
}

impl Columns {
    fn from_header(cells: &[&str]) -> Option<Self> {
        let position = |name: &str| cells.iter().position(|cell| *cell == name);
        Some(Self {
            file: position(FILE_HEADER)?,
            synthesis: position(SYNTHESIS_HEADER),
            licence: position(LICENCE_HEADER),
            levels: position(LEVELS_HEADER),
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
        let levels = self
            .levels
            .and_then(|index| cells.get(index))
            .map_or(LevelCoverage::Unstated, |cell| parse_levels(cell));
        Some(SourceEntry {
            path: path.to_owned(),
            synthesis,
            licence,
            levels,
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

| File | Work | Year | Licence | Synthesis | Levels |
|------|------|-----:|---------|-----------|--------|
| `gd/calder-1923.pdf` | Calder, *A Gaelic Grammar* | 1923 | public domain | yes | B1–C2 |
| `ga/caighdean-oifigiuil-2017.pdf` | *An Caighdeán Oifigiúil* | 2017 | no licence stated | no | C1-C2 |

Prose between the tables.

## Personal copies

| File | Work | Pages | Read by | Synthesis | Levels |
|------|------|------:|---------|-----------|--------|
| `fr/Schaum's Outline of French Grammar.pdf` | Crocker, *Schaum's Outline* | 398 | pdf-extract | yes | all |
| `fr/verbs.pdf` | *501 French Verbs* | 700 | poppler | yes | |
| `fr/odd.pdf` | *Odd* | 10 | poppler | yes | A3 |

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
            "fr/verbs.pdf",
            "fr/odd.pdf",
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
                "fr/verbs.pdf",
                "fr/odd.pdf",
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
        assert!(!found[5].synthesis, "no synthesis column, refused");
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
    fn a_levels_cell_is_a_range_a_list_all_or_nothing() {
        assert_eq!(
            parse_levels("all"),
            LevelCoverage::Stated(ALL_LEVELS.to_vec())
        );
        assert_eq!(
            parse_levels("A2–B1"),
            LevelCoverage::Stated(vec!["A2", "B1"])
        );
        assert_eq!(
            parse_levels("A2-B1"),
            LevelCoverage::Stated(vec!["A2", "B1"])
        );
        assert_eq!(
            parse_levels("A1, C2"),
            LevelCoverage::Stated(vec!["A1", "C2"])
        );
        assert_eq!(parse_levels("b1"), LevelCoverage::Stated(vec!["B1"]));
        assert_eq!(
            parse_levels("C2, A1"),
            LevelCoverage::Stated(vec!["A1", "C2"]),
            "syllabus order"
        );
        assert_eq!(parse_levels(""), LevelCoverage::Unstated);
        assert_eq!(parse_levels("—"), LevelCoverage::Unstated);
        assert_eq!(parse_levels("A3"), LevelCoverage::Invalid("A3".to_owned()));
        assert_eq!(
            parse_levels("B1–A1"),
            LevelCoverage::Invalid("B1–A1".to_owned())
        );
    }

    #[test]
    fn every_row_carries_its_coverage() {
        let found = entries(RECORD);

        assert_eq!(
            found[0].levels,
            LevelCoverage::Stated(vec!["B1", "B2", "C1", "C2"])
        );
        assert_eq!(found[1].levels, LevelCoverage::Stated(vec!["C1", "C2"]));
        assert_eq!(found[2].levels, LevelCoverage::Stated(ALL_LEVELS.to_vec()));
        assert_eq!(found[3].levels, LevelCoverage::Unstated, "an empty cell");
        assert_eq!(found[4].levels, LevelCoverage::Invalid("A3".to_owned()));
        assert_eq!(found[5].levels, LevelCoverage::Unstated, "no column at all");
    }

    #[test]
    fn a_lookup_carries_clearance_and_coverage_together() {
        let root = shelf();

        let calder = lookup(&root.path().join("gd/calder-1923.pdf"));
        assert_eq!(calder.clearance, Clearance::Permitted);
        assert_eq!(
            calder.levels,
            LevelCoverage::Stated(vec!["B1", "B2", "C1", "C2"])
        );

        let loose = lookup(&root.path().join("es/unrecorded.pdf"));
        assert!(matches!(loose.clearance, Clearance::Refused(_)));
        assert_eq!(
            loose.levels,
            LevelCoverage::Unstated,
            "nothing is known, so nothing is stated"
        );
    }

    #[test]
    fn the_levels_to_import_follow_the_row_and_the_flag() {
        let stated = LevelCoverage::Stated(vec!["A1", "A2"]);
        assert_eq!(levels_to_import(&stated, None), Ok(vec!["A1", "A2"]));
        assert_eq!(levels_to_import(&stated, Some("A2")), Ok(vec!["A2"]));
        let refused = levels_to_import(&stated, Some("B1")).expect_err("not covered");
        assert!(refused.contains("covers A1, A2, not B1"), "{refused}");

        let unstated = LevelCoverage::Unstated;
        let refused = levels_to_import(&unstated, None).expect_err("nothing stated");
        assert!(
            refused.contains("states no levels; pass --level"),
            "{refused}"
        );
        assert_eq!(levels_to_import(&unstated, Some("B1")), Ok(vec!["B1"]));

        let invalid = LevelCoverage::Invalid("A3".to_owned());
        let refused = levels_to_import(&invalid, None).expect_err("invalid");
        assert!(refused.contains("Levels cell \"A3\""), "{refused}");
        assert!(levels_to_import(&invalid, Some("B1")).is_err());

        assert!(
            levels_to_import(&stated, Some("A9")).is_err(),
            "an unknown level is refused"
        );
        assert_eq!(
            levels_to_import(&stated, Some("a2")),
            Ok(vec!["A2"]),
            "case does not matter"
        );
    }

    #[test]
    fn the_schedule_walks_levels_then_documents_and_reports_skips() {
        let rows = vec![
            (
                PathBuf::from("es/first.pdf"),
                LevelCoverage::Stated(vec!["A1", "A2"]),
            ),
            (
                PathBuf::from("es/second.pdf"),
                LevelCoverage::Stated(vec!["B1"]),
            ),
            (PathBuf::from("es/third.pdf"), LevelCoverage::Unstated),
        ];

        let whole = import_schedule(&rows, None).expect("no level given");
        let calls: Vec<(&str, &str)> = whole
            .calls
            .iter()
            .map(|(level, document)| (*level, document.to_str().expect("utf-8")))
            .collect();
        assert_eq!(
            calls,
            [
                ("A1", "es/first.pdf"),
                ("A2", "es/first.pdf"),
                ("B1", "es/second.pdf")
            ]
        );
        assert_eq!(whole.skipped.len(), 1);
        assert_eq!(whole.skipped[0].0, Path::new("es/third.pdf"));
        assert!(
            whole.skipped[0].1.contains("states no levels"),
            "{}",
            whole.skipped[0].1
        );

        let one = import_schedule(&rows, Some("A2")).expect("A2 given");
        let calls: Vec<(&str, &str)> = one
            .calls
            .iter()
            .map(|(level, document)| (*level, document.to_str().expect("utf-8")))
            .collect();
        assert_eq!(calls, [("A2", "es/first.pdf"), ("A2", "es/third.pdf")]);
        assert_eq!(one.skipped.len(), 1);
        assert!(
            one.skipped[0].1.contains("covers B1, not A2"),
            "{}",
            one.skipped[0].1
        );

        let none = import_schedule(&rows[1..2], Some("C1")).expect("C1 given");
        assert!(none.calls.is_empty(), "nothing covers C1");
        assert_eq!(none.skipped.len(), 1);

        assert!(
            import_schedule(&rows, Some("A9")).is_err(),
            "an unknown level is an error, not a skip"
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
