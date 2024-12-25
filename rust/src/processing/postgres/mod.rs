use super::LanguageFileProcessor;

pub struct PostgresLanguageFileProcessor {}

impl PostgresLanguageFileProcessor {}

impl LanguageFileProcessor for PostgresLanguageFileProcessor {
    async fn process_freq_language_file(
        &self,
        file: crate::files::LanguageFileInfo,
    ) -> Result<u64, crate::errors::WisecrowError> {
        todo!()
    }

    async fn process_translation_language_file(
        &self,
        file: crate::files::LanguageFileInfo,
    ) -> Result<u64, crate::errors::WisecrowError> {
        todo!()
    }
    async fn process_token_language_file(
        &self,
        file: crate::files::LanguageFileInfo,
    ) -> Result<u64, crate::errors::WisecrowError> {
        todo!()
    }
    async fn process_mono_language_file(
        &self,
        file: crate::files::LanguageFileInfo,
    ) -> Result<u64, crate::errors::WisecrowError> {
        todo!()
    }
}
