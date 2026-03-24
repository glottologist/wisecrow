use std::path::Path;

use crate::errors::WisecrowError;

const BULLET_PREFIXES: &[&str] = &["- ", "• ", "* ", "– "];
const MIN_RULE_LINE_LENGTH: usize = 10;
const MAX_HEADER_LENGTH: usize = 100;
const MAX_HEADER_WORDS: usize = 8;

#[derive(Debug, Clone)]
pub struct GrammarContent {
    pub sections: Vec<GrammarSection>,
}

#[derive(Debug, Clone)]
pub struct GrammarSection {
    pub title: Option<String>,
    pub rules: Vec<String>,
    pub examples: Vec<ExampleSentence>,
}

#[derive(Debug, Clone)]
pub struct ExampleSentence {
    pub text: String,
    pub translation: Option<String>,
}

/// Extracts structured grammar content from a PDF file.
///
/// Parses the PDF text, detects section headers, rules (numbered or
/// bulleted items), and example sentences (quoted or indented text).
///
/// # Errors
///
/// Returns an error if the PDF cannot be read or parsed.
pub fn extract(path: &Path) -> Result<GrammarContent, WisecrowError> {
    let canonical = path
        .canonicalize()
        .map_err(|e| WisecrowError::PdfExtractionError(format!("Invalid path: {e}")))?;

    let text = pdf_extract::extract_text(&canonical)
        .map_err(|e| WisecrowError::PdfExtractionError(format!("PDF extraction failed: {e}")))?;

    let sections = parse_sections(&text);

    if sections.is_empty() {
        return Err(WisecrowError::PdfExtractionError(
            "No grammar content found in PDF".to_owned(),
        ));
    }

    Ok(GrammarContent { sections })
}

fn parse_sections(text: &str) -> Vec<GrammarSection> {
    let mut sections = Vec::new();
    let mut current_title: Option<String> = None;
    let mut current_rules: Vec<String> = Vec::new();
    let mut current_examples: Vec<ExampleSentence> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        if is_section_header(trimmed) {
            if !current_rules.is_empty() || !current_examples.is_empty() {
                sections.push(GrammarSection {
                    title: current_title.take(),
                    rules: std::mem::take(&mut current_rules),
                    examples: std::mem::take(&mut current_examples),
                });
            }
            current_title = Some(trimmed.to_owned());
            continue;
        }

        if is_example_sentence(trimmed) {
            let (text, translation) = split_example(trimmed);
            current_examples.push(ExampleSentence { text, translation });
        } else if is_numbered_rule(trimmed) || is_bulleted_rule(trimmed) {
            let rule = strip_prefix(trimmed);
            if !rule.is_empty() {
                current_rules.push(rule);
            }
        } else if trimmed.len() > MIN_RULE_LINE_LENGTH {
            current_rules.push(trimmed.to_owned());
        }
    }

    if !current_rules.is_empty() || !current_examples.is_empty() {
        sections.push(GrammarSection {
            title: current_title,
            rules: current_rules,
            examples: current_examples,
        });
    }

    sections
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

fn is_example_sentence(line: &str) -> bool {
    (line.starts_with('"') || line.starts_with('\u{201C}'))
        || (line.starts_with("e.g.") || line.starts_with("E.g."))
        || (line.starts_with("Example:") || line.starts_with("Ex:"))
}

fn is_numbered_rule(line: &str) -> bool {
    let mut chars = line.chars();
    let first = chars.next().unwrap_or(' ');
    if !first.is_ascii_digit() {
        return false;
    }
    for ch in chars {
        if ch == '.' || ch == ')' {
            return true;
        }
        if !ch.is_ascii_digit() {
            return false;
        }
    }
    false
}

fn is_bulleted_rule(line: &str) -> bool {
    BULLET_PREFIXES.iter().any(|p| line.starts_with(p))
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

    if let Some((text, translation)) = cleaned.split_once(" — ") {
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
    #[case("1. Use the present tense", true)]
    #[case("12) Another rule", true)]
    #[case("Not a rule", false)]
    fn numbered_rule_detected(#[case] input: &str, #[case] expected: bool) {
        assert_eq!(is_numbered_rule(input), expected);
    }

    #[rstest]
    #[case("- A rule", true)]
    #[case("• Another rule", true)]
    #[case("Not a rule", false)]
    fn bulleted_rule_detected(#[case] input: &str, #[case] expected: bool) {
        assert_eq!(is_bulleted_rule(input), expected);
    }

    #[rstest]
    #[case("\"Hola, mundo\"", true)]
    #[case("e.g. Hola", true)]
    #[case("Example: Hello", true)]
    #[case("This is a normal line", false)]
    fn example_sentence_detected(#[case] input: &str, #[case] expected: bool) {
        assert_eq!(is_example_sentence(input), expected);
    }

    #[rstest]
    #[case("\"Hola — Hello\"", "Hola", Some("Hello"))]
    #[case("\"Hola, mundo\"", "Hola, mundo", None)]
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
    #[case("• Another rule", "Another rule")]
    fn strip_prefix_cases(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(strip_prefix(input), expected);
    }

    #[test]
    fn parse_sections_from_text() {
        let text = "Present Tense\n\n1. Regular verbs end in -ar, -er, -ir\n2. Conjugate by removing the ending\n\n\"Yo hablo — I speak\"\n\nPast Tense\n\n1. Add -é, -aste, -ó endings\n";
        let sections = parse_sections(text);
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].title.as_deref(), Some("Present Tense"));
        assert_eq!(sections[0].rules.len(), 2);
        assert_eq!(sections[0].examples.len(), 1);
        assert_eq!(sections[1].title.as_deref(), Some("Past Tense"));
    }

    proptest! {
        #[test]
        fn parse_sections_never_panics(text in "\\PC{0,500}") {
            let _ = parse_sections(&text);
        }

        #[test]
        fn strip_prefix_never_panics(line in "\\PC{0,100}") {
            let _ = strip_prefix(&line);
        }

        #[test]
        fn is_section_header_never_panics(line in "\\PC{0,200}") {
            let _ = is_section_header(&line);
        }
    }
}
