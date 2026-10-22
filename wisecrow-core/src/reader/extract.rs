//! Finds the body of a graded reader and removes the page furniture.
//!
//! Front matter (title, copyright, contents, introduction) and back matter
//! (activities, glossary, word list) are not the story, and a learner should
//! not be handed them as chapters. The body starts at the first page headed
//! by a chapter heading and ends before the first page headed by back
//! matter; a book without a chapter heading is left for its ledger row to
//! name the pages.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::shelf::PageRange;
use crate::errors::WisecrowError;
use crate::grammar::pdf_text::{density, layout_pages, MIN_CHARS_PER_PAGE};

/// Lines that open back matter, compared in lower case against the start of
/// a page's first line.
const BACK_MATTER: &[&str] = &["activities", "glossary", "word list", "exercises"];
/// Number words a heading may use in place of a numeral.
const NUMBER_WORDS: &[&str] = &[
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
];
/// A running head is a line seen at the edge of at least this many pages.
const RUNNING_HEAD_PAGES: usize = 3;
/// A caption page holds at most this many lines.
const CAPTION_LINES: usize = 6;

/// Where a book's pages come from. Production reads poppler's layout text;
/// tests supply pages directly.
pub trait PageSource: Send + Sync {
    /// The document's pages, as `pdftotext -layout` lays them out.
    ///
    /// # Errors
    ///
    /// Returns an error when the document cannot be read.
    fn pages(&self, path: &Path) -> Result<Vec<String>, WisecrowError>;
}

/// Reads books with `pdftotext -layout`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutPages;

impl PageSource for LayoutPages {
    fn pages(&self, path: &Path) -> Result<Vec<String>, WisecrowError> {
        layout_pages(path)
    }
}

/// What a book's pages yield.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extraction {
    /// The story's pages, furniture removed.
    Body(Body),
    /// No chapter heading opens a page, or the ledger's range lies outside
    /// the document: the row's `Pages` cell must name the body.
    NeedsPages,
    /// Too little text to be anything but a scan.
    Thin { chars_per_page: usize },
}

/// The body of a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// Printed page number of the first body page, counted from one.
    pub first_page: u32,
    pub last_page: u32,
    /// Each body page's lines, page furniture removed, layout kept.
    pub pages: Vec<Vec<String>>,
}

/// Finds the body of a book and removes its furniture.
#[must_use]
pub fn extract(pages: &[String], range: Option<PageRange>) -> Extraction {
    let chars_per_page = density(pages);
    if chars_per_page < MIN_CHARS_PER_PAGE {
        return Extraction::Thin { chars_per_page };
    }
    let Some((start, end)) = (match range {
        Some(range) => ledger_range(pages.len(), range),
        None => heading_range(pages),
    }) else {
        return Extraction::NeedsPages;
    };
    let first_page = u32::try_from(start).map_or(u32::MAX, |index| index.saturating_add(1));
    let last_page = u32::try_from(end).unwrap_or(u32::MAX);
    Extraction::Body(Body {
        first_page,
        last_page,
        pages: remove_furniture(&pages[start..end]),
    })
}

/// Whether a line is a chapter heading rather than prose or a contents entry.
///
/// A heading reads `Chapter`, then a numeral, a Roman numeral or a number
/// word, then an optional title. A contents entry has the same shape with a
/// page number at the end, so a heading of three or more words whose last
/// word is a page number is not one.
#[must_use]
pub fn is_chapter_heading(line: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let [first, number, ..] = words.as_slice() else {
        return false;
    };
    if !first.eq_ignore_ascii_case("chapter") || !is_number(number) {
        return false;
    }
    !(words.len() > 2 && words.last().is_some_and(|last| is_page_number(last)))
}

/// A chapter's number: a numeral, a Roman numeral in either case, or a
/// number word.
fn is_number(word: &str) -> bool {
    let lower = word.to_lowercase();
    is_page_number(&lower) || NUMBER_WORDS.contains(&lower.as_str())
}

/// A printed page number: a numeral, or a lower-case Roman numeral as front
/// matter is numbered. `Story I` ends a title, not a contents line.
fn is_page_number(word: &str) -> bool {
    !word.is_empty()
        && (word.chars().all(|c| c.is_ascii_digit())
            || word
                .chars()
                .all(|c| matches!(c, 'i' | 'v' | 'x' | 'l' | 'c')))
}

/// The first non-blank line of a page.
pub(crate) fn first_line(page: &str) -> Option<&str> {
    page.lines().find(|line| !line.trim().is_empty())
}

fn is_back_matter(page: &str) -> bool {
    first_line(page).is_some_and(|line| {
        let lower = line.trim().to_lowercase();
        BACK_MATTER.iter().any(|heading| lower.starts_with(heading))
    })
}

/// Page indices `start..end` from the first chapter page to the back matter.
fn heading_range(pages: &[String]) -> Option<(usize, usize)> {
    let start = pages
        .iter()
        .position(|page| first_line(page).is_some_and(is_chapter_heading))?;
    let end = pages
        .iter()
        .enumerate()
        .skip(start.saturating_add(1))
        .find(|(_, page)| is_back_matter(page))
        .map_or(pages.len(), |(index, _)| index);
    Some((start, end))
}

/// Page indices for a ledger range, clipped to the document's length.
fn ledger_range(count: usize, range: PageRange) -> Option<(usize, usize)> {
    let start = usize::try_from(range.first).ok()?.checked_sub(1)?;
    let end = usize::try_from(range.last).ok()?.min(count);
    (start < end).then_some((start, end))
}

fn is_lone_number(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.chars().count() <= 4 && is_page_number(trimmed)
}

/// Quotes straightened and runs of space collapsed, so that a caption and
/// the sentence it repeats compare equal.
fn normalise(text: &str) -> String {
    text.replace(['‘', '’'], "'")
        .replace(['“', '”'], "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn remove_furniture(pages: &[String]) -> Vec<Vec<String>> {
    let numbered: Vec<Vec<&str>> = pages
        .iter()
        .map(|page| page.lines().filter(|line| !is_lone_number(line)).collect())
        .collect();
    let heads = running_heads(&numbered);
    let flattened: Vec<String> = numbered
        .iter()
        .map(|lines| normalise(&lines.join("\n")))
        .collect();
    numbered
        .iter()
        .enumerate()
        .filter(|(index, lines)| !is_caption_page(lines, *index, &flattened))
        .map(|(_, lines)| {
            lines
                .iter()
                .filter(|line| !heads.contains(line.trim()))
                .map(|line| (*line).to_owned())
                .collect()
        })
        .collect()
}

/// Lines that open or close at least [`RUNNING_HEAD_PAGES`] pages and are
/// not chapter headings.
fn running_heads<'a>(pages: &[Vec<&'a str>]) -> HashSet<&'a str> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for lines in pages {
        let edges: Vec<&str> = lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect();
        let mut ends: Vec<&str> = edges.first().copied().into_iter().collect();
        if edges.len() > 1 {
            ends.extend(edges.last().copied());
        }
        for line in ends {
            *seen.entry(line).or_insert(0) += 1;
        }
    }
    seen.into_iter()
        .filter(|(line, count)| *count >= RUNNING_HEAD_PAGES && !is_chapter_heading(line))
        .map(|(line, _)| line)
        .collect()
}

/// A short page whose every line is repeated on the page before or after.
fn is_caption_page(lines: &[&str], index: usize, flattened: &[String]) -> bool {
    let prose: Vec<String> = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| normalise(line))
        .collect();
    if prose.is_empty() || prose.len() > CAPTION_LINES {
        return false;
    }
    let neighbours: Vec<&str> = [index.checked_sub(1), index.checked_add(1)]
        .into_iter()
        .flatten()
        .filter_map(|neighbour| flattened.get(neighbour))
        .map(String::as_str)
        .collect();
    prose
        .iter()
        .all(|line| neighbours.iter().any(|page| page.contains(line.as_str())))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rstest::rstest;

    /// Pages laid out the way `pdftotext -layout` lays out a Penguin reader:
    /// a title page, copyright, contents, a prose introduction, a chapter
    /// page with indented paragraphs, a picture caption that repeats the
    /// next page, a second prose page, and the activities.
    pub(crate) fn penguin_like() -> Vec<String> {
        [
            "    The Lighthouse Keeper\n\nANNE WRITER\n\n                      Level 2\n\n                  Retold by Anne Writer\n    Series Editors: Andy Hopkins and Jocelyn Potter\n",
            "                Pearson Education Limited\n                ISBN 0 000 00000 0\n\n      All rights reserved; no part of this publication may be reproduced, stored\n         in a retrieval system, or transmitted in any form or by any means.\n",
            "                         Contents\n\nIntroduction                    v\nChapter 1      The Storm        1\nChapter 2      The Light        3\nActivities                     35\n",
            "                      Introduction\nTom lives in a small house by the sea. He watches the ships. One day a\nstorm comes. What will he do?\n  Anne Writer was born in 1950 in Leith and began writing when she was\na girl. She lives by the sea and writes about the people there.\n\n                                  v\n",
            "   Chapter 1 The Storm\n  Tom lived near the sea. It was a quiet place. One day, an old\nman came to the door. He was tall and strong, and his face was\nbrown. His old blue coat was dirty.\n   ‘I like this place,’ he said. ‘Do many people come here?’\n  ‘No,’ said Tom.\n Then winter came, and it was very cold. The wind was strong and the\nsea was grey. The man looked at the light.\n\n                                    1\n",
            "The man looked at the light.\n      'Is this light for the ships?'\n",
            "  The man looked at the light.\n  ‘Is this light for the ships?’ he asked.\n  ‘Yes,’ said Tom. ‘It is for the ships at night. Mr. Gray made it\n  long ago.’ Tom was happy. The ships were safe. In the evenings he\nsat by the window and in the day he watched the sea. The man stayed\nfor a week, and then he went away in a small boat. Tom never saw\nhim again.\n\n                                    3\n",
            "                            ACTIVITIES\nChapter 1\nBefore you read\n\n1 All these words come in this part of the story. Do you understand\n  them?\n  boat light sea ship storm\n",
        ]
        .iter()
        .map(|page| (*page).to_owned())
        .collect()
    }

    fn body_of(extraction: Extraction) -> Body {
        match extraction {
            Extraction::Body(body) => body,
            other => panic!("expected a body, got {other:?}"),
        }
    }

    #[rstest]
    #[case("Chapter 1", true)]
    #[case("   Chapter 1 Jim Hawkins’ Story I", true)]
    #[case("CHAPTER ONE", true)]
    #[case("Chapter XII The End", true)]
    #[case("Chapter 12", true)]
    #[case("Chapter 22   Guerrero Insures Himself                    62", false)]
    #[case("Chapter 1      The Storm        1", false)]
    #[case("Chapters 5 - 6", false)]
    #[case("Chapter", false)]
    #[case("Chapter seventy", false)]
    #[case("The chapter 1", false)]
    fn a_chapter_heading_is_told_from_a_contents_entry(#[case] line: &str, #[case] heading: bool) {
        assert_eq!(is_chapter_heading(line), heading, "{line}");
    }

    #[test]
    fn the_body_runs_from_chapter_one_to_the_activities() {
        let body = body_of(extract(&penguin_like(), None));

        assert_eq!((body.first_page, body.last_page), (5, 7));
        let text: Vec<String> = body.pages.iter().map(|page| page.join("\n")).collect();
        assert!(text[0].contains("Chapter 1 The Storm"));
        assert!(
            !text.iter().any(|page| page.contains("Introduction")),
            "front matter is cut"
        );
        assert!(
            !text.iter().any(|page| page.contains("ACTIVITIES")),
            "back matter is cut"
        );
    }

    #[test]
    fn page_numbers_and_caption_pages_are_removed() {
        let body = body_of(extract(&penguin_like(), None));

        assert_eq!(body.pages.len(), 2, "the caption page is dropped");
        for page in &body.pages {
            assert!(
                page.iter()
                    .all(|line| line.trim() != "1" && line.trim() != "3"),
                "{page:?}"
            );
        }
        let joined = body.pages.concat().join("\n");
        assert_eq!(joined.matches("The man looked at the light.").count(), 2);
    }

    /// About two hundred characters of prose for day `n`, so that a page
    /// clears the scan threshold.
    fn prose(n: u32) -> String {
        format!(
            "  Tom walked to the sea on day {n}. The wind was cold and the water was grey, \
             and the ships stayed in the harbour all morning while the gulls called over \
             the rocks.\nHe went home on day {n}.\n"
        )
    }

    #[test]
    fn a_running_head_repeated_on_three_pages_is_removed() {
        let pages: Vec<String> = std::iter::once(format!("   Chapter 1 Start\n{}", prose(0)))
            .chain((1..=3).map(|n| format!("THE LIGHTHOUSE KEEPER\n{}", prose(n))))
            .collect();

        let body = body_of(extract(&pages, None));

        assert_eq!(body.pages.len(), 4);
        assert!(body
            .pages
            .iter()
            .flatten()
            .all(|line| line.trim() != "THE LIGHTHOUSE KEEPER"));
        assert!(body.pages[3].iter().any(|line| line.contains("day 3")));
    }

    #[test]
    fn a_book_without_a_chapter_heading_needs_its_pages() {
        let mut pages = penguin_like();
        pages[4] = pages[4].replace("Chapter 1 The Storm", "The Storm");

        assert!(matches!(extract(&pages, None), Extraction::NeedsPages));
    }

    #[test]
    fn a_ledger_range_is_cut_whatever_the_headings_say() {
        let mut pages = penguin_like();
        pages[4] = pages[4].replace("Chapter 1 The Storm", "The Storm");

        let body = body_of(extract(&pages, Some(PageRange { first: 4, last: 5 })));

        assert_eq!((body.first_page, body.last_page), (4, 5));
        assert!(body.pages[0].join("\n").contains("Introduction"));
    }

    #[test]
    fn a_range_past_the_end_needs_its_pages() {
        let range = Some(PageRange {
            first: 30,
            last: 40,
        });

        assert!(matches!(
            extract(&penguin_like(), range),
            Extraction::NeedsPages
        ));
    }

    #[test]
    fn a_scan_is_refused_by_density_before_anything_else() {
        let scan: Vec<String> = (0..34).map(|_| "Adobe Acrobat".to_owned()).collect();

        assert!(matches!(
            extract(&scan, None),
            Extraction::Thin { chars_per_page: 13 }
        ));
    }
}
