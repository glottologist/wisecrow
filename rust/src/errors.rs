use thiserror::Error;

#[derive(Error, Debug)]
pub enum WisecrowError {
    /// An unknown error occurred
    #[error("Unknown wisecrow error")]
    Unknown,
}
