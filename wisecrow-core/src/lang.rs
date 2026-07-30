//! Shared language-code helpers.

/// Maximum length of a well-formed language code.
pub const MAX_LANG_CODE_LEN: usize = 10;

/// Returns `true` if `code` is a well-formed language code: non-empty, at most
/// [`MAX_LANG_CODE_LEN`] characters, and ASCII alphanumeric. This is a syntactic
/// check only — use `cli::is_supported_language` to check membership of the
/// supported set.
#[must_use]
pub fn is_valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= MAX_LANG_CODE_LEN
        && code.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Longest a single token may be and still count as a word.
///
/// A whitespace tokeniser treats an unsegmented run as one word, so a
/// 95-character blob in the CCMatrix Gaelic release counted 166,950 times and
/// outranked every real word in the language. No natural word form comes near
/// this. It is not tighter because German and Welsh compounds are genuinely
/// long, and a false positive silently discards real vocabulary — the worse
/// failure of the two.
pub const MAX_WORD_CHARS: usize = 64;

/// Returns `true` if `token` is short enough to be a word rather than an
/// unsegmented run of text. See [`MAX_WORD_CHARS`].
#[must_use]
pub fn is_word_length(token: &str) -> bool {
    token.chars().count() <= MAX_WORD_CHARS
}

/// Returns `true` if `phrase` is a single run of text too long to be a word.
///
/// This is the shape that dominates a corpus-derived ranking: with no
/// whitespace to split on, the whole phrase becomes one token, and a phrase
/// repeated across a noisy corpus then outranks every real word in it.
#[must_use]
pub fn is_unsegmented_run(phrase: &str) -> bool {
    let mut tokens = phrase.split_whitespace();
    matches!(
        (tokens.next(), tokens.next()),
        (Some(only), None) if !is_word_length(only)
    )
}

/// Characters that occupy no width and carry no meaning in a phrase: the
/// zero-width space and its relatives, the word joiner, and the byte-order mark
/// used as one.
///
/// Subtitle corpora are full of them, and they are invisible in every tool that
/// would otherwise reveal the problem. The Irish deck served a card reading
/// "She" whose English side ended in U+200B, which no amount of looking at it
/// could explain.
const INVISIBLE_CHARS: [char; 6] = [
    '\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}', '\u{00AD}',
];

/// Returns `true` if `phrase` contains a character that renders as nothing.
///
/// Such a phrase is not merely untidy: it defeats deduplication, because two
/// strings a reader cannot tell apart compare as different.
#[must_use]
pub fn has_invisible_chars(phrase: &str) -> bool {
    phrase.chars().any(|c| INVISIBLE_CHARS.contains(&c))
}

/// Returns `true` if both sides of a pair are the same phrase once normalised.
///
/// A card whose prompt equals its answer teaches nothing, however common the
/// word. These arise where a corpus line was not translated at all, or where the
/// two languages genuinely share a spelling — Irish `An` against English `An`,
/// and Gaelic `Air` against English `Air`, both of which reached real decks.
#[must_use]
pub fn is_degenerate_pair(source: &str, target: &str) -> bool {
    normalise_for_match(source) == normalise_for_match(target)
}

/// Lowercases and strips the edge punctuation that
/// [`crate::frequency::MATCH_TRIM_CHARS`] defines, so that "Tha", "Tha." and
/// "Tha?" compare equal. Shared rather than restated, because the deck
/// deduplicates on the same form.
#[must_use]
pub fn normalise_for_match(phrase: &str) -> String {
    phrase
        .trim_matches(crate::frequency::MATCH_TRIM_CHARS)
        .to_lowercase()
}

/// A writing system. Only the scripts used by the languages in
/// [`crate::cli::SUPPORTED_LANGUAGE_INFO`] are represented.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Script {
    Arabic,
    Armenian,
    Bengali,
    Cyrillic,
    Devanagari,
    Ethiopic,
    Georgian,
    Greek,
    Gujarati,
    Gurmukhi,
    Han,
    Hangul,
    Hebrew,
    Kana,
    Kannada,
    Khmer,
    Lao,
    Latin,
    Malayalam,
    Myanmar,
    Oriya,
    Sinhala,
    Tamil,
    Telugu,
    Thai,
}

/// Classifies a character by writing system.
///
/// Returns `None` for anything carrying no evidence of language: digits,
/// punctuation, whitespace, symbols, emoji and combining marks all belong to no
/// script in particular, and a phrase made only of them is judged on nothing.
#[must_use]
pub fn script_of(c: char) -> Option<Script> {
    if !c.is_alphabetic() {
        return None;
    }
    let script = match c as u32 {
        0x0041..=0x005A | 0x0061..=0x007A | 0x00C0..=0x024F | 0x1E00..=0x1EFF => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F => Script::Cyrillic,
        0x0530..=0x058F => Script::Armenian,
        0x0590..=0x05FF => Script::Hebrew,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => {
            Script::Arabic
        }
        0x0900..=0x097F => Script::Devanagari,
        0x0980..=0x09FF => Script::Bengali,
        0x0A00..=0x0A7F => Script::Gurmukhi,
        0x0A80..=0x0AFF => Script::Gujarati,
        0x0B00..=0x0B7F => Script::Oriya,
        0x0B80..=0x0BFF => Script::Tamil,
        0x0C00..=0x0C7F => Script::Telugu,
        0x0C80..=0x0CFF => Script::Kannada,
        0x0D00..=0x0D7F => Script::Malayalam,
        0x0D80..=0x0DFF => Script::Sinhala,
        0x0E00..=0x0E7F => Script::Thai,
        0x0E80..=0x0EFF => Script::Lao,
        0x1000..=0x109F => Script::Myanmar,
        0x10A0..=0x10FF => Script::Georgian,
        0x1200..=0x137F => Script::Ethiopic,
        0x1780..=0x17FF => Script::Khmer,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Script::Hangul,
        0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9D => Script::Kana,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => Script::Han,
        _ => return None,
    };
    Some(script)
}

/// Returns the writing systems a language is normally written in.
///
/// An empty slice means the code is not one of the supported languages, and
/// carries no expectation — [`is_plausible_script`] then accepts anything,
/// which is the safe direction for a guard that drops data.
#[must_use]
pub fn expected_scripts(lang_code: &str) -> &'static [Script] {
    match lang_code {
        "af" | "ast" | "br" | "bs" | "ca" | "ceb" | "cs" | "cy" | "da" | "de" | "en" | "es"
        | "et" | "ff" | "fi" | "fr" | "fy" | "ga" | "gd" | "gl" | "ha" | "hr" | "ht" | "hu"
        | "id" | "ig" | "ilo" | "is" | "it" | "jv" | "lb" | "lg" | "ln" | "lt" | "lv" | "mg"
        | "ms" | "nl" | "no" | "ns" | "oc" | "pl" | "pt" | "ro" | "sk" | "sl" | "so" | "sq"
        | "ss" | "su" | "sv" | "sw" | "tl" | "tn" | "tr" | "uz" | "vi" | "wo" | "xh" | "yo"
        | "zu" => &[Script::Latin],
        "am" => &[Script::Ethiopic],
        "ar" | "fa" | "ps" | "sd" | "ur" => &[Script::Arabic],
        // Azerbaijani has been written in all three within living memory.
        "az" => &[Script::Latin, Script::Cyrillic, Script::Arabic],
        "ba" | "be" | "bg" | "kk" | "mk" | "mn" | "ru" | "tg" | "uk" => &[Script::Cyrillic],
        "bn" => &[Script::Bengali],
        "el" => &[Script::Greek],
        "gu" => &[Script::Gujarati],
        "he" | "yi" => &[Script::Hebrew],
        "hi" | "mr" | "ne" => &[Script::Devanagari],
        "hy" => &[Script::Armenian],
        "ja" => &[Script::Han, Script::Kana],
        "ka" => &[Script::Georgian],
        "km" => &[Script::Khmer],
        "kn" => &[Script::Kannada],
        "ko" => &[Script::Hangul, Script::Han],
        "lo" => &[Script::Lao],
        "ml" => &[Script::Malayalam],
        "my" => &[Script::Myanmar],
        "or" => &[Script::Oriya],
        "pa" => &[Script::Gurmukhi],
        "si" => &[Script::Sinhala],
        // Serbian is written in both, interchangeably.
        "sr" => &[Script::Cyrillic, Script::Latin],
        "ta" => &[Script::Tamil],
        "te" => &[Script::Telugu],
        "th" => &[Script::Thai],
        "zh" => &[Script::Han],
        _ => &[],
    }
}

/// Returns `true` if `phrase` is plausibly written in `lang_code`'s script.
///
/// Judged on letters alone and by majority: a phrase passes when at least half
/// its script-bearing characters belong to the language's own writing system.
/// Loanwords, proper nouns and quoted foreign text therefore survive, while a
/// phrase wholly in someone else's script does not.
///
/// The web-mined releases need this. Over two thirds of the CCMatrix Scottish
/// Gaelic pairs carry no Gaelic at all, and one 95-character kana blob appears
/// 166,950 times against unrelated English sentences — enough, once ranked from
/// the corpus, to outrank every real Gaelic word.
#[must_use]
pub fn is_plausible_script(phrase: &str, lang_code: &str) -> bool {
    let expected = expected_scripts(lang_code);
    if expected.is_empty() {
        return true;
    }
    let (mut matching, mut total) = (0usize, 0usize);
    for script in phrase.chars().filter_map(script_of) {
        total += 1;
        if expected.contains(&script) {
            matching += 1;
        }
    }
    total == 0 || matching * 2 >= total
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    #[test]
    fn accepts_well_formed_codes() {
        assert!(is_valid_code("en"));
        assert!(is_valid_code("zh"));
        assert!(is_valid_code("ceb"));
    }

    #[test]
    fn rejects_malformed_codes() {
        assert!(!is_valid_code("")); // empty
        assert!(!is_valid_code("toolonglangcode")); // > 10 chars
        assert!(!is_valid_code("en-US")); // non-alphanumeric
        assert!(!is_valid_code("e n")); // whitespace
    }

    #[rstest]
    #[case("madainn", true)]
    #[case("Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch", true)]
    #[case(&"a".repeat(MAX_WORD_CHARS), true)]
    #[case(&"a".repeat(MAX_WORD_CHARS + 1), false)]
    // The blob that outranked every real Gaelic word ran to 95 characters with
    // no space in it; the one still topping the deck afterwards ran to 122.
    #[case(&"うぐぅ".repeat(32), false)]
    fn only_word_length_tokens_count_as_words(#[case] token: &str, #[case] expected: bool) {
        assert_eq!(is_word_length(token), expected);
    }

    #[rstest]
    #[case(&"a".repeat(MAX_WORD_CHARS + 1), true)]
    #[case(&"うぐぅ".repeat(32), true)]
    // Long, but segmented, so every token is judged on its own.
    #[case(
        "tha mi a' dol dhachaigh a-nis oir tha an t-uisge ann agus tha mi sgith",
        false
    )]
    #[case("madainn mhath", false)]
    #[case("madainn", false)]
    #[case("", false)]
    fn only_a_single_over_long_token_is_an_unsegmented_run(
        #[case] phrase: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(is_unsegmented_run(phrase), expected);
    }

    #[test]
    fn every_supported_language_has_an_expected_script() {
        // An omission here would silently disable the guard for that language
        // rather than fail, so the supported set is checked exhaustively.
        for (code, name) in crate::cli::SUPPORTED_LANGUAGE_INFO {
            assert!(
                !expected_scripts(code).is_empty(),
                "{name} ({code}) has no expected script"
            );
        }
    }

    #[test]
    fn classifies_representative_characters() {
        assert_eq!(script_of('a'), Some(Script::Latin));
        assert_eq!(script_of('ò'), Some(Script::Latin));
        assert_eq!(script_of('д'), Some(Script::Cyrillic));
        assert_eq!(script_of('α'), Some(Script::Greek));
        assert_eq!(script_of('א'), Some(Script::Hebrew));
        assert_eq!(script_of('ا'), Some(Script::Arabic));
        assert_eq!(script_of('う'), Some(Script::Kana));
        assert_eq!(script_of('東'), Some(Script::Han));
        assert_eq!(script_of('한'), Some(Script::Hangul));
        assert_eq!(script_of('ก'), Some(Script::Thai));
        // Carries no evidence of any language.
        assert_eq!(script_of('7'), None);
        assert_eq!(script_of('.'), None);
        assert_eq!(script_of(' '), None);
    }

    #[test]
    fn rejects_a_phrase_written_wholly_in_another_script() {
        // The blob that dominated the CCMatrix Gaelic release.
        assert!(!is_plausible_script("うぐぅうぐぅうぐぅうぐぅ", "gd"));
        assert!(!is_plausible_script("Halò a charaid", "ru"));
    }

    #[test]
    fn accepts_a_phrase_carrying_a_foreign_name() {
        // Majority, not purity: a borrowed name must not cost a good pair.
        assert!(is_plausible_script("Chaidh e gu 東京 an-dè", "gd"));
        assert!(is_plausible_script("Он поехал в Tokyo", "ru"));
        // Japanese mixes two scripts as a matter of course.
        assert!(is_plausible_script("東京に行った", "ja"));
    }

    #[test]
    fn accepts_anything_for_a_language_it_knows_nothing_about() {
        assert!(is_plausible_script("うぐぅ", "xx"));
    }

    #[test]
    fn accepts_a_phrase_with_no_letters_at_all() {
        // Nothing to judge on, so the guard abstains rather than dropping.
        assert!(is_plausible_script("1939-1945", "gd"));
    }

    proptest! {
        #[test]
        fn script_classification_never_panics(phrase: String, code in "[a-z]{0,10}") {
            let _ = is_plausible_script(&phrase, &code);
            for c in phrase.chars() {
                let _ = script_of(c);
            }
        }
    }

    #[rstest]
    #[case("She\u{200B}", true, "zero-width space")]
    #[case("T\u{FEFF}\u{00E1}", true, "byte-order mark used mid-word")]
    #[case("soft\u{00AD}hyphen", true, "soft hyphen")]
    #[case("She", false, "nothing invisible")]
    #[case("Dìreach", false, "accented Latin is visible")]
    fn invisible_characters_are_detected(
        #[case] phrase: &str,
        #[case] expected: bool,
        #[case] why: &str,
    ) {
        assert_eq!(has_invisible_chars(phrase), expected, "{why}");
    }

    #[rstest]
    #[case("An.", "An.", true, "identical but for nothing")]
    #[case("Air.", "Air", true, "edge punctuation is normalised away")]
    #[case("YES", "yes", true, "case is normalised away")]
    #[case("Yes.", "Tha.", false, "a genuine translation")]
    fn degenerate_pairs_are_recognised(
        #[case] source: &str,
        #[case] target: &str,
        #[case] expected: bool,
        #[case] why: &str,
    ) {
        assert_eq!(is_degenerate_pair(source, target), expected, "{why}");
    }
}
