use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionStoreError {
    #[error("secure session storage is unavailable")]
    Unavailable,
    #[error("secure session could not be read")]
    Read,
    #[error("secure session could not be written")]
    Write,
    #[error("secure session could not be deleted")]
    Delete,
}

pub trait SessionStore: Send + Sync {
    fn load(&self) -> Result<Option<String>, SessionStoreError>;

    fn save(&self, token: &str) -> Result<(), SessionStoreError>;

    fn delete(&self) -> Result<(), SessionStoreError>;
}
