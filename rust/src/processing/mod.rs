use crate::{
    errors::WisecrowError,
    files::{LanguageFileInfo, LanguageFileType},
};

pub mod postgres;

pub trait LanguageFileProcessor {
    async fn process_freq_language_file(
        &self,
        file: LanguageFileInfo,
    ) -> Result<u64, WisecrowError>;
    async fn process_translation_language_file(
        &self,
        file: LanguageFileInfo,
    ) -> Result<u64, WisecrowError>;
    async fn process_mono_language_file(
        &self,
        file: LanguageFileInfo,
    ) -> Result<u64, WisecrowError>;
    async fn process_token_language_file(
        &self,
        file: LanguageFileInfo,
    ) -> Result<u64, WisecrowError>;

    async fn process_language_file(&self, file: LanguageFileInfo) -> Result<u64, WisecrowError> {
        use LanguageFileType::*;
        match file.lang_file_type {
            Frequency => self.process_freq_language_file(file).await,
            Translation => self.process_translation_language_file(file).await,
            Token => self.process_token_language_file(file).await,
            Mono => self.process_mono_language_file(file).await,
        }
    }
}
