pub mod parsing;
pub mod persisting;

use crate::{
    downloader::{DownloadConfig, Downloader},
    errors::WisecrowError,
    files::LanguageFileInfo,
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

    /// Downloads and ingests `file`.
    ///
    /// # Errors
    ///
    /// Returns an error if the download or any parse/database step fails.
    pub async fn download_and_ingest(
        &self,
        file: &LanguageFileInfo,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<(), WisecrowError> {
        let path = Self::download_only(&self.config, file).await?;
        self.ingest_from_file(&path, file, native_lang, foreign_lang)
            .await
    }

    /// Ingests a local file by parsing it and persisting translations to the
    /// database.
    ///
    /// The file extension determines the parser: `.tmx` for TMX translation
    /// memory files, anything else for OPUS XML alignment format.
    ///
    /// # Errors
    ///
    /// Returns an error if language setup, parsing, or persistence fails.
    pub async fn ingest_from_file(
        &self,
        path: &str,
        file: &LanguageFileInfo,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<(), WisecrowError> {
        let (sender, receiver) = mpsc::channel::<TranslationPair>(CHANNEL_BOUND);
        let persister = DatabasePersister::new(self.pool.clone()); // clone: PgPool is Arc-based

        let from_id = persister.ensure_language(native_lang, native_lang).await?;
        let to_id = persister
            .ensure_language(foreign_lang, foreign_lang)
            .await?;

        let path_owned = path.to_owned();
        let native = native_lang.to_owned();
        let foreign = foreign_lang.to_owned();
        let file_name = file.file_name.clone(); // clone: need owned for logging after await

        let parse_handle = tokio::spawn(async move {
            if std::path::Path::new(&path_owned)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("tmx"))
            {
                CorpusParser::parse_tmx_file(&path_owned, &native, &foreign, &sender).await
            } else {
                CorpusParser::parse_xml_alignment_file(&path_owned, &native, &foreign, &sender)
                    .await
            }
        });

        let persist_handle =
            tokio::spawn(async move { persister.consume(receiver, from_id, to_id).await });

        let (parse_result, persist_result) = tokio::try_join!(parse_handle, persist_handle)
            .map_err(|e| WisecrowError::InvalidInput(format!("Task join error: {e}")))?;

        let count = parse_result?;
        persist_result?;
        tracing::info!("Ingested {count} items from {file_name}");

        Ok(())
    }

    #[must_use]
    pub fn spawn(
        pool: PgPool,
        config: DownloadConfig,
        langs: &Langs,
        file: LanguageFileInfo,
    ) -> tokio::task::JoinHandle<()> {
        let native = langs.native_code().to_owned();
        let foreign = langs.foreign_code().to_owned();
        tokio::spawn(async move {
            let ingester = Self::new(pool, config);
            if let Err(e) = ingester.download_and_ingest(&file, &native, &foreign).await {
                tracing::error!("Ingestion failed for {}: {e:?}", file.file_name);
            }
        })
    }
}
