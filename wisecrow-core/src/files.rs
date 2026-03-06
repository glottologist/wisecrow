use crate::errors::WisecrowError;
use crate::Langs;
use derive_more::Display;
use url::Url;

const ALL_CORPORA: [Corpus; 3] = [Corpus::OpenSubtitles, Corpus::CcMatrix, Corpus::Nllb];

#[derive(Debug, Display, Clone, Copy, PartialEq, Eq)]
pub enum Corpus {
    #[display("OpenSubtitles")]
    OpenSubtitles,
    #[display("CcMatrix")]
    CcMatrix,
    #[display("Nllb")]
    Nllb,
}

impl Corpus {
    const fn url_root(self) -> &'static str {
        match self {
            Self::OpenSubtitles => "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/",
            Self::CcMatrix => "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/",
            Self::Nllb => "https://object.pouta.csc.fi/OPUS-NLLB/v1/",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::OpenSubtitles => "OpenSubtitles",
            Self::CcMatrix => "CCMatrix",
            Self::Nllb => "NLLB",
        }
    }
}

impl TryFrom<&str> for Corpus {
    type Error = WisecrowError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "open_subtitles" => Ok(Self::OpenSubtitles),
            "cc_matrix" => Ok(Self::CcMatrix),
            "nllb" => Ok(Self::Nllb),
            other => Err(WisecrowError::InvalidInput(format!(
                "Unknown corpus: {other}. Valid: open_subtitles, cc_matrix, nllb"
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
        let xml_url = base.join(&format!("xml/{lo}-{hi}.xml.gz"))?;

        Ok(vec![
            LanguageFileInfo {
                corpus,
                target_location: tmx_url.into(),
                file_name: format!("{foreign}_{label}.tmx.gz"),
                compressed: Compression::GzCompressed,
            },
            LanguageFileInfo {
                corpus,
                target_location: xml_url.into(),
                file_name: format!("{foreign}_{label}.xml.gz"),
                compressed: Compression::GzCompressed,
            },
        ])
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

        let mut files = Vec::with_capacity(active_corpora.len() * 2);
        for &corpus in active_corpora {
            files.extend(Self::files_for_corpus(corpus, native, foreign)?);
        }

        Ok(Self { files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn test_langs() -> crate::Langs {
        crate::Langs::new("en", "es")
    }

    #[test]
    fn default_generates_all_corpora() {
        let files = LanguageFiles::new(&test_langs(), None).unwrap();
        assert_eq!(files.files.len(), 6);
    }

    #[rstest]
    #[case(Corpus::OpenSubtitles, 2)]
    #[case(Corpus::CcMatrix, 2)]
    #[case(Corpus::Nllb, 2)]
    fn single_corpus_filter(#[case] corpus: Corpus, #[case] expected: usize) {
        let files = LanguageFiles::new(&test_langs(), Some(&[corpus])).unwrap();
        assert_eq!(files.files.len(), expected);
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
    #[case("cc_matrix", true)]
    #[case("nllb", true)]
    #[case("invalid", false)]
    fn corpus_try_from(#[case] input: &str, #[case] is_ok: bool) {
        assert_eq!(Corpus::try_from(input).is_ok(), is_ok);
    }
}
