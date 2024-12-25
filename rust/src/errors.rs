use indicatif::style::TemplateError;
use thiserror::Error;
use url::ParseError;

#[derive(Error, Debug)]
pub enum WisecrowError {
    /// An unknown error occurred
    #[error("Unknown wisecrow error")]
    Unknown,
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
}
