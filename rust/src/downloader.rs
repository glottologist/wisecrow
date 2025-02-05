use crate::errors::WisecrowError;
use crate::files::{Compression, LanguageFileInfo, LanguageFiles};
use crate::Langs;
use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use thiserror::Error;
use tracing::{error, info};
use url::Url;
use zip::read::ZipArchive;

pub struct Downloader {}
impl Downloader {
    pub fn new() -> Result<Self, WisecrowError> {
        Ok(Self {})
    }
    fn unzip(zip_path: &str, output_dir: &str) -> io::Result<()> {
        // Open the ZIP file
        let file = File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;

        // Iterate over the entries in the ZIP archive
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let outpath = Path::new(output_dir).join(file.name());

            // Handle directories and files
            if file.is_dir() {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                // Write the file to the output directory
                let mut outfile = File::create(&outpath)?;
                io::copy(&mut file, &mut outfile)?;
            }
        }

        Ok(())
    }
    fn decompress_gz(input_path: &str, output_path: &str) -> io::Result<()> {
        // Open the .gz file
        let input_file = File::open(input_path)?;
        let buffered_input = BufReader::new(input_file);

        // Create a GzDecoder to decompress the data
        let mut decoder = GzDecoder::new(buffered_input);

        // Create the output file
        let output_file = File::create(output_path)?;
        let mut buffered_output = BufWriter::new(output_file);

        // Decompress the data
        io::copy(&mut decoder, &mut buffered_output)?;

        std::fs::remove_file(input_path)
    }

    pub async fn download(self, file: LanguageFileInfo) -> Result<(), WisecrowError> {
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

        let _ = match &file.compressed {
            Compression::GzCompressed => {
                let decom_name = &file
                    .file_name
                    .strip_suffix(".gz")
                    .unwrap_or(&file.file_name);
                let _ = Self::decompress_gz(&file.file_name, &decom_name);
                *decom_name
            }
            Compression::ZipCompressed => {
                let decom_name = &file
                    .file_name
                    .strip_suffix(".zip")
                    .unwrap_or(&file.file_name);
                let _ = Self::unzip(&file.file_name, &decom_name);
                *decom_name
            }
            Compression::None => &file.file_name,
        };
        Ok(())
    }
}
