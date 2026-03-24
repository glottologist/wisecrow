use crate::errors::WisecrowError;
use crate::files::{Compression, LanguageFileInfo};
use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::time::Duration;
use url::Url;
use zip::read::ZipArchive;

const MAX_FILE_SIZE_OVERFLOW_MSG: &str = "max_file_size_mb overflow";
const CONNECT_TIMEOUT_SECS: u64 = 30;
const MAX_DECOMPRESSED_BYTES: u64 = 1_073_741_824; // 1 GiB

#[derive(Clone, Copy)]
pub struct DownloadConfig {
    pub max_retries: u32,
    pub timeout_seconds: u64,
    pub max_file_size_mb: u64,
    pub unpack: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            timeout_seconds: 300,
            max_file_size_mb: 102_400,
            unpack: true,
        }
    }
}

pub struct Downloader {
    config: DownloadConfig,
    client: Client,
}

impl Downloader {
    /// Creates a new `Downloader` with a shared HTTP client built from `config`.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::UnableToGetFile`] if the HTTP client cannot be
    /// constructed (e.g., TLS initialisation failure).
    pub fn new(config: DownloadConfig) -> Result<Self, WisecrowError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .read_timeout(Duration::from_secs(config.timeout_seconds))
            .build()?;
        Ok(Self { config, client })
    }

    fn unzip(zip_path: &str, output_dir: &str) -> io::Result<()> {
        let file = File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;
        let output_path = Path::new(output_dir);
        std::fs::create_dir_all(output_path)?;
        let canonical_root = output_path.canonicalize()?;

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_owned();

            if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
                tracing::warn!("Skipping suspicious path in ZIP: {name}");
                continue;
            }

            let outpath = canonical_root.join(&name);

            if !outpath.starts_with(&canonical_root) {
                tracing::warn!("Skipping path that escapes extraction root: {name}");
                continue;
            }

            if entry.is_dir() {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut outfile = File::create(&outpath)?;
                io::copy(&mut entry, &mut outfile)?;
            }
        }
        Ok(())
    }

    fn decompress_gz(input_path: &str, output_path: &str) -> io::Result<()> {
        let input_file = File::open(input_path)?;
        let decoder = GzDecoder::new(BufReader::new(input_file));
        let mut limited = decoder.take(MAX_DECOMPRESSED_BYTES.saturating_add(1));
        let output_file = File::create(output_path)?;
        let mut buffered_output = BufWriter::new(output_file);
        let written = io::copy(&mut limited, &mut buffered_output)?;
        if written > MAX_DECOMPRESSED_BYTES {
            drop(buffered_output);
            std::fs::remove_file(output_path).ok();
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Decompressed output exceeds size limit",
            ));
        }
        std::fs::remove_file(input_path)
    }

    /// Downloads `file`, retrying up to `config.max_retries` times with
    /// exponential back-off.
    ///
    /// # Errors
    ///
    /// Returns an error if all retry attempts fail, the server returns a
    /// non-success status, or the response body exceeds `max_file_size_mb`.
    pub async fn download(&self, file: &LanguageFileInfo) -> Result<String, WisecrowError> {
        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            match self.try_download(file).await {
                Ok(path) => return Ok(path),
                Err(e) => {
                    if attempt < self.config.max_retries {
                        let delay = Duration::from_secs(2u64.pow(attempt));
                        tracing::warn!(
                            "Download attempt {} failed: {e}. Retrying in {delay:?}",
                            attempt.saturating_add(1),
                        );
                        tokio::time::sleep(delay).await;
                    }
                    last_err = Some(e);
                }
            }
        }
        if let Err(e) = std::fs::remove_file(&file.file_name) {
            tracing::warn!(
                "Failed to clean up partial download {}: {e}",
                file.file_name
            );
        }
        Err(last_err.unwrap_or(WisecrowError::DownloadRetriesExhausted))
    }

    async fn try_download(&self, file: &LanguageFileInfo) -> Result<String, WisecrowError> {
        tracing::info!(
            "Downloading {} from {}",
            file.file_name,
            file.target_location
        );

        let url = Url::parse(&file.target_location)?;
        let response = self.client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(WisecrowError::InvalidInput(format!(
                "HTTP {} for {}",
                response.status(),
                file.target_location
            )));
        }

        let content_length = response.content_length();
        self.check_file_size(content_length)?;

        let progress_bar = ProgressBar::new(content_length.unwrap_or(0));
        let style = ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")?
            .progress_chars("#>-");
        progress_bar.set_style(style);

        self.stream_to_file(&file.file_name, response, &progress_bar)
            .await?;
        progress_bar.finish_with_message("Download completed!");

        self.decompress_if_needed(file)
    }

    fn check_file_size(&self, content_length: Option<u64>) -> Result<(), WisecrowError> {
        if let Some(size) = content_length {
            let max_bytes = self
                .config
                .max_file_size_mb
                .checked_mul(1024 * 1024)
                .ok_or_else(|| {
                    WisecrowError::InvalidInput(MAX_FILE_SIZE_OVERFLOW_MSG.to_owned())
                })?;
            if size > max_bytes {
                return Err(WisecrowError::InvalidInput(format!(
                    "File too large: {size} bytes (max: {max_bytes} bytes)"
                )));
            }
        }
        Ok(())
    }

    async fn stream_to_file(
        &self,
        path: &str,
        mut response: reqwest::Response,
        progress_bar: &ProgressBar,
    ) -> Result<(), WisecrowError> {
        let max_bytes = self
            .config
            .max_file_size_mb
            .checked_mul(1024 * 1024)
            .ok_or_else(|| WisecrowError::InvalidInput(MAX_FILE_SIZE_OVERFLOW_MSG.to_owned()))?;
        let mut fileio = BufWriter::new(File::create(path)?);
        let mut downloaded: u64 = 0;

        while let Some(chunk) = response.chunk().await? {
            fileio.write_all(&chunk)?;
            let chunk_len = u64::try_from(chunk.len()).map_err(|_| {
                WisecrowError::InvalidInput("Chunk size exceeds u64 range".to_string())
            })?;
            downloaded = downloaded.saturating_add(chunk_len);
            if downloaded > max_bytes {
                return Err(WisecrowError::InvalidInput(format!(
                    "Response body exceeds maximum size of {max_bytes} bytes"
                )));
            }
            progress_bar.set_position(downloaded);
        }
        Ok(())
    }

    fn decompress_if_needed(&self, file: &LanguageFileInfo) -> Result<String, WisecrowError> {
        let output_path = file.decompressed_name();
        if self.config.unpack {
            match file.compressed {
                Compression::GzCompressed => {
                    Self::decompress_gz(&file.file_name, &output_path)?;
                }
                Compression::ZipCompressed => {
                    Self::unzip(&file.file_name, &output_path)?;
                }
            }
        }
        Ok(output_path)
    }
}
