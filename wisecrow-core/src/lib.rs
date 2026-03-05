pub mod cli;
pub mod config;
pub mod downloader;
pub mod dto_convert;
pub mod errors;
pub mod files;
pub mod frequency;
pub mod grammar;
pub mod ingesting;
pub mod media;
pub mod srs;
pub mod tui;
pub mod vocabulary;

use derive_more::Display;

#[derive(Debug, Display, Clone)]
pub struct Native(String);

#[derive(Debug, Display, Clone)]
pub struct Foreign(String);

#[derive(Debug, Display, Clone)]
#[display("Native {} Foreign {}", native, foreign)]
pub struct Langs {
    native: Native,
    foreign: Foreign,
}

impl Native {
    #[must_use]
    pub fn code(&self) -> &str {
        &self.0
    }
}

impl Foreign {
    #[must_use]
    pub fn code(&self) -> &str {
        &self.0
    }
}

impl Langs {
    #[must_use]
    pub fn new(native: impl Into<String>, foreign: impl Into<String>) -> Self {
        Self {
            native: Native(native.into()),
            foreign: Foreign(foreign.into()),
        }
    }

    #[must_use]
    pub fn native_code(&self) -> &str {
        self.native.code()
    }

    #[must_use]
    pub fn foreign_code(&self) -> &str {
        self.foreign.code()
    }
}
