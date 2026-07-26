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

#[derive(Debug, Display, Clone)]
#[display("{} -> {}", corpus, file_name)]
pub struct LanguageFileInfo {
    pub corpus: Corpus,
    pub target_location: String,
    pub file_name: String,
    pub compressed: Compression,
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
        native: &str,
        foreign: &str,
    ) -> Result<Vec<LanguageFileInfo>, WisecrowError> {
        let base = Url::parse(corpus.url_root())?;
        let label = corpus.label();

        let (lo, hi) = if native < foreign {
            (native, foreign)
        } else {
            (foreign, native)
        };
        let tmx_url = base.join(&format!("tmx/{lo}-{hi}.tmx.gz"))?;

        // Only the TMX release carries sentence text. The sibling `xml/` release
        // is a cesAlign link file whose <link> elements reference sentences held
        // in separate monolingual archives, so parsing it yields nothing.
        Ok(vec![LanguageFileInfo {
            corpus,
            target_location: tmx_url.into(),
            file_name: format!("{foreign}_{label}.tmx.gz"),
            compressed: Compression::GzCompressed,
        }])
    }

    /// Creates a [`LanguageFiles`] for `langs`, optionally filtered by corpus.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError`] if any corpus URL cannot be constructed.
    pub fn new(langs: &Langs, corpora: Option<&[Corpus]>) -> Result<Self, WisecrowError> {
        let active_corpora = corpora.unwrap_or(&ALL_CORPORA);
        let native = langs.native_code();
        let foreign = langs.foreign_code();

        let mut files = Vec::with_capacity(active_corpora.len());
        for &corpus in active_corpora {
            files.extend(Self::files_for_corpus(corpus, native, foreign)?);
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
    ) {
        let info = LanguageFileInfo {
            file_name: file_name.to_owned(),
            compressed: compression,
            target_location: "https://example.com".to_owned(),
            corpus: Corpus::OpenSubtitles,
        };
        assert_eq!(info.decompressed_name(), expected);
    }

    proptest! {
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
