use std::path::Path;

use crate::errors::WisecrowError;

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
        } else if trimmed.len() > 10 {
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
    if line.len() > 100 {
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
    if words.len() > 8 || words.is_empty() {
        return false;
    }
    // Title case heuristic: all words start with uppercase (except short prepositions)
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

const BULLET_PREFIXES: &[&str] = &["- ", "• ", "* ", "– "];

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
            return line[pos.saturating_add(2)..].trim().to_owned();
        }
    }
    if let Some(pos) = line.find(") ") {
        if pos < 5 && line[..pos].chars().all(|c| c.is_ascii_digit()) {
            return line[pos.saturating_add(2)..].trim().to_owned();
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

    #[test]
    fn section_header_detected() {
        assert!(is_section_header("Present Tense"));
        assert!(is_section_header("Chapter 1"));
        assert!(is_section_header("Verbs of Motion"));
        assert!(!is_section_header(
            "This is a long sentence that explains grammar rules in detail."
        ));
        assert!(!is_section_header("The cat sat on the mat"));
    }

    #[test]
    fn numbered_rule_detected() {
        assert!(is_numbered_rule("1. Use the present tense"));
        assert!(is_numbered_rule("12) Another rule"));
        assert!(!is_numbered_rule("Not a rule"));
    }

    #[test]
    fn bulleted_rule_detected() {
        assert!(is_bulleted_rule("- A rule"));
        assert!(is_bulleted_rule("• Another rule"));
        assert!(!is_bulleted_rule("Not a rule"));
    }

    #[test]
    fn example_sentence_detected() {
        assert!(is_example_sentence("\"Hola, mundo\""));
        assert!(is_example_sentence("e.g. Hola"));
        assert!(is_example_sentence("Example: Hello"));
        assert!(!is_example_sentence("This is a normal line"));
    }

    #[test]
    fn split_example_with_translation() {
        let (text, trans) = split_example("\"Hola — Hello\"");
        assert_eq!(text, "Hola");
        assert_eq!(trans.unwrap(), "Hello");
    }

    #[test]
    fn split_example_without_translation() {
        let (text, trans) = split_example("\"Hola, mundo\"");
        assert_eq!(text, "Hola, mundo");
        assert!(trans.is_none());
    }

    #[test]
    fn strip_numbered_prefix() {
        assert_eq!(strip_prefix("1. Use the present"), "Use the present");
        assert_eq!(strip_prefix("12) Another rule"), "Another rule");
    }

    #[test]
    fn strip_bullet_prefix() {
        assert_eq!(strip_prefix("- A rule"), "A rule");
        assert_eq!(strip_prefix("• Another rule"), "Another rule");
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
}
