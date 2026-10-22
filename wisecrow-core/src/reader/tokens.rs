//! Word spans over a translated sentence.
//!
//! A span names a word by its byte offsets in the stored sentence, so the
//! page can make the word tappable and still render the sentence exactly as
//! it was stored; its normalised key is what the word's note is filed under.

use unicode_segmentation::UnicodeSegmentation;

use crate::frequency::MATCH_TRIM_CHARS;

/// Longest normalised key stored, in characters.
pub const MAX_TOKEN_CHARS: usize = 64;

/// One word of a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Byte offset of the word's first byte.
    pub start: usize,
    /// Byte offset one past the word's last byte.
    pub end: usize,
    /// The key the word's note is filed under.
    pub normalised: String,
}

impl Span {
    /// The word as it is written in `text`.
    #[must_use]
    pub fn surface<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start..self.end]
    }
}

/// The words of `text`, in order, each with a letter in it.
#[must_use]
pub fn spans(text: &str) -> Vec<Span> {
    text.unicode_word_indices()
        .filter(|(_, word)| word.chars().any(char::is_alphabetic))
        .filter_map(|(start, word)| {
            let normalised = normalise(word);
            if normalised.is_empty() || normalised.chars().count() > MAX_TOKEN_CHARS {
                return None;
            }
            Some(Span {
                start,
                end: start.saturating_add(word.len()),
                normalised,
            })
        })
        .collect()
}

/// The lookup key for a word: the typographic apostrophe folded to `'`,
/// the corpus edge characters trimmed, and the result lowercased.
#[must_use]
pub fn normalise(word: &str) -> String {
    word.replace('\u{2019}', "'")
        .trim_matches(MATCH_TRIM_CHARS)
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surfaces(text: &str) -> Vec<(&str, String)> {
        spans(text)
            .into_iter()
            .map(|span| (span.surface(text), span.normalised))
            .collect()
    }

    /// The sentence rebuilt from its spans and the text between them.
    fn rebuilt(text: &str) -> String {
        let mut out = String::new();
        let mut cursor = 0;
        for span in spans(text) {
            out.push_str(&text[cursor..span.start]);
            out.push_str(span.surface(text));
            cursor = span.end;
        }
        out.push_str(&text[cursor..]);
        out
    }

    const FIXTURES: &[&str] = &[
        "¿Dónde está l'homme?",
        "‘I don’t know,’ he said.",
        "Tha a' chaileag ann an Dùn Èideann.",
        "Robert Louis Stevenson was born in 1850 in Edinburgh.",
        "«Il était une fois…» dit-il — et puis rien.",
        "",
    ];

    #[test]
    fn a_span_covers_the_word_and_keeps_inner_apostrophes() {
        assert_eq!(
            surfaces("¿Dónde está l'homme?"),
            [
                ("Dónde", "dónde".to_owned()),
                ("está", "está".to_owned()),
                ("l'homme", "l'homme".to_owned()),
            ]
        );
        assert_eq!(
            surfaces("‘I don’t know,’ he said."),
            [
                ("I", "i".to_owned()),
                ("don’t", "don't".to_owned()),
                ("know", "know".to_owned()),
                ("he", "he".to_owned()),
                ("said", "said".to_owned()),
            ]
        );
    }

    #[test]
    fn an_apostrophe_after_a_word_is_left_outside_it() {
        let words: Vec<String> = spans("Tha a' chaileag ann.")
            .into_iter()
            .map(|span| span.normalised)
            .collect();

        assert_eq!(words, ["tha", "a", "chaileag", "ann"]);
    }

    #[test]
    fn numbers_are_not_words_to_look_up() {
        let words: Vec<String> = spans("He was born in 1850.")
            .into_iter()
            .map(|span| span.normalised)
            .collect();

        assert_eq!(words, ["he", "was", "born", "in"]);
    }

    #[test]
    fn every_fixture_is_rebuilt_byte_for_byte_and_gaps_hold_no_letters() {
        for text in FIXTURES {
            assert_eq!(&rebuilt(text), text);
            let mut cursor = 0;
            for span in spans(text) {
                assert!(
                    !text[cursor..span.start].chars().any(char::is_alphabetic),
                    "a letter fell between spans in {text:?}"
                );
                assert!(text.is_char_boundary(span.start) && text.is_char_boundary(span.end));
                cursor = span.end;
            }
        }
    }

    #[test]
    fn an_overlong_word_is_not_stored() {
        let long = "a".repeat(MAX_TOKEN_CHARS + 1);

        assert!(spans(&long).is_empty());
        assert_eq!(spans(&"a".repeat(MAX_TOKEN_CHARS)).len(), 1);
    }

    #[test]
    fn the_key_follows_the_corpus_trim_rule() {
        assert_eq!(normalise("Hola"), "hola");
        assert_eq!(normalise("l’homme"), "l'homme");
        assert_eq!(normalise("'quoted'"), "quoted");
    }
}
