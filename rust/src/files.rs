use crate::errors::WisecrowError;
use crate::Langs;
use derive_more::Display;
use thiserror::Error;

#[derive(Debug, Display)]
pub enum LanguageFileType {
    #[display("Frequency")]
    Frequency,
    #[display("Mono")]
    Mono,
    #[display("Token")]
    Token,
    #[display("Translation")]
    Translation,
}

#[derive(Debug, Display)]
pub enum Compression {
    #[display("None")]
    None,
    #[display("GzCompressed")]
    GzCompressed,
    #[display("ZipCompressed")]
    ZipCompressed,
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
    pub compressed: Compression,
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
        compressed: Compression,
    ) -> LanguageFileInfo {
        let url = match file_type {
            LanguageFileType::Frequency => format!("{}{}{}", url_root, foreign, url_suffix),
            LanguageFileType::Token => format!("{}{}{}", url_root, foreign, url_suffix),
            LanguageFileType::Mono => format!("{}{}{}", url_root, foreign, url_suffix),
            LanguageFileType::Translation => {
                format!("{}{}-{}{}", url_root, native, foreign, url_suffix)
            }
        };
        let file_name = format!("{}_{}{}", foreign, source, url_suffix);
        LanguageFileInfo {
            lang_file_type: file_type,
            target_location: url,
            file_name,
            url_root,
            url_suffix,
            compressed,
        }
    }
    //https://object.pouta.csc.fi/OPUS-CCMatrix/v1/freq/es.freq.gz
    //https://object.pouta.csc.fi/OPUS-CCMatrix/v1/mono/es.tok.gz
    //https://object.pouta.csc.fi/OPUS-CCMatrix/v1/xml/es.zip
    //https://object.pouta.csc.fi/OPUS-CCMatrix/v1/xml/en-es.xml.gz
    //https://object.pouta.csc.fi/OPUS-CCMatrix/v1/tmx/en-es.tmx.gz
    pub fn new(langs: &Langs) -> Result<Self, WisecrowError> {
        let files = vec![
            /*Self::generate_file_info(
                LanguageFileType::Frequency,
                &langs.native.0,
                &langs.foreign.0,
                "CCMatrix",
                "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/freq/".to_owned(),
                ".freq.gz".to_owned(),
                Compression::GzCompressed,
            ),
            Self::generate_file_info(
                LanguageFileType::Token,
                &langs.native.0,
                &langs.foreign.0,
                "CCMatrix",
                "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/mono/".to_owned(),
                ".tok.gz".to_owned(),
                Compression::GzCompressed,
            ),*/
            Self::generate_file_info(
                LanguageFileType::Mono,
                &langs.native.0,
                &langs.foreign.0,
                "CCMatrix",
                "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/xml/".to_owned(),
                ".zip".to_owned(),
                Compression::ZipCompressed,
            ),
            Self::generate_file_info(
                LanguageFileType::Translation,
                &langs.native.0,
                &langs.foreign.0,
                "CCMatrix",
                "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/tmx/".to_owned(),
                ".tmx.gz".to_owned(),
                Compression::GzCompressed,
            ),
            Self::generate_file_info(
                LanguageFileType::Translation,
                &langs.native.0,
                &langs.foreign.0,
                "CCMatrix",
                "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/xml/".to_owned(),
                ".xml.gz".to_owned(),
                Compression::GzCompressed,
            ),
        ];
        Ok(Self { files })
    }
}
