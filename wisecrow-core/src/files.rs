use crate::errors::WisecrowError;
use crate::Langs;
use derive_more::Display;
use url::Url;

const ALL_CORPORA: [Corpus; 5] = [
    Corpus::OpenSubtitles,
    Corpus::CcAligned,
    Corpus::CcMatrix,
    Corpus::ParaCrawl,
    Corpus::Nllb,
];

#[derive(Debug, Display, Clone, Copy, PartialEq, Eq)]
pub enum Corpus {
    #[display("OpenSubtitles")]
    OpenSubtitles,
    #[display("CcAligned")]
    CcAligned,
    #[display("CcMatrix")]
    CcMatrix,
    #[display("ParaCrawl")]
    ParaCrawl,
    #[display("Nllb")]
    Nllb,
}

impl Corpus {
    /// Resolves this corpus's exact tags for the requested application pair.
    ///
    /// # Errors
    ///
    /// Rejects unsupported or ambiguous language pairs.
    pub fn ingest_languages(self, langs: &Langs) -> Result<IngestLanguages, WisecrowError> {
        let mapped = |code: &str| {
            let tag = if self == Self::OpenSubtitles && code == "zh" {
                "zh_CN"
            } else {
                code
            };
            IngestLanguage::new(code, tag)
        };
        IngestLanguages::new(mapped(langs.native_code())?, mapped(langs.foreign_code())?)
    }

    const fn url_root(self) -> &'static str {
        match self {
            Self::OpenSubtitles => "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2024/",
            Self::CcAligned => "https://object.pouta.csc.fi/OPUS-CCAligned/v1/",
            Self::CcMatrix => "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/",
            Self::ParaCrawl => "https://object.pouta.csc.fi/OPUS-ParaCrawl/v9/",
            Self::Nllb => "https://object.pouta.csc.fi/OPUS-NLLB/v1/",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::OpenSubtitles => "OpenSubtitles",
            Self::CcAligned => "CCAligned",
            Self::CcMatrix => "CCMatrix",
            Self::ParaCrawl => "ParaCrawl",
            Self::Nllb => "NLLB",
        }
    }
}

impl TryFrom<&str> for Corpus {
    type Error = WisecrowError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "open_subtitles" => Ok(Self::OpenSubtitles),
            "cc_aligned" => Ok(Self::CcAligned),
            "cc_matrix" => Ok(Self::CcMatrix),
            "paracrawl" => Ok(Self::ParaCrawl),
            "nllb" => Ok(Self::Nllb),
            other => Err(WisecrowError::InvalidInput(format!(
                "Unknown corpus: {other}. Valid: open_subtitles, cc_aligned, cc_matrix, paracrawl, nllb"
            ))),
        }
    }
}

#[derive(Debug, Display, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    #[display("GzCompressed")]
    GzCompressed,
    #[display("ZipCompressed")]
    ZipCompressed,
}

/// Supported application language and its exact corpus tag.
#[derive(Debug, Clone)]
pub struct IngestLanguage {
    canonical: String,
    corpus: String,
}

impl IngestLanguage {
    /// Associates a supported application language with a validated TMX tag.
    ///
    /// # Errors
    ///
    /// Rejects unsupported application codes and malformed source tags.
    pub fn new(
        canonical: impl Into<String>,
        corpus: impl Into<String>,
    ) -> Result<Self, WisecrowError> {
        let canonical = canonical.into();
        let corpus = corpus.into();
        let bytes = corpus.as_bytes();
        let valid = (1..=16).contains(&bytes.len())
            && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
            && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
            && bytes
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
        if !crate::cli::is_supported_language(&canonical) || !valid {
            return Err(WisecrowError::InvalidInput(
                "Invalid ingest language mapping".into(),
            ));
        }
        Ok(Self { canonical, corpus })
    }

    /// Application code used by storage, tokenization and script validation.
    #[must_use]
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    /// Exact source tag expected in the selected archive.
    #[must_use]
    pub fn corpus(&self) -> &str {
        &self.corpus
    }
}

/// Unambiguous native and foreign corpus-language mappings.
#[derive(Debug, Clone)]
pub struct IngestLanguages {
    native: IngestLanguage,
    foreign: IngestLanguage,
}

impl IngestLanguages {
    /// Constructs an unambiguous native/foreign mapping.
    ///
    /// # Errors
    ///
    /// Rejects identical application languages or identical source tags.
    pub fn new(native: IngestLanguage, foreign: IngestLanguage) -> Result<Self, WisecrowError> {
        if native.canonical == foreign.canonical || native.corpus == foreign.corpus {
            return Err(WisecrowError::InvalidInput(
                "Ambiguous ingest language pair".into(),
            ));
        }
        Ok(Self { native, foreign })
    }

    /// Uses application codes as the source tags for a local file.
    ///
    /// # Errors
    ///
    /// Rejects unsupported or identical application languages.
    pub fn identity(native: &str, foreign: &str) -> Result<Self, WisecrowError> {
        Self::new(
            IngestLanguage::new(native, native)?,
            IngestLanguage::new(foreign, foreign)?,
        )
    }

    /// Native application code and its source tag.
    #[must_use]
    pub fn native(&self) -> &IngestLanguage {
        &self.native
    }

    /// Foreign application code and its source tag.
    #[must_use]
    pub fn foreign(&self) -> &IngestLanguage {
        &self.foreign
    }
}

#[derive(Debug, Display, Clone)]
#[display("{} -> {}", corpus, file_name)]
pub struct LanguageFileInfo {
    pub corpus: Corpus,
    pub target_location: String,
    pub file_name: String,
    pub compressed: Compression,
    /// Language mapping used to select and parse the archive.
    pub languages: IngestLanguages,
}

impl LanguageFileInfo {
    #[must_use]
    pub fn decompressed_name(&self) -> String {
        match self.compressed {
            Compression::GzCompressed => self
                .file_name
                .strip_suffix(".gz")
                .unwrap_or(&self.file_name)
                .to_owned(),
            Compression::ZipCompressed => self
                .file_name
                .strip_suffix(".zip")
                .unwrap_or(&self.file_name)
                .to_owned(),
        }
    }
}

#[derive(Debug)]
pub struct LanguageFiles {
    pub files: Vec<LanguageFileInfo>,
}

impl LanguageFiles {
    fn files_for_corpus(
        corpus: Corpus,
        langs: &Langs,
    ) -> Result<Vec<LanguageFileInfo>, WisecrowError> {
        let languages = corpus.ingest_languages(langs)?;
        let (native, foreign) = (languages.native().corpus(), languages.foreign().corpus());
        let (lo, hi) = if native < foreign {
            (native, foreign)
        } else {
            (foreign, native)
        };
        let archive = format!("{lo}-{hi}.tmx.gz");
        let mut url = Url::parse(corpus.url_root())?;
        url.path_segments_mut()
            .map_err(|_| {
                WisecrowError::InvalidInput("Corpus URL cannot contain path segments".into())
            })?
            .pop_if_empty()
            .push("tmx")
            .push(&archive);

        // Only the TMX release carries sentence text. The sibling `xml/` release
        // is a cesAlign link file whose <link> elements reference sentences held
        // in separate monolingual archives, so parsing it yields nothing.
        Ok(vec![LanguageFileInfo {
            corpus,
            target_location: url.into(),
            file_name: format!("{}_{}.tmx.gz", langs.foreign_code(), corpus.label()),
            compressed: Compression::GzCompressed,
            languages,
        }])
    }

    /// Creates a [`LanguageFiles`] for `langs`, optionally filtered by corpus.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError`] for invalid language mappings or corpus URLs.
    pub fn new(langs: &Langs, corpora: Option<&[Corpus]>) -> Result<Self, WisecrowError> {
        let active_corpora = corpora.unwrap_or(&ALL_CORPORA);
        let mut files = Vec::with_capacity(active_corpora.len());
        for &corpus in active_corpora {
            files.extend(Self::files_for_corpus(corpus, langs)?);
        }

        Ok(Self { files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    fn test_langs() -> crate::Langs {
        crate::Langs::new("en", "es")
    }

    #[test]
    fn subtitle_chinese_mapping() -> Result<(), Box<dyn std::error::Error>> {
        for (native, foreign) in [("en", "zh"), ("zh", "en")] {
            let langs = Langs::new(native, foreign);
            let files = LanguageFiles::new(&langs, Some(&[Corpus::OpenSubtitles]))?;
            let file = files.files.first().ok_or("missing archive")?;
            assert!(file.target_location.ends_with("/tmx/en-zh_CN.tmx.gz"));
            assert_eq!(file.languages.native().canonical(), native);
            assert_eq!(file.languages.foreign().canonical(), foreign);
        }
        let other = LanguageFiles::new(&Langs::new("en", "zh"), Some(&[Corpus::ParaCrawl]))?;
        assert!(other.files[0]
            .target_location
            .ends_with("/tmx/en-zh.tmx.gz"));
        Ok(())
    }

    #[rstest]
    #[case("")]
    #[case("../zh")]
    #[case("zh/CN")]
    #[case("_zh")]
    #[case("zh_")]
    #[case("zh CN")]
    #[case("abcdefghijklmnopq")]
    fn invalid_source_tags_are_rejected(#[case] tag: &str) {
        assert!(IngestLanguage::new("zh", tag).is_err());
    }

    #[rstest]
    #[case("", "en", "zh", "zh_CN")]
    #[case("zh_CN", "en", "zh", "zh_CN")]
    #[case("en", "en", "en", "eng")]
    #[case("en", "same", "zh", "same")]
    fn invalid_language_pairs_are_rejected(
        #[case] native: &str,
        #[case] native_tag: &str,
        #[case] foreign: &str,
        #[case] foreign_tag: &str,
    ) {
        let mapping = IngestLanguage::new(native, native_tag).and_then(|native| {
            IngestLanguages::new(native, IngestLanguage::new(foreign, foreign_tag)?)
        });
        assert!(mapping.is_err());
    }

    #[test]
    fn default_generates_all_corpora() {
        let files = LanguageFiles::new(&test_langs(), None).unwrap();
        assert_eq!(files.files.len(), 5);
    }

    #[test]
    fn only_tmx_releases_are_requested() {
        let files = LanguageFiles::new(&test_langs(), None).unwrap();
        for file in &files.files {
            assert!(
                file.target_location.contains("/tmx/"),
                "unexpected non-TMX release: {}",
                file.target_location
            );
        }
    }

    #[test]
    fn urls_use_correct_base() {
        let files = LanguageFiles::new(&test_langs(), Some(&[Corpus::OpenSubtitles])).unwrap();
        for file in &files.files {
            assert!(file
                .target_location
                .starts_with("https://object.pouta.csc.fi/OPUS-OpenSubtitles/"));
        }
    }

    #[rstest]
    #[case("open_subtitles", true)]
    #[case("cc_aligned", true)]
    #[case("cc_matrix", true)]
    #[case("paracrawl", true)]
    #[case("nllb", true)]
    #[case("invalid", false)]
    fn corpus_try_from(#[case] input: &str, #[case] is_ok: bool) {
        assert_eq!(Corpus::try_from(input).is_ok(), is_ok);
    }

    #[rstest]
    #[case(Corpus::OpenSubtitles, "OPUS-OpenSubtitles/v2024")]
    #[case(Corpus::CcAligned, "OPUS-CCAligned/v1")]
    #[case(Corpus::CcMatrix, "OPUS-CCMatrix/v1")]
    #[case(Corpus::ParaCrawl, "OPUS-ParaCrawl/v9")]
    #[case(Corpus::Nllb, "OPUS-NLLB/v1")]
    fn corpus_urls_pin_the_expected_release(#[case] corpus: Corpus, #[case] expected: &str) {
        let files = LanguageFiles::new(&test_langs(), Some(&[corpus])).unwrap();
        assert!(files.files[0].target_location.contains(expected));
    }

    #[rstest]
    #[case("corpus.tmx.gz", Compression::GzCompressed, "corpus.tmx")]
    #[case("corpus.xml.gz", Compression::GzCompressed, "corpus.xml")]
    #[case("archive.zip", Compression::ZipCompressed, "archive")]
    #[case("no_suffix", Compression::GzCompressed, "no_suffix")]
    fn decompressed_name_cases(
        #[case] file_name: &str,
        #[case] compression: Compression,
        #[case] expected: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let info = LanguageFileInfo {
            file_name: file_name.to_owned(),
            compressed: compression,
            target_location: "https://example.com".to_owned(),
            corpus: Corpus::OpenSubtitles,
            languages: IngestLanguages::identity("en", "es")?,
        };
        assert_eq!(info.decompressed_name(), expected);
        Ok(())
    }

    proptest! {
        #[test]
        fn source_tags_reject_path_and_invisible_insertions(prefix in "[A-Za-z0-9][A-Za-z0-9_-]{0,14}") {
            let tag = format!("{prefix}x");
            prop_assert!(IngestLanguage::new("zh", &tag).is_ok());
            for suffix in ["/", "\\", "\u{200b}", ".", " "] {
                let invalid = format!("{tag}{suffix}");
                prop_assert!(IngestLanguage::new("zh", invalid).is_err());
            }
        }

        #[test]
        fn corpus_try_from_arbitrary(s in "\\PC{0,30}") {
            let result = Corpus::try_from(s.as_str());
            match s.as_str() {
                "open_subtitles" => prop_assert_eq!(result.unwrap(), Corpus::OpenSubtitles),
                "cc_aligned" => prop_assert_eq!(result.unwrap(), Corpus::CcAligned),
                "cc_matrix" => prop_assert_eq!(result.unwrap(), Corpus::CcMatrix),
                "paracrawl" => prop_assert_eq!(result.unwrap(), Corpus::ParaCrawl),
                "nllb" => prop_assert_eq!(result.unwrap(), Corpus::Nllb),
                _ => prop_assert!(result.is_err()),
            }
        }
    }
}
