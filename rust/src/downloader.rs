use crate::errors::WisecrowError;
use crate::files::LanguageFiles;
use crate::Langs;
use thiserror::Error;

struct Downloader {
    langs: Langs,
    files: LanguageFiles,
}
impl Downloader {
    fn new(langs: Langs) -> Result<Self, WisecrowError> {
        let files = LanguageFiles::new(&langs)?;
        Ok(Self { langs, files })
    }

    fn download() -> Result<(), WisecrowError> {
        Ok(())
    }
}
