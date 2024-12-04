use crate::errors::WisecrowError;
use crate::Langs;
use derive_more::Display;
use thiserror::Error;

#[derive(Debug, Display)]
pub enum LanguageFileType {
    #[display("Frequency")]
    Frequency,
    #[display("Translation")]
    Translation,
}

#[derive(Debug, Display)]
#[display(
    "{} file at {} (URL: {}{}) (FILE: {})",
    lang_file_type,
    target_location,
    url_root,
    url_suffix,
    file_name
)]
pub struct LanguageFileInfo {
    pub lang_file_type: LanguageFileType,
    pub target_location: String,
    pub file_name: String,
    pub url_root: String,
    pub url_suffix: String,
}

#[derive(Debug)]
pub struct LanguageFiles {
    pub files: Vec<LanguageFileInfo>,
}

impl LanguageFiles {
    fn generate_file_info(
        file_type: LanguageFileType,
        native: &str,
        foreign: &str,
        source: &str,
        url_root: String,
        url_suffix: String,
    ) -> LanguageFileInfo {
        let url = match file_type {
            LanguageFileType::Frequency => format!("{}{}{}", url_root, foreign, url_suffix),
            LanguageFileType::Translation => {
                format!("{}{}-{}{}", url_root, native, foreign, url_suffix)
            }
        };
        let file_name = format!("{}{}", foreign, url_suffix);
        LanguageFileInfo {
            lang_file_type: file_type,
            target_location: url,
            file_name,
            url_root,
            url_suffix,
        }
    }
    //https://object.pouta.csc.fi/OPUS-CCMatrix/v1/freq/fr.freq.gz
    //https://object.pouta.csc.fi/OPUS-CCMatrix/v1/tmx/en-fr.tmx.gz
    pub fn new(langs: &Langs) -> Result<Self, WisecrowError> {
        let files = vec![
            Self::generate_file_info(
                LanguageFileType::Frequency,
                &langs.native.0,
                &langs.foreign.0,
                "CCMatrix",
                "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/freq/".to_owned(),
                ".freq.gz".to_owned(),
            ),
            Self::generate_file_info(
                LanguageFileType::Frequency,
                &langs.native.0,
                &langs.foreign.0,
                "NLLB",
                "https://object.pouta.csc.fi/OPUS-NLLB/v1/freq/".to_owned(),
                ".freq.gz".to_owned(),
            ),
            Self::generate_file_info(
                LanguageFileType::Frequency,
                &langs.native.0,
                &langs.foreign.0,
                "ParaCrawl",
                "https://object.pouta.csc.fi/OPUS-ParaCrawl/v9/freq/".to_owned(),
                ".freq.gz".to_owned(),
            ),
            Self::generate_file_info(
                LanguageFileType::Frequency,
                &langs.native.0,
                &langs.foreign.0,
                "OpenSubtitles",
                "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/freq/".to_owned(),
                ".freq.gz".to_owned(),
            ),
            Self::generate_file_info(
                LanguageFileType::Translation,
                &langs.native.0,
                &langs.foreign.0,
                "CCMatrix",
                "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/tmx/".to_owned(),
                ".tmx.gz".to_owned(),
            ),
            Self::generate_file_info(
                LanguageFileType::Translation,
                &langs.native.0,
                &langs.foreign.0,
                "ParaCrawl",
                "https://object.pouta.csc.fi/OPUS-ParaCrawl/v9/tmx/".to_owned(),
                ".tmx.gz".to_owned(),
            ),
            Self::generate_file_info(
                LanguageFileType::Translation,
                &langs.native.0,
                &langs.foreign.0,
                "NLLB",
                "https://object.pouta.csc.fi/OPUS-NLLB/v1/tmx/".to_owned(),
                ".tmx.gz".to_owned(),
            ),
            Self::generate_file_info(
                LanguageFileType::Translation,
                &langs.native.0,
                &langs.foreign.0,
                "OpenSubtitles",
                "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/tmx/".to_owned(),
                ".tmx.gz".to_owned(),
            ),
        ];
        Ok(Self { files })
    }
}
