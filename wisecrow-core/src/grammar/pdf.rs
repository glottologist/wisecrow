use std::path::Path;

use crate::errors::WisecrowError;

const BULLET_PREFIXES: &[&str] = &["- ", "\u{2022} ", "* ", "\u{2013} "];
const MAX_HEADER_LENGTH: usize = 100;
const MAX_HEADER_WORDS: usize = 8;
/// Shortest block of prose kept as a passage.
///
/// A grammar book's text layer holds far more short lines than rules: running
/// heads, page numbers, index entries, the stub of a paradigm table the column
/// detector missed. None of them says anything a learner could be taught, and a
/// rule that a book states in fewer than two hundred characters states it again
/// in the paragraph around it. Dropping them here is what keeps a 256-page
/// standard from arriving as seven thousand fragments.
const MIN_PASSAGE_CHARS: usize = 200;
/// Run of spaces treated as a column gap rather than a word break.
const COLUMN_GAP: &str = "   ";
/// Column gaps a line needs before it is read as a table row.
const MIN_COLUMN_GAPS: usize = 2;

#[derive(Debug, Clone)]
pub struct GrammarContent {
    pub passages: Vec<GrammarPassage>,
}

/// One heading and the prose beneath it, as it stood on one page.
///
/// A passage is a unit a model can be asked to read. It is deliberately not a
/// unit the database stores: turning passages into levelled points is the job of
/// the synthesis step, and the page number is carried so a point can cite where
/// it came from.
#[derive(Debug, Clone)]
pub struct GrammarPassage {
    pub heading: Option<String>,
    pub text: String,
    pub page: usize,
    pub examples: Vec<ExampleSentence>,
}

#[derive(Debug, Clone)]
pub struct ExampleSentence {
    pub text: String,
    pub translation: Option<String>,
}

/// Extracts the prose passages of a grammar PDF, page by page.
///
/// The text comes from [`crate::grammar::pdf_text`], which falls back to poppler
/// for a document the native extractor cannot read. Headings are detected, the
/// prose under each is joined into one passage, quoted and `e.g.` lines are kept
/// as example sentences, and table rows and any passage too short to hold a rule
/// are dropped.
///
/// # Errors
///
/// Returns an error if the PDF cannot be read or parsed, or if it holds no
/// passage long enough to be grammar prose -- the answer for a page-scan PDF
/// whose text layer is empty.
pub fn extract(path: &Path) -> Result<GrammarContent, WisecrowError> {
    let canonical = path
        .canonicalize()
        .map_err(|e| WisecrowError::PdfExtractionError(format!("Invalid path: {e}")))?;

    let text = crate::grammar::pdf_text::pages(&canonical)?;

    let passages = parse_passages(&text.pages);

    if passages.is_empty() {
        return Err(WisecrowError::PdfExtractionError(
            "No grammar content found in PDF".to_owned(),
        ));
    }

    Ok(GrammarContent { passages })
}

/// Collects the passages of a document whose pages are already text.
///
/// Exposed to the crate so that [`crate::grammar::pdf_check`] can report what an
/// import would find without extracting the text a second time.
pub(crate) fn parse_passages(pages: &[String]) -> Vec<GrammarPassage> {
    let mut passages = Vec::new();

    for (index, page) in pages.iter().enumerate() {
        let page_number = index.saturating_add(1);
        let mut heading: Option<String> = None;
        let mut prose: Vec<String> = Vec::new();
        let mut examples: Vec<ExampleSentence> = Vec::new();

        for line in page.lines() {
            let trimmed = line.trim();

            if trimmed.is_empty() || is_table_row(trimmed) {
                continue;
            }

            if is_section_header(trimmed) {
                push_passage(
                    &mut passages,
                    page_number,
                    &mut heading,
                    &mut prose,
                    &mut examples,
                );
                heading = Some(trimmed.to_owned());
                continue;
            }

            if is_example_sentence(trimmed) {
                let (text, translation) = split_example(trimmed);
                examples.push(ExampleSentence { text, translation });
                continue;
            }

            let stripped = strip_prefix(trimmed);
            if !stripped.is_empty() {
                prose.push(stripped);
            }
        }

        push_passage(
            &mut passages,
            page_number,
            &mut heading,
            &mut prose,
            &mut examples,
        );
    }

    passages
}

/// Joins the prose collected so far into a passage, unless it is too short.
///
/// The examples travel with the prose they sat among, and are discarded with it
/// when the block is too short to be a rule -- an example without its rule is
/// what the old line-by-line reading produced in quantity.
fn push_passage(
    passages: &mut Vec<GrammarPassage>,
    page: usize,
    heading: &mut Option<String>,
    prose: &mut Vec<String>,
    examples: &mut Vec<ExampleSentence>,
) {
    let text = std::mem::take(prose).join(" ");
    let taken_examples = std::mem::take(examples);

    if text.chars().count() >= MIN_PASSAGE_CHARS {
        passages.push(GrammarPassage {
            heading: heading.clone(), // clone: the heading also governs the next passage on the page
            text,
            page,
            examples: taken_examples,
        });
    }
}

fn is_section_header(line: &str) -> bool {
    if line.len() > MAX_HEADER_LENGTH {
        return false;
    }
    if line.starts_with("Chapter ")
        || line.starts_with("Lesson ")
        || line.starts_with("Unit ")
        || line.starts_with("Part ")
    {
        return true;
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() > MAX_HEADER_WORDS || words.is_empty() {
        return false;
    }
    let short_prepositions = [
        "a", "an", "the", "in", "on", "of", "for", "and", "to", "with",
    ];
    !line.contains('.')
        && words
            .iter()
            .all(|w| short_prepositions.contains(w) || w.starts_with(|c: char| c.is_uppercase()))
}

/// Reads a line as a row of a table rather than a sentence.
///
/// A text layer keeps a paradigm table's columns as runs of spaces, so a line
/// holding several of them and ending without a full stop is a row of cells.
/// Importing the Irish standard filed one such row, `don fhear throm don
/// chuideachta ghnóthach`, as a grammar rule of its own.
fn is_table_row(line: &str) -> bool {
    if line.contains('\t') {
        return true;
    }
    let gaps = line
        .split(COLUMN_GAP)
        .filter(|part| !part.is_empty())
        .count();
    gaps > MIN_COLUMN_GAPS && !line.ends_with('.')
}

fn is_example_sentence(line: &str) -> bool {
    (line.starts_with('"') || line.starts_with('\u{201C}'))
        || (line.starts_with("e.g.") || line.starts_with("E.g."))
        || (line.starts_with("Example:") || line.starts_with("Ex:"))
}

fn strip_prefix(line: &str) -> String {
    for prefix in BULLET_PREFIXES {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest.trim().to_owned();
        }
    }
    if let Some(pos) = line.find(". ") {
        if pos < 5 && line[..pos].chars().all(|c| c.is_ascii_digit()) {
            return line.get(pos + 2..).unwrap_or("").trim().to_owned();
        }
    }
    if let Some(pos) = line.find(") ") {
        if pos < 5 && line[..pos].chars().all(|c| c.is_ascii_digit()) {
            return line.get(pos + 2..).unwrap_or("").trim().to_owned();
        }
    }
    line.to_owned()
}

fn split_example(line: &str) -> (String, Option<String>) {
    let cleaned = line
        .trim_start_matches(['"', '\u{201C}', ' '])
        .trim_end_matches(['"', '\u{201D}']);

    let cleaned = cleaned
        .strip_prefix("Example: ")
        .or_else(|| cleaned.strip_prefix("Ex: "))
        .or_else(|| cleaned.strip_prefix("e.g. "))
        .or_else(|| cleaned.strip_prefix("E.g. "))
        .unwrap_or(cleaned);

    if let Some((text, translation)) = cleaned.split_once(" \u{2014} ") {
        return (text.trim().to_owned(), Some(translation.trim().to_owned()));
    }
    if let Some((text, translation)) = cleaned.split_once(" - ") {
        if !text.is_empty() && !translation.is_empty() {
            return (text.trim().to_owned(), Some(translation.trim().to_owned()));
        }
    }

    (cleaned.to_owned(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    /// Prose long enough to pass [`MIN_PASSAGE_CHARS`], as a book states a rule.
    const PRESENT_TENSE_PROSE: &str = "Regular verbs in the present tense take the endings of their conjugation class, which is fixed by the infinitive. A verb whose infinitive ends in -ar belongs to the first class, and one ending in -er or -ir to the second and third. The stem is what remains once the ending is removed, and every present-tense form is built on it.";

    #[rstest]
    #[case("Present Tense", true)]
    #[case("Chapter 1", true)]
    #[case("Verbs of Motion", true)]
    #[case(
        "This is a long sentence that explains grammar rules in detail.",
        false
    )]
    #[case("The cat sat on the mat", false)]
    fn section_header_detected(#[case] input: &str, #[case] expected: bool) {
        assert_eq!(is_section_header(input), expected);
    }

    #[rstest]
    #[case("don fhear throm   don chuideachta ghnóthach   don bhean bhocht", true)]
    #[case("feiceann\tchonaic\tní fhaca", true)]
    #[case(
        "The stem is what remains once the ending is removed from the infinitive.",
        false
    )]
    #[case("A sentence with   one gap only.", false)]
    fn table_row_detected(#[case] input: &str, #[case] expected: bool) {
        assert_eq!(is_table_row(input), expected);
    }

    #[rstest]
    #[case("\"Hola, mundo\"", true)]
    #[case("e.g. Hola", true)]
    #[case("Example: Hello", true)]
    #[case("This is a normal line", false)]
    #[case("\u{201C}quoted curly\u{201D}", true)]
    #[case("E.g. something here", true)]
    #[case("Ex: another example", true)]
    fn example_sentence_detected(#[case] input: &str, #[case] expected: bool) {
        assert_eq!(is_example_sentence(input), expected);
    }

    #[rstest]
    #[case("\"Hola \u{2014} Hello\"", "Hola", Some("Hello"))]
    #[case("\"Hola, mundo\"", "Hola, mundo", None)]
    #[case(
        "\"text part - translation part\"",
        "text part",
        Some("translation part")
    )]
    fn split_example_cases(
        #[case] input: &str,
        #[case] expected_text: &str,
        #[case] expected_trans: Option<&str>,
    ) {
        let (text, trans) = split_example(input);
        assert_eq!(text, expected_text);
        assert_eq!(trans.as_deref(), expected_trans);
    }

    #[rstest]
    #[case("1. Use the present", "Use the present")]
    #[case("12) Another rule", "Another rule")]
    #[case("- A rule", "A rule")]
    #[case("\u{2022} Another rule", "Another rule")]
    fn strip_prefix_cases(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(strip_prefix(input), expected);
    }

    #[test]
    fn a_passage_holds_the_prose_of_a_section_rather_than_one_line_each() {
        let lines: Vec<&str> = PRESENT_TENSE_PROSE.split(". ").collect();
        let page = format!(
            "Present Tense\n\n{}\n\n\"Yo hablo \u{2014} I speak\"\n",
            lines.join(".\n")
        );

        let passages = parse_passages(&[page]);

        assert_eq!(
            passages.len(),
            1,
            "the section is one passage, not one per line"
        );
        assert_eq!(passages[0].heading.as_deref(), Some("Present Tense"));
        assert_eq!(passages[0].page, 1);
        assert!(
            passages[0].text.contains("first class")
                && passages[0].text.contains("every present-tense form"),
            "the whole section's prose is in one passage: {}",
            passages[0].text
        );
        assert_eq!(passages[0].examples.len(), 1);
    }

    #[test]
    fn a_paradigm_table_contributes_no_passage_text() {
        let page = format!(
            "An Aidiacht\n\n{PRESENT_TENSE_PROSE}\ndon fhear throm   don chuideachta ghnóthach   don bhean bhocht\nUATHA   IOLRA   GINIDEACH\n"
        );

        let passages = parse_passages(&[page]);

        assert_eq!(passages.len(), 1);
        assert!(
            !passages[0].text.contains("chuideachta") && !passages[0].text.contains("IOLRA"),
            "table rows are dropped, not stored as prose: {}",
            passages[0].text
        );
    }

    #[test]
    fn fragments_shorter_than_a_rule_are_dropped() {
        let page = "Na Forainmnigh Phearsanta\n8.2  Na Forainmnigh\nleathanach 148\n";

        assert!(
            parse_passages(&[page.to_owned()]).is_empty(),
            "a heading, a numbered stub and a running head are not a rule"
        );
    }

    #[test]
    fn each_page_carries_its_own_number() {
        let pages = vec![
            format!("Present Tense\n{PRESENT_TENSE_PROSE}\n"),
            format!("Past Tense\n{PRESENT_TENSE_PROSE}\n"),
        ];

        let passages = parse_passages(&pages);

        assert_eq!(passages.len(), 2);
        assert_eq!(passages[0].page, 1);
        assert_eq!(passages[1].page, 2);
        assert_eq!(passages[1].heading.as_deref(), Some("Past Tense"));
    }

    proptest! {
        #[test]
        fn parse_passages_never_panics(text in "\\PC{0,500}") {
            let _ = parse_passages(&[text]);
        }

        #[test]
        fn strip_prefix_never_panics(line in "\\PC{0,100}") {
            let _ = strip_prefix(&line);
        }

        #[test]
        fn is_section_header_never_panics(line in "\\PC{0,200}") {
            let _ = is_section_header(&line);
        }

        #[test]
        fn is_table_row_never_panics(line in "\\PC{0,200}") {
            let _ = is_table_row(&line);
        }
    }
}
