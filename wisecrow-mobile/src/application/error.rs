use thiserror::Error;

/// Typed failure returned by mobile application and platform boundaries.
#[derive(Debug, Error)]
pub enum MobileError {
    #[error("local storage operation failed")]
    Storage(#[source] sqlx::Error),
    #[error("local storage migration failed")]
    Migration(#[source] sqlx::migrate::MigrateError),
    #[error("local data encoding failed")]
    Serialization(#[source] serde_json::Error),
    #[error("local file operation failed")]
    FileSystem(#[source] std::io::Error),
    #[error("local scheduling failed")]
    Learning(#[source] wisecrow_learning::LearningError),
    #[error("credentials are unavailable")]
    Credentials,
    #[error("authentication is required")]
    Authentication,
    #[error("the registered device is revoked")]
    DeviceRevoked,
    #[error("the server does not support mobile protocol {required}")]
    UnsupportedProtocol { required: u16, actual: u16 },
    #[error("the request is invalid: {0}")]
    InvalidInput(String),
    #[error("local and remote state conflict: {0}")]
    Conflict(String),
    #[error("the operation can be retried")]
    Retryable,
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("the remote operation failed permanently")]
    Permanent,
    #[error("the platform does not support this operation")]
    Unsupported,
}

impl From<sqlx::Error> for MobileError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(error)
    }
}

impl From<sqlx::migrate::MigrateError> for MobileError {
    fn from(error: sqlx::migrate::MigrateError) -> Self {
        Self::Migration(error)
    }
}

impl From<serde_json::Error> for MobileError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<std::io::Error> for MobileError {
    fn from(error: std::io::Error) -> Self {
        Self::FileSystem(error)
    }
}

impl From<wisecrow_learning::LearningError> for MobileError {
    fn from(error: wisecrow_learning::LearningError) -> Self {
        Self::Learning(error)
    }
}
