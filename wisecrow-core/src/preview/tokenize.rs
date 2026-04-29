use crate::errors::WisecrowError;

pub trait Tokenizer: Send + Sync {
    fn tokenize(&self, text: &str) -> Vec<String>;
}

pub struct WhitespaceTokenizer;

impl Tokenizer for WhitespaceTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        text.split_whitespace()
            .map(|s| {
                s.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|s| !s.is_empty())
            .collect()
    }
}

const WHITESPACE_LANGS: &[&str] = &[
    "af", "am", "ar", "ast", "az", "ba", "be", "bg", "bn", "br", "bs", "ca", "ceb", "cs", "cy",
    "da", "de", "el", "en", "es", "et", "fa", "ff", "fi", "fr", "fy", "ga", "gd", "gl", "gu", "ha",
    "he", "hi", "hr", "ht", "hu", "hy", "id", "ig", "ilo", "is", "it", "jv", "ka", "kk", "kn",
    "ko", "lb", "lg", "ln", "lt", "lv", "mg", "mk", "ml", "mn", "mr", "ms", "ne", "nl", "no", "ns",
    "oc", "or", "pa", "pl", "ps", "pt", "ro", "ru", "sd", "si", "sk", "sl", "so", "sq", "sr", "ss",
    "su", "sv", "sw", "ta", "te", "tg", "tl", "tn", "tr", "uk", "ur", "uz", "vi", "wo", "xh", "yi",
    "yo", "zu",
];

const UNSUPPORTED: &[&str] = &["km", "lo", "my"];

/// Returns a tokeniser appropriate for the given language code.
///
/// # Errors
///
/// Returns `WisecrowError::UnsupportedLanguage` if the language has no
/// tokeniser available (e.g. Khmer, Lao, Burmese, Thai — all of which lack
/// reliable whitespace tokenisation and have no integrated segmenter yet).
pub fn for_language(lang_code: &str) -> Result<Box<dyn Tokenizer>, WisecrowError> {
    if lang_code == "zh" {
        return Ok(Box::new(JiebaTokenizer::new()));
    }
    if lang_code == "ja" {
        return Ok(Box::new(LinderaTokenizer::new()?));
    }
    if lang_code == "th" {
        return Ok(Box::new(KhamThaiTokenizer::new()));
    }
    if WHITESPACE_LANGS.contains(&lang_code) {
        return Ok(Box::new(WhitespaceTokenizer));
    }
    if UNSUPPORTED.contains(&lang_code) {
        return Err(WisecrowError::UnsupportedLanguage(format!(
            "{lang_code}: tokeniser not yet implemented for this language"
        )));
    }
    Err(WisecrowError::UnsupportedLanguage(format!(
        "{lang_code}: not in tokenizer capability map"
    )))
}

pub struct JiebaTokenizer {
    inner: jieba_rs::Jieba,
}

impl JiebaTokenizer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: jieba_rs::Jieba::new(),
        }
    }
}

impl Default for JiebaTokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tokenizer for JiebaTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        self.inner
            .cut(text, false)
            .into_iter()
            .map(|s| s.to_lowercase())
            .filter(|s| !s.trim().is_empty())
            .collect()
    }
}

pub struct LinderaTokenizer {
    inner: lindera::tokenizer::Tokenizer,
}

impl LinderaTokenizer {
    /// Creates a Lindera tokenizer with the IPADIC dictionary.
    ///
    /// # Errors
    ///
    /// Returns an error if the dictionary cannot be loaded.
    pub fn new() -> Result<Self, WisecrowError> {
        use lindera::dictionary::{load_embedded_dictionary, DictionaryKind};
        use lindera::mode::Mode;
        use lindera::segmenter::Segmenter;
        use lindera::tokenizer::Tokenizer as LTokenizer;

        let dictionary = load_embedded_dictionary(DictionaryKind::IPADIC).map_err(|e| {
            WisecrowError::UnsupportedLanguage(format!("lindera dictionary load failed: {e}"))
        })?;
        let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
        Ok(Self {
            inner: LTokenizer::new(segmenter),
        })
    }
}

impl Tokenizer for LinderaTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        match self.inner.tokenize(text) {
            Ok(tokens) => tokens
                .into_iter()
                .map(|t| t.surface.to_lowercase())
                .filter(|s| !s.trim().is_empty())
                .collect(),
            Err(e) => {
                tracing::warn!("Lindera tokenize failed: {e}");
                Vec::new()
            }
        }
    }
}

pub struct KhamThaiTokenizer {
    inner: kham_core::Tokenizer,
}

impl KhamThaiTokenizer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: kham_core::Tokenizer::new(),
        }
    }
}

impl Default for KhamThaiTokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tokenizer for KhamThaiTokenizer {
    fn tokenize(&self, text: &str) -> Vec<String> {
        self.inner
            .segment(text)
            .into_iter()
            .map(|t| t.text.to_lowercase())
            .filter(|s| !s.trim().is_empty() && !s.chars().all(|c| !c.is_alphanumeric()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    #[test]
    fn whitespace_tokenizer_lowercases_and_strips_punct() {
        let t = WhitespaceTokenizer;
        assert_eq!(
            t.tokenize("Hola, ¿cómo estás?"),
            vec!["hola", "cómo", "estás"]
        );
    }

    #[rstest]
    #[case("en")]
    #[case("es")]
    #[case("ru")]
    #[case("ar")]
    fn whitespace_langs_return_whitespace_tokenizer(#[case] lang: &str) {
        let t = for_language(lang).expect("lookup failed");
        assert!(!t.tokenize("a b c").is_empty());
    }

    #[rstest]
    #[case("km")]
    #[case("lo")]
    #[case("my")]
    fn unsupported_langs_error(#[case] lang: &str) {
        assert!(matches!(
            for_language(lang),
            Err(WisecrowError::UnsupportedLanguage(_))
        ));
    }

    #[rstest]
    #[case("xx")]
    #[case("?invalid")]
    fn unknown_langs_error(#[case] lang: &str) {
        assert!(matches!(
            for_language(lang),
            Err(WisecrowError::UnsupportedLanguage(_))
        ));
    }

    #[test]
    fn th_returns_kham_tokens() {
        let t = for_language("th").expect("th tokenizer");
        let tokens = t.tokenize("ฉันเรียนภาษาไทย");
        assert!(!tokens.is_empty(), "kham should produce tokens");
    }

    #[test]
    fn zh_returns_jieba_tokens() {
        let t = for_language("zh").expect("zh tokenizer");
        let tokens = t.tokenize("我爱学中文");
        assert!(!tokens.is_empty(), "jieba should produce tokens");
    }

    #[test]
    fn ja_returns_lindera_tokens() {
        let t = for_language("ja").expect("ja tokenizer");
        let tokens = t.tokenize("私は日本語を勉強します");
        assert!(!tokens.is_empty(), "lindera should produce tokens");
    }

    proptest! {
        #[test]
        fn whitespace_tokens_have_no_whitespace(text in ".*") {
            let t = WhitespaceTokenizer;
            for tok in t.tokenize(&text) {
                prop_assert!(!tok.contains(char::is_whitespace));
            }
        }
    }
}
