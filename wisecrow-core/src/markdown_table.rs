//! Reads the pipe tables of a Markdown document.
//!
//! Two ledgers on the shelves are Markdown tables that people edit and the
//! importers read: `grammar/SOURCES.md` and `readers/SOURCES.md`. Each has its
//! own columns and its own rules, but finding the tables, their header lines
//! and their cells is one job, and keeping a single reader means a quirk of
//! the format is handled once.

/// One table: its header cells and the cells of every row beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table<'a> {
    pub header: Vec<&'a str>,
    pub rows: Vec<Vec<&'a str>>,
}

impl Table<'_> {
    /// Where the column headed exactly `name` sits, if the table has one.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|cell| *cell == name)
    }
}

/// Every table in `markdown`, in document order.
///
/// A table is a run of consecutive lines that open with `|`. Its first line
/// that is not a separator is its header; separator lines are skipped
/// wherever they fall, and any other line ends the table.
#[must_use]
pub fn tables(markdown: &str) -> Vec<Table<'_>> {
    let mut found = Vec::new();
    let mut current: Option<Table<'_>> = None;
    for line in markdown.lines() {
        let Some(row) = cells(line) else {
            found.extend(current.take());
            continue;
        };
        if is_separator(&row) {
            continue;
        }
        match current.as_mut() {
            None => {
                current = Some(Table {
                    header: row,
                    rows: Vec::new(),
                });
            }
            Some(table) => table.rows.push(row),
        }
    }
    found.extend(current);
    found
}

/// The trimmed cells of a table line, or `None` for a line that is not one.
#[must_use]
pub fn cells(line: &str) -> Option<Vec<&str>> {
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

    const DOCUMENT: &str = "\
# Ledger

| File | Translate |
|------|:---------:|
| `a.pdf` | yes |
|  b.pdf  |     |

Prose ends the first table.

| Title | File | Notes |
| --- | --- | --- |
| Second | c.pdf | x |
";

    #[test]
    fn every_table_is_read_with_its_header_and_rows() {
        let found = tables(DOCUMENT);

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].header, vec!["File", "Translate"]);
        assert_eq!(
            found[0].rows,
            vec![vec!["`a.pdf`", "yes"], vec!["b.pdf", ""]]
        );
        assert_eq!(found[1].header, vec!["Title", "File", "Notes"]);
        assert_eq!(found[1].rows, vec![vec!["Second", "c.pdf", "x"]]);
    }

    #[test]
    fn a_column_is_found_by_its_exact_header() {
        let found = tables(DOCUMENT);

        assert_eq!(found[1].column("File"), Some(1));
        assert_eq!(found[1].column("file"), None, "headers are matched exactly");
        assert_eq!(found[0].column("Notes"), None);
    }

    #[test]
    fn separators_are_skipped_wherever_they_fall() {
        let found = tables("|---|\n| File |\n|:-:|\n| a.pdf |\n");

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].header, vec!["File"]);
        assert_eq!(found[0].rows, vec![vec!["a.pdf"]]);
    }

    #[test]
    fn a_line_is_a_table_line_only_when_it_opens_with_a_pipe() {
        assert_eq!(cells("  | a | b |  "), Some(vec!["a", "b"]));
        assert_eq!(
            cells("| a | b"),
            Some(vec!["a", "b"]),
            "a closing pipe is optional"
        );
        assert_eq!(cells("a | b |"), None);
        assert_eq!(cells(""), None);
    }

    #[test]
    fn a_document_without_tables_has_none() {
        assert!(tables("# Title\n\nJust prose.\n").is_empty());
    }
}
