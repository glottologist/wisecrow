//! Cuts a book's body into chapters, paragraphs and sentences.
//!
//! A sentence is the unit everything else lines up on: a translation run
//! is a numbered list of them, the dual view pairs them, and a reading
//! position names one by its chapter, paragraph and sentence ordinals.

use unicode_segmentation::UnicodeSegmentation;

use super::extract::{is_chapter_heading, Body};

/// Version of the segmentation rules. It is part of a book's import
/// identity: changing the rules makes a new revision of every book rather
/// than re-cutting the ordinals stored for the old one.
pub const SEGMENTER_VERSION: i16 = 1;
/// Abbreviations that end in a full stop without ending a sentence.
const ABBREVIATIONS: &[&str] = &["Mr.", "Mrs.", "Ms.", "Dr.", "St."];
/// Characters that end a sentence-final line, closing quotation marks
/// included.
const SENTENCE_END: &[char] = &['.', '!', '?', '’', '”', '\'', '"', '…', ':'];

/// One sentence and where it sits, every ordinal counted from one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentedSentence {
    pub chapter: u32,
    pub paragraph: u32,
    pub sentence: u32,
    pub text: String,
}

/// Cuts a body into numbered sentences, in reading order.
#[must_use]
pub fn segment(body: &Body) -> Vec<SegmentedSentence> {
    let mut found = Vec::new();
    let mut chapter = 0u32;
    for paragraphs in chapters(body) {
        if paragraphs.is_empty() {
            continue;
        }
        chapter = chapter.saturating_add(1);
        let mut paragraph = 0u32;
        for lines in paragraphs {
            let cut = sentences(&join_lines(&lines));
            if cut.is_empty() {
                continue;
            }
            paragraph = paragraph.saturating_add(1);
            for (index, text) in cut.into_iter().enumerate() {
                found.push(SegmentedSentence {
                    chapter,
                    paragraph,
                    sentence: u32::try_from(index).map_or(u32::MAX, |n| n.saturating_add(1)),
                    text,
                });
            }
        }
    }
    found
}

/// Splits one paragraph's text at sentence boundaries.
#[must_use]
pub fn sentences(paragraph: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for sentence in paragraph.unicode_sentences() {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        match found.last_mut() {
            Some(previous) if ends_with_abbreviation(previous) || continues_sentence(sentence) => {
                previous.push(' ');
                previous.push_str(sentence);
            }
            _ => found.push(sentence.to_owned()),
        }
    }
    found
}

fn ends_with_abbreviation(sentence: &str) -> bool {
    sentence
        .split_whitespace()
        .last()
        .is_some_and(|word| ABBREVIATIONS.contains(&word))
}

/// A speech tag such as "he asked" follows a closing quote that Annex 29
/// treats as its own sentence. The tag starts with a lower-case letter.
fn continues_sentence(sentence: &str) -> bool {
    sentence.chars().next().is_some_and(char::is_lowercase)
}

/// The body's chapters, each a list of paragraphs, each a list of lines.
fn chapters(body: &Body) -> Vec<Vec<Vec<String>>> {
    let mut chapters: Vec<Vec<Vec<String>>> = vec![Vec::new()];
    let mut paragraph: Vec<String> = Vec::new();
    let mut previous: Option<String> = None;
    for page in &body.pages {
        let margin = page_margin(page);
        for line in page {
            if !is_prose(line) {
                flush(&mut chapters, &mut paragraph);
                continue;
            }
            if is_chapter_heading(line) {
                flush(&mut chapters, &mut paragraph);
                chapters.push(Vec::new());
                previous = None;
                continue;
            }
            let starts_paragraph = indent(line) > margin
                && previous
                    .as_deref()
                    .is_some_and(|before| before.trim_end().ends_with(SENTENCE_END));
            if starts_paragraph {
                flush(&mut chapters, &mut paragraph);
            }
            paragraph.push(line.trim().to_owned());
            previous = Some(line.clone()); // clone: the next line is judged against this one
        }
    }
    flush(&mut chapters, &mut paragraph);
    chapters
}

fn flush(chapters: &mut [Vec<Vec<String>>], paragraph: &mut Vec<String>) {
    if paragraph.is_empty() {
        return;
    }
    if let Some(chapter) = chapters.last_mut() {
        chapter.push(std::mem::take(paragraph));
    }
}

/// A line holding at least one letter or digit; blank lines and ornaments
/// such as `♦` are paragraph breaks.
fn is_prose(line: &str) -> bool {
    line.chars().any(char::is_alphanumeric)
}

fn indent(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

/// The least indentation of any prose line on a page that is not a heading.
fn page_margin(page: &[String]) -> usize {
    page.iter()
        .filter(|line| is_prose(line) && !is_chapter_heading(line))
        .map(|line| indent(line))
        .min()
        .unwrap_or(0)
}

/// Joins a paragraph's lines, repairing a word broken by a hyphen at a line
/// end. A dash with a space before it is punctuation and is kept.
fn join_lines(lines: &[String]) -> String {
    let mut text = String::new();
    for line in lines {
        let broken = text
            .strip_suffix('-')
            .is_some_and(|before| before.chars().last().is_some_and(char::is_alphabetic))
            && line.chars().next().is_some_and(char::is_lowercase);
        if broken {
            text.pop();
        } else if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::extract::{extract, tests::penguin_like, Extraction};

    fn body(lines: &[&str]) -> Body {
        Body {
            first_page: 1,
            last_page: 1,
            pages: vec![lines.iter().map(|line| (*line).to_owned()).collect()],
        }
    }

    fn texts(sentences: &[SegmentedSentence]) -> Vec<&str> {
        sentences
            .iter()
            .map(|sentence| sentence.text.as_str())
            .collect()
    }

    fn at(sentences: &[SegmentedSentence], ordinals: (u32, u32, u32)) -> &str {
        sentences
            .iter()
            .find(|sentence| (sentence.chapter, sentence.paragraph, sentence.sentence) == ordinals)
            .map_or_else(
                || panic!("no sentence at {ordinals:?}"),
                |sentence| sentence.text.as_str(),
            )
    }

    #[test]
    fn a_penguin_page_is_cut_into_indented_paragraphs_and_sentences() {
        let Extraction::Body(body) = extract(&penguin_like(), None) else {
            panic!("the fixture has a body");
        };

        let sentences = segment(&body);

        assert_eq!(sentences.len(), 21);
        assert!(sentences.iter().all(|sentence| sentence.chapter == 1));
        assert_eq!(at(&sentences, (1, 1, 1)), "Tom lived near the sea.");
        assert_eq!(at(&sentences, (1, 1, 5)), "His old blue coat was dirty.");
        assert_eq!(at(&sentences, (1, 2, 1)), "‘I like this place,’ he said.");
        assert_eq!(at(&sentences, (1, 2, 2)), "‘Do many people come here?’");
        assert_eq!(at(&sentences, (1, 3, 1)), "‘No,’ said Tom.");
        assert_eq!(
            at(&sentences, (1, 4, 1)),
            "Then winter came, and it was very cold.",
            "a one-space indent after a full stop starts a paragraph"
        );
        assert_eq!(at(&sentences, (1, 5, 1)), "The man looked at the light.");
        assert_eq!(at(&sentences, (1, 7, 3)), "Mr. Gray made it long ago.’");
        assert_eq!(
            at(&sentences, (1, 7, 4)),
            "Tom was happy.",
            "an indented line after an unfinished one continues the paragraph"
        );
        assert_eq!(
            sentences.last().map(|s| s.text.as_str()),
            Some("Tom never saw him again.")
        );
    }

    #[test]
    fn a_heading_mid_page_starts_the_next_chapter() {
        let sentences = segment(&body(&[
            "   Chapter 1 Start",
            "  The sea was calm.",
            "",
            "            Chapter 2 The Storm",
            "  Then the wind came.",
        ]));

        assert_eq!(
            sentences,
            vec![
                SegmentedSentence {
                    chapter: 1,
                    paragraph: 1,
                    sentence: 1,
                    text: "The sea was calm.".into()
                },
                SegmentedSentence {
                    chapter: 2,
                    paragraph: 1,
                    sentence: 1,
                    text: "Then the wind came.".into()
                },
            ]
        );
    }

    #[test]
    fn hyphenation_at_a_line_end_is_repaired_and_spaced_dashes_are_kept() {
        let sentences = segment(&body(&[
            "  Robert was born in Scot-",
            "land and began writing when he was a boy. But what - or who -",
            "was he afraid of?",
        ]));

        assert_eq!(
            texts(&sentences),
            [
                "Robert was born in Scotland and began writing when he was a boy.",
                "But what - or who - was he afraid of?",
            ]
        );
    }

    #[test]
    fn an_ornament_line_ends_a_paragraph() {
        let sentences = segment(&body(&[
            "  They were happy.",
            "            ♦",
            "The next morning came.",
        ]));

        assert_eq!(sentences[1].paragraph, 2);
    }

    #[test]
    fn dialogue_ellipses_and_abbreviations_stay_whole() {
        assert_eq!(
            sentences("‘Come here, boy,’ he said, and he gave me some money. Dr. Livesey came."),
            [
                "‘Come here, boy,’ he said, and he gave me some money.",
                "Dr. Livesey came."
            ]
        );
        assert_eq!(
            sentences("‘Now, Billy Bones, you must. . .’ He stopped."),
            ["‘Now, Billy Bones, you must. . .’", "He stopped."]
        );
        assert_eq!(
            sentences("They met Mrs. Hall and St. John at noon."),
            ["They met Mrs. Hall and St. John at noon."]
        );
    }

    #[test]
    fn a_body_without_a_heading_is_one_chapter() {
        let sentences = segment(&body(&["  One line.", "  Two lines."]));

        assert!(sentences.iter().all(|sentence| sentence.chapter == 1));
        assert_eq!(sentences.len(), 2);
    }

    /// Runs the segmenter over three books of the local example shelf, which
    /// is not in the repository; the test passes without checking anything
    /// when the shelf is absent.
    #[test]
    #[ignore = "reads the local reader shelf when it is present"]
    fn three_shelf_books_segment_into_their_own_chapter_counts() {
        use crate::reader::extract::{LayoutPages, PageSource};

        let shelf = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../agents/easy_reader_examples/Penguin Readers");
        if !shelf.is_dir() {
            eprintln!("skipped: {} is not present", shelf.display());
            return;
        }
        for (file, chapters) in [
            ("Level 2/Treasure Island.pdf", 3),
            ("Level 2/Robin Hood.pdf", 16),
            ("Level 5-6/Airport.pdf", 41),
        ] {
            let pages = LayoutPages
                .pages(&shelf.join(file))
                .expect("poppler reads the book");
            let Extraction::Body(body) = extract(&pages, None) else {
                panic!("{file} has a chapter heading");
            };
            let sentences = segment(&body);
            let found = sentences.iter().map(|sentence| sentence.chapter).max();
            assert_eq!(found, Some(chapters), "{file}");
            assert!(
                sentences
                    .iter()
                    .all(|sentence| sentence.text.chars().any(char::is_alphabetic)),
                "{file}"
            );
        }
    }
}
