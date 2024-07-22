use crate::errors::WisecrowError;
use crate::Langs;
use thiserror::Error;

enum LanguageFileType {
    Frequency,
    Translation,
}

pub struct LanguageFileInfo {
    lang_file_type: LanguageFileType,
    target_location: String,
    url_root: String,
    url_suffix: String,
}

pub struct LanguageFiles {
    files: Vec<LanguageFileInfo>,
}

impl LanguageFiles {
    fn generate_file_info(
        file_type: LanguageFileType,
        foreign: &str,
        source: &str,
        url_root: String,
        url_suffix: String,
    ) -> LanguageFileInfo {
        LanguageFileInfo {
            lang_file_type: file_type,
            target_location: format!("./download/{}/{}", foreign, source),
            url_root,
            url_suffix,
        }
    }
    pub fn new(langs: &Langs) -> Result<Self, WisecrowError> {
        let files = vec![
            Self::generate_file_info(
                LanguageFileType::Frequency,
                &langs.foreign.0,
                "CCMatrix",
                "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/freq/".to_owned(),
                ".freq.gz".to_owned(),
            ),
            /*  generate_file_info(
                            LanguageFileType::Frequency,
                            langs.foreign,
                            "NLLB",
                            "https://object.pouta.csc.fi/OPUS-NLLB/v1/freq/",
                            ".freq.gz",
                        ),
                        generate_file_info(
                            LanguageFileType::Frequency,
                            langs.foreign,
                            "ParaCrawl",
                            "https://object.pouta.csc.fi/OPUS-ParaCrawl/v9/freq/",
                            ".freq.gz",
                        ),
                        generate_file_info(
                            LanguageFileType::Frequency,
                            langs.foreign,
                            "OpenSubtitles",
                            "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/freq/",
                            ".freq.gz",
                        ),
                        generate_file_info(
                            LanguageFileType::Translation,
                            langs.foreign,
                            "CCMatrix",
                            "https://object.pouta.csc.fi/OPUS-CCMatrix/v1/tmx/",
                            ".tmx.gz",
                        ),
                        generate_file_info(
                            LanguageFileType::Translation,
                            langs.foreign,
                            "ParaCrawl",
                            "https://object.pouta.csc.fi/OPUS-ParaCrawl/v9/tmx/"
                            ".tmx.gz",
                        ),
                        generate_file_info(
                            LanguageFileType::Translation,
                            langs.foreign,
                            "NLLB",
            "https://object.pouta.csc.fi/OPUS-NLLB/v1/tmx/"
                            ".tmx.gz",
                        ),
                        generate_file_info(
                            LanguageFileType::Translation,
                            langs.foreign,
                            "OpenSubtitles",
                            "https://object.pouta.csc.fi/OPUS-OpenSubtitles/v2018/tmx/",
                            ".tmx.gz",
                        ),*/
        ];
        Ok(Self { files })
    }
}
