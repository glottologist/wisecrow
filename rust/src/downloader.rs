use std::fs::File;

use crate::errors::WisecrowError;
use crate::files::LanguageFiles;
use crate::Langs;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use std::io::Read;
use std::io::Write;
use thiserror::Error;
use tracing::{error, info};
use url::Url;

pub struct Downloader {
    pub langs: Langs,
    pub language_files: LanguageFiles,
}
impl Downloader {
    pub fn new(langs: Langs) -> Result<Self, WisecrowError> {
        let language_files = LanguageFiles::new(&langs)?;
        Ok(Self {
            langs,
            language_files,
        })
    }

    pub async fn download(&self) -> Result<(), WisecrowError> {
        for file in self.language_files.files.iter() {
            info!(
                "Downloading language file - Type {}, Location {} , Root {}, Suffix {}",
                &file.lang_file_type, &file.target_location, &file.url_root, &file.url_suffix
            );
            let url = Url::parse(&file.target_location).map_err(WisecrowError::UnableToParseUrl)?;

            info!("Url {}", url);

            let client = Client::new();
            let response = client
                .get(url)
                .send()
                .await
                .map_err(WisecrowError::UnableToGetFile)?;
            // Check if the response is successful
            if !response.status().is_success() {
                error!("Failed to download file: {}", response.status());
            }

            let total_size = response.content_length().unwrap_or(0);

            let progress_bar = ProgressBar::new(total_size);
            let progress_bar_style=ProgressStyle::default_bar().template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})").map_err(WisecrowError::UnableToConstructProgressBarStyle)?.progress_chars("#>-");

            progress_bar.set_style(progress_bar_style);

            // Open the output file for writing
            let mut fileio =
                File::create(&file.file_name).map_err(WisecrowError::UnableToCreateFile)?;

            // Write the response body to the file in chunks
            let mut downloaded = 0;
            let mut body = response;

            while let Some(chunk) = body.chunk().await? {
                fileio.write_all(&chunk)?;
                downloaded += chunk.len() as u64;
                progress_bar.set_position(downloaded);
            }
            progress_bar.finish_with_message("Download completed!");
        }
        Ok(())
    }
}
