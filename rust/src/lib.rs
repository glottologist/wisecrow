pub mod cli;
pub mod downloader;
pub mod errors;
pub mod files;

struct Native(String);
struct Foreign(String);

pub struct Langs {
    native: Native,
    foreign: Foreign,
}
