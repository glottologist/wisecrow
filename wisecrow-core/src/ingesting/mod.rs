pub mod parsing;
pub mod persisting;

use crate::{
    downloader::{DownloadConfig, Downloader},
    errors::WisecrowError,
    files::{IngestLanguages, LanguageFileInfo},
    Langs,
};
use parsing::{CorpusParser, TranslationPair};
use persisting::DatabasePersister;
use sqlx::PgPool;
use tokio::sync::mpsc;

const CHANNEL_BOUND: usize = 1000;

pub struct Ingester {
    pool: PgPool,
    config: DownloadConfig,
}

impl Ingester {
    #[must_use]
    pub const fn new(pool: PgPool, config: DownloadConfig) -> Self {
        Self { pool, config }
    }

    /// Downloads `file` without ingesting it.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP client cannot be built, the download fails
    /// after all retries, or the server returns a non-success status.
    pub async fn download_only(
        config: &DownloadConfig,
        file: &LanguageFileInfo,
    ) -> Result<String, WisecrowError> {
        let downloader = Downloader::new(*config)?;
        downloader.download(file).await
    }

    /// Downloads `file` to the given directory without ingesting it.
    ///
    /// # Errors
    ///
    /// Returns an error if the download fails or the directory cannot be used.
    pub async fn download_to_dir(
        config: &DownloadConfig,
        file: &LanguageFileInfo,
        output_dir: &std::path::Path,
    ) -> Result<String, WisecrowError> {
        let downloader = Downloader::new(*config)?;
        downloader.download_to(file, Some(output_dir)).await
    }

    /// Downloads and ingests `file` using the language mapping it was
    /// selected with. `native_lang` and `foreign_lang` must be the canonical
    /// codes of that mapping.
    ///
    /// # Errors
    ///
    /// Returns an error if the languages disagree with the descriptor, the
    /// download fails, or any parse/database step fails.
    pub async fn download_and_ingest(
        &self,
        file: &LanguageFileInfo,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<(), WisecrowError> {
        let languages = &file.languages;
        if languages.native().canonical() != native_lang
            || languages.foreign().canonical() != foreign_lang
        {
            return Err(WisecrowError::InvalidInput(format!(
                "Requested {native_lang}/{foreign_lang} but {} was selected for {}/{}",
                file.file_name,
                languages.native().canonical(),
                languages.foreign().canonical(),
            )));
        }
        let path = Self::download_only(&self.config, file).await?;
        self.ingest_from_file_with_languages(&path, &file.file_name, languages)
            .await
    }

    /// Ingests a local file by parsing it and persisting translations to the
    /// database. `label` names the source in the completion log; for a
    /// downloaded corpus that is the archive name, for a supplied file its
    /// path.
    ///
    /// The file extension determines the parser: `.tmx` for TMX translation
    /// memory files, anything else for OPUS XML alignment format.
    ///
    /// # Errors
    ///
    /// Returns an error if the language pair is unsupported, language setup,
    /// parsing or persistence fails, or no pair is accepted.
    pub async fn ingest_from_file(
        &self,
        path: &str,
        label: &str,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<(), WisecrowError> {
        let languages = IngestLanguages::identity(native_lang, foreign_lang)?;
        self.ingest_from_file_with_languages(path, label, &languages)
            .await
    }

    /// Ingests a local file whose TMX tags may differ from the canonical
    /// codes the pairs are stored under. The XML alignment format has no
    /// tag mapping and is parsed with canonical codes.
    ///
    /// The parser and persister run as two futures scoped to this call, so
    /// cancelling it drops both and no detached task outlives the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if language setup, parsing or persistence fails, or
    /// no pair is accepted: an import that stores nothing is a failure, not a
    /// success with a zero in the log.
    pub async fn ingest_from_file_with_languages(
        &self,
        path: &str,
        label: &str,
        languages: &IngestLanguages,
    ) -> Result<(), WisecrowError> {
        let (sender, receiver) = mpsc::channel::<TranslationPair>(CHANNEL_BOUND);
        let persister = DatabasePersister::new(self.pool.clone()); // clone: PgPool is Arc-based
        let native = languages.native().canonical();
        let foreign = languages.foreign().canonical();

        let from_id = persister.ensure_language(native, native).await?;
        let to_id = persister.ensure_language(foreign, foreign).await?;

        let parse = async {
            let result = if std::path::Path::new(path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("tmx"))
            {
                CorpusParser::parse_tmx_file_with_languages(path, languages, &sender).await
            } else {
                CorpusParser::parse_xml_alignment_file(path, native, foreign, &sender).await
            };
            // Closing the channel lets the persister flush and finish.
            drop(sender);
            result
        };

        let (parsed, persisted) = tokio::join!(parse, persister.consume(receiver, from_id, to_id));
        let count = parsed?;
        persisted?;
        if count == 0 {
            return Err(WisecrowError::InvalidInput(format!(
                "No accepted pairs in {label} for {native}/{} to {foreign}/{}",
                languages.native().corpus(),
                languages.foreign().corpus(),
            )));
        }
        tracing::info!("Ingested {count} items from {label}");

        Ok(())
    }

    /// Spawns a download-and-ingest job. The job's outcome is the task's
    /// result so the caller can count failures rather than read logs.
    #[must_use]
    pub fn spawn(
        pool: PgPool,
        config: DownloadConfig,
        langs: &Langs,
        file: LanguageFileInfo,
    ) -> tokio::task::JoinHandle<Result<(), WisecrowError>> {
        let native = langs.native_code().to_owned();
        let foreign = langs.foreign_code().to_owned();
        tokio::spawn(async move {
            let ingester = Self::new(pool, config);
            ingester.download_and_ingest(&file, &native, &foreign).await
        })
    }
}
