use indicatif::style::TemplateError;
use thiserror::Error;
use url::ParseError;

#[derive(Error, Debug)]
pub enum WisecrowError {
    #[error("All download retries exhausted")]
    DownloadRetriesExhausted,
    #[error("Unable to parse url: {0}")]
    UnableToParseUrl(#[from] ParseError),
    #[error("Unable to get url: {0}")]
    UnableToGetFile(#[from] reqwest::Error),
    #[error("Unable to create file: {0}")]
    UnableToCreateFile(#[from] std::io::Error),
    #[error("Unable to construct progress bar style: {0}")]
    UnableToConstructProgressBarStyle(#[from] TemplateError),
    #[error("Persistence migration error: {0}")]
    PersistenceMigrationError(#[from] sqlx::migrate::MigrateError),
    #[error("Persistence connection error: {0}")]
    PersistenceConnectionError(#[from] sqlx::Error),
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Media error: {0}")]
    MediaError(String),
    #[error("PDF extraction error: {0}")]
    PdfExtractionError(String),
    #[error("Quiz generation error: {0}")]
    QuizGenerationError(String),
    #[error("LLM error: {0}")]
    LlmError(String),
    #[error("Sync error: {0}")]
    SyncError(String),
}
