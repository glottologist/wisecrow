pub mod cli;
pub mod config;
pub mod downloader;
pub mod errors;
pub mod files;
pub mod processing;

use derive_more::Display;

#[derive(Debug, Display)]
pub struct Native(String);

#[derive(Debug, Display)]
pub struct Foreign(String);

#[derive(Debug, Display)]
#[display("Native {} Foreign {}", native, foreign)]
pub struct Langs {
    native: Native,
    foreign: Foreign,
}

impl Langs {
    pub fn new(native: String, foreign: String) -> Langs {
        Self {
            native: Native(native),
            foreign: Foreign(foreign),
        }
    }
}
